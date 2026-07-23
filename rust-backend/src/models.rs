use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bar {
    pub time: String,
    pub timestamp: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub vwap: f64,
}

#[derive(Debug, Clone)]
pub struct RawOptionQuote {
    pub symbol: String,
    pub strike: f64,
    pub right: String,
    pub bid_size: i64,
    pub ask_size: i64,
    pub bid: f64,
    pub ask: f64,
    pub last: Option<f64>,
    pub volume: i64,
    pub open_interest: i64,
    pub sdk_iv: Option<f64>,
    pub sdk_delta: Option<f64>,
    pub sdk_gamma: Option<f64>,
    pub sdk_theta: Option<f64>,
    pub sdk_vega: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainRow {
    pub symbol: String,
    pub strike: f64,
    pub right: String,
    pub bid: f64,
    pub ask: f64,
    pub last: Option<f64>,
    pub mark: f64,
    pub mid: f64,
    pub microprice: f64,
    pub spread: f64,
    pub spread_pct: f64,
    pub bid_size: i64,
    pub ask_size: i64,
    pub volume: i64,
    pub open_interest: i64,
    pub iv: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub vanna: f64,
    pub charm: f64,
    pub gex: Option<f64>,
    pub moneyness: f64,
    pub log_moneyness: f64,
    pub quality_score: f64,
    pub quality_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SviPoint {
    pub k: f64,
    pub moneyness: f64,
    pub iv: f64,
    pub density: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SviResidual {
    pub k: f64,
    pub observed_iv: f64,
    pub fitted_iv: f64,
    pub residual: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SviFit {
    pub params: BTreeMap<String, f64>,
    pub rmse_total_variance: f64,
    pub butterfly_violations: usize,
    pub curve: Vec<SviPoint>,
    pub residuals: Vec<SviResidual>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SviDiagnostics {
    pub status: String,
    pub eligible_samples: usize,
    pub required_samples: usize,
    pub tte_minutes: f64,
    pub minimum_tte_minutes: f64,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainMetrics {
    pub contracts: usize,
    pub call_oi: Option<i64>,
    pub put_oi: Option<i64>,
    pub pcr: Option<f64>,
    pub net_gex: Option<f64>,
    pub call_wall: Option<f64>,
    pub put_wall: Option<f64>,
    pub gamma_flip: Option<f64>,
    pub atm_iv: Option<f64>,
    pub rr25: Option<f64>,
    pub bf25: Option<f64>,
    pub avg_quality: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainQuality {
    pub counts: BTreeMap<String, usize>,
    pub usable_pct: f64,
    pub quote_coverage_pct: f64,
    pub fresh_quote_coverage_pct: f64,
    pub metadata_coverage_pct: f64,
    pub spot_age_ms: Option<i64>,
    pub gex_ready: bool,
    pub blocked_metrics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GexPoint {
    pub strike: f64,
    pub gex: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DealerScenario {
    pub spot: f64,
    pub gex: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub quote_interval: String,
    pub oi_frequency: String,
    pub risk_free_rate: f64,
    pub dividend_yield: f64,
    pub forward_source: String,
    pub exercise_model: String,
    pub model: String,
    pub sdk_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainSnapshot {
    pub snapshot_id: String,
    pub symbol: String,
    pub date: String,
    pub minute: String,
    pub timestamp: String,
    pub expiration: String,
    pub spot: f64,
    pub forward: f64,
    pub pricing_mode: String,
    pub dealer_model: String,
    pub provenance: Provenance,
    pub dte: i64,
    pub tte_years: f64,
    pub metrics: ChainMetrics,
    pub quality: ChainQuality,
    pub svi: Option<SviFit>,
    pub svi_diagnostics: SviDiagnostics,
    pub dealer_scenarios: Vec<DealerScenario>,
    pub rows: Vec<ChainRow>,
    pub gex_by_strike: Vec<GexPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfacePoint {
    pub moneyness: f64,
    pub dte: i64,
    pub tte_days: f64,
    pub iv: f64,
    pub right: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermPoint {
    pub expiration: String,
    pub dte: i64,
    pub iv: f64,
    pub net_gex: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceArbitrage {
    pub calendar_violations: usize,
    pub butterfly_violations: usize,
    pub post_projection_calendar_violations: usize,
    pub post_projection_convexity_violations: usize,
    pub convexity_adjustments: usize,
    pub calendar_adjustments: usize,
    pub price_monotonicity_violations: usize,
    pub price_convexity_violations: usize,
    pub confidence_score: f64,
    pub trusted: bool,
    pub projection_model: String,
    pub warning: Option<String>,
}

impl Default for SurfaceArbitrage {
    fn default() -> Self {
        Self {
            calendar_violations: 0,
            butterfly_violations: 0,
            post_projection_calendar_violations: 0,
            post_projection_convexity_violations: 0,
            convexity_adjustments: 0,
            calendar_adjustments: 0,
            price_monotonicity_violations: 0,
            price_convexity_violations: 0,
            confidence_score: 0.0,
            trusted: false,
            projection_model: "constrained_total_variance_v2".into(),
            warning: Some("insufficient surface data".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceSnapshot {
    pub symbol: String,
    pub date: String,
    pub minute: String,
    pub timestamp: String,
    pub spot: f64,
    pub points: Vec<SurfacePoint>,
    pub grid_raw: Vec<Vec<[f64; 3]>>,
    pub grid: Vec<Vec<[f64; 3]>>,
    pub term: Vec<TermPoint>,
    pub svi_slices: Vec<serde_json::Value>,
    pub arbitrage: SurfaceArbitrage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialRequest {
    pub app_key: String,
    pub app_secret: String,
    pub access_token: String,
}

impl CredentialRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.app_key.trim().is_empty()
            || self.app_secret.trim().is_empty()
            || self.access_token.trim().is_empty()
        {
            return Err("App Key、App Secret 和 Access Token 均为必填项");
        }
        if self.app_key.len() > 256 || self.app_secret.len() > 512 || self.access_token.len() > 4096
        {
            return Err("凭证字段长度异常");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStatus {
    pub connected: bool,
    pub state: String,
    pub account_hint: Option<String>,
    pub quote_level: Option<String>,
    pub packages: Vec<String>,
    pub subscribed_contracts: usize,
    pub last_event_at: Option<String>,
    pub error: Option<String>,
    pub credential_storage: &'static str,
    pub trade_connected: bool,
    pub paper_account: bool,
    pub account_type: Option<String>,
    pub buy_power: Option<String>,
    pub order_execution_enabled: bool,
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self {
            connected: false,
            state: "disconnected".into(),
            account_hint: None,
            quote_level: None,
            packages: Vec::new(),
            subscribed_contracts: 0,
            last_event_at: None,
            error: None,
            credential_storage: "process_memory_only",
            trade_connected: false,
            paper_account: false,
            account_type: None,
            buy_power: None,
            order_execution_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveSessionRequest {
    pub symbol: String,
    pub expiration: Option<String>,
    #[serde(default = "default_contract_limit")]
    pub max_contracts: usize,
    #[serde(default = "default_surface_expiries")]
    pub surface_expiries: usize,
    #[serde(default = "default_moneyness")]
    pub moneyness_window: f64,
    #[serde(default = "default_pricing_mode")]
    pub pricing_mode: String,
    #[serde(default = "default_dealer_model")]
    pub dealer_model: String,
}

fn default_contract_limit() -> usize {
    420
}
fn default_surface_expiries() -> usize {
    4
}
fn default_moneyness() -> f64 {
    0.12
}
fn default_pricing_mode() -> String {
    "micro".into()
}
fn default_dealer_model() -> String {
    "classic".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveFeedInfo {
    pub source: &'static str,
    pub transport: &'static str,
    pub sdk_version: &'static str,
    pub symbol: String,
    pub expiration: String,
    pub expirations: Vec<String>,
    pub subscribed_contracts: usize,
    pub quote_contracts: usize,
    pub metadata_contracts: usize,
    pub quote_coverage_pct: f64,
    pub fresh_quote_coverage_pct: f64,
    pub metadata_coverage_pct: f64,
    pub subscription_limit: usize,
    pub as_of: String,
    pub stale_after_ms: u64,
    pub latency_ms: i64,
    pub quality_state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveSnapshot {
    pub kind: &'static str,
    pub sequence: u64,
    pub feed: LiveFeedInfo,
    pub bars: Vec<Bar>,
    pub chain: ChainSnapshot,
    pub surface: SurfaceSnapshot,
}
