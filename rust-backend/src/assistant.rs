use std::{
    collections::{HashMap, HashSet},
    env,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::{Context, anyhow};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock, mpsc};

const MAX_SESSIONS: usize = 30;
const MAX_MESSAGES: usize = 40;
const MAX_CONTEXTS: usize = 2;
const MAX_MESSAGE_CHARS: usize = 8_000;
const MAX_PROVIDER_ERROR_CHARS: usize = 1_000;

const SYSTEM_PROMPT: &str = r#"你是 Option Workstation 的截面解盘助手。你只能依据附带的冻结市场截面和会话内容进行分析。

必须遵守：
1. 先检查数据时间、来源、新鲜度、覆盖率、质量门禁和模型警告；不可靠时先说明不能得出什么。
2. 明确区分观察事实、模型推断和未知信息。GEX、Gamma Flip、Dealer Exposure 是模型假设，不是真实交易商持仓。
3. 引用具体数值和对应 snapshot_id，不得编造快照中不存在的价格、成交或新闻。
4. 评价用户观点时分别给出支持证据、反对证据、失效条件和风险；方向正确不等于期权策略盈利。
5. 复核策略时检查每条腿、Bid/Ask、价差、流动性、最大亏损、盈亏平衡、Greeks、期限与波动率风险。
6. 有两个截面时，先说明时间与口径是否可比，再解释变化；不要把相关性写成因果。
7. 可以得出“不交易”或“证据不足”。不要调用交易接口，不要声称已下单，不承诺收益。

默认使用中文，结论简洁但证据完整。建议结构：结论、关键证据、反证与未知、策略表达、风险与失效条件。"#;

#[derive(Debug, Clone)]
struct AssistantConfig {
    enabled: bool,
    provider: String,
    base_url: String,
    api_key: Option<String>,
    model: String,
    mock: bool,
}

impl AssistantConfig {
    fn from_env() -> Self {
        let mock = env::var("OPTION_WORKSTATION_LLM_MOCK")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE"));
        let api_key = env::var("OPTION_WORKSTATION_LLM_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let model = env::var("OPTION_WORKSTATION_LLM_MODEL").unwrap_or_default();
        let base_url = env::var("OPTION_WORKSTATION_LLM_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into())
            .trim_end_matches('/')
            .to_string();
        Self {
            enabled: mock || (api_key.is_some() && !model.trim().is_empty()),
            provider: if mock {
                "mock".into()
            } else {
                "openai_compatible".into()
            },
            base_url,
            api_key,
            model: if mock && model.trim().is_empty() {
                "snapshot-assistant-test".into()
            } else {
                model
            },
            mock,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantStatus {
    pub enabled: bool,
    pub provider: String,
    pub model: Option<String>,
    pub streaming: bool,
    pub credential_storage: &'static str,
    pub retention: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantContext {
    pub id: String,
    pub label: String,
    pub mode: String,
    pub symbol: String,
    pub snapshot_id: String,
    pub as_of: String,
    pub model_version: Option<String>,
    pub quality_state: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub context_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantSession {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub contexts: Vec<AssistantContext>,
    pub messages: Vec<AssistantMessage>,
    pub imported_from: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantSessionSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    pub message_count: usize,
    pub context_count: usize,
    pub symbol: Option<String>,
}

pub struct AssistantContextInput {
    pub label: String,
    pub mode: String,
    pub symbol: String,
    pub snapshot_id: String,
    pub as_of: String,
    pub model_version: Option<String>,
    pub payload: Value,
    pub market_bars: Option<Value>,
    pub strategy: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantCreateRequest {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantImportRequest {
    pub audit_record_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssistantSnapshotRef {
    Replay {
        symbol: String,
        date: String,
        minute: String,
        expiration: String,
        #[serde(default = "default_pricing_mode")]
        pricing_mode: String,
        #[serde(default = "default_dealer_model")]
        dealer_model: String,
        #[serde(default = "default_max_dte")]
        max_dte: i64,
    },
    Live,
    Audit {
        record_id: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantChatRequest {
    pub message: String,
    #[serde(default)]
    pub context_refs: Vec<AssistantSnapshotRef>,
    pub strategy: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantStreamEvent {
    Meta {
        session_id: String,
        contexts: Vec<AssistantContext>,
    },
    Delta {
        content: String,
    },
    Done {
        message: AssistantMessage,
    },
    Error {
        detail: String,
    },
}

impl AssistantStreamEvent {
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Meta { .. } => "meta",
            Self::Delta { .. } => "delta",
            Self::Done { .. } => "done",
            Self::Error { .. } => "error",
        }
    }
}

pub struct AssistantManager {
    config: AssistantConfig,
    client: Client,
    sessions: RwLock<HashMap<String, AssistantSession>>,
    active_sessions: Mutex<HashSet<String>>,
    sequence: AtomicU64,
}

impl AssistantManager {
    pub fn new() -> Self {
        Self {
            config: AssistantConfig::from_env(),
            client: Client::builder()
                .timeout(Duration::from_secs(150))
                .build()
                .expect("build LLM HTTP client"),
            sessions: RwLock::new(HashMap::new()),
            active_sessions: Mutex::new(HashSet::new()),
            sequence: AtomicU64::new(0),
        }
    }

    pub fn status(&self) -> AssistantStatus {
        AssistantStatus {
            enabled: self.config.enabled,
            provider: self.config.provider.clone(),
            model: self
                .config
                .enabled
                .then(|| self.config.model.clone())
                .filter(|value| !value.is_empty()),
            streaming: true,
            credential_storage: "server_process_environment",
            retention: "process_memory_until_favorited",
        }
    }

    pub async fn create_session(&self, request: AssistantCreateRequest) -> AssistantSession {
        let id = self.next_id("assistant");
        let now = Utc::now().to_rfc3339();
        let title = request
            .title
            .unwrap_or_else(|| "新解盘".into())
            .trim()
            .chars()
            .take(80)
            .collect::<String>();
        let session = AssistantSession {
            id: id.clone(),
            title: if title.is_empty() {
                "新解盘".into()
            } else {
                title
            },
            created_at: now.clone(),
            updated_at: now,
            contexts: Vec::new(),
            messages: Vec::new(),
            imported_from: None,
        };
        let mut sessions = self.sessions.write().await;
        if sessions.len() >= MAX_SESSIONS
            && let Some(oldest) = sessions
                .values()
                .min_by(|left, right| left.updated_at.cmp(&right.updated_at))
                .map(|item| item.id.clone())
        {
            sessions.remove(&oldest);
        }
        sessions.insert(id, session.clone());
        session
    }

    pub async fn import_session(
        &self,
        mut session: AssistantSession,
        audit_record_id: String,
    ) -> AssistantSession {
        let id = self.next_id("assistant");
        let now = Utc::now().to_rfc3339();
        session.id = id.clone();
        session.title = format!("收藏 · {}", session.title)
            .chars()
            .take(80)
            .collect();
        session.created_at = now.clone();
        session.updated_at = now;
        session.imported_from = Some(audit_record_id);
        session.messages.truncate(MAX_MESSAGES);
        session.contexts.truncate(MAX_CONTEXTS);
        self.sessions.write().await.insert(id, session.clone());
        session
    }

    pub async fn list_sessions(&self) -> Vec<AssistantSessionSummary> {
        let sessions = self.sessions.read().await;
        let mut values = sessions
            .values()
            .map(|session| AssistantSessionSummary {
                id: session.id.clone(),
                title: session.title.clone(),
                updated_at: session.updated_at.clone(),
                message_count: session.messages.len(),
                context_count: session.contexts.len(),
                symbol: session.contexts.first().map(|item| item.symbol.clone()),
            })
            .collect::<Vec<_>>();
        values.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        values
    }

    pub async fn get_session(&self, id: &str) -> anyhow::Result<AssistantSession> {
        self.sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("assistant session not found"))
    }

    pub async fn delete_session(&self, id: &str) -> bool {
        if self.active_sessions.lock().await.contains(id) {
            return false;
        }
        self.sessions.write().await.remove(id).is_some()
    }

    pub async fn stream_chat(
        self: &std::sync::Arc<Self>,
        session_id: String,
        message: String,
        contexts: Vec<AssistantContext>,
    ) -> anyhow::Result<mpsc::Receiver<AssistantStreamEvent>> {
        anyhow::ensure!(
            self.config.enabled,
            "LLM 未配置；请在 Rust 服务端设置 OPTION_WORKSTATION_LLM_API_KEY 和 OPTION_WORKSTATION_LLM_MODEL"
        );
        validate_user_message(&message)?;
        anyhow::ensure!(contexts.len() <= MAX_CONTEXTS, "最多附加两个截面");
        anyhow::ensure!(
            self.active_sessions.lock().await.insert(session_id.clone()),
            "当前会话正在生成回答，请等待完成"
        );

        let session = {
            let mut sessions = self.sessions.write().await;
            let Some(session) = sessions.get_mut(&session_id) else {
                drop(sessions);
                self.active_sessions.lock().await.remove(&session_id);
                return Err(anyhow!("assistant session not found"));
            };
            if !contexts.is_empty() {
                session.contexts = deduplicate_contexts(contexts);
            }
            if session.contexts.is_empty() {
                drop(sessions);
                self.active_sessions.lock().await.remove(&session_id);
                return Err(anyhow!("请先附加一个市场截面"));
            }
            let now = Utc::now().to_rfc3339();
            let user_message = AssistantMessage {
                id: self.next_id("message"),
                role: "user".into(),
                content: message.trim().into(),
                created_at: now.clone(),
                context_ids: session
                    .contexts
                    .iter()
                    .map(|context| context.id.clone())
                    .collect(),
            };
            if session.messages.is_empty() {
                session.title = message.trim().chars().take(36).collect();
            }
            session.messages.push(user_message);
            if session.messages.len() > MAX_MESSAGES {
                let excess = session.messages.len() - MAX_MESSAGES;
                session.messages.drain(0..excess);
            }
            session.updated_at = now;
            session.clone()
        };

        let (sender, receiver) = mpsc::channel(128);
        sender
            .send(AssistantStreamEvent::Meta {
                session_id: session_id.clone(),
                contexts: session.contexts.clone(),
            })
            .await
            .ok();
        let manager = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let outcome = if manager.config.mock {
                manager.stream_mock(&session, &sender).await
            } else {
                manager.stream_provider(&session, &sender).await
            };
            match outcome {
                Ok(content) => {
                    let message = AssistantMessage {
                        id: manager.next_id("message"),
                        role: "assistant".into(),
                        content,
                        created_at: Utc::now().to_rfc3339(),
                        context_ids: session
                            .contexts
                            .iter()
                            .map(|context| context.id.clone())
                            .collect(),
                    };
                    if let Some(stored) = manager.sessions.write().await.get_mut(&session_id) {
                        stored.messages.push(message.clone());
                        trim_messages(stored);
                        stored.updated_at = message.created_at.clone();
                    }
                    let _ = sender.send(AssistantStreamEvent::Done { message }).await;
                }
                Err(error) => {
                    let detail = format!("{error:#}");
                    let message = AssistantMessage {
                        id: manager.next_id("message"),
                        role: "error".into(),
                        content: format!("请求失败：{detail}"),
                        created_at: Utc::now().to_rfc3339(),
                        context_ids: session
                            .contexts
                            .iter()
                            .map(|context| context.id.clone())
                            .collect(),
                    };
                    if let Some(stored) = manager.sessions.write().await.get_mut(&session_id) {
                        stored.messages.push(message);
                        trim_messages(stored);
                        stored.updated_at = Utc::now().to_rfc3339();
                    }
                    let _ = sender.send(AssistantStreamEvent::Error { detail }).await;
                }
            }
            manager.active_sessions.lock().await.remove(&session_id);
        });
        Ok(receiver)
    }

    async fn stream_mock(
        &self,
        session: &AssistantSession,
        sender: &mpsc::Sender<AssistantStreamEvent>,
    ) -> anyhow::Result<String> {
        let contexts = session
            .contexts
            .iter()
            .map(|context| {
                format!(
                    "{} {} {} Q={}",
                    context.symbol, context.snapshot_id, context.as_of, context.quality_state
                )
            })
            .collect::<Vec<_>>()
            .join("；");
        let content = format!(
            "结论\n测试模式已接收并冻结 {} 个截面：{}。\n\n关键证据\n上下文包含期权链、波动率、曲面、Dealer Exposure、数据质量与策略风险字段。实际部署时将由配置的 OpenAI-compatible 模型基于这些证据解盘。\n\n反证与未知\n测试模式不生成投资判断，也不会调用交易接口。",
            session.contexts.len(),
            contexts
        );
        let characters = content.chars().collect::<Vec<_>>();
        for chunk in characters.chunks(12) {
            let text = chunk.iter().collect::<String>();
            sender
                .send(AssistantStreamEvent::Delta { content: text })
                .await
                .map_err(|_| anyhow!("assistant client disconnected"))?;
        }
        Ok(content)
    }

    async fn stream_provider(
        &self,
        session: &AssistantSession,
        sender: &mpsc::Sender<AssistantStreamEvent>,
    ) -> anyhow::Result<String> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("LLM API key is unavailable"))?;
        let messages = provider_messages(session)?;
        let request_body = provider_request_body(&self.config, messages);
        let response = self
            .client
            .post(format!("{}/chat/completions", self.config.base_url))
            .bearer_auth(api_key)
            .json(&request_body)
            .send()
            .await
            .context("request OpenAI-compatible LLM")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "LLM upstream returned {status}: {}",
                detail
                    .chars()
                    .take(MAX_PROVIDER_ERROR_CHARS)
                    .collect::<String>()
            ));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut answer = String::new();
        let mut finished = false;
        while !finished {
            let Some(chunk) = stream.next().await else {
                break;
            };
            let chunk = chunk.context("read LLM stream")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find('\n') {
                let line = buffer[..index].trim().to_string();
                buffer.drain(..=index);
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    finished = true;
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                let delta = value["choices"][0]["delta"]["content"]
                    .as_str()
                    .or_else(|| value["choices"][0]["message"]["content"].as_str())
                    .unwrap_or_default();
                if delta.is_empty() {
                    continue;
                }
                answer.push_str(delta);
                sender
                    .send(AssistantStreamEvent::Delta {
                        content: delta.into(),
                    })
                    .await
                    .map_err(|_| anyhow!("assistant client disconnected"))?;
            }
        }
        anyhow::ensure!(!answer.trim().is_empty(), "LLM returned an empty response");
        Ok(answer)
    }

    fn next_id(&self, prefix: &str) -> String {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{prefix}-{}-{sequence}", Utc::now().timestamp_millis())
    }
}

pub fn build_context(input: AssistantContextInput) -> AssistantContext {
    let compact = compact_payload(&input.payload, input.market_bars, input.strategy);
    let quality_state = quality_state(&compact);
    let digest = Sha256::digest(
        serde_json::to_vec(&json!({
            "snapshot_id": &input.snapshot_id,
            "as_of": &input.as_of,
            "payload": &compact,
        }))
        .unwrap_or_default(),
    );
    AssistantContext {
        id: format!("context:{}", &hex::encode(digest)[..20]),
        label: input.label,
        mode: input.mode,
        symbol: input.symbol,
        snapshot_id: input.snapshot_id,
        as_of: input.as_of,
        model_version: input.model_version,
        quality_state,
        payload: compact,
    }
}

fn compact_payload(payload: &Value, market_bars: Option<Value>, strategy: Option<Value>) -> Value {
    let root = payload.get("snapshot").unwrap_or(payload);
    let chain = root.get("chain").or_else(|| payload.get("chain"));
    let surface = root.get("surface").or_else(|| payload.get("surface"));
    let volatility = root.get("volatility").or_else(|| payload.get("volatility"));
    let feed = payload.get("feed").or_else(|| root.get("feed"));

    let compact_chain = chain.map(|chain| {
        let rows = chain["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        json!({
            "snapshot_id": chain["snapshot_id"],
            "symbol": chain["symbol"],
            "date": chain["date"],
            "minute": chain["minute"],
            "timestamp": chain["timestamp"],
            "expiration": chain["expiration"],
            "spot": chain["spot"],
            "forward": chain["forward"],
            "dte": chain["dte"],
            "tte_years": chain["tte_years"],
            "pricing_mode": chain["pricing_mode"],
            "dealer_model": chain["dealer_model"],
            "metrics": chain["metrics"],
            "quality": chain["quality"],
            "provenance": chain["provenance"],
            "svi": compact_svi(chain.get("svi")),
            "svi_diagnostics": chain["svi_diagnostics"],
            "gex_by_strike": chain["gex_by_strike"],
            "dealer_scenarios": chain["dealer_scenarios"],
            "chain_row_count": rows.len(),
            "key_chain_rows": select_chain_rows(rows, chain["spot"].as_f64().unwrap_or_default()),
        })
    });

    let compact_surface = surface.map(|surface| {
        json!({
            "symbol": surface["symbol"],
            "date": surface["date"],
            "minute": surface["minute"],
            "timestamp": surface["timestamp"],
            "spot": surface["spot"],
            "term": surface["term"],
            "arbitrage": surface["arbitrage"],
            "svi_slices": surface["svi_slices"].as_array().map(|items| {
                items.iter().map(compact_svi_slice).collect::<Vec<_>>()
            }).unwrap_or_default(),
            "surface_grid": downsample_grid(surface["grid"].as_array()),
            "surface_point_count": surface["points"].as_array().map(Vec::len).unwrap_or_default(),
        })
    });

    let bars = market_bars
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let market_bars = bars
        .into_iter()
        .rev()
        .take(90)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    json!({
        "chain": compact_chain,
        "surface": compact_surface,
        "volatility": volatility,
        "feed": feed,
        "market_bars": market_bars,
        "strategy": strategy.or_else(|| payload.get("strategy").cloned()),
        "context_policy": {
            "frozen": true,
            "max_key_chain_rows": 96,
            "market_bar_window": 90,
            "dealer_exposure_is_model_inference": true,
            "raw_chain_available_in_source_snapshot": true,
        },
    })
}

fn compact_svi(value: Option<&Value>) -> Value {
    value
        .map(|svi| {
            json!({
                "params": svi["params"],
                "rmse_total_variance": svi["rmse_total_variance"],
                "butterfly_violations": svi["butterfly_violations"],
                "residual_count": svi["residuals"].as_array().map(Vec::len).unwrap_or_default(),
                "curve": svi["curve"].as_array().map(|curve| {
                    curve.iter().step_by((curve.len() / 15).max(1)).cloned().collect::<Vec<_>>()
                }).unwrap_or_default(),
            })
        })
        .unwrap_or(Value::Null)
}

fn compact_svi_slice(value: &Value) -> Value {
    json!({
        "expiration": value["expiration"],
        "dte": value["dte"],
        "params": value["params"],
        "rmse_total_variance": value["rmse_total_variance"],
        "butterfly_violations": value["butterfly_violations"],
    })
}

fn downsample_grid(rows: Option<&Vec<Value>>) -> Vec<Value> {
    let Some(rows) = rows else {
        return Vec::new();
    };
    let row_step = (rows.len() / 7).max(1);
    rows.iter()
        .step_by(row_step)
        .map(|row| {
            row.as_array()
                .map(|cells| {
                    cells
                        .iter()
                        .step_by((cells.len() / 12).max(1))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .map(Value::Array)
                .unwrap_or(Value::Null)
        })
        .collect()
}

fn select_chain_rows(rows: &[Value], spot: f64) -> Vec<Value> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut indexes = HashSet::new();
    let mut by_distance = (0..rows.len()).collect::<Vec<_>>();
    by_distance.sort_by(|left, right| {
        distance_to_spot(&rows[*left], spot).total_cmp(&distance_to_spot(&rows[*right], spot))
    });
    indexes.extend(by_distance.into_iter().take(48));

    let mut by_oi = (0..rows.len()).collect::<Vec<_>>();
    by_oi.sort_by(|left, right| {
        numeric(&rows[*right], "open_interest").total_cmp(&numeric(&rows[*left], "open_interest"))
    });
    indexes.extend(by_oi.into_iter().take(20));

    let mut by_gex = (0..rows.len()).collect::<Vec<_>>();
    by_gex.sort_by(|left, right| {
        numeric(&rows[*right], "gex")
            .abs()
            .total_cmp(&numeric(&rows[*left], "gex").abs())
    });
    indexes.extend(by_gex.into_iter().take(20));

    for right in ["CALL", "PUT"] {
        if let Some((index, _)) = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row["right"].as_str() == Some(right))
            .map(|(index, row)| (index, (numeric(row, "delta").abs() - 0.25).abs()))
            .min_by(|left, right| left.1.total_cmp(&right.1))
        {
            indexes.insert(index);
        }
    }

    let mut selected = indexes.into_iter().collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        numeric(&rows[*left], "strike")
            .total_cmp(&numeric(&rows[*right], "strike"))
            .then_with(|| {
                rows[*left]["right"]
                    .as_str()
                    .cmp(&rows[*right]["right"].as_str())
            })
    });
    selected
        .into_iter()
        .take(96)
        .map(|index| project_chain_row(&rows[index]))
        .collect()
}

fn project_chain_row(row: &Value) -> Value {
    const FIELDS: &[&str] = &[
        "symbol",
        "strike",
        "right",
        "bid",
        "ask",
        "mark",
        "spread",
        "spread_pct",
        "bid_size",
        "ask_size",
        "volume",
        "open_interest",
        "iv",
        "delta",
        "gamma",
        "theta",
        "vega",
        "vanna",
        "charm",
        "gex",
        "moneyness",
        "quality_score",
        "quality_flags",
    ];
    let mut projected = Map::new();
    for field in FIELDS {
        if let Some(value) = row.get(*field) {
            projected.insert((*field).into(), value.clone());
        }
    }
    Value::Object(projected)
}

fn distance_to_spot(row: &Value, spot: f64) -> f64 {
    (numeric(row, "strike") - spot).abs()
}

fn numeric(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap_or_default()
}

fn quality_state(payload: &Value) -> String {
    let quality = &payload["chain"]["quality"];
    if quality["gex_ready"].as_bool() == Some(false) {
        return "limited".into();
    }
    let fresh = quality["fresh_quote_coverage_pct"]
        .as_f64()
        .unwrap_or_default();
    let quote = quality["quote_coverage_pct"].as_f64().unwrap_or_default();
    if fresh >= 80.0 && quote >= 80.0 {
        "ready".into()
    } else if fresh > 0.0 || quote > 0.0 {
        "partial".into()
    } else {
        "unknown".into()
    }
}

fn provider_messages(session: &AssistantSession) -> anyhow::Result<Vec<Value>> {
    let context_json = serde_json::to_string(&session.contexts)?;
    anyhow::ensure!(
        context_json.len() <= 500_000,
        "assistant context exceeds the provider safety limit"
    );
    let mut messages = vec![
        json!({"role": "system", "content": SYSTEM_PROMPT}),
        json!({
            "role": "system",
            "content": format!(
                "以下是服务器冻结并压缩的权威截面上下文。每个上下文保留全部面板指标、关键链行、曲面采样、行情窗口和策略风险；原始链行数在 chain_row_count 中。\\n{context_json}"
            )
        }),
    ];
    messages.extend(
        session
            .messages
            .iter()
            .filter(|message| matches!(message.role.as_str(), "user" | "assistant"))
            .map(|message| {
                json!({
                    "role": message.role,
                    "content": message.content,
                })
            }),
    );
    Ok(messages)
}

fn validate_user_message(message: &str) -> anyhow::Result<()> {
    let trimmed = message.trim();
    anyhow::ensure!(!trimmed.is_empty(), "消息不能为空");
    anyhow::ensure!(
        trimmed.chars().count() <= MAX_MESSAGE_CHARS,
        "消息最多 8000 个字符"
    );
    let lower = trimmed.to_ascii_lowercase();
    anyhow::ensure!(
        ![
            "app_secret=",
            "access_token=",
            "api_key=",
            "authorization: bearer",
            "hk_m_",
        ]
        .iter()
        .any(|marker| lower.contains(marker)),
        "消息疑似包含凭证，请移除后再发送"
    );
    Ok(())
}

fn deduplicate_contexts(contexts: Vec<AssistantContext>) -> Vec<AssistantContext> {
    let mut ids = HashSet::new();
    contexts
        .into_iter()
        .filter(|context| ids.insert(context.id.clone()))
        .take(MAX_CONTEXTS)
        .collect()
}

fn trim_messages(session: &mut AssistantSession) {
    if session.messages.len() > MAX_MESSAGES {
        let excess = session.messages.len() - MAX_MESSAGES;
        session.messages.drain(0..excess);
    }
}

fn provider_request_body(config: &AssistantConfig, messages: Vec<Value>) -> Value {
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "temperature": 0.2,
        "stream": true,
    });
    if config.base_url.contains("deepseek") || config.model.starts_with("deepseek") {
        body["thinking"] = json!({"type": "disabled"});
    }
    body
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn mock_manager() -> AssistantManager {
        AssistantManager {
            config: AssistantConfig {
                enabled: true,
                provider: "mock".into(),
                base_url: "http://127.0.0.1".into(),
                api_key: None,
                model: "snapshot-assistant-test".into(),
                mock: true,
            },
            client: Client::new(),
            sessions: RwLock::new(HashMap::new()),
            active_sessions: Mutex::new(HashSet::new()),
            sequence: AtomicU64::new(0),
        }
    }

    fn sample_context() -> AssistantContext {
        build_context(AssistantContextInput {
            label: "SPY 10:00".into(),
            mode: "replay".into(),
            symbol: "SPY".into(),
            snapshot_id: "replay:test".into(),
            as_of: "2026-07-10T14:00:00Z".into(),
            model_version: Some("BSM+SVI-v1".into()),
            payload: json!({
                "chain": {
                    "snapshot_id": "test",
                    "symbol": "SPY",
                    "spot": 100.0,
                    "metrics": {"atm_iv": 20.0},
                    "quality": {"gex_ready": true, "quote_coverage_pct": 100.0, "fresh_quote_coverage_pct": 100.0},
                    "rows": (0..150).map(|index| json!({
                        "strike": 70.0 + index as f64 * 0.5,
                        "right": if index % 2 == 0 {"CALL"} else {"PUT"},
                        "bid": 1.0,
                        "ask": 1.1,
                        "open_interest": index,
                        "gex": index as f64 * 100.0,
                        "delta": 0.25,
                    })).collect::<Vec<_>>()
                },
                "surface": {"grid": [], "term": [], "arbitrage": {}},
                "volatility": {"iv_rank": 50.0}
            }),
            market_bars: None,
            strategy: None,
        })
    }

    #[test]
    fn context_compaction_bounds_chain_rows() {
        let context = sample_context();
        assert_eq!(context.quality_state, "ready");
        assert!(
            context.payload["chain"]["key_chain_rows"]
                .as_array()
                .unwrap()
                .len()
                <= 96
        );
        assert_eq!(context.payload["chain"]["chain_row_count"], 150);
    }

    #[test]
    fn credential_like_messages_are_rejected() {
        assert!(validate_user_message("分析这个截面").is_ok());
        assert!(validate_user_message("access_token=secret").is_err());
        assert!(validate_user_message("hk_m_example-token").is_err());
    }

    #[test]
    fn deepseek_requests_disable_reasoning_mode() {
        let mut manager = mock_manager();
        manager.config.base_url = "https://api.deepseek.com".into();
        manager.config.model = "deepseek-v4-flash".into();
        let body = provider_request_body(&manager.config, vec![]);
        assert_eq!(body["thinking"]["type"], "disabled");
    }

    #[tokio::test]
    async fn mock_provider_streams_and_persists_a_reply() {
        let manager = Arc::new(mock_manager());
        let session = manager
            .create_session(AssistantCreateRequest { title: None })
            .await;
        let mut receiver = manager
            .stream_chat(
                session.id.clone(),
                "分析这个截面".into(),
                vec![sample_context()],
            )
            .await
            .unwrap();
        let mut done = false;
        while let Some(event) = receiver.recv().await {
            if matches!(event, AssistantStreamEvent::Done { .. }) {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(
            manager
                .get_session(&session.id)
                .await
                .unwrap()
                .messages
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn active_session_rejects_duplicate_turn_without_mutating_history() {
        let manager = Arc::new(mock_manager());
        let session = manager
            .create_session(AssistantCreateRequest { title: None })
            .await;
        manager
            .active_sessions
            .lock()
            .await
            .insert(session.id.clone());
        let result = manager
            .stream_chat(
                session.id.clone(),
                "重复请求".into(),
                vec![sample_context()],
            )
            .await;
        assert!(result.is_err());
        assert!(
            manager
                .get_session(&session.id)
                .await
                .unwrap()
                .messages
                .is_empty()
        );
    }
}
