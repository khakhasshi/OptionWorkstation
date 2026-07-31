mod analytics;
mod assistant;
mod audit;
mod live;
mod models;
mod replay;
mod strategy;
mod volatility;

use std::{convert::Infallible, env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt, stream};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    assistant::{
        AssistantChatRequest, AssistantContext, AssistantContextInput, AssistantCreateRequest,
        AssistantImportRequest, AssistantManager, AssistantSession, AssistantSnapshotRef,
        build_context,
    },
    audit::{AuditCaptureRequest, AuditStore},
    live::{LiveManager, option_retry_after_ms},
    models::{CredentialRequest, LiveSessionRequest, OAuthStartRequest},
    replay::{ReplaySnapshotParams, ReplayStore},
    strategy::{PaperOrderRequest, StrategyRequest, analyze_strategy},
};

#[derive(Clone)]
struct AppState {
    replay: Arc<ReplayStore>,
    live: Arc<LiveManager>,
    audit: Arc<AuditStore>,
    assistant: Arc<AssistantManager>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
    retry_after_ms: Option<u64>,
}

impl ApiError {
    fn bad_request(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
            retry_after_ms: None,
        }
    }

    fn conflict(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: error.to_string(),
            retry_after_ms: None,
        }
    }

    fn upstream(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            retry_after_ms: None,
        }
    }

    fn rate_limited(error: impl std::fmt::Display, retry_after_ms: u64) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: error.to_string(),
            retry_after_ms: Some(retry_after_ms),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "detail": self.message,
                "retry_after_ms": self.retry_after_ms,
            })),
        )
            .into_response()
    }
}

#[derive(Deserialize)]
struct SessionQuery {
    symbols: String,
    #[serde(rename = "date")]
    trading_date: String,
}

#[derive(Deserialize)]
struct ChainQuery {
    symbol: String,
    #[serde(rename = "date")]
    trading_date: String,
    minute: String,
    expiration: String,
    #[serde(default = "default_pricing_mode")]
    pricing_mode: String,
    #[serde(default = "default_dealer_model")]
    dealer_model: String,
}

#[derive(Deserialize)]
struct SurfaceQuery {
    symbol: String,
    #[serde(rename = "date")]
    trading_date: String,
    minute: String,
    #[serde(default = "default_max_dte")]
    max_dte: i64,
}

#[derive(Deserialize)]
struct VolatilityQuery {
    symbol: String,
    #[serde(rename = "date")]
    trading_date: String,
    minute: String,
    expiration: String,
}

#[derive(Deserialize)]
struct ReplaySnapshotQuery {
    symbol: String,
    #[serde(rename = "date")]
    trading_date: String,
    minute: String,
    expiration: String,
    #[serde(default = "default_pricing_mode")]
    pricing_mode: String,
    #[serde(default = "default_dealer_model")]
    dealer_model: String,
    #[serde(default = "default_max_dte")]
    max_dte: i64,
}

#[derive(Deserialize)]
struct AuditListQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
}

fn default_pricing_mode() -> String {
    "micro".into()
}
fn default_dealer_model() -> String {
    "classic".into()
}
fn default_max_dte() -> i64 {
    180
}
fn default_audit_limit() -> usize {
    30
}

fn validate_minute(value: &str) -> Result<(), ApiError> {
    let Some((hour, minute)) = value.split_once(':') else {
        return Err(ApiError::bad_request("minute must use HH:MM"));
    };
    let hour: u8 = hour.parse().map_err(ApiError::bad_request)?;
    let minute: u8 = minute.parse().map_err(ApiError::bad_request)?;
    if hour > 23 || minute > 59 {
        return Err(ApiError::bad_request("invalid minute"));
    }
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let connection = state.live.status().await;
    let assistant = state.assistant.status();
    Json(json!({
        "ok": state.replay.root().is_dir(),
        "engine": "rust",
        "version": env!("CARGO_PKG_VERSION"),
        "longbridge_sdk": "4.4.1",
        "data_root": state.replay.root(),
        "audit_ledger": state.audit.path(),
        "live_connected": connection.connected,
        "assistant_enabled": assistant.enabled,
        "assistant_model": assistant.model,
    }))
}

async fn catalog(State(state): State<AppState>) -> Json<Value> {
    Json(state.replay.catalog())
}

async fn session(
    State(state): State<AppState>,
    Query(query): Query<SessionQuery>,
) -> Result<Json<Value>, ApiError> {
    state
        .replay
        .session(&query.symbols, &query.trading_date)
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn chain(
    State(state): State<AppState>,
    Query(query): Query<ChainQuery>,
) -> Result<Json<Value>, ApiError> {
    validate_minute(&query.minute)?;
    state
        .replay
        .chain(
            &query.symbol,
            &query.trading_date,
            &query.minute,
            &query.expiration,
            &query.pricing_mode,
            &query.dealer_model,
        )
        .and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn surface(
    State(state): State<AppState>,
    Query(query): Query<SurfaceQuery>,
) -> Result<Json<Value>, ApiError> {
    validate_minute(&query.minute)?;
    if !(1..=1000).contains(&query.max_dte) {
        return Err(ApiError::bad_request("max_dte must be between 1 and 1000"));
    }
    state
        .replay
        .surface(
            &query.symbol,
            &query.trading_date,
            &query.minute,
            query.max_dte,
        )
        .and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn volatility_context(
    State(state): State<AppState>,
    Query(query): Query<VolatilityQuery>,
) -> Result<Json<Value>, ApiError> {
    validate_minute(&query.minute)?;
    state
        .replay
        .volatility_context(
            &query.symbol,
            &query.trading_date,
            &query.minute,
            &query.expiration,
        )
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn replay_snapshot(
    State(state): State<AppState>,
    Query(query): Query<ReplaySnapshotQuery>,
) -> Result<Json<Value>, ApiError> {
    validate_minute(&query.minute)?;
    if !(1..=1000).contains(&query.max_dte) {
        return Err(ApiError::bad_request("max_dte must be between 1 and 1000"));
    }
    state
        .replay
        .snapshot(ReplaySnapshotParams {
            symbol: &query.symbol,
            trading_date: &query.trading_date,
            minute: &query.minute,
            expiration: &query.expiration,
            pricing_mode: &query.pricing_mode,
            dealer_model: &query.dealer_model,
            max_dte: query.max_dte,
        })
        .and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn connection_status(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(state.live.status().await).expect("serialize connection status"))
}

async fn oauth_status(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(state.live.oauth_status().await).expect("serialize OAuth status"))
}

async fn start_oauth(
    State(state): State<AppState>,
    Json(request): Json<OAuthStartRequest>,
) -> Result<Json<Value>, ApiError> {
    request.validate().map_err(ApiError::bad_request)?;
    state
        .live
        .start_oauth(request.client_id)
        .await
        .and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::upstream)
}

async fn connect_longbridge(
    State(state): State<AppState>,
    Json(credentials): Json<CredentialRequest>,
) -> Result<Json<Value>, ApiError> {
    credentials.validate().map_err(ApiError::bad_request)?;
    state
        .live
        .connect(credentials)
        .await
        .and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::upstream)
}

async fn disconnect_longbridge(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(state.live.disconnect().await).expect("serialize connection status"))
}

async fn setup_live_session(
    State(state): State<AppState>,
    Json(request): Json<LiveSessionRequest>,
) -> Result<Json<Value>, ApiError> {
    state
        .live
        .setup_session(request)
        .await
        .and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(|error| {
            let detail = format!("{error:#}");
            tracing::warn!(error = %detail, "live session setup failed");
            if error.to_string().contains("请先") {
                ApiError::conflict(detail)
            } else if let Some(retry_after_ms) = option_retry_after_ms(&detail)
                .or_else(|| detail.contains("301607").then_some(65_000))
            {
                ApiError::rate_limited(detail, retry_after_ms)
            } else {
                ApiError::upstream(detail)
            }
        })
}

async fn live_snapshot(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    state
        .live
        .snapshot()
        .await
        .and_then(|value| serde_json::to_value(value).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::conflict)
}

async fn live_volatility_context(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let snapshot = state.live.snapshot().await.map_err(ApiError::conflict)?;
    let closes = state
        .live
        .daily_closes(45)
        .await
        .map_err(ApiError::upstream)?;
    state
        .replay
        .live_volatility_context(&snapshot.chain, &closes)
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn strategy_analyze(
    State(state): State<AppState>,
    Json(request): Json<StrategyRequest>,
) -> Result<Json<Value>, ApiError> {
    let chain = if request.mode == "live" {
        let snapshot = state.live.snapshot().await.map_err(ApiError::conflict)?;
        if !request.symbol.eq_ignore_ascii_case(&snapshot.chain.symbol) {
            return Err(ApiError::conflict(
                "live symbol changed; refresh the strategy",
            ));
        }
        snapshot.chain
    } else if request.mode == "replay" {
        let date = request
            .date
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("date is required for replay analysis"))?;
        let minute = request
            .minute
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("minute is required for replay analysis"))?;
        let expiration = request
            .expiration
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("expiration is required for replay analysis"))?;
        validate_minute(minute)?;
        state
            .replay
            .chain(
                &request.symbol,
                date,
                minute,
                expiration,
                &request.pricing_mode,
                &request.dealer_model,
            )
            .map_err(ApiError::bad_request)?
    } else {
        return Err(ApiError::bad_request("mode must be live or replay"));
    };
    analyze_strategy(&chain, &request.legs, request.quantity)
        .and_then(|analysis| serde_json::to_value(analysis).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn audit_records(
    State(state): State<AppState>,
    Query(query): Query<AuditListQuery>,
) -> Result<Json<Value>, ApiError> {
    state
        .audit
        .list(query.limit)
        .await
        .and_then(|records| serde_json::to_value(records).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn audit_record(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    state
        .audit
        .get(&id)
        .await
        .and_then(|record| serde_json::to_value(record).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn append_audit_record(
    State(state): State<AppState>,
    Json(request): Json<AuditCaptureRequest>,
) -> Result<Json<Value>, ApiError> {
    state
        .audit
        .append(request)
        .await
        .and_then(|record| serde_json::to_value(record).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn assistant_status(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(state.assistant.status()).expect("serialize assistant status"))
}

async fn assistant_sessions(State(state): State<AppState>) -> Json<Value> {
    Json(
        serde_json::to_value(state.assistant.list_sessions().await)
            .expect("serialize assistant sessions"),
    )
}

async fn create_assistant_session(
    State(state): State<AppState>,
    Json(request): Json<AssistantCreateRequest>,
) -> Json<Value> {
    Json(
        serde_json::to_value(state.assistant.create_session(request).await)
            .expect("serialize assistant session"),
    )
}

async fn get_assistant_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    state
        .assistant
        .get_session(&id)
        .await
        .and_then(|session| serde_json::to_value(session).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn delete_assistant_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if state.assistant.delete_session(&id).await {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::bad_request("assistant session not found"))
    }
}

async fn resolve_assistant_context(
    state: &AppState,
    reference: AssistantSnapshotRef,
    strategy: Option<Value>,
) -> Result<AssistantContext, ApiError> {
    match reference {
        AssistantSnapshotRef::Replay {
            symbol,
            date,
            minute,
            expiration,
            pricing_mode,
            dealer_model,
            max_dte,
        } => {
            validate_minute(&minute)?;
            if !(1..=1000).contains(&max_dte) {
                return Err(ApiError::bad_request("max_dte must be between 1 and 1000"));
            }
            let snapshot = state
                .replay
                .snapshot(ReplaySnapshotParams {
                    symbol: &symbol,
                    trading_date: &date,
                    minute: &minute,
                    expiration: &expiration,
                    pricing_mode: &pricing_mode,
                    dealer_model: &dealer_model,
                    max_dte,
                })
                .map_err(ApiError::bad_request)?;
            let session = state
                .replay
                .session(&snapshot.symbol, &snapshot.date)
                .map_err(ApiError::bad_request)?;
            let bars = session["series"][&snapshot.symbol]["bars"].clone();
            let label = format!(
                "{} {} {} ET",
                snapshot.symbol, snapshot.date, snapshot.minute
            );
            let symbol = snapshot.symbol.clone();
            let snapshot_id = snapshot.snapshot_id.clone();
            let as_of = snapshot.as_of.clone();
            let model_version = Some(snapshot.model_version.clone());
            let payload = serde_json::to_value(snapshot).map_err(ApiError::bad_request)?;
            Ok(build_context(AssistantContextInput {
                label,
                mode: "replay".into(),
                symbol,
                snapshot_id,
                as_of,
                model_version,
                payload,
                market_bars: Some(bars),
                strategy,
            }))
        }
        AssistantSnapshotRef::Live => {
            let snapshot = state.live.snapshot().await.map_err(ApiError::conflict)?;
            let closes = state
                .live
                .daily_closes(45)
                .await
                .map_err(ApiError::upstream)?;
            let volatility = state
                .replay
                .live_volatility_context(&snapshot.chain, &closes)
                .map_err(ApiError::bad_request)?;
            let label = format!("{} LIVE {}", snapshot.chain.symbol, snapshot.chain.minute);
            let symbol = snapshot.chain.symbol.clone();
            let snapshot_id = snapshot.chain.snapshot_id.clone();
            let as_of = snapshot.feed.as_of.clone();
            let model_version = Some(snapshot.chain.provenance.model.clone());
            let bars = serde_json::to_value(&snapshot.bars).map_err(ApiError::bad_request)?;
            let mut payload = serde_json::to_value(snapshot).map_err(ApiError::bad_request)?;
            payload["volatility"] = volatility;
            Ok(build_context(AssistantContextInput {
                label,
                mode: "live".into(),
                symbol,
                snapshot_id,
                as_of,
                model_version,
                payload,
                market_bars: Some(bars),
                strategy,
            }))
        }
        AssistantSnapshotRef::Audit { record_id } => {
            let record = state
                .audit
                .get(&record_id)
                .await
                .map_err(ApiError::bad_request)?;
            if record.kind == "assistant_analysis" {
                return Err(ApiError::bad_request(
                    "收藏的助手分析请使用导入会话，不可作为市场截面",
                ));
            }
            let root = record.payload.get("snapshot").unwrap_or(&record.payload);
            let chain = root.get("chain").or_else(|| record.payload.get("chain"));
            let snapshot_id = record
                .snapshot_id
                .clone()
                .or_else(|| chain.and_then(|value| value["snapshot_id"].as_str().map(String::from)))
                .unwrap_or_else(|| format!("audit:{}", record.id));
            let as_of = chain
                .and_then(|value| {
                    value["timestamp"]
                        .as_str()
                        .or_else(|| value["as_of"].as_str())
                })
                .unwrap_or(&record.created_at)
                .to_string();
            let model_version = chain
                .and_then(|value| value["provenance"]["model"].as_str())
                .map(String::from);
            let bars = root
                .get("bars")
                .or_else(|| root.get("market_bars"))
                .cloned();
            Ok(build_context(AssistantContextInput {
                label: format!("收藏截面 {} · {}", record.symbol, &record.id[..8]),
                mode: record.mode,
                symbol: record.symbol,
                snapshot_id,
                as_of,
                model_version,
                payload: record.payload,
                market_bars: bars,
                strategy,
            }))
        }
    }
}

async fn assistant_chat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AssistantChatRequest>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    if request.context_refs.len() > 2 {
        return Err(ApiError::bad_request("最多附加两个截面"));
    }
    let mut contexts = Vec::with_capacity(request.context_refs.len());
    for reference in request.context_refs {
        contexts
            .push(resolve_assistant_context(&state, reference, request.strategy.clone()).await?);
    }
    let receiver = state
        .assistant
        .stream_chat(id, request.message, contexts)
        .await
        .map_err(ApiError::bad_request)?;
    let events = stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|event| {
            let item = Event::default()
                .event(event.event_name())
                .json_data(event)
                .unwrap_or_else(|_| Event::default().event("error").data("serialization error"));
            (Ok(item), receiver)
        })
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

async fn favorite_assistant_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let session = state
        .assistant
        .get_session(&id)
        .await
        .map_err(ApiError::bad_request)?;
    if session.messages.is_empty() {
        return Err(ApiError::bad_request("空会话不能收藏"));
    }
    let context = session.contexts.first();
    state
        .audit
        .append(AuditCaptureRequest {
            kind: "assistant_analysis".into(),
            mode: context
                .map(|item| item.mode.clone())
                .unwrap_or_else(|| "research".into()),
            symbol: context
                .map(|item| item.symbol.clone())
                .unwrap_or_else(|| "ASSISTANT".into()),
            snapshot_id: context.map(|item| item.snapshot_id.clone()),
            payload: json!({
                "session": session,
                "assistant": state.assistant.status(),
            }),
        })
        .await
        .and_then(|record| serde_json::to_value(record).map_err(anyhow::Error::from))
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn import_assistant_session(
    State(state): State<AppState>,
    Json(request): Json<AssistantImportRequest>,
) -> Result<Json<Value>, ApiError> {
    let record = state
        .audit
        .get(&request.audit_record_id)
        .await
        .map_err(ApiError::bad_request)?;
    if record.kind != "assistant_analysis" {
        return Err(ApiError::bad_request("该审计记录不是助手收藏"));
    }
    let session: AssistantSession =
        serde_json::from_value(record.payload["session"].clone()).map_err(ApiError::bad_request)?;
    let imported = state.assistant.import_session(session, record.id).await;
    serde_json::to_value(imported)
        .map(Json)
        .map_err(ApiError::bad_request)
}

async fn trade_account(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    state
        .live
        .trade_account()
        .await
        .map(Json)
        .map_err(ApiError::upstream)
}

async fn trade_orders(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    state
        .live
        .today_orders()
        .await
        .map(Json)
        .map_err(ApiError::upstream)
}

async fn submit_paper_orders(
    State(state): State<AppState>,
    Json(request): Json<PaperOrderRequest>,
) -> Result<Json<Value>, ApiError> {
    if request.strategy.mode != "live" {
        return Err(ApiError::bad_request(
            "paper orders require a live strategy",
        ));
    }
    let snapshot = state.live.snapshot().await.map_err(ApiError::conflict)?;
    if !request
        .strategy
        .symbol
        .eq_ignore_ascii_case(&snapshot.chain.symbol)
    {
        return Err(ApiError::conflict(
            "live symbol changed; create a new preview",
        ));
    }
    let analysis = analyze_strategy(
        &snapshot.chain,
        &request.strategy.legs,
        request.strategy.quantity,
    )
    .map_err(ApiError::bad_request)?;
    if analysis.preview_id != request.preview_id {
        return Err(ApiError::conflict(
            "strategy preview is stale; review the latest executable prices",
        ));
    }
    if !analysis.executable {
        return Err(ApiError::conflict(analysis.blockers.join("; ")));
    }
    let result = state
        .live
        .submit_paper_orders(
            &analysis.orders,
            &analysis.preview_id,
            &request.confirmation,
        )
        .await
        .map_err(ApiError::upstream)?;
    let _ = state
        .audit
        .append(AuditCaptureRequest {
            kind: "paper_order_submit".into(),
            mode: "live".into(),
            symbol: snapshot.chain.symbol,
            snapshot_id: Some(snapshot.chain.snapshot_id),
            payload: json!({"analysis": analysis, "result": result.clone()}),
        })
        .await;
    Ok(Json(result))
}

async fn cancel_paper_order(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let result = state
        .live
        .cancel_paper_order(&order_id)
        .await
        .map_err(ApiError::upstream)?;
    let _ = state
        .audit
        .append(AuditCaptureRequest {
            kind: "paper_order_cancel".into(),
            mode: "live".into(),
            symbol: "ACCOUNT".into(),
            snapshot_id: None,
            payload: result.clone(),
        })
        .await;
    Ok(Json(result))
}

async fn live_stream(
    State(state): State<AppState>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(move |socket| stream_socket(socket, state.live))
}

async fn stream_socket(socket: WebSocket, live: Arc<LiveManager>) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = live.subscribe();
    if let Ok(snapshot) = live.snapshot().await
        && sender
            .send(Message::Text(
                serde_json::to_string(&snapshot).unwrap().into(),
            ))
            .await
            .is_err()
    {
        return;
    }
    let mut last_sent = tokio::time::Instant::now() - Duration::from_secs(1);
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
                let elapsed = last_sent.elapsed();
                if elapsed < Duration::from_millis(200) {
                    tokio::time::sleep(Duration::from_millis(200) - elapsed).await;
                }
                let payload = match live.snapshot().await {
                    Ok(snapshot) => serde_json::to_string(&snapshot).unwrap(),
                    Err(error) => json!({"kind": "live_error", "detail": error.to_string()}).to_string(),
                };
                if sender.send(Message::Text(payload.into())).await.is_err() { break; }
                last_sent = tokio::time::Instant::now();
            }
            message = receiver.next() => {
                match message {
                    Some(Ok(Message::Ping(value)))
                        if sender.send(Message::Pong(value.clone())).await.is_err() => break,
                    Some(Ok(Message::Ping(_))) => {}
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

fn app(state: AppState, frontend_dist: PathBuf) -> Router {
    let index = frontend_dist.join("index.html");
    Router::new()
        .route("/api/health", get(health))
        .route("/api/catalog", get(catalog))
        .route("/api/session", get(session))
        .route("/api/chain", get(chain))
        .route("/api/surface", get(surface))
        .route("/api/volatility-context", get(volatility_context))
        .route("/api/replay/snapshot", get(replay_snapshot))
        .route("/api/v1/replay/snapshot", get(replay_snapshot))
        .route(
            "/api/connection",
            get(connection_status)
                .post(connect_longbridge)
                .delete(disconnect_longbridge),
        )
        .route("/api/oauth/status", get(oauth_status))
        .route("/api/oauth/start", post(start_oauth))
        .route("/api/live/session", post(setup_live_session))
        .route("/api/live/snapshot", get(live_snapshot))
        .route("/api/live/volatility-context", get(live_volatility_context))
        .route("/api/strategy/analyze", post(strategy_analyze))
        .route(
            "/api/audit/records",
            get(audit_records).post(append_audit_record),
        )
        .route("/api/audit/records/{id}", get(audit_record))
        .route("/api/assistant/status", get(assistant_status))
        .route(
            "/api/assistant/sessions",
            get(assistant_sessions).post(create_assistant_session),
        )
        .route(
            "/api/assistant/sessions/import",
            post(import_assistant_session),
        )
        .route(
            "/api/assistant/sessions/{id}",
            get(get_assistant_session).delete(delete_assistant_session),
        )
        .route(
            "/api/assistant/sessions/{id}/messages",
            post(assistant_chat),
        )
        .route(
            "/api/assistant/sessions/{id}/favorite",
            post(favorite_assistant_session),
        )
        .route("/api/trade/account", get(trade_account))
        .route(
            "/api/trade/orders",
            get(trade_orders).post(submit_paper_orders),
        )
        .route(
            "/api/trade/orders/{order_id}",
            axum::routing::delete(cancel_paper_order),
        )
        .route("/api/live/stream", get(live_stream))
        .fallback_service(ServeDir::new(frontend_dist).not_found_service(ServeFile::new(index)))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "option_workstation=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let data_root = env::var("OPTION_WORKSTATION_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| project_root.join("data"));
    let frontend_dist = env::var("OPTION_WORKSTATION_FRONTEND_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| project_root.join("frontend/dist"));
    let risk_free_rate = env::var("OPTION_WORKSTATION_RISK_FREE_RATE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.043);
    let host = env::var("OPTION_WORKSTATION_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = env::var("OPTION_WORKSTATION_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(7311);
    let live = LiveManager::new(risk_free_rate);
    live.start_refresh_loop();
    let audit_path = env::var("OPTION_WORKSTATION_AUDIT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("HOME").unwrap_or_else(|_| project_root.display().to_string());
            PathBuf::from(home)
                .join(".option-workstation")
                .join("audit.jsonl")
        });
    let state = AppState {
        replay: Arc::new(ReplayStore::new(data_root, risk_free_rate)),
        live,
        audit: Arc::new(AuditStore::new(audit_path)),
        assistant: Arc::new(AssistantManager::new()),
    };
    let address: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Option Workstation Rust service listening");
    axum::serve(listener, app(state, frontend_dist))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
