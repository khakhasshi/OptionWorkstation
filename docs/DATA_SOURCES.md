# Data Sources and Replay Contract

## Licensing Boundary

Option Workstation source code is Apache-2.0. Market data is not.

This repository does not contain and does not grant rights to redistribute
ThetaData, Longbridge, exchange, or broker data. Operators must obtain their own
subscriptions and comply with provider terms. Do not attach raw market data to
issues or pull requests.

## Historical Replay

The Rust replay engine expects Hive-style partitions under
`OPTION_WORKSTATION_DATA_ROOT`.

```text
<root>/
├── underlying/
│   └── symbol=<SYMBOL>/
│       └── date=<YYYY-MM-DD>/
│           └── ohlc.parquet
└── options/
    └── symbol=<SYMBOL>/
        └── date=<YYYY-MM-DD>/
            └── expiration=<YYYY-MM-DD>/
                ├── quote_1m.parquet
                └── open_interest.parquet
```

Symbols are normalized to uppercase and may be requested with or without the
`.US` suffix.

### Underlying Bars

`ohlc.parquet` requires:

| Column | Arrow type | Meaning |
| --- | --- | --- |
| `timestamp` | timestamp microsecond | UTC event timestamp |
| `open` | float64 | minute open |
| `high` | float64 | minute high |
| `low` | float64 | minute low |
| `close` | float64 | minute close |
| `volume` | int64 | minute volume |
| `vwap` | float64 | minute VWAP |

Timestamps are converted to `America/New_York` for replay labels. The engine
does not infer missing bars.

### Option Quotes

`quote_1m.parquet` requires:

| Column | Arrow type | Meaning |
| --- | --- | --- |
| `timestamp` | timestamp microsecond | UTC quote timestamp |
| `strike` | float64 | unscaled strike |
| `right` | string/string-view/large-string | `CALL` or `PUT` |
| `bid_size` | int64 | displayed bid size |
| `ask_size` | int64 | displayed ask size |
| `bid` | float64 | bid |
| `ask` | float64 | ask |

Rows are selected by exact ET minute. Missing bid/ask values remain unusable;
the engine does not replace them with last or midpoint values.

### Open Interest

`open_interest.parquet` requires:

| Column | Arrow type | Meaning |
| --- | --- | --- |
| `strike` | float64 | unscaled strike |
| `right` | string/string-view/large-string | `CALL` or `PUT` |
| `open_interest` | int64 | provider OI observation |

Open interest is daily metadata. The operator is responsible for ensuring that
the file's publication timestamp is valid for the replay timestamp. A same-date
partition name alone does not prove point-in-time availability.

## Point-in-Time Rules

- A replay request must use a date partition that exists for the symbol.
- Underlying and option observations are selected at the active minute.
- Historical IV context only uses dates through the active trading date.
- Intraday history samples use the active minute, capped at 15:45 ET for later
  observations.
- Completed-close RV excludes the active live session.
- Inputs whose original availability cannot be established should be labeled
  research-only or excluded.

Do not silently backfill a missing quote with a later observation.

## Live Longbridge Data

Live mode uses the official Longbridge Rust SDK for:

- underlying quotes and candlesticks;
- option-chain definitions and depth subscriptions;
- provider metrics when entitled;
- paper account and order operations.

Provider permissions and endpoint limits vary by account and market. The live
engine keeps quote freshness and slower OI/volume metadata coverage separate.
Unavailable metadata must not be interpreted as zero inventory.

Longbridge aggregate option data is not a substitute for a historical full-chain
NBBO replay dataset. A backtest claiming executable historical fills needs the
actual contract and quote observations available at each signal time.

## Data Quality States

Every consumer should distinguish:

- **available**: a field exists;
- **fresh**: its timestamp is within the active threshold;
- **usable**: it passes price, spread, and model input gates;
- **complete**: required metadata coverage passes the metric threshold;
- **trusted**: all conditions for the panel's stated interpretation pass.

Zero, null, stale, and unavailable are not interchangeable.

## Issue Reports

For a data-quality bug, provide:

- provider and acquisition method;
- schema only, unless redistribution is permitted;
- symbol/date/minute/expiry and timezone;
- redacted row counts and min/max statistics;
- expected and actual quality state;
- a synthetic reproducer when possible.

Never upload licensed rows or account-linked data without explicit rights.
