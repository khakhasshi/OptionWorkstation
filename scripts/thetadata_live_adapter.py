#!/usr/bin/env python3
"""Single-session ThetaData snapshot adapter for the Rust workstation.

The process speaks one-request/one-response NDJSON over stdin/stdout. Provider
credentials are accepted only by the initial ``connect`` request and are never
written to disk or included in responses. Keeping one long-lived client also
avoids ThetaData session invalidation and crossed gRPC responses.
"""

from __future__ import annotations

import json
import logging
import math
import os
import sys
from collections.abc import Iterable
from datetime import date, datetime, timedelta, timezone
from importlib.metadata import PackageNotFoundError, version
from typing import Any
from zoneinfo import ZoneInfo


ET = ZoneInfo("America/New_York")
MAX_CONTRACTS = 1_000
MAX_SURFACE_EXPIRIES = 6
EXPIRATION_CACHE_SECONDS = 300
OI_CACHE_SECONDS = 300

logging.basicConfig(stream=sys.stderr, level=logging.ERROR)
logging.getLogger("thetadata").setLevel(logging.ERROR)


def _finite(value: Any, default: float = 0.0) -> float:
    try:
        number = float(value)
    except (TypeError, ValueError):
        return default
    return number if math.isfinite(number) else default


def _integer(value: Any, default: int = 0) -> int:
    number = _finite(value, float(default))
    return int(number) if math.isfinite(number) else default


def _records(frame: Any) -> list[dict[str, Any]]:
    if frame is None:
        return []
    if hasattr(frame, "to_dict"):
        try:
            rows = frame.to_dict("records")
        except TypeError:
            rows = None
        if rows is not None:
            return [dict(row) for row in rows]
    if isinstance(frame, list):
        return [dict(row) for row in frame if isinstance(row, dict)]
    return []


def _text(value: Any) -> str:
    if value is None:
        return ""
    if hasattr(value, "isoformat"):
        return str(value.isoformat())
    return str(value)


def _timestamp(value: Any) -> str | None:
    if value is None:
        return None
    candidate: datetime | None = None
    if isinstance(value, datetime):
        candidate = value
    elif hasattr(value, "to_pydatetime"):
        candidate = value.to_pydatetime()
    elif isinstance(value, (int, float)) and math.isfinite(float(value)):
        raw = float(value)
        divisor = 1_000.0 if raw > 10_000_000_000 else 1.0
        candidate = datetime.fromtimestamp(raw / divisor, tz=timezone.utc)
    else:
        raw = str(value).strip().replace("Z", "+00:00")
        try:
            candidate = datetime.fromisoformat(raw)
        except ValueError:
            return None
    if candidate.tzinfo is None:
        candidate = candidate.replace(tzinfo=ET)
    return candidate.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def _date_text(value: Any) -> str:
    if isinstance(value, datetime):
        return value.date().isoformat()
    if isinstance(value, date):
        return value.isoformat()
    raw = _text(value).strip()
    return raw[:10] if len(raw) >= 10 else raw


def _right(value: Any) -> str:
    raw = _text(value).strip().upper()
    return "CALL" if raw in {"C", "CALL"} else "PUT" if raw in {"P", "PUT"} else raw


def _row_timestamp(row: dict[str, Any]) -> str | None:
    for key in ("timestamp", "created", "last_updated", "time"):
        if key in row:
            parsed = _timestamp(row.get(key))
            if parsed:
                return parsed
    return None


def _contract_key(row: dict[str, Any]) -> tuple[str, int, str] | None:
    expiration = _date_text(row.get("expiration") or row.get("expiry"))
    strike = _finite(row.get("strike"), -1.0)
    right = _right(row.get("right"))
    if not expiration or strike <= 0 or right not in {"CALL", "PUT"}:
        return None
    return expiration, round(strike * 1_000), right


def _contract_symbol(symbol: str, expiration: str, strike: float, right: str) -> str:
    compact_date = expiration[2:4] + expiration[5:7] + expiration[8:10]
    side = "C" if right == "CALL" else "P"
    return f"{symbol}{compact_date}{side}{round(strike * 1_000):08d}"


def _json_safe(value: Any) -> Any:
    if value is None or isinstance(value, (str, int, bool)):
        return value
    if isinstance(value, float):
        return value if math.isfinite(value) else None
    if isinstance(value, (date, datetime)) or hasattr(value, "isoformat"):
        return _text(value)
    if isinstance(value, dict):
        return {str(key): _json_safe(item) for key, item in value.items()}
    if isinstance(value, Iterable) and not isinstance(value, (str, bytes)):
        return [_json_safe(item) for item in value]
    return str(value)


class ThetaAdapter:
    def __init__(self) -> None:
        self.client: Any | None = None
        self.auth: tuple[str, str] | None = None
        self.sdk_version = "unknown"
        self.expiration_cache: dict[str, tuple[float, list[str]]] = {}
        self.oi_cache: dict[tuple[str, int, int], tuple[float, dict[tuple[str, int, str], int]]] = {}

    def connect(self, request: dict[str, Any]) -> dict[str, Any]:
        from thetadata import ThetaClient

        email = str(request.get("email") or "").strip()
        password = str(request.get("password") or "")
        if bool(email) != bool(password):
            raise ValueError("ThetaData email and password must be provided together")
        if email:
            self.client = ThetaClient(email=email, password=password, dataframe_type="pandas")
            self.auth = (email, password)
            auth_method = "credentials"
        else:
            env_email = os.getenv("THETADATA_EMAIL") or os.getenv("AI_OPTION_THETADATA_EMAIL")
            env_password = os.getenv("THETADATA_PASSWORD") or os.getenv("AI_OPTION_THETADATA_PASSWORD")
            if env_email and env_password:
                self.client = ThetaClient(
                    email=env_email,
                    password=env_password,
                    dataframe_type="pandas",
                )
                self.auth = (env_email, env_password)
            else:
                self.client = ThetaClient(dataframe_type="pandas")
                self.auth = None
            auth_method = "environment"
        try:
            self.sdk_version = version("thetadata")
        except PackageNotFoundError:
            self.sdk_version = "unknown"
        packages = [
            f"Stocks: {getattr(self.client, 'stock_subscription', 'unknown')}",
            f"Options: {getattr(self.client, 'options_subscription', 'unknown')}",
        ]
        index_package = getattr(self.client, "index_subscription", None)
        if index_package:
            packages.append(f"Indices: {index_package}")
        return {
            "provider": "thetadata",
            "auth_method": auth_method,
            "sdk_version": self.sdk_version,
            "packages": packages,
        }

    def disconnect(self) -> dict[str, Any]:
        self.client = None
        self.auth = None
        self.expiration_cache.clear()
        self.oi_cache.clear()
        return {"disconnected": True}

    def reconnect(self) -> None:
        from thetadata import ThetaClient

        if self.auth is None:
            env_email = os.getenv("THETADATA_EMAIL") or os.getenv("AI_OPTION_THETADATA_EMAIL")
            env_password = os.getenv("THETADATA_PASSWORD") or os.getenv("AI_OPTION_THETADATA_PASSWORD")
            if env_email and env_password:
                self.client = ThetaClient(
                    email=env_email,
                    password=env_password,
                    dataframe_type="pandas",
                )
            else:
                self.client = ThetaClient(dataframe_type="pandas")
        else:
            self.client = ThetaClient(
                email=self.auth[0],
                password=self.auth[1],
                dataframe_type="pandas",
            )
        self.expiration_cache.clear()
        self.oi_cache.clear()

    def _require_client(self) -> Any:
        if self.client is None:
            raise RuntimeError("ThetaData is not connected")
        return self.client

    def expirations(self, symbol: str) -> list[str]:
        now = datetime.now(tz=timezone.utc).timestamp()
        cached = self.expiration_cache.get(symbol)
        if cached and now - cached[0] <= EXPIRATION_CACHE_SECONDS:
            return list(cached[1])
        client = self._require_client()
        today = datetime.now(tz=ET).date()
        values = sorted(
            {
                expiration
                for row in _records(client.option_list_expirations(symbol))
                if (expiration := _date_text(row.get("expiration")))
                and _valid_future_date(expiration, today)
            }
        )
        if not values:
            raise RuntimeError(f"ThetaData returned no current expirations for {symbol}")
        self.expiration_cache[symbol] = (now, values)
        return values

    def _open_interest(
        self,
        symbol: str,
        max_dte: int,
        strike_range: int,
    ) -> dict[tuple[str, int, str], int]:
        key = (symbol, max_dte, strike_range)
        now = datetime.now(tz=timezone.utc).timestamp()
        cached = self.oi_cache.get(key)
        if cached and now - cached[0] <= OI_CACHE_SECONDS:
            return dict(cached[1])
        frame = self._require_client().option_snapshot_open_interest(
            symbol,
            expiration="*",
            strike="*",
            right="both",
            max_dte=max_dte,
            strike_range=strike_range,
        )
        values: dict[tuple[str, int, str], int] = {}
        for row in _records(frame):
            contract_key = _contract_key(row)
            if contract_key:
                values[contract_key] = _integer(row.get("open_interest"))
        self.oi_cache[key] = (now, values)
        return values

    def snapshot(self, request: dict[str, Any]) -> dict[str, Any]:
        client = self._require_client()
        symbol = _normalize_symbol(request.get("symbol"))
        contract_limit = max(20, min(MAX_CONTRACTS, _integer(request.get("max_contracts"), 420)))
        expiry_count = max(2, min(MAX_SURFACE_EXPIRIES, _integer(request.get("surface_expiries"), 4)))
        window = max(0.04, min(0.30, _finite(request.get("moneyness_window"), 0.12)))
        include_history = bool(request.get("include_history"))

        expirations = self.expirations(symbol)
        requested_expiration = str(request.get("expiration") or "").strip()
        if requested_expiration and requested_expiration not in expirations:
            raise ValueError(f"expiration not available: {requested_expiration}")
        selected_expiration = requested_expiration or expirations[0]
        selected_expirations = expirations[:expiry_count]
        if selected_expiration not in selected_expirations:
            selected_expirations = sorted(selected_expirations[:-1] + [selected_expiration])

        today = datetime.now(tz=ET).date()
        max_dte = max((date.fromisoformat(item) - today).days for item in selected_expirations)
        per_expiry = max(10, contract_limit // max(1, len(selected_expirations)))
        strike_range = max(8, min(80, per_expiry // 2 + 4))

        stock_quote_rows = _records(client.stock_snapshot_quote(symbol))
        stock_ohlc_rows = _records(client.stock_snapshot_ohlc(symbol))
        stock_quote = stock_quote_rows[0] if stock_quote_rows else {}
        stock_ohlc = stock_ohlc_rows[0] if stock_ohlc_rows else {}
        bid = _finite(stock_quote.get("bid"))
        ask = _finite(stock_quote.get("ask"))
        spot = _finite(stock_ohlc.get("close"))
        if spot <= 0 and bid > 0 and ask >= bid:
            spot = (bid + ask) / 2.0
        if spot <= 0:
            spot = _finite(stock_quote.get("last") or stock_quote.get("price"))
        if spot <= 0:
            raise RuntimeError(f"ThetaData returned no usable stock price for {symbol}")

        option_frame = client.option_snapshot_quote(
            symbol,
            expiration="*",
            strike="*",
            right="both",
            max_dte=max_dte,
            strike_range=strike_range,
        )
        option_rows = _records(option_frame)
        oi = self._open_interest(symbol, max_dte, strike_range)
        contracts = _normalize_contracts(
            symbol,
            option_rows,
            oi,
            set(selected_expirations),
            spot,
            window,
            contract_limit,
        )
        if not contracts:
            raise RuntimeError(f"ThetaData returned no usable option quotes for {symbol}")

        bars = self._intraday_bars(client, symbol, today) if include_history else []
        stock_timestamp = _row_timestamp(stock_ohlc) or _row_timestamp(stock_quote)
        stock_bar = {
            "time": _et_minute(stock_timestamp),
            "timestamp": stock_timestamp,
            "open": _finite(stock_ohlc.get("open"), spot),
            "high": _finite(stock_ohlc.get("high"), spot),
            "low": _finite(stock_ohlc.get("low"), spot),
            "close": spot,
            "volume": _integer(stock_ohlc.get("volume")),
            "vwap": _finite(stock_ohlc.get("vwap"), spot),
        }
        return {
            "provider": "thetadata",
            "sdk_version": self.sdk_version,
            "symbol": symbol,
            "spot": spot,
            "stock_timestamp": stock_timestamp,
            "stock_bar": stock_bar,
            "bars": bars,
            "selected_expiration": selected_expiration,
            "expirations": selected_expirations,
            "contracts": contracts,
            "contract_limit": contract_limit,
        }

    def _intraday_bars(self, client: Any, symbol: str, trading_date: date) -> list[dict[str, Any]]:
        try:
            frame = client.stock_history_ohlc(symbol, date=trading_date, interval="1m")
        except Exception:
            return []
        bars = []
        for row in _records(frame):
            timestamp = _row_timestamp(row)
            close = _finite(row.get("close"))
            if not timestamp or close <= 0:
                continue
            bars.append(
                {
                    "time": _et_minute(timestamp),
                    "timestamp": timestamp,
                    "open": _finite(row.get("open"), close),
                    "high": _finite(row.get("high"), close),
                    "low": _finite(row.get("low"), close),
                    "close": close,
                    "volume": _integer(row.get("volume")),
                    "vwap": _finite(row.get("vwap"), close),
                }
            )
        return bars[-500:]

    def daily_closes(self, request: dict[str, Any]) -> dict[str, Any]:
        symbol = _normalize_symbol(request.get("symbol"))
        count = max(21, min(120, _integer(request.get("count"), 45)))
        end = datetime.now(tz=ET).date()
        start = end - timedelta(days=count * 3)
        rows = _records(self._require_client().stock_history_eod(symbol, start_date=start, end_date=end))
        closes = []
        for row in rows:
            close = _finite(row.get("close"))
            trading_date = _date_text(row.get("date") or row.get("created") or row.get("timestamp"))
            if close > 0 and trading_date:
                closes.append({"date": trading_date, "close": close})
        deduped = {item["date"]: item for item in closes}
        return {"symbol": symbol, "closes": [deduped[key] for key in sorted(deduped)][-count:]}


def _normalize_symbol(value: Any) -> str:
    symbol = str(value or "").strip().upper().removesuffix(".US")
    if not symbol or len(symbol) > 15 or not all(character.isalnum() or character in ".-" for character in symbol):
        raise ValueError("invalid US symbol")
    return symbol


def _valid_future_date(value: str, today: date) -> bool:
    try:
        return date.fromisoformat(value) >= today
    except ValueError:
        return False


def _et_minute(timestamp: str | None) -> str:
    if not timestamp:
        return ""
    try:
        parsed = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError:
        return ""
    return parsed.astimezone(ET).strftime("%H:%M")


def _normalize_contracts(
    symbol: str,
    quote_rows: list[dict[str, Any]],
    open_interest: dict[tuple[str, int, str], int],
    expirations: set[str],
    spot: float,
    window: float,
    limit: int,
) -> list[dict[str, Any]]:
    candidates = []
    for row in quote_rows:
        key = _contract_key(row)
        if key is None or key[0] not in expirations:
            continue
        expiration, strike_milli, right = key
        strike = strike_milli / 1_000.0
        if abs(strike / spot - 1.0) > window:
            continue
        bid = _finite(row.get("bid"))
        ask = _finite(row.get("ask"))
        last = _finite(row.get("last") or row.get("close"))
        candidates.append(
            {
                "symbol": _contract_symbol(symbol, expiration, strike, right),
                "expiration": expiration,
                "strike": strike,
                "right": right,
                "bid": bid,
                "ask": ask,
                "bid_size": _integer(row.get("bid_size") or row.get("bid_size_condition")),
                "ask_size": _integer(row.get("ask_size") or row.get("ask_size_condition")),
                "last": last if last > 0 else None,
                "volume": _integer(row.get("volume")),
                "open_interest": open_interest.get(key),
                "timestamp": _row_timestamp(row),
            }
        )
    per_expiry = max(1, limit // max(1, len(expirations)))
    selected = []
    for expiration in sorted(expirations):
        rows = [row for row in candidates if row["expiration"] == expiration]
        rows.sort(key=lambda row: (abs(row["strike"] / spot - 1.0), row["strike"], row["right"]))
        selected.extend(rows[:per_expiry])
    selected.sort(key=lambda row: (row["expiration"], row["strike"], row["right"]))
    return selected[:limit]


def _handle(adapter: ThetaAdapter, request: dict[str, Any]) -> Any:
    operation = request.get("op")
    if operation == "connect":
        return adapter.connect(request)
    if operation == "disconnect":
        return adapter.disconnect()
    if operation == "snapshot":
        return adapter.snapshot(request)
    if operation == "daily_closes":
        return adapter.daily_closes(request)
    if operation == "ping":
        return {"provider": "thetadata", "connected": adapter.client is not None}
    raise ValueError(f"unsupported operation: {operation}")


def _handle_with_session_retry(adapter: ThetaAdapter, request: dict[str, Any]) -> Any:
    try:
        return _handle(adapter, request)
    except Exception as exc:
        detail = str(exc).lower()
        operation = request.get("op")
        invalid_session = "invalid session id" in detail or (
            "unauthenticated" in detail and "session" in detail
        )
        if operation in {"connect", "disconnect"} or not invalid_session:
            raise
        adapter.reconnect()
        return _handle(adapter, request)


def main() -> int:
    adapter = ThetaAdapter()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        request_id: Any = None
        try:
            request = json.loads(line)
            if not isinstance(request, dict):
                raise ValueError("request must be a JSON object")
            request_id = request.get("id")
            payload = {
                "id": request_id,
                "ok": True,
                "result": _handle_with_session_retry(adapter, request),
            }
        except Exception as exc:  # noqa: BLE001 - process boundary returns a bounded diagnostic.
            payload = {"id": request_id, "ok": False, "error": str(exc)[:1_000]}
        sys.stdout.write(json.dumps(_json_safe(payload), separators=(",", ":")) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
