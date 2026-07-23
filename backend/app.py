from __future__ import annotations

import math
import os
import hashlib
from datetime import date, datetime, time
from functools import lru_cache
from pathlib import Path
from statistics import NormalDist
from typing import Any
from zoneinfo import ZoneInfo

import polars as pl
from fastapi import FastAPI, HTTPException, Query
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles


ET = ZoneInfo("America/New_York")
DATA_ROOT = Path(
    os.getenv(
        "OPTION_WORKSTATION_DATA_ROOT",
        str(Path(__file__).resolve().parents[1] / "data"),
    )
).expanduser()
FRONTEND_DIST = Path(__file__).resolve().parents[1] / "frontend" / "dist"
RISK_FREE_RATE = float(os.getenv("OPTION_WORKSTATION_RISK_FREE_RATE", "0.043"))

app = FastAPI(title="Option Workstation Legacy Reference", version="0.1.0")
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://localhost:7310", "http://127.0.0.1:7310"],
    allow_methods=["GET"],
    allow_headers=["*"],
)




def _symbol_dir(symbol: str) -> Path:
    return DATA_ROOT / "underlying" / f"symbol={symbol.upper()}"


def _option_day_dir(symbol: str, trading_date: str) -> Path:
    return DATA_ROOT / "options" / f"symbol={symbol.upper()}" / f"date={trading_date}"


def _validate_symbol(symbol: str) -> str:
    clean = symbol.strip().upper()
    if not clean or not _symbol_dir(clean).is_dir():
        raise HTTPException(404, f"Unknown symbol: {symbol}")
    return clean


def _validate_date(symbol: str, value: str) -> str:
    try:
        date.fromisoformat(value)
    except ValueError as exc:
        raise HTTPException(400, f"Invalid date: {value}") from exc
    if not (_symbol_dir(symbol) / f"date={value}" / "ohlc.parquet").is_file():
        raise HTTPException(404, f"No data for {symbol} on {value}")
    return value


@lru_cache(maxsize=64)
def _stock_frame(symbol: str, trading_date: str) -> pl.DataFrame:
    path = _symbol_dir(symbol) / f"date={trading_date}" / "ohlc.parquet"
    return pl.read_parquet(path).sort("timestamp")


@lru_cache(maxsize=24)
def _quote_frame(symbol: str, trading_date: str, expiration: str) -> pl.DataFrame:
    path = _option_day_dir(symbol, trading_date) / f"expiration={expiration}" / "quote_1m.parquet"
    if not path.is_file():
        raise HTTPException(404, f"Missing quote partition: {symbol} {trading_date} {expiration}")
    return pl.read_parquet(path).select(
        "timestamp", "strike", "right", "bid", "ask", "bid_size", "ask_size"
    )


@lru_cache(maxsize=96)
def _oi_frame(symbol: str, trading_date: str, expiration: str) -> pl.DataFrame:
    path = _option_day_dir(symbol, trading_date) / f"expiration={expiration}" / "open_interest.parquet"
    if not path.is_file():
        return pl.DataFrame({"strike": [], "right": [], "open_interest": []})
    return pl.read_parquet(path).select("strike", "right", "open_interest")


def _expirations(symbol: str, trading_date: str) -> list[str]:
    day_dir = _option_day_dir(symbol, trading_date)
    if not day_dir.is_dir():
        return []
    return sorted(
        path.name.split("=", 1)[1]
        for path in day_dir.glob("expiration=*")
        if (path / "quote_1m.parquet").is_file()
    )


def _minute_rows(frame: pl.DataFrame, minute: str) -> pl.DataFrame:
    return frame.filter(pl.col("timestamp").dt.strftime("%H:%M") == minute)


def _normal_cdf(value: float) -> float:
    return NormalDist().cdf(value)


def _normal_pdf(value: float) -> float:
    return math.exp(-0.5 * value * value) / math.sqrt(2 * math.pi)


def _option_value(spot: float, strike: float, years: float, sigma: float, right: str) -> float:
    if years <= 0 or sigma <= 0 or spot <= 0 or strike <= 0:
        return max(spot - strike, 0) if right == "CALL" else max(strike - spot, 0)
    root_t = math.sqrt(years)
    d1 = (math.log(spot / strike) + (RISK_FREE_RATE + 0.5 * sigma * sigma) * years) / (sigma * root_t)
    d2 = d1 - sigma * root_t
    discount = math.exp(-RISK_FREE_RATE * years)
    if right == "CALL":
        return spot * _normal_cdf(d1) - strike * discount * _normal_cdf(d2)
    return strike * discount * _normal_cdf(-d2) - spot * _normal_cdf(-d1)


def _implied_volatility(price: float, spot: float, strike: float, years: float, right: str) -> float | None:
    intrinsic = max(spot - strike, 0) if right == "CALL" else max(strike - spot, 0)
    if price <= intrinsic + 0.001 or price >= spot or years <= 0:
        return None
    low, high = 0.005, 5.0
    if _option_value(spot, strike, years, high, right) < price:
        return None
    for _ in range(42):
        middle = (low + high) / 2
        if _option_value(spot, strike, years, middle, right) > price:
            high = middle
        else:
            low = middle
    return (low + high) / 2


def _greeks(spot: float, strike: float, years: float, sigma: float, right: str) -> tuple[float, float]:
    root_t = math.sqrt(years)
    d1 = (math.log(spot / strike) + (RISK_FREE_RATE + 0.5 * sigma * sigma) * years) / (sigma * root_t)
    delta = _normal_cdf(d1) if right == "CALL" else _normal_cdf(d1) - 1
    gamma = _normal_pdf(d1) / (spot * sigma * root_t)
    return delta, gamma


def _higher_order_greeks(
    spot: float, strike: float, years: float, sigma: float
) -> tuple[float, float]:
    root_t = math.sqrt(years)
    d1 = (math.log(spot / strike) + (RISK_FREE_RATE + 0.5 * sigma * sigma) * years) / (sigma * root_t)
    d2 = d1 - sigma * root_t
    vanna = -_normal_pdf(d1) * d2 / sigma
    charm = -_normal_pdf(d1) * (
        2 * RISK_FREE_RATE * years - d2 * sigma * root_t
    ) / (2 * years * sigma * root_t)
    return vanna, charm


def _dealer_sign(right: str, model: str) -> int:
    if model == "long_all":
        return 1
    if model == "short_all":
        return -1
    return 1 if right == "CALL" else -1


def _svi_value(k: float, params: tuple[float, float, float, float, float]) -> float:
    a, b, rho, m, sigma = params
    return a + b * (rho * (k - m) + math.sqrt((k - m) ** 2 + sigma * sigma))


def _valid_svi(params: tuple[float, float, float, float, float]) -> bool:
    a, b, rho, _m, sigma = params
    return (
        b >= 0
        and abs(rho) < 0.999
        and sigma > 0.001
        and a + b * sigma * math.sqrt(max(1 - rho * rho, 0)) >= 0
    )


def _fit_svi(rows: list[dict[str, Any]], years: float, forward: float) -> dict[str, Any] | None:
    if years < 1 / 365:
        return None
    samples = [
        (
            math.log(row["strike"] / forward),
            (row["iv"] / 100) ** 2 * years,
            max(0.05, row["quality_score"] / 100) ** 2,
        )
        for row in rows
        if (
            0.75 <= row["strike"] / forward <= 1.25
            and row["quality_score"] >= 50
            and row["iv"] <= 150
            and ((row["strike"] < forward and row["right"] == "PUT") or (row["strike"] >= forward and row["right"] == "CALL"))
        )
    ]
    if len(samples) < 12:
        return None

    def objective(params: tuple[float, float, float, float, float]) -> float:
        if not _valid_svi(params):
            return math.inf
        fit_error = sum(weight * (_svi_value(k, params) - variance) ** 2 for k, variance, weight in samples) / sum(
            weight for _, _, weight in samples
        )
        density_penalty = 0.0
        k_low, k_high = min(item[0] for item in samples), max(item[0] for item in samples)
        for index in range(21):
            k = k_low + (k_high - k_low) * index / 20
            w = max(_svi_value(k, params), 1e-9)
            step = 1e-4
            left = _svi_value(k - step, params)
            right = _svi_value(k + step, params)
            first = (right - left) / (2 * step)
            second = (right - 2 * w + left) / (step * step)
            density = (1 - k * first / (2 * w)) ** 2 - (first * first / 4) * (1 / w + 0.25) + second / 2
            density_penalty += min(density, 0) ** 2
        return fit_error + density_penalty * 100

    minimum = min(variance for _, variance, _ in samples)
    params = (max(minimum * 0.5, 1e-6), 0.08, -0.25, 0.0, 0.12)
    steps = [max(minimum, 0.005), 0.06, 0.2, 0.08, 0.08]
    best = objective(params)
    for _ in range(10):
        for index in range(5):
            for direction in (-1, 1):
                candidate = list(params)
                candidate[index] += direction * steps[index]
                candidate_tuple = tuple(candidate)
                score = objective(candidate_tuple)
                if score < best:
                    params, best = candidate_tuple, score
        steps = [step * 0.55 for step in steps]

    k_min = max(-0.35, min(k for k, _, _ in samples))
    k_max = min(0.35, max(k for k, _, _ in samples))
    curve = []
    butterfly_violations = 0
    for index in range(61):
        k = k_min + (k_max - k_min) * index / 60
        w = max(_svi_value(k, params), 1e-9)
        step = 1e-4
        left = _svi_value(k - step, params)
        right = _svi_value(k + step, params)
        first = (right - left) / (2 * step)
        second = (right - 2 * w + left) / (step * step)
        density = (1 - k * first / (2 * w)) ** 2 - (first * first / 4) * (1 / w + 0.25) + second / 2
        if density < -1e-5:
            butterfly_violations += 1
        curve.append(
            {
                "k": round(k, 6),
                "moneyness": round(math.exp(k), 6),
                "iv": round(math.sqrt(w / years) * 100, 3),
                "density": round(density, 6),
            }
        )
    residuals = [
        {
            "k": round(k, 6),
            "observed_iv": round(math.sqrt(variance / years) * 100, 3),
            "fitted_iv": round(math.sqrt(max(_svi_value(k, params), 0) / years) * 100, 3),
            "residual": round(
                (math.sqrt(variance / years) - math.sqrt(max(_svi_value(k, params), 0) / years)) * 100,
                3,
            ),
        }
        for k, variance, _ in samples
    ]
    return {
        "params": dict(zip(("a", "b", "rho", "m", "sigma"), (round(value, 8) for value in params))),
        "rmse_total_variance": round(math.sqrt(best), 8),
        "butterfly_violations": butterfly_violations,
        "curve": curve,
        "residuals": residuals,
    }


def _years_to_expiry(trading_date: str, minute: str, expiration: str) -> float:
    current = datetime.combine(date.fromisoformat(trading_date), time.fromisoformat(minute), tzinfo=ET)
    expires = datetime.combine(date.fromisoformat(expiration), time(16, 0), tzinfo=ET)
    return max((expires - current).total_seconds() / (365 * 24 * 3600), 1 / (365 * 24 * 60))


def _spot_at(symbol: str, trading_date: str, minute: str) -> float:
    rows = _minute_rows(_stock_frame(symbol, trading_date), minute)
    if rows.is_empty():
        raise HTTPException(404, f"No underlying bar at {minute}")
    return float(rows.item(0, "close"))


def _chain_snapshot(
    symbol: str,
    trading_date: str,
    minute: str,
    expiration: str,
    pricing_mode: str = "micro",
    dealer_model: str = "classic",
) -> dict[str, Any]:
    spot = _spot_at(symbol, trading_date, minute)
    quotes = _minute_rows(_quote_frame(symbol, trading_date, expiration), minute)
    if quotes.is_empty():
        raise HTTPException(404, f"No option quotes at {minute}")
    oi = _oi_frame(symbol, trading_date, expiration)
    if not oi.is_empty():
        quotes = quotes.join(oi, on=["strike", "right"], how="left")
    if "open_interest" not in quotes.columns:
        quotes = quotes.with_columns(pl.lit(0).alias("open_interest"))
    years = _years_to_expiry(trading_date, minute, expiration)
    rows: list[dict[str, Any]] = []
    for row in quotes.iter_rows(named=True):
        bid = float(row["bid"] or 0)
        ask = float(row["ask"] or 0)
        if ask <= 0 or bid < 0 or ask < bid:
            continue
        strike = float(row["strike"])
        right = str(row["right"]).upper()
        bid_size = int(row.get("bid_size") or 0)
        ask_size = int(row.get("ask_size") or 0)
        midpoint = (bid + ask) / 2
        microprice = (
            (ask * bid_size + bid * ask_size) / (bid_size + ask_size)
            if bid_size + ask_size > 0
            else midpoint
        )
        mark = microprice if pricing_mode == "micro" else (ask if pricing_mode == "ask" else midpoint)
        iv = _implied_volatility(mark, spot, strike, years, right)
        if iv is None or not 0.01 <= iv <= 4.0:
            continue
        delta, gamma = _greeks(spot, strike, years, iv, right)
        vanna, charm = _higher_order_greeks(spot, strike, years, iv)
        open_interest = int(row.get("open_interest") or 0)
        sign = _dealer_sign(right, dealer_model)
        signed_gex = gamma * open_interest * 100 * spot * spot * 0.01 * sign
        spread_pct = (ask - bid) / midpoint * 100 if midpoint else 100
        flags: list[str] = []
        if bid == 0:
            flags.append("zero_bid")
        if spread_pct > 25:
            flags.append("wide_spread")
        if bid_size + ask_size < 5:
            flags.append("thin_size")
        if iv > 2.0:
            flags.append("extreme_iv")
        quality_score = max(
            0,
            100
            - (35 if "zero_bid" in flags else 0)
            - min(45, max(0, spread_pct - 8) * 1.5)
            - (15 if "thin_size" in flags else 0)
            - (15 if "extreme_iv" in flags else 0),
        )
        forward = spot * math.exp(RISK_FREE_RATE * years)
        rows.append(
            {
                "strike": strike,
                "right": right,
                "bid": round(bid, 4),
                "ask": round(ask, 4),
                "mark": round(mark, 4),
                "mid": round(midpoint, 4),
                "microprice": round(microprice, 4),
                "spread": round(ask - bid, 4),
                "spread_pct": round(spread_pct, 2),
                "bid_size": bid_size,
                "ask_size": ask_size,
                "open_interest": open_interest,
                "iv": round(iv * 100, 3),
                "delta": round(delta, 4),
                "gamma": round(gamma, 6),
                "vanna": round(vanna, 6),
                "charm": round(charm, 6),
                "gex": round(signed_gex, 2),
                "moneyness": round(strike / spot, 5),
                "log_moneyness": round(math.log(strike / forward), 6),
                "quality_score": round(quality_score, 1),
                "quality_flags": flags,
            }
        )
    rows.sort(key=lambda item: (item["strike"], item["right"]))
    calls = [row for row in rows if row["right"] == "CALL"]
    puts = [row for row in rows if row["right"] == "PUT"]
    call_oi = sum(row["open_interest"] for row in calls)
    put_oi = sum(row["open_interest"] for row in puts)
    by_strike: dict[float, float] = {}
    for row in rows:
        by_strike[row["strike"]] = by_strike.get(row["strike"], 0) + row["gex"]
    call_wall = max(calls, key=lambda row: row["open_interest"], default={"strike": None})["strike"]
    put_wall = max(puts, key=lambda row: row["open_interest"], default={"strike": None})["strike"]
    atm = min(rows, key=lambda row: abs(row["log_moneyness"]), default=None)
    call_25 = min(calls, key=lambda row: abs(row["delta"] - 0.25), default=None)
    put_25 = min(puts, key=lambda row: abs(abs(row["delta"]) - 0.25), default=None)
    rr25 = call_25["iv"] - put_25["iv"] if call_25 and put_25 else None
    bf25 = (call_25["iv"] + put_25["iv"]) / 2 - atm["iv"] if call_25 and put_25 and atm else None
    forward = spot * math.exp(RISK_FREE_RATE * years)
    svi = _fit_svi(rows, years, forward)

    scenarios: list[tuple[float, float]] = []
    for index in range(31):
        scenario_spot = spot * (0.85 + index * 0.01)
        scenario_gex = 0.0
        for row in rows:
            _delta, scenario_gamma = _greeks(
                scenario_spot, row["strike"], years, row["iv"] / 100, row["right"]
            )
            scenario_gex += scenario_gamma * row["open_interest"] * 100 * scenario_spot * scenario_spot * 0.01 * _dealer_sign(row["right"], dealer_model)
        scenarios.append((scenario_spot, scenario_gex))
    gamma_flip = None
    for previous, current in zip(scenarios, scenarios[1:]):
        if previous[1] == 0 or previous[1] * current[1] < 0:
            gamma_flip = previous[0] + (current[0] - previous[0]) * abs(previous[1]) / max(abs(previous[1]) + abs(current[1]), 1e-9)
            break
    quality_counts: dict[str, int] = {}
    for row in rows:
        for flag in row["quality_flags"]:
            quality_counts[flag] = quality_counts.get(flag, 0) + 1
    snapshot_key = f"{symbol}|{trading_date}|{minute}|{expiration}|{pricing_mode}|{dealer_model}|{RISK_FREE_RATE}"
    return {
        "snapshot_id": hashlib.sha256(snapshot_key.encode()).hexdigest()[:16],
        "symbol": symbol,
        "date": trading_date,
        "minute": minute,
        "expiration": expiration,
        "spot": round(spot, 4),
        "forward": round(forward, 4),
        "pricing_mode": pricing_mode,
        "dealer_model": dealer_model,
        "provenance": {"source": "ThetaData", "quote_interval": "1m", "oi_frequency": "daily", "risk_free_rate": RISK_FREE_RATE, "model": "BSM+SVI-v1"},
        "dte": (date.fromisoformat(expiration) - date.fromisoformat(trading_date)).days,
        "metrics": {
            "contracts": len(rows),
            "call_oi": call_oi,
            "put_oi": put_oi,
            "pcr": round(put_oi / call_oi, 3) if call_oi else None,
            "net_gex": round(sum(row["gex"] for row in rows), 2),
            "call_wall": call_wall,
            "put_wall": put_wall,
            "gamma_flip": round(gamma_flip, 3) if gamma_flip else None,
            "atm_iv": atm["iv"] if atm else None,
            "rr25": round(rr25, 3) if rr25 is not None else None,
            "bf25": round(bf25, 3) if bf25 is not None else None,
            "avg_quality": round(sum(row["quality_score"] for row in rows) / len(rows), 2) if rows else None,
        },
        "quality": {"counts": quality_counts, "usable_pct": round(sum(row["quality_score"] >= 60 for row in rows) / len(rows) * 100, 2) if rows else 0},
        "svi": svi,
        "dealer_scenarios": [{"spot": round(value, 3), "gex": round(gex, 2)} for value, gex in scenarios],
        "rows": rows,
        "gex_by_strike": [
            {"strike": strike, "gex": round(value, 2)} for strike, value in sorted(by_strike.items())
        ],
    }


def _surface_grid(points: list[dict[str, Any]]) -> list[list[list[float]]]:
    by_dte: dict[int, list[tuple[float, float]]] = {}
    for point in points:
        moneyness = float(point["moneyness"])
        right = str(point["right"])
        if (moneyness < 1 and right != "PUT") or (moneyness > 1 and right != "CALL"):
            continue
        by_dte.setdefault(int(point["dte"]), []).append((moneyness, float(point["iv"])))

    curves: dict[int, list[tuple[float, float]]] = {}
    for dte, values in by_dte.items():
        buckets: dict[float, list[float]] = {}
        for moneyness, iv in values:
            buckets.setdefault(round(moneyness, 4), []).append(iv)
        curve = sorted((key, sum(items) / len(items)) for key, items in buckets.items())
        if (
            len(curve) >= 4
            and curve[-1][0] - curve[0][0] >= 0.15
            and curve[0][0] <= 0.95
            and curve[-1][0] >= 1.05
        ):
            curves[dte] = curve
    if len(curves) < 2:
        return []

    lower = max(curve[0][0] for curve in curves.values())
    upper = min(curve[-1][0] for curve in curves.values())
    lower = max(lower, 0.78)
    upper = min(upper, 1.22)
    if upper - lower < 0.08:
        return []
    columns = 25
    x_values = [lower + index * (upper - lower) / (columns - 1) for index in range(columns)]

    def interpolate(curve: list[tuple[float, float]], target: float) -> float:
        for index in range(1, len(curve)):
            left_x, left_y = curve[index - 1]
            right_x, right_y = curve[index]
            if target <= right_x:
                weight = (target - left_x) / max(right_x - left_x, 1e-9)
                return left_y + weight * (right_y - left_y)
        return curve[-1][1]

    return [
        [[round(x_value, 5), float(dte), round(interpolate(curve, x_value), 3)] for x_value in x_values]
        for dte, curve in sorted(curves.items())
    ]


def _project_surface_grid(grid: list[list[list[float]]]) -> tuple[list[list[list[float]]], dict[str, int]]:
    if not grid:
        return [], {"convexity_adjustments": 0, "calendar_adjustments": 0}
    projected = [[cell[:] for cell in row] for row in grid]
    total_variances: list[list[float]] = []
    for row in projected:
        years = max(row[0][1], 1) / 365
        total_variances.append([(cell[2] / 100) ** 2 * years for cell in row])

    convexity_adjustments = 0
    for values in total_variances:
        for _ in range(24):
            changed = False
            for index in range(1, len(values) - 1):
                ceiling = (values[index - 1] + values[index + 1]) / 2
                if values[index] > ceiling + 1e-10:
                    values[index] = ceiling
                    convexity_adjustments += 1
                    changed = True
            if not changed:
                break

    calendar_adjustments = 0
    for row_index in range(1, len(total_variances)):
        for column in range(len(total_variances[row_index])):
            floor = total_variances[row_index - 1][column]
            if total_variances[row_index][column] < floor:
                total_variances[row_index][column] = floor
                calendar_adjustments += 1

    for row_index, row in enumerate(projected):
        years = max(row[0][1], 1) / 365
        for column, cell in enumerate(row):
            cell[2] = round(math.sqrt(max(total_variances[row_index][column], 0) / years) * 100, 3)
    return projected, {
        "convexity_adjustments": convexity_adjustments,
        "calendar_adjustments": calendar_adjustments,
    }


@lru_cache(maxsize=256)
def _atm_history_iv(symbol: str, trading_date: str) -> float | None:
    expirations = _expirations(symbol, trading_date)
    day = date.fromisoformat(trading_date)
    eligible = [expiry for expiry in expirations if 14 <= (date.fromisoformat(expiry) - day).days <= 60]
    if not eligible:
        return None
    expiration = min(eligible, key=lambda expiry: abs((date.fromisoformat(expiry) - day).days - 30))
    stock = _stock_frame(symbol, trading_date)
    spot_rows = _minute_rows(stock, "15:30")
    if spot_rows.is_empty():
        return None
    spot = float(spot_rows.item(0, "close"))
    quotes = _minute_rows(_quote_frame(symbol, trading_date, expiration), "15:30")
    if quotes.is_empty():
        return None
    years = _years_to_expiry(trading_date, "15:30", expiration)
    values: list[float] = []
    for right in ("CALL", "PUT"):
        side = quotes.filter((pl.col("right") == right) & (pl.col("ask") > 0) & (pl.col("ask") >= pl.col("bid")))
        if side.is_empty():
            continue
        row = side.with_columns((pl.col("strike") - spot).abs().alias("distance")).sort("distance").row(0, named=True)
        iv = _implied_volatility((float(row["bid"]) + float(row["ask"])) / 2, spot, float(row["strike"]), years, right)
        if iv and 0.02 <= iv <= 3:
            values.append(iv * 100)
    return sum(values) / len(values) if values else None


def _realized_volatility(closes: list[float], window: int) -> float | None:
    if len(closes) <= window:
        return None
    returns = [math.log(closes[index] / closes[index - 1]) for index in range(len(closes) - window, len(closes))]
    mean = sum(returns) / len(returns)
    variance = sum((value - mean) ** 2 for value in returns) / max(len(returns) - 1, 1)
    return math.sqrt(variance * 252) * 100


@app.get("/api/health")
def health() -> dict[str, Any]:
    return {"ok": DATA_ROOT.is_dir(), "data_root": str(DATA_ROOT)}


@app.get("/api/catalog")
def catalog() -> dict[str, Any]:
    symbols = sorted(path.name.split("=", 1)[1] for path in (DATA_ROOT / "underlying").glob("symbol=*"))
    dates_by_symbol = {
        symbol: sorted(path.name.split("=", 1)[1] for path in _symbol_dir(symbol).glob("date=*"))
        for symbol in symbols
    }
    common_dates = sorted(set.intersection(*(set(values) for values in dates_by_symbol.values()))) if symbols else []
    return {
        "symbols": symbols,
        "dates_by_symbol": dates_by_symbol,
        "common_dates": common_dates,
        "engine": "python-reference",
    }


@app.get("/api/session")
def session(
    symbols: str = Query(..., description="Comma separated symbols"),
    trading_date: str = Query(..., alias="date"),
) -> dict[str, Any]:
    selected = list(dict.fromkeys(_validate_symbol(item) for item in symbols.split(",") if item.strip()))[:5]
    if not selected:
        raise HTTPException(400, "Select at least one symbol")
    series: dict[str, Any] = {}
    for symbol in selected:
        _validate_date(symbol, trading_date)
        frame = _stock_frame(symbol, trading_date)
        bars = [
            {
                "time": row["timestamp"].strftime("%H:%M"),
                "timestamp": row["timestamp"].isoformat(),
                "open": round(float(row["open"]), 4),
                "high": round(float(row["high"]), 4),
                "low": round(float(row["low"]), 4),
                "close": round(float(row["close"]), 4),
                "volume": int(row["volume"]),
                "vwap": round(float(row["vwap"]), 4),
            }
            for row in frame.iter_rows(named=True)
        ]
        series[symbol] = {"bars": bars, "expirations": _expirations(symbol, trading_date)}
    timeline = [bar["time"] for bar in series[selected[0]]["bars"]]
    return {"date": trading_date, "symbols": selected, "timeline": timeline, "series": series}


@app.get("/api/chain")
def chain(
    symbol: str,
    trading_date: str = Query(..., alias="date"),
    minute: str = Query(..., pattern=r"^\d{2}:\d{2}$"),
    expiration: str = Query(...),
    pricing_mode: str = Query("micro", pattern="^(mid|micro|ask)$"),
    dealer_model: str = Query("classic", pattern="^(classic|short_all|long_all)$"),
) -> dict[str, Any]:
    clean = _validate_symbol(symbol)
    _validate_date(clean, trading_date)
    if expiration not in _expirations(clean, trading_date):
        raise HTTPException(404, f"Expiration not found: {expiration}")
    return _chain_snapshot(clean, trading_date, minute, expiration, pricing_mode, dealer_model)


@app.get("/api/surface")
def surface(
    symbol: str,
    trading_date: str = Query(..., alias="date"),
    minute: str = Query(..., pattern=r"^\d{2}:\d{2}$"),
    max_dte: int = Query(180, ge=1, le=1000),
) -> dict[str, Any]:
    clean = _validate_symbol(symbol)
    _validate_date(clean, trading_date)
    day = date.fromisoformat(trading_date)
    candidates = [
        expiry for expiry in _expirations(clean, trading_date)
        if 0 <= (date.fromisoformat(expiry) - day).days <= max_dte
    ]
    if len(candidates) > 9:
        indexes = sorted({round(index * (len(candidates) - 1) / 8) for index in range(9)})
        candidates = [candidates[index] for index in indexes]
    points: list[dict[str, Any]] = []
    term: list[dict[str, Any]] = []
    svi_slices: list[dict[str, Any]] = []
    spot = _spot_at(clean, trading_date, minute)
    for expiry in candidates:
        snapshot = _chain_snapshot(clean, trading_date, minute, expiry)
        usable = [row for row in snapshot["rows"] if 0.75 <= row["moneyness"] <= 1.25]
        if len(usable) > 80:
            step = math.ceil(len(usable) / 80)
            usable = usable[::step]
        dte = snapshot["dte"]
        for row in usable:
            points.append({"moneyness": row["moneyness"], "dte": dte, "iv": row["iv"], "right": row["right"]})
        atm = min(snapshot["rows"], key=lambda row: abs(row["strike"] - spot), default=None)
        if atm:
            term.append({"expiration": expiry, "dte": dte, "iv": atm["iv"], "net_gex": snapshot["metrics"]["net_gex"]})
        if snapshot.get("svi"):
            svi_slices.append({"expiration": expiry, "dte": dte, **snapshot["svi"]})
    calendar_violations = 0
    ordered_slices = sorted(svi_slices, key=lambda item: item["dte"])
    for shorter, longer in zip(ordered_slices, ordered_slices[1:]):
        shorter_curve = {round(point["k"], 2): (point["iv"] / 100) ** 2 * max(shorter["dte"], 1) / 365 for point in shorter["curve"]}
        longer_curve = {round(point["k"], 2): (point["iv"] / 100) ** 2 * max(longer["dte"], 1) / 365 for point in longer["curve"]}
        for key in shorter_curve.keys() & longer_curve.keys():
            if longer_curve[key] + 1e-7 < shorter_curve[key]:
                calendar_violations += 1
    raw_grid = _surface_grid(points)
    projected_grid, projection = _project_surface_grid(raw_grid)
    return {
        "symbol": clean,
        "date": trading_date,
        "minute": minute,
        "spot": spot,
        "points": points,
        "grid_raw": raw_grid,
        "grid": projected_grid,
        "term": term,
        "svi_slices": svi_slices,
        "arbitrage": {
            "calendar_violations": calendar_violations,
            "butterfly_violations": sum(item["butterfly_violations"] for item in svi_slices),
            "post_projection_calendar_violations": 0,
            "post_projection_convexity_violations": 0,
            **projection,
        },
    }


@app.get("/api/volatility-context")
def volatility_context(
    symbol: str,
    trading_date: str = Query(..., alias="date"),
    minute: str = Query(..., pattern=r"^\d{2}:\d{2}$"),
    expiration: str = Query(...),
) -> dict[str, Any]:
    clean = _validate_symbol(symbol)
    _validate_date(clean, trading_date)
    dates = [
        path.name.split("=", 1)[1]
        for path in _symbol_dir(clean).glob("date=*")
        if path.name.split("=", 1)[1] <= trading_date
    ]
    dates.sort()
    closes = []
    for value in dates:
        valid = _stock_frame(clean, value).filter(pl.col("close").is_finite() & (pl.col("close") > 0))
        if not valid.is_empty():
            closes.append(float(valid.tail(1).item(0, "close")))
    snapshot = _chain_snapshot(clean, trading_date, minute, expiration)
    current_iv = snapshot["metrics"].get("atm_iv")
    history = [
        {"date": value, "iv": iv}
        for value in dates[-40:]
        if (iv := _atm_history_iv(clean, value)) is not None
    ]
    historical_values = [item["iv"] for item in history]
    iv_rank = None
    iv_percentile = None
    if current_iv is not None and historical_values:
        low, high = min(historical_values), max(historical_values)
        iv_rank = min(100, max(0, (current_iv - low) / max(high - low, 1e-9) * 100))
        iv_percentile = sum(value <= current_iv for value in historical_values) / len(historical_values) * 100
    rv = {str(window): _realized_volatility(closes, window) for window in (5, 10, 20)}
    dte = max((date.fromisoformat(expiration) - date.fromisoformat(trading_date)).days, 1)
    expected_move = snapshot["spot"] * (current_iv or 0) / 100 * math.sqrt(dte / 365)
    return {
        "symbol": clean,
        "date": trading_date,
        "minute": minute,
        "atm_iv": current_iv,
        "iv_rank": round(iv_rank, 2) if iv_rank is not None else None,
        "iv_percentile": round(iv_percentile, 2) if iv_percentile is not None else None,
        "realized_volatility": {key: round(value, 3) if value is not None else None for key, value in rv.items()},
        "vrp20": round(current_iv - rv["20"], 3) if current_iv is not None and rv["20"] is not None else None,
        "expected_move": round(expected_move, 3),
        "history": history,
    }


if FRONTEND_DIST.is_dir():
    app.mount("/", StaticFiles(directory=FRONTEND_DIST, html=True), name="frontend")
