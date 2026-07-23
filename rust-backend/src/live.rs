use std::{
    collections::HashSet,
    env,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::America::New_York;
use dashmap::DashMap;
use longbridge::{
    Config,
    quote::{
        AdjustType, Candlestick, Period, PushEvent, PushEventDetail, QuoteContext, StrikePriceInfo,
        SubFlags, TradeSessions,
    },
    trade::{
        GetTodayOrdersOptions, OrderSide, OrderType, OutsideRTH, SubmitOrderOptions,
        TimeInForceType, TradeContext,
    },
};
use rust_decimal::prelude::ToPrimitive;
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::{
    analytics::{ChainBuild, build_chain, build_surface},
    models::{
        Bar, ConnectionStatus, CredentialRequest, LiveFeedInfo, LiveSessionRequest, LiveSnapshot,
        RawOptionQuote,
    },
    strategy::ExecutionLeg,
};

const SDK_VERSION: &str = "4.4.1";
const OPENAPI_SUBSCRIPTION_LIMIT: usize = 500;
const RESERVED_SUBSCRIPTIONS: usize = 20;
const OPTION_REQUEST_COOLDOWN: Duration = Duration::from_secs(65);
const OPTION_RETRY_MARKER: &str = "option_retry_after_ms=";

#[derive(Debug, Clone)]
struct TopOfBook {
    bid: f64,
    ask: f64,
    bid_size: i64,
    ask_size: i64,
    timestamp: Option<DateTime<Utc>>,
}

impl Default for TopOfBook {
    fn default() -> Self {
        Self {
            bid: 0.0,
            ask: 0.0,
            bid_size: 0,
            ask_size: 0,
            timestamp: None,
        }
    }
}

#[derive(Debug, Clone)]
struct LiveQuote {
    last: f64,
    timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
struct OptionMetrics {
    last: Option<f64>,
    volume: i64,
    open_interest: i64,
    iv: Option<f64>,
    delta: Option<f64>,
    gamma: Option<f64>,
    theta: Option<f64>,
    vega: Option<f64>,
    timestamp: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct LiveCache {
    quotes: DashMap<String, LiveQuote>,
    depth: DashMap<String, TopOfBook>,
    metrics: DashMap<String, OptionMetrics>,
    bars: DashMap<String, Vec<Bar>>,
}

#[derive(Debug, Clone)]
struct ContractDef {
    symbol: String,
    expiration: NaiveDate,
    strike: f64,
    right: String,
}

#[derive(Debug, Clone)]
struct ActiveUniverse {
    underlying: String,
    display_symbol: String,
    selected_expiration: NaiveDate,
    expirations: Vec<NaiveDate>,
    contracts: Vec<ContractDef>,
    pricing_mode: String,
    dealer_model: String,
}

#[derive(Clone)]
struct LiveEngine {
    quote: QuoteContext,
    trade: Option<TradeContext>,
}

#[derive(Default)]
struct ManagerState {
    engine: Option<LiveEngine>,
    active: Option<ActiveUniverse>,
    status: ConnectionStatus,
}

pub struct LiveManager {
    state: RwLock<ManagerState>,
    session_setup: Mutex<()>,
    option_request_at: Mutex<Option<Instant>>,
    cache: Arc<LiveCache>,
    events: broadcast::Sender<u64>,
    sequence: AtomicU64,
    refresh_started: AtomicBool,
    snapshot_cache: Mutex<Option<(u64, LiveSnapshot)>>,
    risk_free_rate: f64,
    paper_execution_requested: bool,
}

impl LiveManager {
    pub fn new(risk_free_rate: f64) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            state: RwLock::new(ManagerState::default()),
            session_setup: Mutex::new(()),
            option_request_at: Mutex::new(None),
            cache: Arc::new(LiveCache::default()),
            events,
            sequence: AtomicU64::new(0),
            refresh_started: AtomicBool::new(false),
            snapshot_cache: Mutex::new(None),
            risk_free_rate,
            paper_execution_requested: env::var("OPTION_WORKSTATION_PAPER_ORDER_EXECUTION")
                .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE")),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<u64> {
        self.events.subscribe()
    }

    fn notify(&self) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.events.send(sequence);
    }

    async fn reserve_option_request(&self, operation: &str) -> anyhow::Result<()> {
        let mut last_request = self.option_request_at.lock().await;
        if let Some(previous) = *last_request {
            let elapsed = previous.elapsed();
            if elapsed < OPTION_REQUEST_COOLDOWN {
                let retry_after = OPTION_REQUEST_COOLDOWN - elapsed;
                let retry_after_ms = retry_after.as_millis().max(1) as u64;
                return Err(anyhow!(
                    "{OPTION_RETRY_MARKER}{retry_after_ms}; Longbridge 期权请求限频保护中，\
                     当前行情流保持不变，约 {} 秒后重试 {operation}",
                    retry_after.as_secs() + 1
                ));
            }
        }
        *last_request = Some(Instant::now());
        Ok(())
    }

    async fn rollback_option_switch(quote: &QuoteContext, added: &[String], removed: &[String]) {
        if !added.is_empty()
            && let Err(error) = quote.unsubscribe(added.to_vec(), SubFlags::DEPTH).await
        {
            tracing::error!(%error, contracts = added.len(), "failed to remove partially switched option subscriptions");
        }
        if !removed.is_empty()
            && let Err(error) = quote.subscribe(removed.to_vec(), SubFlags::DEPTH).await
        {
            tracing::error!(%error, contracts = removed.len(), "failed to restore previous option subscriptions");
        }
    }

    pub async fn status(&self) -> ConnectionStatus {
        self.state.read().await.status.clone()
    }

    pub async fn daily_closes(&self, count: usize) -> anyhow::Result<Vec<(String, f64)>> {
        let (engine, underlying) = {
            let state = self.state.read().await;
            let engine = state
                .engine
                .clone()
                .ok_or_else(|| anyhow!("Longbridge is not connected"))?;
            let underlying = state
                .active
                .as_ref()
                .map(|active| active.underlying.clone())
                .ok_or_else(|| anyhow!("尚未建立实时期权会话"))?;
            (engine, underlying)
        };
        let mut closes: Vec<(String, f64)> = engine
            .quote
            .candlesticks(
                underlying,
                Period::Day,
                count.clamp(21, 120),
                AdjustType::ForwardAdjust,
                TradeSessions::Intraday,
            )
            .await
            .context("load daily candles for realized volatility")?
            .into_iter()
            .filter_map(|candle| {
                let close = candle.close.to_f64()?;
                (close.is_finite() && close > 0.0).then(|| {
                    (
                        sdk_time(candle.timestamp)
                            .with_timezone(&New_York)
                            .format("%Y-%m-%d")
                            .to_string(),
                        close,
                    )
                })
            })
            .collect();
        let today_et = Utc::now()
            .with_timezone(&New_York)
            .format("%Y-%m-%d")
            .to_string();
        closes.retain(|(date, _)| date != &today_et);
        closes.sort_by(|left, right| left.0.cmp(&right.0));
        closes.dedup_by(|left, right| left.0 == right.0);
        Ok(closes)
    }

    pub async fn trade_account(&self) -> anyhow::Result<Value> {
        let (engine, known_account_type, known_paper) = {
            let state = self.state.read().await;
            (
                state
                    .engine
                    .clone()
                    .ok_or_else(|| anyhow!("Longbridge is not connected"))?,
                state.status.account_type.clone(),
                state.status.paper_account,
            )
        };
        let trade = engine
            .trade
            .ok_or_else(|| anyhow!("trade permission is unavailable for this token"))?;
        let (account_type, paper_account, buy_power, currency) =
            if let Ok(overview) = trade.us_asset_overview().await {
                let account_type = overview.account_type.trim().to_string();
                let paper_account = known_paper || is_paper_account_type(&account_type);
                (
                    (!account_type.is_empty())
                        .then_some(account_type)
                        .or(known_account_type)
                        .unwrap_or_else(|| "unknown".into()),
                    paper_account,
                    overview.cash_buy_power,
                    overview.currency,
                )
            } else {
                let balances = trade
                    .account_balance(Some("USD"))
                    .await
                    .context("load account balance fallback")?;
                let balance = balances
                    .iter()
                    .find(|balance| balance.currency == "USD")
                    .or_else(|| balances.first())
                    .ok_or_else(|| anyhow!("Longbridge returned no account balance"))?;
                (
                    known_account_type.unwrap_or_else(|| "unknown".into()),
                    known_paper,
                    balance.buy_power.to_string(),
                    balance.currency.clone(),
                )
            };
        let enabled = paper_account && self.paper_execution_requested;
        {
            let mut state = self.state.write().await;
            state.status.trade_connected = true;
            state.status.paper_account = paper_account;
            state.status.account_type = Some(account_type.clone());
            state.status.buy_power = Some(buy_power.clone());
            state.status.order_execution_enabled = enabled;
        }
        Ok(json!({
            "connected": true,
            "paper_account": paper_account,
            "account_type": account_type,
            "buy_power": buy_power,
            "currency": currency,
            "execution_enabled": enabled,
            "execution_mode": "paper_sequential_guarded",
        }))
    }

    pub async fn today_orders(&self) -> anyhow::Result<Value> {
        let engine = self
            .state
            .read()
            .await
            .engine
            .clone()
            .ok_or_else(|| anyhow!("Longbridge is not connected"))?;
        let trade = engine
            .trade
            .ok_or_else(|| anyhow!("trade permission is unavailable for this token"))?;
        let orders = trade
            .today_orders(GetTodayOrdersOptions::new())
            .await
            .context("load today's paper orders")?;
        serde_json::to_value(orders).map_err(anyhow::Error::from)
    }

    pub async fn cancel_paper_order(&self, order_id: &str) -> anyhow::Result<Value> {
        let (trade, enabled, paper) = {
            let state = self.state.read().await;
            let trade = state
                .engine
                .as_ref()
                .and_then(|engine| engine.trade.clone())
                .ok_or_else(|| anyhow!("trade permission is unavailable for this token"))?;
            (
                trade,
                state.status.order_execution_enabled,
                state.status.paper_account,
            )
        };
        anyhow::ensure!(paper && enabled, "paper order execution is locked");
        anyhow::ensure!(
            !order_id.is_empty()
                && order_id.len() <= 80
                && order_id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()),
            "invalid order id"
        );
        trade
            .cancel_order(order_id)
            .await
            .context("cancel paper order")?;
        Ok(json!({"order_id": order_id, "status": "cancel_requested"}))
    }

    pub async fn submit_paper_orders(
        &self,
        orders: &[ExecutionLeg],
        preview_id: &str,
        confirmation: &str,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(confirmation == "PAPER", "confirmation must equal PAPER");
        anyhow::ensure!(
            !orders.is_empty() && orders.len() <= 8,
            "invalid order plan"
        );
        let (trade, enabled, paper) = {
            let state = self.state.read().await;
            let trade = state
                .engine
                .as_ref()
                .and_then(|engine| engine.trade.clone())
                .ok_or_else(|| anyhow!("trade permission is unavailable for this token"))?;
            (
                trade,
                state.status.order_execution_enabled,
                state.status.paper_account,
            )
        };
        anyhow::ensure!(
            paper,
            "connected account is not identified as a paper account"
        );
        anyhow::ensure!(
            enabled,
            "paper order execution is locked by server configuration"
        );
        let mut plan = orders.to_vec();
        plan.sort_by_key(|order| if order.side == "BUY" { 0 } else { 1 });
        let mut submitted = Vec::new();
        for (index, order) in plan.iter().enumerate() {
            anyhow::ensure!(
                (1..=100).contains(&order.quantity),
                "invalid order quantity"
            );
            anyhow::ensure!(order.limit_price > 0.0, "invalid limit price");
            let side = if order.side == "BUY" {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            };
            let price = rust_decimal::Decimal::from_f64_retain(order.limit_price)
                .ok_or_else(|| anyhow!("invalid decimal limit price"))?;
            let options = SubmitOrderOptions::new(
                order.symbol.clone(),
                OrderType::LO,
                side,
                rust_decimal::Decimal::from(order.quantity),
                TimeInForceType::Day,
            )
            .submitted_price(price)
            .outside_rth(OutsideRTH::RTHOnly)
            .remark(format!("OW PAPER {preview_id}"))
            .client_request_id(format!("ow-{preview_id}-{index}"));
            match trade.submit_order(options).await {
                Ok(response) => submitted.push(response.order_id),
                Err(error) => {
                    for order_id in &submitted {
                        let _ = trade.cancel_order(order_id.clone()).await;
                    }
                    return Err(error).context(format!(
                        "paper leg {} failed; cancellation requested for prior legs",
                        index + 1
                    ));
                }
            }
        }
        Ok(json!({
            "preview_id": preview_id,
            "mode": "paper_sequential_guarded",
            "order_ids": submitted,
            "status": "submitted",
        }))
    }

    pub fn start_refresh_loop(self: &Arc<Self>) {
        if self.refresh_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(90));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(error) = manager.refresh_option_metadata().await {
                    let detail = format!("{error:#}");
                    if option_retry_after_ms(&detail).is_some() || detail.contains("301607") {
                        tracing::debug!(error = %detail, "option metadata refresh deferred by provider rate limit");
                        continue;
                    }
                    let mut state = manager.state.write().await;
                    if state.engine.is_some() {
                        state.status.error = Some(detail);
                    }
                }
            }
        });
    }

    pub async fn connect(
        self: &Arc<Self>,
        credentials: CredentialRequest,
    ) -> anyhow::Result<ConnectionStatus> {
        credentials.validate().map_err(|message| anyhow!(message))?;
        let token_account_type = token_account_type(&credentials.access_token);
        {
            let mut state = self.state.write().await;
            state.status.state = "connecting".into();
            state.status.error = None;
        }
        let config = Arc::new(Config::from_apikey(
            credentials.app_key.trim(),
            credentials.app_secret.trim(),
            credentials.access_token.trim(),
        ));
        let (quote, mut receiver) = QuoteContext::new(Arc::clone(&config));
        let (trade, mut trade_receiver) = TradeContext::new(config);
        let validation = async {
            let member_id = quote
                .member_id()
                .await
                .context("Longbridge credential validation failed")?;
            let quote_level = quote.quote_level().await.context("read quote level")?;
            let packages = quote
                .quote_package_details()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|package| package.name)
                .collect::<Vec<_>>();
            anyhow::Ok((member_id, quote_level, packages))
        }
        .await;
        let (member_id, quote_level, packages) = match validation {
            Ok(values) => values,
            Err(error) => {
                let mut state = self.state.write().await;
                state.status.connected = false;
                state.status.state = "error".into();
                state.status.error = Some(error.to_string());
                return Err(error);
            }
        };

        let cache = Arc::clone(&self.cache);
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                apply_push(&cache, event);
                {
                    let mut state = manager.state.write().await;
                    state.status.last_event_at = Some(Utc::now().to_rfc3339());
                    state.status.error = None;
                }
                manager.notify();
            }
        });
        tokio::spawn(async move { while trade_receiver.recv().await.is_some() {} });

        let trade_overview = trade.us_asset_overview().await.ok();
        let trade_balances = if trade_overview.is_none() {
            trade.account_balance(Some("USD")).await.ok()
        } else {
            None
        };
        let account_type = trade_overview
            .as_ref()
            .map(|overview| overview.account_type.trim().to_string())
            .filter(|value| !value.is_empty())
            .or(token_account_type);
        let paper_account = account_type.as_deref().is_some_and(is_paper_account_type);
        let trade_connected = trade_overview.is_some() || trade_balances.is_some();
        let buy_power = trade_overview
            .as_ref()
            .map(|overview| overview.cash_buy_power.clone())
            .or_else(|| {
                trade_balances.as_ref().and_then(|balances| {
                    balances
                        .iter()
                        .find(|balance| balance.currency == "USD")
                        .or_else(|| balances.first())
                        .map(|balance| balance.buy_power.to_string())
                })
            });
        let order_execution_enabled =
            trade_connected && paper_account && self.paper_execution_requested;

        let hint = format!(
            "***{}",
            member_id
                .to_string()
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
        let status = ConnectionStatus {
            connected: true,
            state: "connected".into(),
            account_hint: Some(hint),
            quote_level: Some(quote_level),
            packages,
            subscribed_contracts: 0,
            last_event_at: None,
            error: None,
            credential_storage: "process_memory_only",
            trade_connected,
            paper_account,
            account_type,
            buy_power,
            order_execution_enabled,
        };
        let old_engine = {
            let mut state = self.state.write().await;
            state.active = None;
            state.status = status.clone();
            state.engine.replace(LiveEngine {
                quote,
                trade: trade_connected.then_some(trade),
            })
        };
        drop(old_engine);
        self.start_refresh_loop();
        Ok(status)
    }

    pub async fn disconnect(&self) -> ConnectionStatus {
        let mut state = self.state.write().await;
        state.engine = None;
        state.active = None;
        state.status = ConnectionStatus::default();
        self.cache.quotes.clear();
        self.cache.depth.clear();
        self.cache.metrics.clear();
        self.cache.bars.clear();
        self.notify();
        state.status.clone()
    }

    pub async fn setup_session(&self, request: LiveSessionRequest) -> anyhow::Result<LiveSnapshot> {
        let _setup_guard = self.session_setup.lock().await;
        let engine = self
            .state
            .read()
            .await
            .engine
            .clone()
            .ok_or_else(|| anyhow!("请先在连接设置中填写 Longbridge 凭证"))?;
        let underlying = normalize_us_symbol(&request.symbol)?;
        let display_symbol = underlying.trim_end_matches(".US").to_string();
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
        let quote = engine
            .quote
            .quote([underlying.clone()])
            .await
            .context("load underlying quote")?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Longbridge returned no quote for {underlying}"))?;
        let spot = quote.last_done.to_f64().unwrap_or_default();
        anyhow::ensure!(spot > 0.0, "invalid live price for {underlying}");
        self.cache.quotes.insert(
            underlying.clone(),
            LiveQuote {
                last: spot,
                timestamp: sdk_time(quote.timestamp),
            },
        );

        let today = Utc::now().with_timezone(&New_York).date_naive();
        let available: Vec<NaiveDate> = engine
            .quote
            .option_chain_expiry_date_list(underlying.clone())
            .await
            .context("load option expirations")?
            .into_iter()
            .filter_map(|value| NaiveDate::parse_from_str(&value.to_string(), "%Y-%m-%d").ok())
            .filter(|value| *value >= today)
            .collect();
        anyhow::ensure!(
            !available.is_empty(),
            "no live option expirations for {underlying}"
        );
        let selected_expiration = match request.expiration.as_deref() {
            Some(value) => {
                let parsed =
                    NaiveDate::parse_from_str(value, "%Y-%m-%d").context("invalid expiration")?;
                anyhow::ensure!(
                    available.contains(&parsed),
                    "expiration not available: {value}"
                );
                parsed
            }
            None => available[0],
        };
        let expiry_count = request.surface_expiries.clamp(2, 6);
        let mut expirations: Vec<NaiveDate> =
            available.iter().copied().take(expiry_count).collect();
        if !expirations.contains(&selected_expiration) {
            if expirations.len() == expiry_count {
                expirations.pop();
            }
            expirations.push(selected_expiration);
            expirations.sort();
        }
        let contract_budget = option_contract_budget(request.max_contracts);
        let per_expiry = (contract_budget / expirations.len()).max(10);
        let window = request.moneyness_window.clamp(0.04, 0.30);
        let mut contracts = Vec::new();
        for expiration in &expirations {
            let sdk_date = time::Date::parse(
                &expiration.to_string(),
                &time::format_description::well_known::Iso8601::DATE,
            )?;
            let mut chain = engine
                .quote
                .option_chain_info_by_date(underlying.clone(), sdk_date)
                .await
                .with_context(|| format!("load option chain {expiration}"))?;
            chain.retain(|item| item.standard);
            let mut within: Vec<_> = chain
                .iter()
                .filter(|item| {
                    item.price
                        .to_f64()
                        .is_some_and(|strike| (strike / spot - 1.0).abs() <= window)
                })
                .cloned()
                .collect();
            if within.len() < 6 {
                within = chain;
            }
            within = select_strikes(within, spot, (per_expiry / 2).max(5));
            within.sort_by_key(|item| item.price);
            contracts.extend(contract_defs(*expiration, within));
        }
        contracts.sort_by(|left, right| {
            left.expiration
                .cmp(&right.expiration)
                .then_with(|| left.strike.total_cmp(&right.strike))
                .then_with(|| left.right.cmp(&right.right))
        });
        contracts.truncate(contract_budget);
        anyhow::ensure!(
            !contracts.is_empty(),
            "no option contracts inside the subscription window"
        );
        let option_symbols: Vec<String> = contracts
            .iter()
            .map(|contract| contract.symbol.clone())
            .collect();

        let previous = self.state.read().await.active.clone();
        let previous_options: HashSet<String> = previous
            .as_ref()
            .map(|active| {
                active
                    .contracts
                    .iter()
                    .map(|item| item.symbol.clone())
                    .collect()
            })
            .unwrap_or_default();
        let next_options: HashSet<String> = option_symbols.iter().cloned().collect();
        let removed: Vec<String> = previous_options
            .difference(&next_options)
            .cloned()
            .collect();
        let added: Vec<String> = next_options
            .difference(&previous_options)
            .cloned()
            .collect();
        let underlying_changed = previous
            .as_ref()
            .is_none_or(|active| active.underlying != underlying);

        // Reserve the provider's option-security request window before touching the
        // active subscriptions. A rejected preflight leaves the current stream intact.
        if !added.is_empty() {
            self.reserve_option_request("订阅切换").await?;
        }
        if !removed.is_empty() {
            engine
                .quote
                .unsubscribe(removed.clone(), SubFlags::DEPTH)
                .await
                .context("remove previous option subscriptions")?;
        }
        if !added.is_empty() {
            for symbol in &added {
                self.cache.depth.remove(symbol);
            }
            if let Err(error) = engine.quote.subscribe(added.clone(), SubFlags::DEPTH).await {
                Self::rollback_option_switch(&engine.quote, &added, &removed).await;
                return Err(error).context("subscribe live option depth");
            }
        }
        if underlying_changed {
            if let Err(error) = engine
                .quote
                .subscribe([underlying.clone()], SubFlags::QUOTE)
                .await
            {
                Self::rollback_option_switch(&engine.quote, &added, &removed).await;
                return Err(error).context("subscribe live underlying quote");
            }
            let candles = match engine
                .quote
                .subscribe_candlesticks(
                    underlying.clone(),
                    Period::OneMinute,
                    TradeSessions::Intraday,
                )
                .await
            {
                Ok(candles) => candles,
                Err(error) => {
                    let _ = engine
                        .quote
                        .unsubscribe([underlying.clone()], SubFlags::QUOTE)
                        .await;
                    Self::rollback_option_switch(&engine.quote, &added, &removed).await;
                    return Err(error).context("subscribe 1m candlesticks");
                }
            };
            self.cache.bars.insert(
                underlying.clone(),
                candles.into_iter().map(bar_from_sdk).collect(),
            );
            if let Some(previous) = &previous {
                let _ = engine
                    .quote
                    .unsubscribe([previous.underlying.clone()], SubFlags::QUOTE)
                    .await;
                let _ = engine
                    .quote
                    .unsubscribe_candlesticks(previous.underlying.clone(), Period::OneMinute)
                    .await;
            }
        }
        for symbol in &removed {
            self.cache.depth.remove(symbol);
        }

        let missing_metadata: Vec<String> = option_symbols
            .iter()
            .filter(|symbol| !self.cache.metrics.contains_key(*symbol))
            .cloned()
            .collect();
        if !missing_metadata.is_empty() && added.len().saturating_add(missing_metadata.len()) <= 480
        {
            if let Err(error) = self
                .load_option_metadata(&engine.quote, &missing_metadata)
                .await
            {
                tracing::warn!(%error, "initial option metadata deferred");
            }
        } else if !missing_metadata.is_empty() {
            tracing::info!(
                contracts = missing_metadata.len(),
                "option metadata deferred to stay inside provider request budget"
            );
        }

        let active = ActiveUniverse {
            underlying,
            display_symbol,
            selected_expiration,
            expirations,
            contracts,
            pricing_mode: request.pricing_mode,
            dealer_model: request.dealer_model,
        };
        {
            let mut state = self.state.write().await;
            state.status.subscribed_contracts = active.contracts.len();
            state.status.state = "streaming".into();
            state.status.error = None;
            state.active = Some(active);
        }
        for _ in 0..10 {
            let populated = option_symbols
                .iter()
                .filter(|symbol| self.cache.depth.contains_key(*symbol))
                .count();
            if populated >= option_symbols.len().min(20) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.notify();
        self.snapshot().await
    }

    async fn load_option_metadata(
        &self,
        quote: &QuoteContext,
        symbols: &[String],
    ) -> anyhow::Result<()> {
        if symbols.is_empty() {
            return Ok(());
        }
        anyhow::ensure!(
            symbols.len() <= OPENAPI_SUBSCRIPTION_LIMIT,
            "option metadata request exceeds provider limit"
        );
        self.reserve_option_request("期权元数据刷新").await?;
        let options = quote
            .option_quote(symbols.to_vec())
            .await
            .context("load option quotes")?;
        for option in options {
            self.cache.metrics.insert(
                option.symbol,
                OptionMetrics {
                    last: option.last_done.to_f64(),
                    volume: option.volume,
                    open_interest: option.open_interest,
                    iv: option.implied_volatility.to_f64().and_then(normalize_iv),
                    timestamp: Some(sdk_time(option.timestamp)),
                    ..Default::default()
                },
            );
        }
        Ok(())
    }

    async fn refresh_option_metadata(&self) -> anyhow::Result<()> {
        let _setup_guard = self.session_setup.lock().await;
        let (engine, symbols) = {
            let state = self.state.read().await;
            let Some(engine) = state.engine.clone() else {
                return Ok(());
            };
            let Some(active) = state.active.clone() else {
                return Ok(());
            };
            (
                engine,
                active
                    .contracts
                    .into_iter()
                    .map(|item| item.symbol)
                    .collect::<Vec<_>>(),
            )
        };
        self.load_option_metadata(&engine.quote, &symbols).await?;
        self.notify();
        Ok(())
    }

    pub async fn snapshot(&self) -> anyhow::Result<LiveSnapshot> {
        let sequence = self.sequence.load(Ordering::Relaxed);
        let mut cache = self.snapshot_cache.lock().await;
        if let Some((cached_sequence, snapshot)) = cache.as_ref()
            && *cached_sequence == sequence
        {
            return Ok(snapshot.clone());
        }
        let snapshot = self.build_snapshot(sequence).await?;
        *cache = Some((sequence, snapshot.clone()));
        Ok(snapshot)
    }

    async fn build_snapshot(&self, sequence: u64) -> anyhow::Result<LiveSnapshot> {
        let active = self
            .state
            .read()
            .await
            .active
            .clone()
            .ok_or_else(|| anyhow!("尚未建立实时期权会话"))?;
        let underlying = self
            .cache
            .quotes
            .get(&active.underlying)
            .map(|value| value.clone())
            .ok_or_else(|| anyhow!("waiting for underlying quote"))?;
        let now = Utc::now();
        let as_of = active
            .contracts
            .iter()
            .filter_map(|contract| {
                self.cache
                    .depth
                    .get(&contract.symbol)
                    .and_then(|book| book.timestamp)
            })
            .fold(underlying.timestamp, std::cmp::max);
        let spot_age_ms = (now - underlying.timestamp).num_milliseconds().max(0);
        let mut chains = Vec::new();
        for expiration in &active.expirations {
            let expiration_contracts: Vec<_> = active
                .contracts
                .iter()
                .filter(|contract| &contract.expiration == expiration)
                .collect();
            let quote_contracts = expiration_contracts
                .iter()
                .filter(|contract| self.cache.depth.contains_key(&contract.symbol))
                .count();
            let metadata_contracts = expiration_contracts
                .iter()
                .filter(|contract| self.cache.metrics.contains_key(&contract.symbol))
                .count();
            let fresh_contracts = expiration_contracts
                .iter()
                .filter(|contract| {
                    self.cache
                        .depth
                        .get(&contract.symbol)
                        .and_then(|book| book.timestamp)
                        .is_some_and(|timestamp| (now - timestamp).num_milliseconds() <= 5_000)
                })
                .count();
            let contract_count = expiration_contracts.len().max(1) as f64;
            let raw: Vec<RawOptionQuote> = active
                .contracts
                .iter()
                .filter(|contract| &contract.expiration == expiration)
                .map(|contract| {
                    let book = self
                        .cache
                        .depth
                        .get(&contract.symbol)
                        .map(|value| value.clone())
                        .unwrap_or_default();
                    let metrics = self
                        .cache
                        .metrics
                        .get(&contract.symbol)
                        .map(|value| value.clone())
                        .unwrap_or_default();
                    RawOptionQuote {
                        symbol: contract.symbol.clone(),
                        strike: contract.strike,
                        right: contract.right.clone(),
                        bid_size: book.bid_size,
                        ask_size: book.ask_size,
                        bid: book.bid,
                        ask: book.ask,
                        last: metrics.last,
                        volume: metrics.volume,
                        open_interest: metrics.open_interest,
                        sdk_iv: metrics.iv,
                        sdk_delta: metrics.delta,
                        sdk_gamma: metrics.gamma,
                        sdk_theta: metrics.theta,
                        sdk_vega: metrics.vega,
                    }
                })
                .collect();
            if let Ok(chain) = build_chain(ChainBuild {
                symbol: &active.display_symbol,
                spot: underlying.last,
                as_of,
                expiration: *expiration,
                quotes: &raw,
                pricing_mode: &active.pricing_mode,
                dealer_model: &active.dealer_model,
                risk_free_rate: self.risk_free_rate,
                source: "Longbridge",
                quote_interval: "tick",
                oi_frequency: "realtime_snapshot",
                prefer_sdk_greeks: false,
                quote_coverage: quote_contracts as f64 / contract_count * 100.0,
                fresh_quote_coverage: fresh_contracts as f64 / contract_count * 100.0,
                metadata_coverage: metadata_contracts as f64 / contract_count * 100.0,
                spot_age_ms: Some(spot_age_ms),
            }) {
                chains.push(chain);
            }
        }
        let chain = chains
            .iter()
            .find(|chain| chain.expiration == active.selected_expiration.to_string())
            .cloned()
            .ok_or_else(|| anyhow!("selected expiration has no usable quotes yet"))?;
        let surface = build_surface(&active.display_symbol, &chains, as_of);
        let bars = self
            .cache
            .bars
            .get(&active.underlying)
            .map(|value| value.clone())
            .unwrap_or_default();
        let quote_contracts = active
            .contracts
            .iter()
            .filter(|contract| self.cache.depth.contains_key(&contract.symbol))
            .count();
        let fresh_contracts = active
            .contracts
            .iter()
            .filter(|contract| {
                self.cache
                    .depth
                    .get(&contract.symbol)
                    .and_then(|book| book.timestamp)
                    .is_some_and(|timestamp| (now - timestamp).num_milliseconds() <= 5_000)
            })
            .count();
        let metadata_contracts = active
            .contracts
            .iter()
            .filter(|contract| self.cache.metrics.contains_key(&contract.symbol))
            .count();
        let total = active.contracts.len().max(1) as f64;
        let quote_coverage_pct = quote_contracts as f64 / total * 100.0;
        let fresh_quote_coverage_pct = fresh_contracts as f64 / total * 100.0;
        let metadata_coverage_pct = metadata_contracts as f64 / total * 100.0;
        let quality_state = if quote_coverage_pct < 80.0 {
            "degraded_quotes"
        } else if spot_age_ms > 5_000 {
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
                source: "Longbridge",
                transport: "Longbridge Rust SDK WebSocket -> local WebSocket",
                sdk_version: SDK_VERSION,
                symbol: active.display_symbol,
                expiration: active.selected_expiration.to_string(),
                expirations: active.expirations.iter().map(ToString::to_string).collect(),
                subscribed_contracts: active.contracts.len(),
                quote_contracts,
                metadata_contracts,
                quote_coverage_pct: (quote_coverage_pct * 100.0).round() / 100.0,
                fresh_quote_coverage_pct: (fresh_quote_coverage_pct * 100.0).round() / 100.0,
                metadata_coverage_pct: (metadata_coverage_pct * 100.0).round() / 100.0,
                subscription_limit: OPENAPI_SUBSCRIPTION_LIMIT,
                as_of: as_of.to_rfc3339(),
                stale_after_ms: 5_000,
                latency_ms: spot_age_ms,
                quality_state: quality_state.into(),
            },
            bars,
            chain,
            surface,
        })
    }
}

fn token_account_type(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice::<Value>(&decoded)
        .ok()?
        .get("ac")?
        .as_str()
        .map(str::to_string)
}

fn is_paper_account_type(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    normalized.contains("paper") || normalized.contains("simulat") || normalized.contains("demo")
}

fn normalize_us_symbol(value: &str) -> anyhow::Result<String> {
    let clean = value.trim().to_uppercase();
    anyhow::ensure!(
        !clean.is_empty()
            && clean.len() <= 20
            && clean
                .chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || character == '.'
                    || character == '-'),
        "invalid symbol"
    );
    Ok(if clean.ends_with(".US") {
        clean
    } else {
        format!("{clean}.US")
    })
}

fn contract_defs(expiration: NaiveDate, rows: Vec<StrikePriceInfo>) -> Vec<ContractDef> {
    rows.into_iter()
        .flat_map(|item| {
            let strike = item.price.to_f64().unwrap_or_default();
            [
                ContractDef {
                    symbol: item.call_symbol,
                    expiration,
                    strike,
                    right: "CALL".into(),
                },
                ContractDef {
                    symbol: item.put_symbol,
                    expiration,
                    strike,
                    right: "PUT".into(),
                },
            ]
        })
        .filter(|item| !item.symbol.is_empty() && item.strike > 0.0)
        .collect()
}

fn select_strikes(rows: Vec<StrikePriceInfo>, spot: f64, limit: usize) -> Vec<StrikePriceInfo> {
    if rows.len() <= limit {
        return rows;
    }
    let mut by_distance = rows.clone();
    by_distance.sort_by(|left, right| {
        (left.price.to_f64().unwrap_or_default() - spot)
            .abs()
            .total_cmp(&(right.price.to_f64().unwrap_or_default() - spot).abs())
    });
    let core_count = (limit * 3 / 5).max(6).min(limit);
    let mut selected: Vec<StrikePriceInfo> = by_distance.iter().take(core_count).cloned().collect();

    let mut by_strike = rows;
    by_strike.sort_by_key(|item| item.price);
    let wing_slots = limit.saturating_sub(selected.len());
    if wing_slots > 0 {
        for index in 0..wing_slots {
            let source = if wing_slots == 1 {
                by_strike.len() / 2
            } else {
                index * (by_strike.len() - 1) / (wing_slots - 1)
            };
            let candidate = &by_strike[source];
            if !selected.iter().any(|item| item.price == candidate.price) {
                selected.push(candidate.clone());
            }
        }
    }
    for candidate in by_distance {
        if selected.len() >= limit {
            break;
        }
        if !selected.iter().any(|item| item.price == candidate.price) {
            selected.push(candidate);
        }
    }
    selected.truncate(limit);
    selected
}

fn sdk_time(value: time::OffsetDateTime) -> DateTime<Utc> {
    DateTime::from_timestamp(value.unix_timestamp(), value.nanosecond()).unwrap_or_else(Utc::now)
}

fn bar_from_sdk(value: Candlestick) -> Bar {
    let timestamp = sdk_time(value.timestamp);
    let et = timestamp.with_timezone(&New_York);
    Bar {
        time: et.format("%H:%M").to_string(),
        timestamp: timestamp.to_rfc3339(),
        open: value.open.to_f64().unwrap_or_default(),
        high: value.high.to_f64().unwrap_or_default(),
        low: value.low.to_f64().unwrap_or_default(),
        close: value.close.to_f64().unwrap_or_default(),
        volume: value.volume,
        vwap: value.close.to_f64().unwrap_or_default(),
    }
}

fn normalize_iv(value: f64) -> Option<f64> {
    let normalized = if value > 4.0 { value / 100.0 } else { value };
    (0.001..=4.0).contains(&normalized).then_some(normalized)
}

fn option_contract_budget(requested: usize) -> usize {
    requested.clamp(20, OPENAPI_SUBSCRIPTION_LIMIT - RESERVED_SUBSCRIPTIONS)
}

pub fn option_retry_after_ms(detail: &str) -> Option<u64> {
    let start = detail.find(OPTION_RETRY_MARKER)? + OPTION_RETRY_MARKER.len();
    let digits: String = detail[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn apply_push(cache: &LiveCache, event: PushEvent) {
    match event.detail {
        PushEventDetail::Quote(value) => {
            let timestamp = sdk_time(value.timestamp);
            let last = value.last_done.to_f64().unwrap_or_default();
            cache
                .quotes
                .insert(event.symbol.clone(), LiveQuote { last, timestamp });
            if let Some(mut metrics) = cache.metrics.get_mut(&event.symbol) {
                metrics.last = Some(last);
                metrics.volume = value.volume;
                metrics.timestamp = Some(timestamp);
            }
        }
        PushEventDetail::Depth(value) => {
            let bid = value.bids.first();
            let ask = value.asks.first();
            cache.depth.insert(
                event.symbol,
                TopOfBook {
                    bid: bid
                        .and_then(|level| level.price.and_then(|price| price.to_f64()))
                        .unwrap_or_default(),
                    ask: ask
                        .and_then(|level| level.price.and_then(|price| price.to_f64()))
                        .unwrap_or_default(),
                    bid_size: bid.map(|level| level.volume).unwrap_or_default(),
                    ask_size: ask.map(|level| level.volume).unwrap_or_default(),
                    timestamp: Some(Utc::now()),
                },
            );
        }
        PushEventDetail::Candlestick(value) => {
            let bar = bar_from_sdk(value.candlestick);
            cache
                .bars
                .entry(event.symbol)
                .and_modify(|bars| {
                    if let Some(existing) =
                        bars.iter_mut().find(|existing| existing.time == bar.time)
                    {
                        *existing = bar.clone();
                    } else {
                        bars.push(bar.clone());
                        bars.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
                        if bars.len() > 500 {
                            bars.drain(0..bars.len() - 500);
                        }
                    }
                })
                .or_insert_with(|| vec![bar]);
        }
        PushEventDetail::Trade(_) | PushEventDetail::Brokers(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_normalization_is_strict() {
        assert_eq!(normalize_us_symbol("spy").unwrap(), "SPY.US");
        assert_eq!(normalize_us_symbol("QQQ.US").unwrap(), "QQQ.US");
        assert!(normalize_us_symbol("SPY;rm").is_err());
    }

    #[test]
    fn contract_budget_stays_below_provider_limit() {
        assert_eq!(option_contract_budget(1), 20);
        assert_eq!(option_contract_budget(420), 420);
        assert_eq!(option_contract_budget(10_000), 480);
    }

    #[test]
    fn iv_normalization_accepts_ratio_and_percentage_forms() {
        assert_eq!(normalize_iv(0.2341), Some(0.2341));
        assert_eq!(normalize_iv(23.41), Some(0.2341));
        assert_eq!(normalize_iv(0.0), None);
    }

    #[test]
    fn paper_account_claim_is_detected_without_retaining_token() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"ac":"lb_papertrading"}"#);
        let token = format!("header.{payload}.signature");
        assert_eq!(
            token_account_type(&token).as_deref(),
            Some("lb_papertrading")
        );
        assert!(is_paper_account_type("lb_papertrading"));
        assert!(!is_paper_account_type("lb_cash"));
    }

    #[test]
    fn option_retry_marker_round_trips() {
        assert_eq!(
            option_retry_after_ms("option_retry_after_ms=61234; wait"),
            Some(61_234)
        );
        assert_eq!(option_retry_after_ms("ordinary upstream error"), None);
    }
}
