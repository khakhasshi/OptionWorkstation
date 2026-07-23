use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use statrs::function::erf::erf;

use crate::{
    analytics::option_value,
    models::{ChainRow, ChainSnapshot},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StrategyLegInput {
    pub symbol: Option<String>,
    pub strike: f64,
    pub right: String,
    pub side: String,
    #[serde(default = "default_ratio")]
    pub ratio: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StrategyRequest {
    #[serde(default = "default_mode")]
    pub mode: String,
    pub symbol: String,
    pub date: Option<String>,
    pub minute: Option<String>,
    pub expiration: Option<String>,
    #[serde(default = "default_pricing_mode")]
    pub pricing_mode: String,
    #[serde(default = "default_dealer_model")]
    pub dealer_model: String,
    #[serde(default = "default_quantity")]
    pub quantity: u32,
    pub legs: Vec<StrategyLegInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaperOrderRequest {
    pub preview_id: String,
    pub confirmation: String,
    pub strategy: StrategyRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLeg {
    pub symbol: String,
    pub side: String,
    pub quantity: u32,
    pub limit_price: f64,
    pub bid: f64,
    pub ask: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyGreeks {
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub vanna: f64,
    pub charm: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioPoint {
    pub spot_shock_pct: f64,
    pub iv_shock_points: f64,
    pub elapsed_fraction: f64,
    pub pnl: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyAnalysis {
    pub preview_id: String,
    pub snapshot_id: String,
    pub symbol: String,
    pub expiration: String,
    pub quantity: u32,
    pub entry_cash_flow: f64,
    pub liquidation_value: f64,
    pub immediate_pnl: f64,
    pub max_profit: Option<f64>,
    pub max_loss: Option<f64>,
    pub margin_estimate: Option<f64>,
    pub break_evens: Vec<f64>,
    pub probability_of_profit: Option<f64>,
    pub greeks: StrategyGreeks,
    pub payoff: Vec<[f64; 2]>,
    pub scenarios: Vec<ScenarioPoint>,
    pub orders: Vec<ExecutionLeg>,
    pub executable: bool,
    pub blockers: Vec<String>,
    pub pricing_basis: String,
}

struct ResolvedLeg<'a> {
    request: &'a StrategyLegInput,
    row: &'a ChainRow,
    sign: f64,
    contracts: f64,
}

pub fn analyze_strategy(
    chain: &ChainSnapshot,
    legs: &[StrategyLegInput],
    quantity: u32,
) -> anyhow::Result<StrategyAnalysis> {
    anyhow::ensure!(
        !legs.is_empty() && legs.len() <= 8,
        "strategy requires 1-8 legs"
    );
    anyhow::ensure!(
        (1..=20).contains(&quantity),
        "quantity must be between 1 and 20"
    );
    let mut resolved = Vec::with_capacity(legs.len());
    let mut contracts = HashSet::new();
    for leg in legs {
        anyhow::ensure!(
            (1..=20).contains(&leg.ratio),
            "leg ratio must be between 1 and 20"
        );
        let right = leg.right.trim().to_uppercase();
        let side = leg.side.trim().to_uppercase();
        anyhow::ensure!(
            matches!(right.as_str(), "CALL" | "PUT"),
            "invalid option right"
        );
        anyhow::ensure!(
            matches!(side.as_str(), "BUY" | "SELL"),
            "invalid order side"
        );
        anyhow::ensure!(
            leg.ratio.saturating_mul(quantity) <= 100,
            "leg order quantity exceeds 100 contracts"
        );
        let row = if let Some(symbol) = leg.symbol.as_deref() {
            chain.rows.iter().find(|row| row.symbol == symbol)
        } else {
            chain
                .rows
                .iter()
                .find(|row| (row.strike - leg.strike).abs() < 1e-6 && row.right == right)
        }
        .ok_or_else(|| {
            anyhow::anyhow!("contract not present in snapshot: {right} {}", leg.strike)
        })?;
        anyhow::ensure!(
            (row.strike - leg.strike).abs() < 1e-6 && row.right == right,
            "contract symbol does not match strike/right"
        );
        anyhow::ensure!(
            contracts.insert(row.symbol.clone()),
            "duplicate contract in strategy: {}",
            row.symbol
        );
        resolved.push(ResolvedLeg {
            request: leg,
            row,
            sign: if side == "BUY" { 1.0 } else { -1.0 },
            contracts: leg.ratio as f64 * quantity as f64,
        });
    }

    let entry_cash_flow = resolved
        .iter()
        .map(|leg| {
            let price = if leg.sign > 0.0 {
                leg.row.ask
            } else {
                leg.row.bid
            };
            -leg.sign * leg.contracts * price * 100.0
        })
        .sum::<f64>();
    let liquidation_value = resolved
        .iter()
        .map(|leg| {
            let price = if leg.sign > 0.0 {
                leg.row.bid
            } else {
                leg.row.ask
            };
            leg.sign * leg.contracts * price * 100.0
        })
        .sum::<f64>();
    let payoff_at = |spot: f64| -> f64 {
        entry_cash_flow
            + resolved
                .iter()
                .map(|leg| {
                    let intrinsic = if leg.row.right == "CALL" {
                        (spot - leg.row.strike).max(0.0)
                    } else {
                        (leg.row.strike - spot).max(0.0)
                    };
                    leg.sign * leg.contracts * intrinsic * 100.0
                })
                .sum::<f64>()
    };
    let mut critical_spots = vec![0.0];
    critical_spots.extend(resolved.iter().map(|leg| leg.row.strike));
    critical_spots.sort_by(f64::total_cmp);
    critical_spots.dedup_by(|left, right| (*left - *right).abs() < 1e-8);
    let high_call_slope: f64 = resolved
        .iter()
        .filter(|leg| leg.row.right == "CALL")
        .map(|leg| leg.sign * leg.contracts * 100.0)
        .sum();
    let finite_values: Vec<f64> = critical_spots.iter().map(|spot| payoff_at(*spot)).collect();
    let mut break_evens = critical_spots
        .windows(2)
        .filter_map(|pair| {
            let (x0, x1) = (pair[0], pair[1]);
            let (y0, y1) = (payoff_at(x0), payoff_at(x1));
            if y0.abs() < 1e-8 {
                Some(x0)
            } else if y0 * y1 < 0.0 {
                Some(x0 + (x1 - x0) * y0.abs() / (y0.abs() + y1.abs()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if let Some(last_spot) = critical_spots.last().copied() {
        let last_value = payoff_at(last_spot);
        if last_value.abs() < 1e-8 {
            break_evens.push(last_spot);
        } else if high_call_slope.abs() > 1e-8 {
            let root = last_spot - last_value / high_call_slope;
            if root > last_spot {
                break_evens.push(root);
            }
        }
    }
    break_evens.sort_by(f64::total_cmp);
    break_evens.dedup_by(|left, right| (*left - *right).abs() < 1e-6);
    let break_evens: Vec<f64> = break_evens
        .into_iter()
        .map(|value| round(value, 3))
        .collect();
    let upper = break_evens.last().map_or(chain.spot * 2.0, |value| {
        (value * 1.15).max(chain.spot * 2.0)
    });
    let payoff: Vec<[f64; 2]> = (0..=160)
        .map(|index| {
            let spot = upper * index as f64 / 160.0;
            [round(spot, 3), round(payoff_at(spot), 2)]
        })
        .collect();
    let max_profit = (high_call_slope <= 0.0).then(|| {
        finite_values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
    });
    let max_loss = (high_call_slope >= 0.0)
        .then(|| finite_values.iter().copied().fold(f64::INFINITY, f64::min));
    let margin_estimate = max_loss.map(|value| value.abs());
    let greeks = StrategyGreeks {
        delta: round(sum_greek(&resolved, |row| row.delta), 3),
        gamma: round(sum_greek(&resolved, |row| row.gamma), 5),
        theta: round(sum_greek(&resolved, |row| row.theta), 3),
        vega: round(sum_greek(&resolved, |row| row.vega), 3),
        vanna: round(sum_greek(&resolved, |row| row.vanna), 3),
        charm: round(sum_greek(&resolved, |row| row.charm), 3),
    };
    let scenarios = scenario_matrix(chain, &resolved, entry_cash_flow);
    let probability_of_profit = probability_of_profit(chain, &payoff_at);
    let orders: Vec<ExecutionLeg> = resolved
        .iter()
        .map(|leg| ExecutionLeg {
            symbol: leg.row.symbol.clone(),
            side: if leg.sign > 0.0 { "BUY" } else { "SELL" }.into(),
            quantity: leg.request.ratio * quantity,
            limit_price: round(
                if leg.sign > 0.0 {
                    leg.row.ask
                } else {
                    leg.row.bid
                },
                4,
            ),
            bid: leg.row.bid,
            ask: leg.row.ask,
        })
        .collect();
    let mut blockers = Vec::new();
    if chain.quality.quote_coverage_pct < 80.0 {
        blockers.push(format!(
            "quote coverage {:.1}% is below 80%",
            chain.quality.quote_coverage_pct
        ));
    }
    if chain.quality.fresh_quote_coverage_pct < 80.0 {
        blockers.push(format!(
            "fresh quote coverage {:.1}% is below 80%",
            chain.quality.fresh_quote_coverage_pct
        ));
    }
    if chain.quality.spot_age_ms.is_some_and(|age| age > 5_000) {
        blockers.push(format!(
            "underlying quote age {}ms exceeds 5000ms",
            chain.quality.spot_age_ms.unwrap_or_default()
        ));
    }
    for leg in &resolved {
        if leg.row.bid <= 0.0 || leg.row.ask <= 0.0 || leg.row.ask < leg.row.bid {
            blockers.push(format!("invalid NBBO for {}", leg.row.symbol));
        }
        if leg.row.quality_score < 50.0 {
            blockers.push(format!("low quote quality for {}", leg.row.symbol));
        }
    }
    blockers.sort();
    blockers.dedup();
    let preview_key = serde_json::to_vec(&(
        &chain.snapshot_id,
        legs,
        quantity,
        round(entry_cash_flow, 2),
    ))?;
    let preview_id = hex::encode(Sha256::digest(preview_key))[..20].to_string();
    Ok(StrategyAnalysis {
        preview_id,
        snapshot_id: chain.snapshot_id.clone(),
        symbol: chain.symbol.clone(),
        expiration: chain.expiration.clone(),
        quantity,
        entry_cash_flow: round(entry_cash_flow, 2),
        liquidation_value: round(liquidation_value, 2),
        immediate_pnl: round(entry_cash_flow + liquidation_value, 2),
        max_profit: max_profit.map(|value| round(value, 2)),
        max_loss: max_loss.map(|value| round(value, 2)),
        margin_estimate: margin_estimate.map(|value| round(value, 2)),
        break_evens,
        probability_of_profit: probability_of_profit.map(|value| round(value * 100.0, 2)),
        greeks,
        payoff,
        scenarios,
        orders,
        executable: blockers.is_empty(),
        blockers,
        pricing_basis: "entry BUY@ask / SELL@bid; liquidation BUY@ask / SELL@bid".into(),
    })
}

fn sum_greek(resolved: &[ResolvedLeg<'_>], value: impl Fn(&ChainRow) -> f64) -> f64 {
    resolved
        .iter()
        .map(|leg| leg.sign * leg.contracts * value(leg.row) * 100.0)
        .sum()
}

fn scenario_matrix(
    chain: &ChainSnapshot,
    resolved: &[ResolvedLeg<'_>],
    entry_cash_flow: f64,
) -> Vec<ScenarioPoint> {
    let mut scenarios = Vec::new();
    for elapsed_fraction in [0.0, 0.5, 0.9] {
        let remaining =
            (chain.tte_years * (1.0 - elapsed_fraction)).max(1.0 / (365.0 * 24.0 * 60.0));
        for iv_shock_points in [-15.0, 0.0, 15.0] {
            for spot_shock_pct in [-10.0, -5.0, 0.0, 5.0, 10.0] {
                let spot = chain.spot * (1.0 + spot_shock_pct / 100.0);
                let value = resolved
                    .iter()
                    .map(|leg| {
                        let iv = ((leg.row.iv + iv_shock_points) / 100.0).clamp(0.01, 4.0);
                        leg.sign
                            * leg.contracts
                            * option_value(
                                spot,
                                leg.row.strike,
                                remaining,
                                iv,
                                &leg.row.right,
                                chain.provenance.risk_free_rate,
                                chain.provenance.dividend_yield,
                            )
                            * 100.0
                    })
                    .sum::<f64>();
                scenarios.push(ScenarioPoint {
                    spot_shock_pct,
                    iv_shock_points,
                    elapsed_fraction,
                    pnl: round(entry_cash_flow + value, 2),
                });
            }
        }
    }
    scenarios
}

fn probability_of_profit(chain: &ChainSnapshot, payoff_at: &impl Fn(f64) -> f64) -> Option<f64> {
    let sigma = chain.metrics.atm_iv? / 100.0;
    let years = chain.tte_years;
    if sigma <= 0.0 || years <= 0.0 {
        return None;
    }
    let lower = chain.spot * 0.2;
    let upper = chain.spot * 3.0;
    let steps = 2_000;
    let width = (upper - lower) / steps as f64;
    let drift =
        (chain.provenance.risk_free_rate - chain.provenance.dividend_yield - 0.5 * sigma * sigma)
            * years;
    let denominator = sigma * years.sqrt();
    let density = |price: f64| {
        let z = ((price / chain.spot).ln() - drift) / denominator;
        (-0.5 * z * z).exp() / (price * denominator * (2.0 * std::f64::consts::PI).sqrt())
    };
    let mut probability = 0.0;
    for index in 0..steps {
        let price = lower + (index as f64 + 0.5) * width;
        if payoff_at(price) > 0.0 {
            probability += density(price) * width;
        }
    }
    let tail_low = normal_cdf(((lower / chain.spot).ln() - drift) / denominator);
    let tail_high = 1.0 - normal_cdf(((upper / chain.spot).ln() - drift) / denominator);
    if payoff_at(0.0) > 0.0 {
        probability += tail_low;
    }
    if payoff_at(upper * 2.0) > 0.0 {
        probability += tail_high;
    }
    Some(probability.clamp(0.0, 1.0))
}

fn normal_cdf(value: f64) -> f64 {
    0.5 * (1.0 + erf(value / 2_f64.sqrt()))
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale).round() / scale
}

fn default_ratio() -> u32 {
    1
}
fn default_quantity() -> u32 {
    1
}
fn default_mode() -> String {
    "live".into()
}
fn default_pricing_mode() -> String {
    "micro".into()
}
fn default_dealer_model() -> String {
    "classic".into()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::models::{ChainMetrics, ChainQuality, Provenance};

    use super::*;

    #[test]
    fn executable_prices_include_the_spread() {
        let row = |symbol: &str, right: &str, strike: f64, bid: f64, ask: f64| ChainRow {
            symbol: symbol.into(),
            strike,
            right: right.into(),
            bid,
            ask,
            last: None,
            mark: (bid + ask) / 2.0,
            mid: (bid + ask) / 2.0,
            microprice: (bid + ask) / 2.0,
            spread: ask - bid,
            spread_pct: 5.0,
            bid_size: 10,
            ask_size: 10,
            volume: 10,
            open_interest: 100,
            iv: 25.0,
            delta: 0.5,
            gamma: 0.01,
            theta: -0.02,
            vega: 0.1,
            vanna: 0.0,
            charm: 0.0,
            gex: Some(1.0),
            moneyness: strike / 100.0,
            log_moneyness: (strike / 100.0).ln(),
            quality_score: 90.0,
            quality_flags: vec![],
        };
        let chain = ChainSnapshot {
            snapshot_id: "snapshot".into(),
            symbol: "SPY".into(),
            date: "2026-07-22".into(),
            minute: "10:00".into(),
            timestamp: "2026-07-22T14:00:00Z".into(),
            expiration: "2026-07-24".into(),
            spot: 100.0,
            forward: 100.0,
            pricing_mode: "micro".into(),
            dealer_model: "classic".into(),
            provenance: Provenance {
                source: "test".into(),
                quote_interval: "tick".into(),
                oi_frequency: "daily".into(),
                risk_free_rate: 0.04,
                dividend_yield: 0.0,
                forward_source: "test".into(),
                exercise_model: "test".into(),
                model: "test".into(),
                sdk_version: None,
            },
            dte: 2,
            tte_years: 2.0 / 365.0,
            metrics: ChainMetrics {
                contracts: 2,
                call_oi: Some(200),
                put_oi: Some(0),
                pcr: Some(0.0),
                net_gex: Some(2.0),
                call_wall: Some(100.0),
                put_wall: None,
                gamma_flip: None,
                atm_iv: Some(25.0),
                rr25: None,
                bf25: None,
                avg_quality: Some(90.0),
            },
            quality: ChainQuality {
                counts: BTreeMap::new(),
                usable_pct: 100.0,
                quote_coverage_pct: 100.0,
                fresh_quote_coverage_pct: 100.0,
                metadata_coverage_pct: 100.0,
                spot_age_ms: Some(0),
                gex_ready: true,
                blocked_metrics: vec![],
            },
            svi: None,
            svi_diagnostics: crate::models::SviDiagnostics {
                status: "test".into(),
                eligible_samples: 0,
                required_samples: 12,
                tte_minutes: 2.0 * 24.0 * 60.0,
                minimum_tte_minutes: 2.0,
                reason: None,
            },
            dealer_scenarios: vec![],
            gex_by_strike: vec![],
            rows: vec![
                row("C95", "CALL", 95.0, 6.0, 6.2),
                row("C105", "CALL", 105.0, 1.0, 1.2),
            ],
        };
        let analysis = analyze_strategy(
            &chain,
            &[
                StrategyLegInput {
                    symbol: Some("C95".into()),
                    strike: 95.0,
                    right: "CALL".into(),
                    side: "BUY".into(),
                    ratio: 1,
                },
                StrategyLegInput {
                    symbol: Some("C105".into()),
                    strike: 105.0,
                    right: "CALL".into(),
                    side: "SELL".into(),
                    ratio: 1,
                },
            ],
            1,
        )
        .unwrap();
        assert_eq!(analysis.entry_cash_flow, -520.0);
        assert_eq!(analysis.immediate_pnl, -40.0);
        assert!(analysis.max_loss.is_some());
        assert!(analysis.executable);
    }
}
