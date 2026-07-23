mod analytics;
mod audit;
mod live;
mod models;
mod replay;
mod strategy;
mod volatility;

use std::{env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    audit::{AuditCaptureRequest, AuditStore},
    live::{LiveManager, option_retry_after_ms},
    models::{CredentialRequest, LiveSessionRequest},
    replay::ReplayStore,
    strategy::{PaperOrderRequest, StrategyRequest, analyze_strategy},
};

#[derive(Clone)]
struct AppState {
    replay: Arc<ReplayStore>,
    live: Arc<LiveManager>,
    audit: Arc<AuditStore>,
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
    Json(json!({
        "ok": state.replay.root().is_dir(),
        "engine": "rust",
        "version": env!("CARGO_PKG_VERSION"),
        "longbridge_sdk": "4.4.1",
        "data_root": state.replay.root(),
        "audit_ledger": state.audit.path(),
        "live_connected": connection.connected,
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

async fn connection_status(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(state.live.status().await).expect("serialize connection status"))
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
        .route(
            "/api/connection",
            get(connection_status)
                .post(connect_longbridge)
                .delete(disconnect_longbridge),
        )
        .route("/api/live/session", post(setup_live_session))
        .route("/api/live/snapshot", get(live_snapshot))
        .route("/api/live/volatility-context", get(live_volatility_context))
        .route("/api/strategy/analyze", post(strategy_analyze))
        .route(
            "/api/audit/records",
            get(audit_records).post(append_audit_record),
        )
        .route("/api/audit/records/{id}", get(audit_record))
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
