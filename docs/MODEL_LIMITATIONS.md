# Model Limitations

This document defines what Option Workstation analytics do and do not claim.

## Implied Volatility and Greeks

The local solver uses Black-Scholes-Merton. US listed equity and ETF options are
generally American-style, so early exercise, discrete dividends, borrow costs,
and assignment behavior are approximated.

Dividend yield is inferred from paired put-call quotes when enough usable pairs
exist; otherwise a cost-of-carry fallback is used. Bad or sparse pairs can
destabilize the inference. SDK-provided values and locally calculated values
must retain distinct provenance.

## SVI

SVI fits a total-variance smile slice from usable quotes. A successful optimizer
status means the fit completed; it does not prove that:

- every input quote was executable;
- the fit is stable outside the observed strike range;
- the slice is free from butterfly arbitrage in price space;
- neighboring expiries are calendar-consistent.

Residuals, constraint checks, coverage, and confidence are first-class outputs.

## Constrained Surface

The 3D surface is a research projection over sparse expiry/strike observations.
It applies total-variance constraints and reports adjustment counts plus
post-projection checks. It must not be described as a mathematically guaranteed
arbitrage-free production surface.

Sparse or heavily adjusted surfaces should remain visibly labeled as research.

## Realized Volatility and VRP

RV5/RV10/RV20 use completed forward-adjusted daily closes before the active
session. They do not include intraday range estimators or overnight/intraday
decomposition.

VRP is displayed as ATM IV minus realized volatility for the selected window.
It is not a forecast of option return and ignores path, skew, convexity,
transaction costs, and hedging error.

## IV Rank and Percentile

History is matched by DTE and ATM context. The UI reports sample count and the
last observation date. A 50-session sample is not equivalent to a one-year
rank. Small samples and changing market regimes can make rank or percentile
unstable.

## Expected Move

Expected move is proportional to:

```text
spot * ATM IV * sqrt(time to 16:00 ET expiry)
```

It is a volatility-scale estimate, not a confidence interval or directional
forecast. For 0DTE it uses the exact remaining fraction of the session.

## Dealer Exposure

GEX, vanna, and charm aggregate contract Greeks with OI and a configurable
dealer-sign convention. Dealer positions are not observed. Results are scenario
indicators under an inventory assumption, not a map of actual dealer books.

OI is slower metadata and may be stale. Metrics that require it stay unavailable
until coverage passes the relevant gate.

## Strategy Risk

Strategy entry and liquidation use executable sides:

- buy at ask;
- sell at bid.

The payoff graph and scenario matrix are model outputs. They do not include all
sources of risk:

- queue position and price impact;
- spread changes between leg submissions;
- partial fills and rejection;
- early assignment and ex-dividend events;
- locate/borrow constraints;
- commissions, fees, taxes, and regulatory charges unless explicitly added;
- volatility dynamics beyond the selected scenario shocks.

Probability of profit and margin are approximations, not broker guarantees.

## Replay Claims

A historical result is only as valid as its acquisition timestamps, contract
selection, bid/ask observations, fee model, and holdout discipline. Configuration
success, passing tests, and profitable history are separate claims.

The workstation does not ship a strategy performance claim.
