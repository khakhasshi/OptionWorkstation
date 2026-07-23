use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, anyhow};
use arrow_array::{
    Array, Float64Array, Int64Array, LargeStringArray, RecordBatch, StringArray, StringViewArray,
    TimestampMicrosecondArray,
};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::America::New_York;
use moka::sync::Cache;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::{Value, json};

use crate::{
    analytics::{ChainBuild, build_chain, build_surface},
    models::{Bar, ChainSnapshot, RawOptionQuote, SurfaceSnapshot},
    volatility::{IvHistoryPoint, VolatilityInput, build_context},
};

type OiMap = HashMap<(i64, String), i64>;

#[derive(Clone)]
pub struct ReplayStore {
    root: PathBuf,
    risk_free_rate: f64,
    stock_cache: Cache<String, Arc<Vec<Bar>>>,
    quote_cache: Cache<String, Arc<Vec<RawOptionQuote>>>,
    oi_cache: Cache<String, Arc<OiMap>>,
}

impl ReplayStore {
    pub fn new(root: PathBuf, risk_free_rate: f64) -> Self {
        Self {
            root,
            risk_free_rate,
            stock_cache: Cache::new(64),
            quote_cache: Cache::new(512),
            oi_cache: Cache::new(128),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn symbol_dir(&self, symbol: &str) -> PathBuf {
        self.root
            .join("underlying")
            .join(format!("symbol={symbol}"))
    }

    fn option_day_dir(&self, symbol: &str, date: &str) -> PathBuf {
        self.root
            .join("options")
            .join(format!("symbol={symbol}"))
            .join(format!("date={date}"))
    }

    pub fn validate_symbol(&self, symbol: &str) -> anyhow::Result<String> {
        let clean = symbol.trim().trim_end_matches(".US").to_uppercase();
        anyhow::ensure!(
            !clean.is_empty() && self.symbol_dir(&clean).is_dir(),
            "Unknown symbol: {symbol}"
        );
        Ok(clean)
    }

    pub fn validate_date(&self, symbol: &str, date: &str) -> anyhow::Result<NaiveDate> {
        let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d").context("Invalid date")?;
        anyhow::ensure!(
            self.symbol_dir(symbol)
                .join(format!("date={date}/ohlc.parquet"))
                .is_file(),
            "No data for {symbol} on {date}"
        );
        Ok(parsed)
    }

    pub fn symbols(&self) -> Vec<String> {
        partition_values(&self.root.join("underlying"), "symbol=")
    }

    pub fn dates(&self, symbol: &str) -> Vec<String> {
        partition_values(&self.symbol_dir(symbol), "date=")
    }

    pub fn expirations(&self, symbol: &str, trading_date: &str) -> Vec<String> {
        partition_values(&self.option_day_dir(symbol, trading_date), "expiration=")
            .into_iter()
            .filter(|expiry| {
                self.option_day_dir(symbol, trading_date)
                    .join(format!("expiration={expiry}/quote_1m.parquet"))
                    .is_file()
            })
            .collect()
    }

    pub fn catalog(&self) -> Value {
        let symbols = self.symbols();
        let dates_by_symbol: BTreeMap<String, Vec<String>> = symbols
            .iter()
            .map(|symbol| (symbol.clone(), self.dates(symbol)))
            .collect();
        let common_dates = symbols
            .iter()
            .filter_map(|symbol| dates_by_symbol.get(symbol))
            .map(|dates| dates.iter().cloned().collect::<HashSet<_>>())
            .reduce(|left, right| left.intersection(&right).cloned().collect())
            .unwrap_or_default();
        let mut common_dates: Vec<_> = common_dates.into_iter().collect();
        common_dates.sort();
        json!({
            "symbols": symbols,
            "dates_by_symbol": dates_by_symbol,
            "common_dates": common_dates,
            "engine": "rust",
            "data_source": "ThetaData",
        })
    }

    pub fn stock_bars(&self, symbol: &str, trading_date: &str) -> anyhow::Result<Arc<Vec<Bar>>> {
        let key = format!("{symbol}|{trading_date}");
        self.stock_cache
            .try_get_with(key, || {
                let path = self
                    .symbol_dir(symbol)
                    .join(format!("date={trading_date}/ohlc.parquet"));
                Ok::<_, Arc<anyhow::Error>>(Arc::new(read_stock_bars(&path).map_err(Arc::new)?))
            })
            .map_err(|error| anyhow!(error.to_string()))
    }

    fn open_interest(
        &self,
        symbol: &str,
        trading_date: &str,
        expiration: &str,
    ) -> anyhow::Result<Arc<OiMap>> {
        let key = format!("{symbol}|{trading_date}|{expiration}");
        self.oi_cache
            .try_get_with(key, || {
                let path = self
                    .option_day_dir(symbol, trading_date)
                    .join(format!("expiration={expiration}/open_interest.parquet"));
                let map = if path.is_file() {
                    read_open_interest(&path)
                } else {
                    Ok(HashMap::new())
                };
                Ok::<_, Arc<anyhow::Error>>(Arc::new(map.map_err(Arc::new)?))
            })
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub fn option_quotes(
        &self,
        symbol: &str,
        trading_date: &str,
        expiration: &str,
        minute: &str,
    ) -> anyhow::Result<Arc<Vec<RawOptionQuote>>> {
        let key = format!("{symbol}|{trading_date}|{expiration}|{minute}");
        self.quote_cache
            .try_get_with(key, || {
                let path = self
                    .option_day_dir(symbol, trading_date)
                    .join(format!("expiration={expiration}/quote_1m.parquet"));
                let expiry = NaiveDate::parse_from_str(expiration, "%Y-%m-%d")
                    .map_err(anyhow::Error::from)
                    .map_err(Arc::new)?;
                let oi = self
                    .open_interest(symbol, trading_date, expiration)
                    .map_err(Arc::new)?;
                let rows =
                    read_option_quotes(&path, symbol, expiry, minute, &oi).map_err(Arc::new)?;
                Ok::<_, Arc<anyhow::Error>>(Arc::new(rows))
            })
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub fn session(&self, symbols: &str, trading_date: &str) -> anyhow::Result<Value> {
        let selected: Vec<String> = symbols
            .split(',')
            .filter(|item| !item.trim().is_empty())
            .map(|item| self.validate_symbol(item))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .fold(Vec::new(), |mut values, item| {
                if !values.contains(&item) && values.len() < 5 {
                    values.push(item);
                }
                values
            });
        anyhow::ensure!(!selected.is_empty(), "Select at least one symbol");
        let mut series = serde_json::Map::new();
        for symbol in &selected {
            self.validate_date(symbol, trading_date)?;
            let bars = self.stock_bars(symbol, trading_date)?;
            series.insert(
                symbol.clone(),
                json!({
                    "bars": bars.as_ref(),
                    "expirations": self.expirations(symbol, trading_date),
                }),
            );
        }
        let timeline = series
            .get(&selected[0])
            .and_then(|value| value["bars"].as_array())
            .map(|bars| {
                bars.iter()
                    .filter_map(|bar| bar["time"].as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(json!({
            "date": trading_date,
            "symbols": selected,
            "timeline": timeline,
            "series": series,
            "engine": "rust",
        }))
    }

    pub fn spot_at(&self, symbol: &str, trading_date: &str, minute: &str) -> anyhow::Result<f64> {
        self.stock_bars(symbol, trading_date)?
            .iter()
            .find(|bar| bar.time == minute)
            .map(|bar| bar.close)
            .ok_or_else(|| anyhow!("No underlying bar at {minute}"))
    }

    pub fn chain(
        &self,
        symbol: &str,
        trading_date: &str,
        minute: &str,
        expiration: &str,
        pricing_mode: &str,
        dealer_model: &str,
    ) -> anyhow::Result<ChainSnapshot> {
        let clean = self.validate_symbol(symbol)?;
        self.validate_date(&clean, trading_date)?;
        anyhow::ensure!(
            self.expirations(&clean, trading_date)
                .contains(&expiration.to_string()),
            "Expiration not found: {expiration}"
        );
        let expiry = NaiveDate::parse_from_str(expiration, "%Y-%m-%d")?;
        let quotes = self.option_quotes(&clean, trading_date, expiration, minute)?;
        anyhow::ensure!(!quotes.is_empty(), "No option quotes at {minute}");
        build_chain(ChainBuild {
            symbol: &clean,
            spot: self.spot_at(&clean, trading_date, minute)?,
            as_of: replay_as_of(trading_date, minute)?,
            expiration: expiry,
            quotes: &quotes,
            pricing_mode,
            dealer_model,
            risk_free_rate: self.risk_free_rate,
            source: "ThetaData",
            quote_interval: "1m",
            oi_frequency: "daily",
            prefer_sdk_greeks: false,
            quote_coverage: 100.0,
            fresh_quote_coverage: 100.0,
            metadata_coverage: if self
                .option_day_dir(&clean, trading_date)
                .join(format!("expiration={expiration}/open_interest.parquet"))
                .is_file()
            {
                100.0
            } else {
                0.0
            },
            spot_age_ms: Some(0),
        })
    }

    pub fn surface(
        &self,
        symbol: &str,
        trading_date: &str,
        minute: &str,
        max_dte: i64,
    ) -> anyhow::Result<SurfaceSnapshot> {
        let clean = self.validate_symbol(symbol)?;
        let day = self.validate_date(&clean, trading_date)?;
        let mut candidates: Vec<String> = self
            .expirations(&clean, trading_date)
            .into_iter()
            .filter(|expiry| {
                NaiveDate::parse_from_str(expiry, "%Y-%m-%d")
                    .map(|expiry| (0..=max_dte).contains(&(expiry - day).num_days()))
                    .unwrap_or(false)
            })
            .collect();
        if candidates.len() > 9 {
            let indexes: HashSet<usize> = (0..9)
                .map(|index| {
                    ((index as f64 * (candidates.len() - 1) as f64 / 8.0).round()) as usize
                })
                .collect();
            candidates = candidates
                .into_iter()
                .enumerate()
                .filter_map(|(index, value)| indexes.contains(&index).then_some(value))
                .collect();
        }
        let chains = candidates
            .iter()
            .filter_map(|expiry| {
                self.chain(&clean, trading_date, minute, expiry, "micro", "classic")
                    .ok()
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(!chains.is_empty(), "No usable expirations at {minute}");
        Ok(build_surface(
            &clean,
            &chains,
            replay_as_of(trading_date, minute)?,
        ))
    }

    pub fn volatility_context(
        &self,
        symbol: &str,
        trading_date: &str,
        minute: &str,
        expiration: &str,
    ) -> anyhow::Result<Value> {
        let clean = self.validate_symbol(symbol)?;
        self.validate_date(&clean, trading_date)?;
        let close_dates: Vec<_> = self
            .available_dates_through(&clean, trading_date)
            .into_iter()
            .filter(|date| date.as_str() < trading_date)
            .collect();
        let closes: Vec<f64> = close_dates
            .iter()
            .filter_map(|value| {
                self.stock_bars(&clean, value)
                    .ok()?
                    .last()
                    .map(|bar| bar.close)
            })
            .filter(|value| value.is_finite() && *value > 0.0)
            .collect();
        let snapshot = self.chain(&clean, trading_date, minute, expiration, "micro", "classic")?;
        let history = self.matched_iv_history(&clean, trading_date, snapshot.dte, minute, 50);
        serde_json::to_value(build_context(VolatilityInput {
            symbol: clean,
            as_of: snapshot.timestamp.clone(),
            expiration: expiration.into(),
            reference_dte: snapshot.dte,
            tte_years: snapshot.tte_years,
            spot: snapshot.spot,
            atm_iv: snapshot.metrics.atm_iv,
            history,
            closes,
            rv_through: close_dates.last().cloned(),
            iv_source: "ThetaData matched-DTE ATM IV".into(),
            rv_source: "ThetaData adjusted underlying closes".into(),
        }))
        .map_err(anyhow::Error::from)
    }

    pub fn live_volatility_context(
        &self,
        snapshot: &ChainSnapshot,
        daily_closes: &[(String, f64)],
    ) -> anyhow::Result<Value> {
        let history = if self.symbol_dir(&snapshot.symbol).is_dir() {
            self.matched_iv_history(
                &snapshot.symbol,
                &snapshot.date,
                snapshot.dte,
                &snapshot.minute,
                50,
            )
        } else {
            Vec::new()
        };
        serde_json::to_value(build_context(VolatilityInput {
            symbol: snapshot.symbol.clone(),
            as_of: snapshot.timestamp.clone(),
            expiration: snapshot.expiration.clone(),
            reference_dte: snapshot.dte,
            tte_years: snapshot.tte_years,
            spot: snapshot.spot,
            atm_iv: snapshot.metrics.atm_iv,
            history,
            closes: daily_closes.iter().map(|(_, close)| *close).collect(),
            rv_through: daily_closes.last().map(|(date, _)| date.clone()),
            iv_source: "ThetaData 50-session matched-DTE ATM IV".into(),
            rv_source: "Longbridge forward-adjusted daily closes".into(),
        }))
        .map_err(anyhow::Error::from)
    }

    fn available_dates_through(&self, symbol: &str, trading_date: &str) -> Vec<String> {
        let mut dates: Vec<_> = self
            .dates(symbol)
            .into_iter()
            .filter(|value| value.as_str() <= trading_date)
            .collect();
        dates.sort();
        dates
    }

    fn matched_iv_history(
        &self,
        symbol: &str,
        trading_date: &str,
        target_dte: i64,
        minute: &str,
        limit: usize,
    ) -> Vec<IvHistoryPoint> {
        let history_minute = point_in_time_history_minute(minute);
        self.available_dates_through(symbol, trading_date)
            .into_iter()
            .rev()
            .filter_map(|date| {
                self.atm_history_iv_for_dte(symbol, &date, target_dte, history_minute)
            })
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    fn atm_history_iv_for_dte(
        &self,
        symbol: &str,
        trading_date: &str,
        target_dte: i64,
        minute: &str,
    ) -> Option<IvHistoryPoint> {
        let day = NaiveDate::parse_from_str(trading_date, "%Y-%m-%d").ok()?;
        let (expiry_date, expiry) = self
            .expirations(symbol, trading_date)
            .into_iter()
            .filter_map(|value| Some((NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok()?, value)))
            .filter(|(date, _)| *date >= day)
            .min_by_key(|(date, _)| ((*date - day).num_days() - target_dte).abs())?;
        let matched_dte = (expiry_date - day).num_days();
        let tolerance = (target_dte.abs() / 2 + 2).clamp(2, 10);
        if (matched_dte - target_dte).abs() > tolerance {
            return None;
        }
        let chain = self
            .chain(symbol, trading_date, minute, &expiry, "mid", "classic")
            .ok()
            .or_else(|| {
                self.chain(symbol, trading_date, "15:30", &expiry, "mid", "classic")
                    .ok()
            })?;
        Some(IvHistoryPoint {
            date: trading_date.into(),
            iv: chain.metrics.atm_iv?,
            dte: matched_dte,
        })
    }
}

fn rounded(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale).round() / scale
}

fn replay_as_of(trading_date: &str, minute: &str) -> anyhow::Result<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(trading_date, "%Y-%m-%d")?;
    let (hour, minute_value) = minute
        .split_once(':')
        .ok_or_else(|| anyhow!("Invalid minute: {minute}"))?;
    New_York
        .with_ymd_and_hms(
            date.year(),
            date.month(),
            date.day(),
            hour.parse()?,
            minute_value.parse()?,
            0,
        )
        .single()
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| anyhow!("Invalid replay time"))
}

fn partition_values(root: &Path, prefix: &str) -> Vec<String> {
    let mut values = std::fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter_map(|name| name.strip_prefix(prefix).map(str::to_owned))
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn point_in_time_history_minute(minute: &str) -> &str {
    if minute > "15:45" { "15:45" } else { minute }
}

fn record_batches(
    path: &Path,
) -> anyhow::Result<impl Iterator<Item = anyhow::Result<RecordBatch>>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?
        .with_batch_size(16_384)
        .build()?;
    Ok(reader.map(|batch| batch.map_err(anyhow::Error::from)))
}

fn typed<'a, T: Array + 'static>(batch: &'a RecordBatch, name: &str) -> anyhow::Result<&'a T> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("missing parquet column {name}"))?
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| anyhow!("unexpected parquet type for {name}"))
}

enum TextColumn<'a> {
    String(&'a StringArray),
    View(&'a StringViewArray),
    Large(&'a LargeStringArray),
}

impl<'a> TextColumn<'a> {
    fn from_batch(batch: &'a RecordBatch, name: &str) -> anyhow::Result<Self> {
        let column = batch
            .column_by_name(name)
            .ok_or_else(|| anyhow!("missing parquet column {name}"))?;
        if let Some(value) = column.as_any().downcast_ref::<StringArray>() {
            return Ok(Self::String(value));
        }
        if let Some(value) = column.as_any().downcast_ref::<StringViewArray>() {
            return Ok(Self::View(value));
        }
        if let Some(value) = column.as_any().downcast_ref::<LargeStringArray>() {
            return Ok(Self::Large(value));
        }
        Err(anyhow!(
            "unexpected parquet text type for {name}: {}",
            column.data_type()
        ))
    }

    fn is_null(&self, row: usize) -> bool {
        match self {
            Self::String(value) => value.is_null(row),
            Self::View(value) => value.is_null(row),
            Self::Large(value) => value.is_null(row),
        }
    }

    fn value(&self, row: usize) -> &str {
        match self {
            Self::String(value) => value.value(row),
            Self::View(value) => value.value(row),
            Self::Large(value) => value.value(row),
        }
    }
}

fn read_stock_bars(path: &Path) -> anyhow::Result<Vec<Bar>> {
    let mut bars = Vec::new();
    for batch in record_batches(path)? {
        let batch = batch?;
        let timestamp = typed::<TimestampMicrosecondArray>(&batch, "timestamp")?;
        let open = typed::<Float64Array>(&batch, "open")?;
        let high = typed::<Float64Array>(&batch, "high")?;
        let low = typed::<Float64Array>(&batch, "low")?;
        let close = typed::<Float64Array>(&batch, "close")?;
        let volume = typed::<Int64Array>(&batch, "volume")?;
        let vwap = typed::<Float64Array>(&batch, "vwap")?;
        for row in 0..batch.num_rows() {
            if timestamp.is_null(row) || close.is_null(row) {
                continue;
            }
            let utc = DateTime::<Utc>::from_timestamp_micros(timestamp.value(row))
                .ok_or_else(|| anyhow!("invalid timestamp"))?;
            let et = utc.with_timezone(&New_York);
            bars.push(Bar {
                time: et.format("%H:%M").to_string(),
                timestamp: utc.to_rfc3339(),
                open: rounded(open.value(row), 4),
                high: rounded(high.value(row), 4),
                low: rounded(low.value(row), 4),
                close: rounded(close.value(row), 4),
                volume: volume.value(row),
                vwap: rounded(vwap.value(row), 4),
            });
        }
    }
    bars.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    Ok(bars)
}

fn read_open_interest(path: &Path) -> anyhow::Result<OiMap> {
    let mut values = HashMap::new();
    for batch in record_batches(path)? {
        let batch = batch?;
        let strike = typed::<Float64Array>(&batch, "strike")?;
        let right = TextColumn::from_batch(&batch, "right")?;
        let oi = typed::<Int64Array>(&batch, "open_interest")?;
        for row in 0..batch.num_rows() {
            if strike.is_null(row) || right.is_null(row) || oi.is_null(row) {
                continue;
            }
            values.insert(
                (
                    (strike.value(row) * 1000.0).round() as i64,
                    right.value(row).to_uppercase(),
                ),
                oi.value(row),
            );
        }
    }
    Ok(values)
}

fn read_option_quotes(
    path: &Path,
    underlying: &str,
    expiration: NaiveDate,
    minute: &str,
    oi: &OiMap,
) -> anyhow::Result<Vec<RawOptionQuote>> {
    let mut quotes = Vec::new();
    for batch in record_batches(path)? {
        let batch = batch?;
        let timestamp = typed::<TimestampMicrosecondArray>(&batch, "timestamp")?;
        let strike = typed::<Float64Array>(&batch, "strike")?;
        let right = TextColumn::from_batch(&batch, "right")?;
        let bid_size = typed::<Int64Array>(&batch, "bid_size")?;
        let ask_size = typed::<Int64Array>(&batch, "ask_size")?;
        let bid = typed::<Float64Array>(&batch, "bid")?;
        let ask = typed::<Float64Array>(&batch, "ask")?;
        for row in 0..batch.num_rows() {
            if timestamp.is_null(row) || strike.is_null(row) || right.is_null(row) {
                continue;
            }
            let utc = DateTime::<Utc>::from_timestamp_micros(timestamp.value(row))
                .ok_or_else(|| anyhow!("invalid timestamp"))?;
            if utc.with_timezone(&New_York).format("%H:%M").to_string() != minute {
                continue;
            }
            let strike_value = strike.value(row);
            let right_value = right.value(row).to_uppercase();
            let bid_value = if bid.is_null(row) {
                f64::NAN
            } else {
                bid.value(row)
            };
            let ask_value = if ask.is_null(row) {
                f64::NAN
            } else {
                ask.value(row)
            };
            quotes.push(RawOptionQuote {
                symbol: format!(
                    "{}{}{}{:08}",
                    underlying,
                    expiration.format("%y%m%d"),
                    if right_value == "CALL" { "C" } else { "P" },
                    (strike_value * 1000.0).round() as i64
                ),
                strike: strike_value,
                right: right_value.clone(),
                bid_size: if bid_size.is_null(row) {
                    0
                } else {
                    bid_size.value(row)
                },
                ask_size: if ask_size.is_null(row) {
                    0
                } else {
                    ask_size.value(row)
                },
                bid: bid_value,
                ask: ask_value,
                last: None,
                volume: 0,
                open_interest: *oi
                    .get(&((strike_value * 1000.0).round() as i64, right_value))
                    .unwrap_or(&0),
                sdk_iv: None,
                sdk_delta: None,
                sdk_gamma: None,
                sdk_theta: None,
                sdk_vega: None,
            });
        }
    }
    Ok(quotes)
}

#[cfg(test)]
mod tests {
    use super::point_in_time_history_minute;

    #[test]
    fn historical_iv_never_looks_ahead_of_early_replay_time() {
        assert_eq!(point_in_time_history_minute("09:31"), "09:31");
        assert_eq!(point_in_time_history_minute("12:30"), "12:30");
        assert_eq!(point_in_time_history_minute("15:55"), "15:45");
    }
}
