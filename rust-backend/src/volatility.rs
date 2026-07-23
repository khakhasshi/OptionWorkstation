use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct IvHistoryPoint {
    pub date: String,
    pub iv: f64,
    pub dte: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct VolatilityContext {
    pub symbol: String,
    pub as_of: String,
    pub expiration: String,
    pub reference_dte: i64,
    pub tte_years: f64,
    pub atm_iv: Option<f64>,
    pub iv_rank: Option<f64>,
    pub iv_percentile: Option<f64>,
    pub realized_volatility: BTreeMap<String, Option<f64>>,
    pub vrp20: Option<f64>,
    pub expected_move: Option<f64>,
    pub expected_move_basis: String,
    pub history: Vec<IvHistoryPoint>,
    pub sample_size: usize,
    pub history_through: Option<String>,
    pub rv_through: Option<String>,
    pub iv_source: String,
    pub rv_source: String,
    pub status: String,
    pub notes: Vec<String>,
}

pub struct VolatilityInput {
    pub symbol: String,
    pub as_of: String,
    pub expiration: String,
    pub reference_dte: i64,
    pub tte_years: f64,
    pub spot: f64,
    pub atm_iv: Option<f64>,
    pub history: Vec<IvHistoryPoint>,
    pub closes: Vec<f64>,
    pub rv_through: Option<String>,
    pub iv_source: String,
    pub rv_source: String,
}

pub fn realized_volatility(closes: &[f64], window: usize) -> Option<f64> {
    if closes.len() <= window {
        return None;
    }
    let returns: Vec<f64> = closes
        .windows(2)
        .filter(|pair| pair[0] > 0.0 && pair[1] > 0.0)
        .map(|pair| (pair[1] / pair[0]).ln())
        .collect();
    if returns.len() < window {
        return None;
    }
    let sample = &returns[returns.len() - window..];
    let mean = sample.iter().sum::<f64>() / sample.len() as f64;
    let variance = sample
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (sample.len() - 1).max(1) as f64;
    Some(variance.sqrt() * 252.0_f64.sqrt() * 100.0)
}

pub fn build_context(input: VolatilityInput) -> VolatilityContext {
    let historical_values: Vec<f64> = input.history.iter().map(|point| point.iv).collect();
    let (iv_rank, iv_percentile) = match (input.atm_iv, historical_values.len() >= 5) {
        (Some(current), true) => {
            let low = historical_values
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min);
            let high = historical_values
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            let rank = if (high - low).abs() < 1e-9 {
                Some(50.0)
            } else {
                Some(((current - low) / (high - low) * 100.0).clamp(0.0, 100.0))
            };
            let percentile = Some(
                historical_values
                    .iter()
                    .filter(|value| **value <= current)
                    .count() as f64
                    / historical_values.len() as f64
                    * 100.0,
            );
            (rank, percentile)
        }
        _ => (None, None),
    };
    let rv5 = realized_volatility(&input.closes, 5);
    let rv10 = realized_volatility(&input.closes, 10);
    let rv20 = realized_volatility(&input.closes, 20);
    let expected_move = input.atm_iv.map(|iv| {
        input.spot * iv / 100.0 * input.tte_years.max(1.0 / (365.0 * 24.0 * 60.0)).sqrt()
    });
    let mut realized = BTreeMap::new();
    realized.insert("5".into(), rv5.map(|value| round(value, 3)));
    realized.insert("10".into(), rv10.map(|value| round(value, 3)));
    realized.insert("20".into(), rv20.map(|value| round(value, 3)));
    let mut notes = Vec::new();
    if historical_values.len() < 20 {
        notes.push(format!(
            "IV rank has only {} matched-DTE observations",
            historical_values.len()
        ));
    }
    if rv20.is_none() {
        notes.push("RV20 requires at least 21 valid daily closes".into());
    }
    let status = if input.atm_iv.is_none() {
        "unavailable"
    } else if historical_values.len() < 20 || rv20.is_none() {
        "partial"
    } else {
        "ready"
    };
    VolatilityContext {
        symbol: input.symbol,
        as_of: input.as_of,
        expiration: input.expiration,
        reference_dte: input.reference_dte,
        tte_years: round(input.tte_years, 10),
        atm_iv: input.atm_iv.map(|value| round(value, 3)),
        iv_rank: iv_rank.map(|value| round(value, 2)),
        iv_percentile: iv_percentile.map(|value| round(value, 2)),
        realized_volatility: realized,
        vrp20: input.atm_iv.zip(rv20).map(|(iv, rv)| round(iv - rv, 3)),
        expected_move: expected_move.map(|value| round(value, 3)),
        expected_move_basis: "spot * ATM IV * sqrt(exact time to 16:00 ET expiry)".into(),
        sample_size: input.history.len(),
        history_through: input.history.last().map(|point| point.date.clone()),
        history: input.history,
        rv_through: input.rv_through,
        iv_source: input.iv_source,
        rv_source: input.rv_source,
        status: status.into(),
        notes,
    }
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_and_rv_require_real_samples() {
        let history = (0..25)
            .map(|index| IvHistoryPoint {
                date: format!("2026-06-{index:02}"),
                iv: 20.0 + index as f64,
                dte: 7,
            })
            .collect();
        let closes = (0..30).map(|index| 100.0 + index as f64).collect();
        let context = build_context(VolatilityInput {
            symbol: "SPY".into(),
            as_of: "2026-07-22T15:00:00Z".into(),
            expiration: "2026-07-24".into(),
            reference_dte: 2,
            tte_years: 2.0 / 365.0,
            spot: 600.0,
            atm_iv: Some(32.0),
            history,
            closes,
            rv_through: Some("2026-07-22".into()),
            iv_source: "ThetaData".into(),
            rv_source: "Longbridge".into(),
        });
        assert_eq!(context.status, "ready");
        assert!(context.iv_rank.is_some());
        assert!(context.realized_volatility["20"].is_some());
        assert!(context.expected_move.unwrap() > 0.0);
    }
}
