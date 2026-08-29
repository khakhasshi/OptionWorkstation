use std::{
    env,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, anyhow};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, RwLock, broadcast},
};

use crate::{
    analytics::{ChainBuild, build_chain, build_surface},
    models::{
        Bar, ConnectionStatus, LiveFeedInfo, LiveSessionRequest, LiveSnapshot, RawOptionQuote,
        ThetaCredentialRequest,
    },
};

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_STALE_AFTER_MS: u64 = 15_000;
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_STALE_BRIDGE_RESPONSES: usize = 16;

struct ThetaBridge {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    sequence: u64,
}

#[derive(Debug, Deserialize)]
struct BridgeResponse {
    id: u64,
    ok: bool,
    result: Option<Value>,
    error: Option<String>,
}

impl ThetaBridge {
    async fn spawn() -> anyhow::Result<Self> {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("Rust crate has a project root")
            .to_path_buf();
        let bundled_python = project_root.join(".venv-thetadata/bin/python");
        let python = env::var("OPTION_WORKSTATION_THETADATA_PYTHON").unwrap_or_else(|_| {
            if bundled_python.is_file() {
                bundled_python.display().to_string()
            } else {
                "python3".into()
            }
        });
        let script = env::var("OPTION_WORKSTATION_THETADATA_ADAPTER")
            .map(PathBuf::from)
            .unwrap_or_else(|_| project_root.join("scripts/thetadata_live_adapter.py"));
        anyhow::ensure!(
            script.is_file(),
            "ThetaData adapter not found: {}",
            script.display()
        );
        let mut child = Command::new(&python)
            .arg("-u")
            .arg(&script)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "start ThetaData adapter with {python}; install requirements-thetadata.txt or set OPTION_WORKSTATION_THETADATA_PYTHON"
                )
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("ThetaData adapter stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("ThetaData adapter stdout unavailable"))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            sequence: 0,
        })
    }

    async fn request(&mut self, mut payload: Value) -> anyhow::Result<Value> {
        self.sequence += 1;
        let request_id = self.sequence;
        payload["id"] = json!(request_id);
        let mut encoded = serde_json::to_vec(&payload)?;
        encoded.push(b'\n');
        self.stdin
            .write_all(&encoded)
            .await
            .context("write ThetaData adapter request")?;
        self.stdin.flush().await?;
        for _ in 0..=MAX_STALE_BRIDGE_RESPONSES {
            let mut line = String::new();
            let bytes = tokio::time::timeout(BRIDGE_TIMEOUT, self.stdout.read_line(&mut line))
                .await
                .context("ThetaData adapter request timed out")??;
            anyhow::ensure!(bytes > 0, "ThetaData adapter exited unexpectedly");
            let response: BridgeResponse =
                serde_json::from_str(&line).context("decode ThetaData adapter response")?;
            if !bridge_response_matches(response.id, request_id)? {
                tracing::debug!(
                    stale_response_id = response.id,
                    request_id,
                    "discarding response left by a cancelled ThetaData request"
                );
                continue;
            }
            if !response.ok {
                return Err(anyhow!(
                    "{}",
                    response
                        .error
                        .unwrap_or_else(|| "ThetaData adapter request failed".into())
                ));
            }
            return response
                .result
                .ok_or_else(|| anyhow!("ThetaData adapter returned no result"));
        }
        Err(anyhow!(
            "ThetaData adapter produced too many stale responses after cancelled requests"
        ))
    }

    async fn stop(mut self) {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            self.request(json!({"op": "disconnect"})),
        )
        .await;
        let _ = self.child.kill().await;
    }
}

fn bridge_response_matches(response_id: u64, request_id: u64) -> anyhow::Result<bool> {
    if response_id < request_id {
        return Ok(false);
    }
    anyhow::ensure!(
        response_id == request_id,
        "ThetaData adapter response sequence moved ahead unexpectedly"
    );
    Ok(true)
}

#[derive(Debug, Clone, Deserialize)]
struct ThetaContract {
    symbol: String,
    expiration: String,
    strike: f64,
    right: String,
    bid: f64,
    ask: f64,
    bid_size: i64,
    ask_size: i64,
    last: Option<f64>,
    volume: i64,
    open_interest: Option<i64>,
    timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThetaBar {
    time: String,
    timestamp: Option<String>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
    vwap: f64,
}

impl ThetaBar {
    fn into_bar(self) -> Option<Bar> {
        let timestamp = self.timestamp?;
        Some(Bar {
            time: if self.time.is_empty() {
                parse_timestamp(&timestamp)?
                    .with_timezone(&chrono_tz::America::New_York)
                    .format("%H:%M")
                    .to_string()
            } else {
                self.time
            },
            timestamp,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
            vwap: self.vwap,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ThetaSnapshotPayload {
    sdk_version: String,
    symbol: String,
    spot: f64,
    stock_timestamp: Option<String>,
    stock_bar: ThetaBar,
    #[serde(default)]
    bars: Vec<ThetaBar>,
    selected_expiration: String,
    expirations: Vec<String>,
    contracts: Vec<ThetaContract>,
    contract_limit: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct ThetaClose {
    date: String,
    close: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct ThetaClosesPayload {
    closes: Vec<ThetaClose>,
}

#[derive(Default)]
struct ThetaState {
    active: Option<LiveSessionRequest>,
    latest: Option<LiveSnapshot>,
    status: ConnectionStatus,
}

pub struct ThetaLiveManager {
    state: RwLock<ThetaState>,
    bridge: Mutex<Option<ThetaBridge>>,
    session_setup: Mutex<()>,
    events: broadcast::Sender<u64>,
    sequence: AtomicU64,
    refresh_started: AtomicBool,
    risk_free_rate: f64,
    poll_interval: Duration,
    stale_after_ms: u64,
}

impl ThetaLiveManager {
    pub fn new(risk_free_rate: f64) -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        let poll_interval = env::var("OPTION_WORKSTATION_THETADATA_POLL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(|value| Duration::from_secs(value.clamp(2, 60)))
            .unwrap_or(DEFAULT_POLL_INTERVAL);
        let stale_after_ms = env::var("OPTION_WORKSTATION_THETADATA_STALE_AFTER_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(|value| value.clamp(5_000, 120_000))
            .unwrap_or(DEFAULT_STALE_AFTER_MS);
        let mut state = ThetaState::default();
        state.status.provider = "thetadata".into();
        Arc::new(Self {
            state: RwLock::new(state),
            bridge: Mutex::new(None),
            session_setup: Mutex::new(()),
            events,
            sequence: AtomicU64::new(0),
            refresh_started: AtomicBool::new(false),
            risk_free_rate,
            poll_interval,
            stale_after_ms,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<u64> {
        self.events.subscribe()
    }

    fn notify(&self) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.events.send(sequence);
    }

    pub async fn status(&self) -> ConnectionStatus {
        self.state.read().await.status.clone()
    }

    pub async fn connect(
        self: &Arc<Self>,
        credentials: ThetaCredentialRequest,
    ) -> anyhow::Result<ConnectionStatus> {
        credentials.validate().map_err(|message| anyhow!(message))?;
        let _guard = self.session_setup.lock().await;
        {
            let mut state = self.state.write().await;
            state.status.state = "connecting".into();
            state.status.error = None;
        }
        let mut bridge = ThetaBridge::spawn().await?;
        let result = bridge
            .request(json!({
                "op": "connect",
                "email": credentials.email.trim(),
                "password": credentials.password,
            }))
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                bridge.stop().await;
                let mut state = self.state.write().await;
                state.status.state = "error".into();
                state.status.error = Some(error.to_string());
                return Err(error);
            }
        };
        let packages = result["packages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.as_str().map(String::from))
            .collect();
        let auth_method = result["auth_method"]
            .as_str()
            .unwrap_or(if credentials.uses_inline_credentials() {
                "credentials"
            } else {
                "environment"
            })
            .to_string();
        if let Some(previous) = self.bridge.lock().await.replace(bridge) {
            previous.stop().await;
        }
        let mut state = self.state.write().await;
        state.active = None;
        state.latest = None;
        state.status = ConnectionStatus {
            provider: "thetadata".into(),
            connected: true,
            state: "ready".into(),
            auth_method,
            account_hint: Some("ThetaData".into()),
            quote_level: Some("Realtime snapshots".into()),
            packages,
            switch_state: "idle".into(),
            ..ConnectionStatus::default()
        };
        Ok(state.status.clone())
    }

    pub async fn disconnect(&self) -> ConnectionStatus {
        let _guard = self.session_setup.lock().await;
        if let Some(bridge) = self.bridge.lock().await.take() {
            bridge.stop().await;
        }
        let mut state = self.state.write().await;
        *state = ThetaState::default();
        state.status.provider = "thetadata".into();
        self.notify();
        state.status.clone()
    }

    pub async fn setup_session(
        &self,
        mut request: LiveSessionRequest,
    ) -> anyhow::Result<LiveSnapshot> {
        validate_request(&request)?;
        request.provider = "thetadata".into();
        let _guard = self.session_setup.lock().await;
        anyhow::ensure!(
            self.state.read().await.status.connected,
            "请先连接 ThetaData 实时数据源"
        );
        {
            let mut state = self.state.write().await;
            state.status.switch_state = "switching".into();
            state.status.error = None;
        }
        let previous_bars = self
            .state
            .read()
            .await
            .latest
            .as_ref()
            .map(|snapshot| snapshot.bars.clone())
            .unwrap_or_default();
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let result = {
            let mut bridge = self.bridge.lock().await;
            let bridge = bridge
                .as_mut()
                .ok_or_else(|| anyhow!("ThetaData adapter is not running"))?;
            fetch_snapshot(
                bridge,
                &request,
                true,
                previous_bars,
                self.risk_free_rate,
                self.stale_after_ms,
                sequence,
            )
            .await
        };
        match result {
            Ok(snapshot) => {
                self.store_snapshot(request, snapshot.clone()).await;
                let _ = self.events.send(sequence);
                Ok(snapshot)
            }
            Err(error) => {
                let mut state = self.state.write().await;
                state.status.switch_state = "error".into();
                state.status.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub async fn snapshot(&self) -> anyhow::Result<LiveSnapshot> {
        self.state
            .read()
            .await
            .latest
            .clone()
            .ok_or_else(|| anyhow!("尚未建立 ThetaData 实时期权会话"))
    }

    pub async fn daily_closes(&self, count: usize) -> anyhow::Result<Vec<(String, f64)>> {
        let symbol = self
            .state
            .read()
            .await
            .active
            .as_ref()
            .map(|request| request.symbol.clone())
            .ok_or_else(|| anyhow!("尚未建立 ThetaData 实时期权会话"))?;
        let value = {
            let mut bridge = self.bridge.lock().await;
            bridge
                .as_mut()
                .ok_or_else(|| anyhow!("ThetaData adapter is not running"))?
                .request(json!({
                    "op": "daily_closes",
                    "symbol": symbol,
                    "count": count.clamp(21, 120),
                }))
                .await?
        };
        let payload: ThetaClosesPayload = serde_json::from_value(value)?;
        Ok(payload
            .closes
            .into_iter()
            .filter(|item| item.close.is_finite() && item.close > 0.0)
            .map(|item| (item.date, item.close))
            .collect())
    }

    pub fn start_refresh_loop(self: &Arc<Self>) {
        if self.refresh_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(manager.poll_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(error) = manager.refresh_snapshot().await {
                    let mut state = manager.state.write().await;
                    if state.status.connected && state.active.is_some() {
                        state.status.state = "degraded".into();
                        state.status.error = Some(error.to_string());
                    }
                }
            }
        });
    }

    async fn refresh_snapshot(&self) -> anyhow::Result<()> {
        let _guard = self.session_setup.lock().await;
        let request = match self.state.read().await.active.clone() {
            Some(request) => request,
            None => return Ok(()),
        };
        let previous_bars = self
            .state
            .read()
            .await
            .latest
            .as_ref()
            .map(|snapshot| snapshot.bars.clone())
            .unwrap_or_default();
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let snapshot = {
            let mut bridge = self.bridge.lock().await;
            let bridge = bridge
                .as_mut()
                .ok_or_else(|| anyhow!("ThetaData adapter is not running"))?;
            fetch_snapshot(
                bridge,
                &request,
                false,
                previous_bars,
                self.risk_free_rate,
                self.stale_after_ms,
                sequence,
            )
            .await?
        };
        self.store_snapshot(request, snapshot).await;
        let _ = self.events.send(sequence);
        Ok(())
    }

    async fn store_snapshot(&self, request: LiveSessionRequest, snapshot: LiveSnapshot) {
        let mut state = self.state.write().await;
        state.status.connected = true;
        state.status.state = "polling".into();
        state.status.switch_state = "ready".into();
        state.status.active_symbol = Some(snapshot.feed.symbol.clone());
        state.status.subscribed_contracts = snapshot.feed.subscribed_contracts;
        state.status.last_event_at = Some(Utc::now().to_rfc3339());
        state.status.last_snapshot_at = Some(snapshot.feed.as_of.clone());
        state.status.last_snapshot_sequence = Some(snapshot.sequence);
        state.status.latency_ms = Some(snapshot.feed.latency_ms);
        state.status.stale_after_ms = snapshot.feed.stale_after_ms;
        state.status.error = None;
        state.active = Some(request);
        state.latest = Some(snapshot);
    }
}

async fn fetch_snapshot(
    bridge: &mut ThetaBridge,
    request: &LiveSessionRequest,
    include_history: bool,
    previous_bars: Vec<Bar>,
    risk_free_rate: f64,
    stale_after_ms: u64,
    sequence: u64,
) -> anyhow::Result<LiveSnapshot> {
    let value = bridge
        .request(json!({
            "op": "snapshot",
            "symbol": request.symbol,
            "expiration": request.expiration,
            "max_contracts": request.max_contracts.clamp(20, 1_000),
            "surface_expiries": request.surface_expiries.clamp(2, 6),
            "moneyness_window": request.moneyness_window.clamp(0.04, 0.30),
            "include_history": include_history,
        }))
        .await?;
    let payload: ThetaSnapshotPayload =
        serde_json::from_value(value).context("decode normalized ThetaData snapshot")?;
    build_live_snapshot(
        payload,
        request,
        previous_bars,
        risk_free_rate,
        stale_after_ms,
        sequence,
    )
}

fn build_live_snapshot(
    payload: ThetaSnapshotPayload,
    request: &LiveSessionRequest,
    previous_bars: Vec<Bar>,
    risk_free_rate: f64,
    stale_after_ms: u64,
    sequence: u64,
) -> anyhow::Result<LiveSnapshot> {
    anyhow::ensure!(
        payload.spot.is_finite() && payload.spot > 0.0,
        "invalid ThetaData spot"
    );
    let now = Utc::now();
    let stock_timestamp = payload.stock_timestamp.as_deref().and_then(parse_timestamp);
    let option_timestamps: Vec<Option<DateTime<Utc>>> = payload
        .contracts
        .iter()
        .map(|contract| contract.timestamp.as_deref().and_then(parse_timestamp))
        .collect();
    let as_of = option_timestamps
        .iter()
        .flatten()
        .copied()
        .chain(stock_timestamp)
        .max()
        .unwrap_or(now);
    let spot_age_ms = stock_timestamp.map(|timestamp| (now - timestamp).num_milliseconds().max(0));

    let mut chains = Vec::new();
    for expiration_text in &payload.expirations {
        let expiration = NaiveDate::parse_from_str(expiration_text, "%Y-%m-%d")
            .with_context(|| format!("invalid ThetaData expiration: {expiration_text}"))?;
        let expiration_contracts: Vec<&ThetaContract> = payload
            .contracts
            .iter()
            .filter(|contract| &contract.expiration == expiration_text)
            .collect();
        if expiration_contracts.is_empty() {
            continue;
        }
        let total = expiration_contracts.len() as f64;
        let quote_contracts = expiration_contracts
            .iter()
            .filter(|contract| valid_quote(contract))
            .count();
        let metadata_contracts = expiration_contracts
            .iter()
            .filter(|contract| contract.open_interest.is_some())
            .count();
        let fresh_contracts = expiration_contracts
            .iter()
            .filter(|contract| {
                contract
                    .timestamp
                    .as_deref()
                    .and_then(parse_timestamp)
                    .is_some_and(|timestamp| {
                        (now - timestamp).num_milliseconds().max(0) <= stale_after_ms as i64
                    })
            })
            .count();
        let raw: Vec<RawOptionQuote> = expiration_contracts
            .iter()
            .map(|contract| RawOptionQuote {
                symbol: contract.symbol.clone(),
                strike: contract.strike,
                right: contract.right.clone(),
                bid_size: contract.bid_size,
                ask_size: contract.ask_size,
                bid: contract.bid,
                ask: contract.ask,
                last: contract.last,
                volume: contract.volume,
                open_interest: contract.open_interest.unwrap_or_default(),
                sdk_iv: None,
                sdk_delta: None,
                sdk_gamma: None,
                sdk_theta: None,
                sdk_vega: None,
            })
            .collect();
        if let Ok(chain) = build_chain(ChainBuild {
            symbol: &payload.symbol,
            spot: payload.spot,
            as_of,
            expiration,
            quotes: &raw,
            pricing_mode: &request.pricing_mode,
            dealer_model: &request.dealer_model,
            risk_free_rate,
            source: "ThetaData",
            quote_interval: "realtime_snapshot_poll",
            oi_frequency: "provider_snapshot_cached_5m",
            prefer_sdk_greeks: false,
            quote_coverage: quote_contracts as f64 / total * 100.0,
            fresh_quote_coverage: fresh_contracts as f64 / total * 100.0,
            metadata_coverage: metadata_contracts as f64 / total * 100.0,
            spot_age_ms,
        }) {
            chains.push(chain);
        }
    }
    let chain = chains
        .iter()
        .find(|chain| chain.expiration == payload.selected_expiration)
        .cloned()
        .ok_or_else(|| anyhow!("selected ThetaData expiration has no usable quotes"))?;
    let surface = build_surface(&payload.symbol, &chains, as_of);

    let mut bars: Vec<Bar> = payload
        .bars
        .into_iter()
        .filter_map(ThetaBar::into_bar)
        .collect();
    if bars.is_empty() {
        bars = previous_bars;
    }
    if let Some(stock_bar) = payload.stock_bar.into_bar() {
        if let Some(existing) = bars.iter_mut().find(|bar| bar.time == stock_bar.time) {
            *existing = stock_bar;
        } else {
            bars.push(stock_bar);
        }
    }
    bars.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    if bars.len() > 500 {
        bars.drain(0..bars.len() - 500);
    }

    let total = payload.contracts.len().max(1) as f64;
    let quote_contracts = payload.contracts.iter().filter(valid_quote).count();
    let metadata_contracts = payload
        .contracts
        .iter()
        .filter(|contract| contract.open_interest.is_some())
        .count();
    let fresh_contracts = option_timestamps
        .iter()
        .filter(|timestamp| {
            timestamp.is_some_and(|timestamp| {
                (now - timestamp).num_milliseconds().max(0) <= stale_after_ms as i64
            })
        })
        .count();
    let quote_coverage_pct = quote_contracts as f64 / total * 100.0;
    let metadata_coverage_pct = metadata_contracts as f64 / total * 100.0;
    let fresh_quote_coverage_pct = fresh_contracts as f64 / total * 100.0;
    let latency_ms = spot_age_ms.unwrap_or(stale_after_ms as i64 + 1);
    let quality_state = if stock_timestamp.is_none() {
        "missing_timestamp"
    } else if quote_coverage_pct < 80.0 {
        "degraded_quotes"
    } else if latency_ms > stale_after_ms as i64 {
        "stale_underlying"
    } else if fresh_quote_coverage_pct < 80.0 {
        "stale_options"
    } else if metadata_coverage_pct < 90.0 {
        "waiting_metadata"
    } else {
        "ready"
    };
    Ok(LiveSnapshot {
        kind: "live_snapshot",
        sequence,
        feed: LiveFeedInfo {
            source: "ThetaData".into(),
            transport: "ThetaData Python SDK snapshot polling -> local WebSocket".into(),
            sdk_version: payload.sdk_version,
            symbol: payload.symbol,
            expiration: payload.selected_expiration,
            expirations: payload.expirations,
            subscribed_contracts: payload.contracts.len(),
            quote_contracts,
            metadata_contracts,
            quote_coverage_pct: round(quote_coverage_pct, 2),
            fresh_quote_coverage_pct: round(fresh_quote_coverage_pct, 2),
            metadata_coverage_pct: round(metadata_coverage_pct, 2),
            subscription_limit: payload.contract_limit,
            as_of: as_of.to_rfc3339(),
            stale_after_ms,
            latency_ms,
            quality_state: quality_state.into(),
        },
        bars,
        chain,
        surface,
    })
}

fn valid_quote(contract: &&ThetaContract) -> bool {
    contract.bid.is_finite()
        && contract.ask.is_finite()
        && contract.bid >= 0.0
        && contract.ask > 0.0
        && contract.ask >= contract.bid
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn validate_request(request: &LiveSessionRequest) -> anyhow::Result<()> {
    let symbol = request.symbol.trim().trim_end_matches(".US");
    anyhow::ensure!(
        !symbol.is_empty()
            && symbol.len() <= 15
            && symbol.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-')
            ),
        "invalid US symbol"
    );
    anyhow::ensure!(
        matches!(request.pricing_mode.as_str(), "mid" | "micro" | "ask"),
        "invalid pricing mode"
    );
    anyhow::ensure!(
        matches!(
            request.dealer_model.as_str(),
            "classic" | "short_all" | "long_all"
        ),
        "invalid dealer model"
    );
    Ok(())
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_snapshot_uses_rust_analytics_and_theta_provenance() {
        let contracts: Vec<Value> = (0..16)
            .flat_map(|index| {
                let strike = 590.0 + index as f64 * 2.0;
                ["CALL", "PUT"].map(move |right| {
                    json!({
                        "symbol": format!("SPY260904{}{:08}", if right == "CALL" { "C" } else { "P" }, (strike * 1000.0) as i64),
                        "expiration": "2026-09-04",
                        "strike": strike,
                        "right": right,
                        "bid": 4.8,
                        "ask": 5.0,
                        "bid_size": 10,
                        "ask_size": 12,
                        "last": 4.9,
                        "volume": 100,
                        "open_interest": 500,
                        "timestamp": "2026-08-28T19:00:00Z"
                    })
                })
            })
            .collect();
        let payload: ThetaSnapshotPayload = serde_json::from_value(json!({
            "sdk_version": "1.0.7",
            "symbol": "SPY",
            "spot": 605.0,
            "stock_timestamp": "2026-08-28T19:00:00Z",
            "stock_bar": {"time":"15:00","timestamp":"2026-08-28T19:00:00Z","open":604.0,"high":606.0,"low":603.5,"close":605.0,"volume":1000,"vwap":604.8},
            "bars": [],
            "selected_expiration": "2026-09-04",
            "expirations": ["2026-09-04"],
            "contracts": contracts,
            "contract_limit": 420
        }))
        .unwrap();
        let request = LiveSessionRequest {
            provider: "thetadata".into(),
            symbol: "SPY".into(),
            expiration: Some("2026-09-04".into()),
            max_contracts: 420,
            surface_expiries: 2,
            moneyness_window: 0.12,
            pricing_mode: "mid".into(),
            dealer_model: "classic".into(),
        };
        let snapshot =
            build_live_snapshot(payload, &request, Vec::new(), 0.043, 15_000, 7).unwrap();
        assert_eq!(snapshot.feed.source, "ThetaData");
        assert_eq!(snapshot.feed.sdk_version, "1.0.7");
        assert_eq!(snapshot.chain.provenance.model, "Rust-BSM+SVI-v2");
        assert!(!snapshot.chain.rows.is_empty());
    }

    #[test]
    fn request_validation_rejects_shell_like_symbols() {
        let request = LiveSessionRequest {
            provider: "thetadata".into(),
            symbol: "SPY;rm".into(),
            expiration: None,
            max_contracts: 420,
            surface_expiries: 4,
            moneyness_window: 0.12,
            pricing_mode: "micro".into(),
            dealer_model: "classic".into(),
        };
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn bridge_sequence_discards_cancelled_responses_without_accepting_future_ids() {
        assert!(!bridge_response_matches(6, 7).unwrap());
        assert!(bridge_response_matches(7, 7).unwrap());
        assert!(bridge_response_matches(8, 7).is_err());
    }
}
