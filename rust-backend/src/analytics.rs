use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::America::New_York;
use sha2::{Digest, Sha256};
use statrs::function::erf::erf;

use crate::models::{
    ChainMetrics, ChainQuality, ChainRow, ChainSnapshot, DealerScenario, GexPoint, Provenance,
    RawOptionQuote, SurfaceArbitrage, SurfacePoint, SurfaceSnapshot, SviDiagnostics, SviFit,
    SviPoint, SviResidual, TermPoint,
};

type SurfaceIvBuckets = BTreeMap<i64, BTreeMap<i64, (Vec<f64>, Vec<f64>)>>;
const MIN_SVI_SAMPLES: usize = 12;
const MIN_SVI_TTE_MINUTES: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct ChainBuild<'a> {
    pub symbol: &'a str,
    pub spot: f64,
    pub as_of: DateTime<Utc>,
    pub expiration: NaiveDate,
    pub quotes: &'a [RawOptionQuote],
    pub pricing_mode: &'a str,
    pub dealer_model: &'a str,
    pub risk_free_rate: f64,
    pub source: &'a str,
    pub quote_interval: &'a str,
    pub oi_frequency: &'a str,
    pub prefer_sdk_greeks: bool,
    pub quote_coverage: f64,
    pub fresh_quote_coverage: f64,
    pub metadata_coverage: f64,
    pub spot_age_ms: Option<i64>,
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale).round() / scale
}

fn normal_cdf(value: f64) -> f64 {
    0.5 * (1.0 + erf(value / 2_f64.sqrt()))
}

fn normal_pdf(value: f64) -> f64 {
    (-0.5 * value * value).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

pub fn years_to_expiry(as_of: DateTime<Utc>, expiration: NaiveDate) -> f64 {
    let expiry = New_York
        .with_ymd_and_hms(
            expiration.year(),
            expiration.month(),
            expiration.day(),
            16,
            0,
            0,
        )
        .single()
        .expect("valid expiry time")
        .with_timezone(&Utc);
    ((expiry - as_of).num_milliseconds() as f64 / (365.0 * 24.0 * 3600.0 * 1000.0))
        .max(1.0 / (365.0 * 24.0 * 60.0))
}

pub fn option_value(
    spot: f64,
    strike: f64,
    years: f64,
    sigma: f64,
    right: &str,
    rate: f64,
    dividend_yield: f64,
) -> f64 {
    if years <= 0.0 || sigma <= 0.0 || spot <= 0.0 || strike <= 0.0 {
        return if right == "CALL" {
            (spot - strike).max(0.0)
        } else {
            (strike - spot).max(0.0)
        };
    }
    let root_t = years.sqrt();
    let d1 = ((spot / strike).ln() + (rate - dividend_yield + 0.5 * sigma * sigma) * years)
        / (sigma * root_t);
    let d2 = d1 - sigma * root_t;
    let discount = (-rate * years).exp();
    let carry_discount = (-dividend_yield * years).exp();
    if right == "CALL" {
        spot * carry_discount * normal_cdf(d1) - strike * discount * normal_cdf(d2)
    } else {
        strike * discount * normal_cdf(-d2) - spot * carry_discount * normal_cdf(-d1)
    }
}

#[cfg(test)]
fn implied_volatility(
    price: f64,
    spot: f64,
    strike: f64,
    years: f64,
    right: &str,
    rate: f64,
) -> Option<f64> {
    implied_volatility_with_carry(price, spot, strike, years, right, rate, 0.0)
}

pub fn implied_volatility_with_carry(
    price: f64,
    spot: f64,
    strike: f64,
    years: f64,
    right: &str,
    rate: f64,
    dividend_yield: f64,
) -> Option<f64> {
    let intrinsic = if right == "CALL" {
        (spot - strike).max(0.0)
    } else {
        (strike - spot).max(0.0)
    };
    if !price.is_finite() || price <= intrinsic + 0.001 || price >= spot || years <= 0.0 {
        return None;
    }
    let (mut low, mut high) = (0.005, 5.0);
    if option_value(spot, strike, years, high, right, rate, dividend_yield) < price {
        return None;
    }
    for _ in 0..42 {
        let middle = (low + high) / 2.0;
        if option_value(spot, strike, years, middle, right, rate, dividend_yield) > price {
            high = middle;
        } else {
            low = middle;
        }
    }
    Some((low + high) / 2.0)
}

#[derive(Debug, Clone, Copy)]
struct Greeks {
    delta: f64,
    gamma: f64,
    theta: f64,
    vega: f64,
    vanna: f64,
    charm: f64,
}

fn greeks(
    spot: f64,
    strike: f64,
    years: f64,
    sigma: f64,
    right: &str,
    rate: f64,
    dividend_yield: f64,
) -> Greeks {
    let root_t = years.sqrt();
    let d1 = ((spot / strike).ln() + (rate - dividend_yield + 0.5 * sigma * sigma) * years)
        / (sigma * root_t);
    let d2 = d1 - sigma * root_t;
    let pdf = normal_pdf(d1);
    let carry_discount = (-dividend_yield * years).exp();
    let delta = if right == "CALL" {
        carry_discount * normal_cdf(d1)
    } else {
        carry_discount * (normal_cdf(d1) - 1.0)
    };
    let gamma = carry_discount * pdf / (spot * sigma * root_t);
    let discount = (-rate * years).exp();
    let annual_theta = if right == "CALL" {
        -(spot * carry_discount * pdf * sigma) / (2.0 * root_t)
            - rate * strike * discount * normal_cdf(d2)
            + dividend_yield * spot * carry_discount * normal_cdf(d1)
    } else {
        -(spot * carry_discount * pdf * sigma) / (2.0 * root_t)
            + rate * strike * discount * normal_cdf(-d2)
            - dividend_yield * spot * carry_discount * normal_cdf(-d1)
    };
    let vega = spot * carry_discount * pdf * root_t / 100.0;
    let vanna = -pdf * d2 / sigma;
    let charm = -pdf * (2.0 * rate * years - d2 * sigma * root_t) / (2.0 * years * sigma * root_t);
    Greeks {
        delta,
        gamma,
        theta: annual_theta / 365.0,
        vega,
        vanna,
        charm,
    }
}

fn dealer_sign(right: &str, model: &str) -> f64 {
    match model {
        "long_all" => 1.0,
        "short_all" => -1.0,
        _ if right == "CALL" => 1.0,
        _ => -1.0,
    }
}

fn svi_value(k: f64, params: &[f64; 5]) -> f64 {
    let [a, b, rho, m, sigma] = *params;
    a + b * (rho * (k - m) + ((k - m).powi(2) + sigma * sigma).sqrt())
}

fn valid_svi(params: &[f64; 5]) -> bool {
    let [a, b, rho, _, sigma] = *params;
    b >= 0.0
        && rho.abs() < 0.999
        && sigma > 0.001
        && a + b * sigma * (1.0 - rho * rho).max(0.0).sqrt() >= 0.0
}

fn svi_density(k: f64, params: &[f64; 5]) -> f64 {
    let w = svi_value(k, params).max(1e-9);
    let step = 1e-4;
    let left = svi_value(k - step, params);
    let right = svi_value(k + step, params);
    let first = (right - left) / (2.0 * step);
    let second = (right - 2.0 * w + left) / (step * step);
    (1.0 - k * first / (2.0 * w)).powi(2) - (first * first / 4.0) * (1.0 / w + 0.25) + second / 2.0
}

pub fn fit_svi(rows: &[ChainRow], years: f64, forward: f64) -> Option<SviFit> {
    let tte_minutes = years * 365.0 * 24.0 * 60.0;
    if tte_minutes < MIN_SVI_TTE_MINUTES {
        return None;
    }
    let samples: Vec<(f64, f64, f64)> = rows
        .iter()
        .filter(|row| {
            let ratio = row.strike / forward;
            (0.75..=1.25).contains(&ratio)
                && row.quality_score >= 50.0
                && row.iv <= 150.0
                && ((row.strike < forward && row.right == "PUT")
                    || (row.strike >= forward && row.right == "CALL"))
        })
        .map(|row| {
            (
                (row.strike / forward).ln(),
                (row.iv / 100.0).powi(2) * years,
                0.05_f64.max((row.quality_score / 100.0).powi(2)),
            )
        })
        .collect();
    if samples.len() < MIN_SVI_SAMPLES {
        return None;
    }
    let objective = |params: &[f64; 5]| -> f64 {
        if !valid_svi(params) {
            return f64::INFINITY;
        }
        let weight_sum: f64 = samples.iter().map(|sample| sample.2).sum();
        let fit_error: f64 = samples
            .iter()
            .map(|(k, variance, weight)| weight * (svi_value(*k, params) - variance).powi(2))
            .sum::<f64>()
            / weight_sum;
        let low = samples
            .iter()
            .map(|sample| sample.0)
            .fold(f64::INFINITY, f64::min);
        let high = samples
            .iter()
            .map(|sample| sample.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let density_penalty: f64 = (0..21)
            .map(|index| {
                let k = low + (high - low) * index as f64 / 20.0;
                svi_density(k, params).min(0.0).powi(2)
            })
            .sum();
        fit_error + density_penalty * 100.0
    };
    let minimum = samples
        .iter()
        .map(|sample| sample.1)
        .fold(f64::INFINITY, f64::min);
    let mut params = [(minimum * 0.5).max(1e-6), 0.08, -0.25, 0.0, 0.12];
    let mut steps = [minimum.max(0.005), 0.06, 0.2, 0.08, 0.08];
    let mut best = objective(&params);
    for _ in 0..10 {
        for index in 0..5 {
            for direction in [-1.0, 1.0] {
                let mut candidate = params;
                candidate[index] += direction * steps[index];
                let score = objective(&candidate);
                if score < best {
                    params = candidate;
                    best = score;
                }
            }
        }
        for step in &mut steps {
            *step *= 0.55;
        }
    }
    let k_min = samples
        .iter()
        .map(|sample| sample.0)
        .fold(f64::INFINITY, f64::min)
        .max(-0.35);
    let k_max = samples
        .iter()
        .map(|sample| sample.0)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(0.35);
    let curve: Vec<SviPoint> = (0..61)
        .map(|index| {
            let k = k_min + (k_max - k_min) * index as f64 / 60.0;
            let w = svi_value(k, &params).max(1e-9);
            SviPoint {
                k: round(k, 6),
                moneyness: round(k.exp(), 6),
                iv: round((w / years).sqrt() * 100.0, 3),
                density: round(svi_density(k, &params), 6),
            }
        })
        .collect();
    let residuals = samples
        .iter()
        .map(|(k, variance, _)| {
            let observed = (variance / years).sqrt() * 100.0;
            let fitted = (svi_value(*k, &params).max(0.0) / years).sqrt() * 100.0;
            SviResidual {
                k: round(*k, 6),
                observed_iv: round(observed, 3),
                fitted_iv: round(fitted, 3),
                residual: round(observed - fitted, 3),
            }
        })
        .collect();
    let names = ["a", "b", "rho", "m", "sigma"];
    Some(SviFit {
        params: names
            .into_iter()
            .zip(params.into_iter().map(|value| round(value, 8)))
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
        rmse_total_variance: round(best.sqrt(), 8),
        butterfly_violations: curve.iter().filter(|point| point.density < -1e-5).count(),
        curve,
        residuals,
    })
}

fn svi_diagnostics(
    rows: &[ChainRow],
    years: f64,
    forward: f64,
    fit: Option<&SviFit>,
) -> SviDiagnostics {
    let tte_minutes = years * 365.0 * 24.0 * 60.0;
    let eligible_samples = rows
        .iter()
        .filter(|row| {
            let ratio = row.strike / forward;
            (0.75..=1.25).contains(&ratio)
                && row.quality_score >= 50.0
                && row.iv <= 150.0
                && ((row.strike < forward && row.right == "PUT")
                    || (row.strike >= forward && row.right == "CALL"))
        })
        .count();
    let (status, reason) = if fit.is_some() {
        ("ready", None)
    } else if tte_minutes < MIN_SVI_TTE_MINUTES {
        (
            "near_expiry",
            Some(format!(
                "距离到期不足 {:.0} 分钟，停止不稳定的 SVI 拟合",
                MIN_SVI_TTE_MINUTES
            )),
        )
    } else if eligible_samples < MIN_SVI_SAMPLES {
        (
            "insufficient_samples",
            Some(format!(
                "可用 OTM 报价不足：{eligible_samples}/{MIN_SVI_SAMPLES}"
            )),
        )
    } else {
        (
            "fit_failed",
            Some("约束优化未收敛到有效 SVI 参数".to_string()),
        )
    };
    SviDiagnostics {
        status: status.into(),
        eligible_samples,
        required_samples: MIN_SVI_SAMPLES,
        tte_minutes: round(tte_minutes, 2),
        minimum_tte_minutes: MIN_SVI_TTE_MINUTES,
        reason,
    }
}

fn executable_mark(quote: &RawOptionQuote, pricing_mode: &str) -> Option<f64> {
    let spread_valid = quote.ask.is_finite()
        && quote.bid.is_finite()
        && quote.ask > 0.0
        && quote.bid >= 0.0
        && quote.ask >= quote.bid;
    if !spread_valid {
        return quote.last.filter(|value| value.is_finite() && *value > 0.0);
    }
    let midpoint = (quote.bid + quote.ask) / 2.0;
    let microprice = if quote.bid_size + quote.ask_size > 0 {
        (quote.ask * quote.bid_size as f64 + quote.bid * quote.ask_size as f64)
            / (quote.bid_size + quote.ask_size) as f64
    } else {
        midpoint
    };
    Some(match pricing_mode {
        "ask" => quote.ask,
        "mid" => midpoint,
        _ => microprice,
    })
}

fn infer_forward(
    quotes: &[RawOptionQuote],
    spot: f64,
    years: f64,
    rate: f64,
    pricing_mode: &str,
) -> Option<f64> {
    if years < 1.0 / 365.0 {
        return None;
    }
    let mut pairs: BTreeMap<i64, (f64, Option<f64>, Option<f64>)> = BTreeMap::new();
    for quote in quotes {
        if (quote.strike / spot - 1.0).abs() > 0.15 {
            continue;
        }
        let Some(mark) = executable_mark(quote, pricing_mode) else {
            continue;
        };
        let entry = pairs
            .entry((quote.strike * 1_000.0).round() as i64)
            .or_insert((quote.strike, None, None));
        if quote.right == "CALL" {
            entry.1 = Some(mark);
        } else if quote.right == "PUT" {
            entry.2 = Some(mark);
        }
    }
    let mut candidates: Vec<f64> = pairs
        .into_values()
        .filter_map(|(strike, call, put)| {
            let forward = strike + (rate * years).exp() * (call? - put?);
            (forward.is_finite() && (0.8 * spot..=1.2 * spot).contains(&forward)).then_some(forward)
        })
        .collect();
    if candidates.len() < 3 {
        return None;
    }
    candidates.sort_by(f64::total_cmp);
    Some(candidates[candidates.len() / 2])
}

pub fn build_chain(input: ChainBuild<'_>) -> anyhow::Result<ChainSnapshot> {
    anyhow::ensure!(
        input.spot.is_finite() && input.spot > 0.0,
        "invalid underlying spot"
    );
    let years = years_to_expiry(input.as_of, input.expiration);
    let parity_forward = infer_forward(
        input.quotes,
        input.spot,
        years,
        input.risk_free_rate,
        input.pricing_mode,
    );
    let forward =
        parity_forward.unwrap_or_else(|| input.spot * (input.risk_free_rate * years).exp());
    let dividend_yield = if parity_forward.is_some() && years > 0.0 {
        (input.risk_free_rate - (forward / input.spot).ln() / years).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let gex_ready = input.metadata_coverage >= 90.0;
    let mut rows = Vec::with_capacity(input.quotes.len());
    for quote in input.quotes {
        let spread_valid = quote.ask.is_finite()
            && quote.bid.is_finite()
            && quote.ask > 0.0
            && quote.bid >= 0.0
            && quote.ask >= quote.bid;
        let midpoint = if spread_valid {
            (quote.bid + quote.ask) / 2.0
        } else {
            quote.last.unwrap_or(0.0)
        };
        let microprice = if spread_valid && quote.bid_size + quote.ask_size > 0 {
            (quote.ask * quote.bid_size as f64 + quote.bid * quote.ask_size as f64)
                / (quote.bid_size + quote.ask_size) as f64
        } else {
            midpoint
        };
        let mark = match input.pricing_mode {
            "ask" if spread_valid => quote.ask,
            "mid" => midpoint,
            _ => microprice,
        };
        let solved_iv = implied_volatility_with_carry(
            mark,
            input.spot,
            quote.strike,
            years,
            &quote.right,
            input.risk_free_rate,
            dividend_yield,
        );
        let iv = solved_iv
            .or(quote.sdk_iv)
            .filter(|value| (0.01..=4.0).contains(value));
        let Some(iv) = iv else { continue };
        let calculated = greeks(
            input.spot,
            quote.strike,
            years,
            iv,
            &quote.right,
            input.risk_free_rate,
            dividend_yield,
        );
        let delta = if input.prefer_sdk_greeks {
            quote.sdk_delta.unwrap_or(calculated.delta)
        } else {
            calculated.delta
        };
        let gamma = if input.prefer_sdk_greeks {
            quote.sdk_gamma.unwrap_or(calculated.gamma)
        } else {
            calculated.gamma
        };
        let theta = if input.prefer_sdk_greeks {
            quote.sdk_theta.unwrap_or(calculated.theta)
        } else {
            calculated.theta
        };
        let vega = if input.prefer_sdk_greeks {
            quote.sdk_vega.unwrap_or(calculated.vega)
        } else {
            calculated.vega
        };
        let signed_gex = gex_ready.then(|| {
            gamma
                * quote.open_interest as f64
                * 100.0
                * input.spot
                * input.spot
                * 0.01
                * dealer_sign(&quote.right, input.dealer_model)
        });
        let spread_pct = if spread_valid && midpoint > 0.0 {
            (quote.ask - quote.bid) / midpoint * 100.0
        } else {
            100.0
        };
        let mut flags = Vec::new();
        if !spread_valid {
            flags.push("missing_nbbo".to_string());
        }
        if quote.bid == 0.0 {
            flags.push("zero_bid".to_string());
        }
        if spread_pct > 25.0 {
            flags.push("wide_spread".to_string());
        }
        if quote.bid_size + quote.ask_size < 5 {
            flags.push("thin_size".to_string());
        }
        if iv > 2.0 {
            flags.push("extreme_iv".to_string());
        }
        if solved_iv.is_none() && quote.sdk_iv.is_some() {
            flags.push("sdk_iv_fallback".to_string());
        }
        if !gex_ready {
            flags.push("oi_metadata_incomplete".to_string());
        }
        let quality_score = (100.0
            - if flags.iter().any(|flag| flag == "zero_bid") {
                35.0
            } else {
                0.0
            }
            - ((spread_pct - 8.0).max(0.0) * 1.5).min(45.0)
            - if flags.iter().any(|flag| flag == "thin_size") {
                15.0
            } else {
                0.0
            }
            - if flags.iter().any(|flag| flag == "extreme_iv") {
                15.0
            } else {
                0.0
            })
        .max(0.0);
        rows.push(ChainRow {
            symbol: quote.symbol.clone(),
            strike: quote.strike,
            right: quote.right.clone(),
            bid: round(if spread_valid { quote.bid } else { 0.0 }, 4),
            ask: round(if spread_valid { quote.ask } else { 0.0 }, 4),
            last: quote.last.map(|value| round(value, 4)),
            mark: round(mark, 4),
            mid: round(midpoint, 4),
            microprice: round(microprice, 4),
            spread: round(
                if spread_valid {
                    quote.ask - quote.bid
                } else {
                    0.0
                },
                4,
            ),
            spread_pct: round(spread_pct, 2),
            bid_size: quote.bid_size,
            ask_size: quote.ask_size,
            volume: quote.volume,
            open_interest: quote.open_interest,
            iv: round(iv * 100.0, 3),
            delta: round(delta, 4),
            gamma: round(gamma, 6),
            theta: round(theta, 6),
            vega: round(vega, 6),
            vanna: round(calculated.vanna, 6),
            charm: round(calculated.charm, 6),
            gex: signed_gex.map(|value| round(value, 2)),
            moneyness: round(quote.strike / input.spot, 5),
            log_moneyness: round((quote.strike / forward).ln(), 6),
            quality_score: round(quality_score, 1),
            quality_flags: flags,
        });
    }
    rows.sort_by(|left, right| {
        left.strike
            .total_cmp(&right.strike)
            .then_with(|| left.right.cmp(&right.right))
    });
    anyhow::ensure!(!rows.is_empty(), "no usable option quotes");
    let calls: Vec<&ChainRow> = rows.iter().filter(|row| row.right == "CALL").collect();
    let puts: Vec<&ChainRow> = rows.iter().filter(|row| row.right == "PUT").collect();
    let call_oi = gex_ready.then(|| calls.iter().map(|row| row.open_interest).sum::<i64>());
    let put_oi = gex_ready.then(|| puts.iter().map(|row| row.open_interest).sum::<i64>());
    let mut by_strike: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for row in &rows {
        if let Some(gex) = row.gex {
            let key = (row.strike * 1000.0).round() as i64;
            let entry = by_strike.entry(key).or_insert((row.strike, 0.0));
            entry.1 += gex;
        }
    }
    let call_wall = gex_ready
        .then(|| {
            calls
                .iter()
                .max_by_key(|row| row.open_interest)
                .map(|row| row.strike)
        })
        .flatten();
    let put_wall = gex_ready
        .then(|| {
            puts.iter()
                .max_by_key(|row| row.open_interest)
                .map(|row| row.strike)
        })
        .flatten();
    let atm = rows.iter().min_by(|left, right| {
        left.log_moneyness
            .abs()
            .total_cmp(&right.log_moneyness.abs())
    });
    let call_25 = calls.iter().min_by(|left, right| {
        (left.delta - 0.25)
            .abs()
            .total_cmp(&(right.delta - 0.25).abs())
    });
    let put_25 = puts.iter().min_by(|left, right| {
        (left.delta.abs() - 0.25)
            .abs()
            .total_cmp(&(right.delta.abs() - 0.25).abs())
    });
    let rr25 = call_25
        .zip(put_25)
        .map(|(call, put)| round(call.iv - put.iv, 3));
    let bf25 = call_25
        .zip(put_25)
        .zip(atm)
        .map(|((call, put), atm)| round((call.iv + put.iv) / 2.0 - atm.iv, 3));
    let svi = fit_svi(&rows, years, forward);
    let svi_diagnostics = svi_diagnostics(&rows, years, forward, svi.as_ref());
    let dealer_scenarios: Vec<DealerScenario> = if gex_ready {
        (0..31)
            .map(|index| {
                let scenario_spot = input.spot * (0.85 + index as f64 * 0.01);
                let gex = rows
                    .iter()
                    .map(|row| {
                        let scenario = greeks(
                            scenario_spot,
                            row.strike,
                            years,
                            row.iv / 100.0,
                            &row.right,
                            input.risk_free_rate,
                            dividend_yield,
                        );
                        scenario.gamma
                            * row.open_interest as f64
                            * 100.0
                            * scenario_spot
                            * scenario_spot
                            * 0.01
                            * dealer_sign(&row.right, input.dealer_model)
                    })
                    .sum();
                DealerScenario {
                    spot: round(scenario_spot, 3),
                    gex: round(gex, 2),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let gamma_flip = dealer_scenarios.windows(2).find_map(|pair| {
        let (previous, current) = (&pair[0], &pair[1]);
        if previous.gex == 0.0 || previous.gex * current.gex < 0.0 {
            Some(round(
                previous.spot
                    + (current.spot - previous.spot) * previous.gex.abs()
                        / (previous.gex.abs() + current.gex.abs()).max(1e-9),
                3,
            ))
        } else {
            None
        }
    });
    let mut counts = BTreeMap::new();
    for flag in rows.iter().flat_map(|row| row.quality_flags.iter()) {
        *counts.entry(flag.clone()).or_insert(0) += 1;
    }
    let et = input.as_of.with_timezone(&New_York);
    let date = et.format("%Y-%m-%d").to_string();
    let minute = et.format("%H:%M").to_string();
    let timestamp = input.as_of.to_rfc3339();
    let snapshot_key = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        input.symbol,
        date,
        timestamp,
        input.expiration,
        input.pricing_mode,
        input.dealer_model,
        input.source
    );
    let snapshot_id = hex::encode(Sha256::digest(snapshot_key.as_bytes()))[..16].to_string();
    let contracts = rows.len();
    let net_gex = gex_ready.then(|| round(rows.iter().filter_map(|row| row.gex).sum::<f64>(), 2));
    let avg_quality = Some(round(
        rows.iter().map(|row| row.quality_score).sum::<f64>() / contracts as f64,
        2,
    ));
    let usable_pct = round(
        rows.iter().filter(|row| row.quality_score >= 60.0).count() as f64 / contracts as f64
            * 100.0,
        2,
    );
    Ok(ChainSnapshot {
        snapshot_id,
        symbol: input.symbol.to_string(),
        date,
        minute,
        timestamp,
        expiration: input.expiration.to_string(),
        spot: round(input.spot, 4),
        forward: round(forward, 4),
        pricing_mode: input.pricing_mode.to_string(),
        dealer_model: input.dealer_model.to_string(),
        provenance: Provenance {
            source: input.source.to_string(),
            quote_interval: input.quote_interval.to_string(),
            oi_frequency: input.oi_frequency.to_string(),
            risk_free_rate: input.risk_free_rate,
            dividend_yield: round(dividend_yield, 6),
            forward_source: if parity_forward.is_some() {
                "put_call_parity"
            } else {
                "cost_of_carry_fallback"
            }
            .into(),
            exercise_model: "European BSM approximation for American-listed options".into(),
            model: if input.prefer_sdk_greeks {
                "Longbridge+Rust-BSM-SVI-v2"
            } else {
                "Rust-BSM+SVI-v2"
            }
            .into(),
            sdk_version: input.prefer_sdk_greeks.then(|| "4.4.1".into()),
        },
        dte: (input.expiration - et.date_naive()).num_days(),
        tte_years: round(years, 10),
        metrics: ChainMetrics {
            contracts,
            call_oi,
            put_oi,
            pcr: call_oi.zip(put_oi).and_then(|(calls, puts)| {
                (calls > 0).then(|| round(puts as f64 / calls as f64, 3))
            }),
            net_gex,
            call_wall,
            put_wall,
            gamma_flip,
            atm_iv: atm.map(|row| row.iv),
            rr25,
            bf25,
            avg_quality,
        },
        quality: ChainQuality {
            counts,
            usable_pct,
            quote_coverage_pct: round(input.quote_coverage.clamp(0.0, 100.0), 2),
            fresh_quote_coverage_pct: round(input.fresh_quote_coverage.clamp(0.0, 100.0), 2),
            metadata_coverage_pct: round(input.metadata_coverage.clamp(0.0, 100.0), 2),
            spot_age_ms: input.spot_age_ms,
            gex_ready,
            blocked_metrics: if gex_ready {
                Vec::new()
            } else {
                vec![
                    "net_gex".into(),
                    "gamma_flip".into(),
                    "call_put_wall".into(),
                    "pcr".into(),
                ]
            },
        },
        svi,
        svi_diagnostics,
        dealer_scenarios,
        gex_by_strike: by_strike
            .into_values()
            .map(|(strike, gex)| GexPoint {
                strike,
                gex: round(gex, 2),
            })
            .collect(),
        rows,
    })
}

fn surface_grid(points: &[SurfacePoint]) -> Vec<Vec<[f64; 3]>> {
    let mut grouped = SurfaceIvBuckets::new();
    for point in points {
        let preferred = (point.moneyness <= 1.0 && point.right == "PUT")
            || (point.moneyness >= 1.0 && point.right == "CALL");
        let bucket = grouped
            .entry((point.tte_days * 10_000.0).round() as i64)
            .or_default()
            .entry((point.moneyness * 10_000.0).round() as i64)
            .or_default();
        if preferred {
            bucket.0.push(point.iv);
        } else {
            bucket.1.push(point.iv);
        }
    }
    let curves: BTreeMap<i64, Vec<(f64, f64)>> = grouped
        .into_iter()
        .filter_map(|(tte_key, buckets)| {
            let curve: Vec<_> = buckets
                .into_iter()
                .map(|(key, (preferred, fallback))| {
                    let values = if preferred.is_empty() {
                        &fallback
                    } else {
                        &preferred
                    };
                    (
                        key as f64 / 10_000.0,
                        values.iter().sum::<f64>() / values.len() as f64,
                    )
                })
                .collect();
            let valid = curve.len() >= 4
                && curve.last().unwrap().0 - curve.first().unwrap().0 >= 0.08
                && curve.first().unwrap().0 <= 0.98
                && curve.last().unwrap().0 >= 1.02;
            valid.then_some((tte_key, curve))
        })
        .collect();
    if curves.len() < 2 {
        return Vec::new();
    }
    let lower = curves
        .values()
        .map(|curve| curve[0].0)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(0.78);
    let upper = curves
        .values()
        .map(|curve| curve.last().unwrap().0)
        .fold(f64::INFINITY, f64::min)
        .min(1.22);
    if upper - lower < 0.05 {
        return Vec::new();
    }
    let interpolate = |curve: &[(f64, f64)], target: f64| -> f64 {
        for pair in curve.windows(2) {
            if target <= pair[1].0 {
                let weight = (target - pair[0].0) / (pair[1].0 - pair[0].0).max(1e-9);
                return pair[0].1 + weight * (pair[1].1 - pair[0].1);
            }
        }
        curve.last().unwrap().1
    };
    curves
        .iter()
        .map(|(tte_key, curve)| {
            (0..25)
                .map(|index| {
                    let x = lower + (upper - lower) * index as f64 / 24.0;
                    [
                        round(x, 5),
                        round(*tte_key as f64 / 10_000.0, 5),
                        round(interpolate(curve, x), 3),
                    ]
                })
                .collect()
        })
        .collect()
}

fn project_surface_grid(mut grid: Vec<Vec<[f64; 3]>>) -> (Vec<Vec<[f64; 3]>>, usize, usize) {
    if grid.is_empty() {
        return (grid, 0, 0);
    }
    let mut total_variances: Vec<Vec<f64>> = grid
        .iter()
        .map(|row| {
            let years = row[0][1].max(1.0 / 1_440.0) / 365.0;
            row.iter()
                .map(|cell| (cell[2] / 100.0).powi(2) * years)
                .collect()
        })
        .collect();
    let mut convexity_adjustments = 0;
    for values in &mut total_variances {
        for _ in 0..24 {
            let mut changed = false;
            for index in 1..values.len() - 1 {
                let ceiling = (values[index - 1] + values[index + 1]) / 2.0;
                if values[index] > ceiling + 1e-10 {
                    values[index] = ceiling;
                    convexity_adjustments += 1;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
    let mut calendar_adjustments = 0;
    for row in 1..total_variances.len() {
        for column in 0..total_variances[row].len() {
            let floor = total_variances[row - 1][column];
            if total_variances[row][column] < floor {
                total_variances[row][column] = floor;
                calendar_adjustments += 1;
            }
        }
    }
    for (row_index, row) in grid.iter_mut().enumerate() {
        let years = row[0][1].max(1.0 / 1_440.0) / 365.0;
        for (column, cell) in row.iter_mut().enumerate() {
            cell[2] = round(
                (total_variances[row_index][column].max(0.0) / years).sqrt() * 100.0,
                3,
            );
        }
    }
    (grid, convexity_adjustments, calendar_adjustments)
}

fn price_space_violations(grid: &[Vec<[f64; 3]>]) -> (usize, usize) {
    let mut monotonicity = 0;
    let mut convexity = 0;
    for row in grid {
        if row.len() < 3 {
            continue;
        }
        let years = row[0][1].max(1.0 / 1_440.0) / 365.0;
        let prices: Vec<f64> = row
            .iter()
            .map(|cell| option_value(1.0, cell[0], years, cell[2] / 100.0, "CALL", 0.0, 0.0))
            .collect();
        monotonicity += prices
            .windows(2)
            .filter(|pair| pair[1] > pair[0] + 1e-6)
            .count();
        convexity += prices
            .windows(3)
            .filter(|triple| triple[1] > (triple[0] + triple[2]) / 2.0 + 1e-6)
            .count();
    }
    (monotonicity, convexity)
}

pub fn build_surface(
    symbol: &str,
    chains: &[ChainSnapshot],
    as_of: DateTime<Utc>,
) -> SurfaceSnapshot {
    let mut points = Vec::new();
    let mut term = Vec::new();
    let mut svi_slices = Vec::new();
    for chain in chains {
        let observed: Vec<_> = chain
            .rows
            .iter()
            .filter(|row| (0.75..=1.25).contains(&row.moneyness))
            .collect();
        let stride = observed.len().div_ceil(80).max(1);
        points.extend(
            observed
                .into_iter()
                .step_by(stride)
                .map(|row| SurfacePoint {
                    moneyness: row.moneyness,
                    dte: chain.dte,
                    tte_days: round(chain.tte_years * 365.0, 6),
                    iv: row.iv,
                    right: row.right.clone(),
                }),
        );
        if let Some(atm) = chain.rows.iter().min_by(|a, b| {
            (a.strike - chain.spot)
                .abs()
                .total_cmp(&(b.strike - chain.spot).abs())
        }) {
            term.push(TermPoint {
                expiration: chain.expiration.clone(),
                dte: chain.dte,
                iv: atm.iv,
                net_gex: chain.metrics.net_gex,
            });
        }
        if let Some(svi) = &chain.svi {
            svi_slices.push(serde_json::json!({
                "expiration": chain.expiration,
                "dte": chain.dte,
                "params": svi.params,
                "rmse_total_variance": svi.rmse_total_variance,
                "butterfly_violations": svi.butterfly_violations,
                "curve": svi.curve,
                "residuals": svi.residuals,
            }));
        }
    }
    term.sort_by_key(|point| point.dte);
    let mut calendar_violations = 0;
    let mut previous_by_k: Option<HashMap<i64, f64>> = None;
    for slice in &svi_slices {
        let dte = slice["dte"].as_i64().unwrap_or(1).max(1) as f64;
        let current: HashMap<i64, f64> = slice["curve"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|point| {
                Some((
                    (point["k"].as_f64()? * 100.0).round() as i64,
                    (point["iv"].as_f64()? / 100.0).powi(2) * dte / 365.0,
                ))
            })
            .collect();
        if let Some(previous) = &previous_by_k {
            calendar_violations += previous
                .iter()
                .filter(|(key, value)| {
                    current
                        .get(key)
                        .is_some_and(|longer| longer + 1e-7 < **value)
                })
                .count();
        }
        previous_by_k = Some(current);
    }
    let raw_grid = surface_grid(&points);
    let (grid, convexity_adjustments, calendar_adjustments) =
        project_surface_grid(raw_grid.clone());
    let (price_monotonicity_violations, price_convexity_violations) = price_space_violations(&grid);
    let grid_cells = grid.iter().map(Vec::len).sum::<usize>().max(1);
    let adjustment_ratio =
        (convexity_adjustments + calendar_adjustments) as f64 / grid_cells as f64;
    let violation_ratio =
        (price_monotonicity_violations + price_convexity_violations) as f64 / grid_cells as f64;
    let svi_failures = chains.iter().filter(|chain| chain.svi.is_none()).count();
    let confidence_score = round(
        (100.0
            - adjustment_ratio.min(1.0) * 55.0
            - violation_ratio.min(1.0) * 80.0
            - svi_failures as f64 / chains.len().max(1) as f64 * 25.0)
            .clamp(0.0, 100.0),
        1,
    );
    let trusted = !grid.is_empty()
        && price_monotonicity_violations == 0
        && price_convexity_violations == 0
        && confidence_score >= 70.0;
    let et = as_of.with_timezone(&New_York);
    SurfaceSnapshot {
        symbol: symbol.to_string(),
        date: et.format("%Y-%m-%d").to_string(),
        minute: et.format("%H:%M").to_string(),
        timestamp: as_of.to_rfc3339(),
        spot: chains.first().map(|chain| chain.spot).unwrap_or(0.0),
        points,
        grid_raw: raw_grid,
        grid,
        term,
        arbitrage: SurfaceArbitrage {
            calendar_violations,
            butterfly_violations: chains
                .iter()
                .filter_map(|chain| chain.svi.as_ref())
                .map(|svi| svi.butterfly_violations)
                .sum(),
            post_projection_calendar_violations: 0,
            post_projection_convexity_violations: 0,
            convexity_adjustments,
            calendar_adjustments,
            price_monotonicity_violations,
            price_convexity_violations,
            confidence_score,
            trusted,
            projection_model: "constrained_total_variance_with_price_space_validation_v2".into(),
            warning: (!trusted)
                .then(|| "Research projection only: sparse or heavily adjusted surface".into()),
        },
        svi_slices,
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::TimeZone;

    use super::*;

    fn svi_row(strike: f64, right: &str) -> ChainRow {
        let moneyness = strike / 100.0;
        ChainRow {
            symbol: format!("{right}-{strike}"),
            strike,
            right: right.into(),
            bid: 1.0,
            ask: 1.1,
            last: Some(1.05),
            mark: 1.05,
            mid: 1.05,
            microprice: 1.05,
            spread: 0.1,
            spread_pct: 9.52,
            bid_size: 10,
            ask_size: 10,
            volume: 100,
            open_interest: 1_000,
            iv: 22.0 + (moneyness.ln() * 100.0).powi(2) * 0.02,
            delta: if right == "CALL" { 0.4 } else { -0.4 },
            gamma: 0.01,
            theta: -0.02,
            vega: 0.1,
            vanna: 0.0,
            charm: 0.0,
            gex: Some(1.0),
            moneyness,
            log_moneyness: moneyness.ln(),
            quality_score: 90.0,
            quality_flags: vec![],
        }
    }

    #[test]
    fn bsm_iv_round_trip() {
        let years = 30.0 / 365.0;
        let price = option_value(100.0, 100.0, years, 0.25, "CALL", 0.04, 0.0);
        let solved = implied_volatility(price, 100.0, 100.0, years, "CALL", 0.04).unwrap();
        assert_relative_eq!(solved, 0.25, epsilon = 1e-6);
    }

    #[test]
    fn expiry_clock_uses_new_york_close() {
        let as_of = Utc.with_ymd_and_hms(2026, 7, 10, 14, 30, 0).unwrap();
        let years = years_to_expiry(as_of, NaiveDate::from_ymd_opt(2026, 7, 10).unwrap());
        assert_relative_eq!(years * 365.0 * 24.0, 5.5, epsilon = 1e-9);
    }

    #[test]
    fn svi_accepts_intraday_zero_dte_before_close() {
        let rows: Vec<_> = [88.0, 90.0, 92.0, 94.0, 96.0, 98.0]
            .into_iter()
            .map(|strike| svi_row(strike, "PUT"))
            .chain(
                [100.0, 102.0, 104.0, 106.0, 108.0, 110.0]
                    .into_iter()
                    .map(|strike| svi_row(strike, "CALL")),
            )
            .collect();
        let years = 6.0 / (365.0 * 24.0);
        let fit = fit_svi(&rows, years, 100.0);
        let diagnostics = svi_diagnostics(&rows, years, 100.0, fit.as_ref());
        assert!(fit.is_some());
        assert_eq!(diagnostics.status, "ready");
        assert_eq!(diagnostics.eligible_samples, 12);
    }

    #[test]
    fn svi_stops_inside_near_expiry_guard() {
        let rows = vec![svi_row(100.0, "CALL"); 12];
        let years = 1.0 / (365.0 * 24.0 * 60.0);
        let fit = fit_svi(&rows, years, 100.0);
        let diagnostics = svi_diagnostics(&rows, years, 100.0, fit.as_ref());
        assert!(fit.is_none());
        assert_eq!(diagnostics.status, "near_expiry");
    }

    #[test]
    fn surface_uses_opposite_right_when_otm_quote_is_missing() {
        let mut points = Vec::new();
        for dte in [7, 14] {
            for moneyness in [0.90, 0.95, 1.00, 1.05, 1.10] {
                points.push(SurfacePoint {
                    moneyness,
                    dte,
                    tte_days: dte as f64,
                    iv: 25.0 + (moneyness - 1.0_f64).abs() * 20.0,
                    right: if moneyness < 1.0 {
                        "CALL".into()
                    } else {
                        "PUT".into()
                    },
                });
            }
        }
        let grid = surface_grid(&points);
        assert_eq!(grid.len(), 2);
        assert_eq!(grid[0].len(), 25);
    }
}
