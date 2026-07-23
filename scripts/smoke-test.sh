#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${1:-http://127.0.0.1:7311}"
DATE="$(curl -fsS "$BASE_URL/api/catalog" | jq -r '.common_dates[-1]')"

[[ "$(curl -fsS "$BASE_URL/api/health" | jq -r '.engine')" == "rust" ]]
[[ -n "$DATE" && "$DATE" != "null" ]]

SESSION="$(curl -fsS "$BASE_URL/api/session?symbols=SPY&date=$DATE")"
MINUTE="$(jq -r '.timeline[60]' <<<"$SESSION")"
EXPIRATION="$(jq -r '.series.SPY.expirations[0]' <<<"$SESSION")"

CHAIN="$(curl -fsS "$BASE_URL/api/chain?symbol=SPY&date=$DATE&minute=$MINUTE&expiration=$EXPIRATION&pricing_mode=micro&dealer_model=classic")"
jq -e '.rows | length > 0' <<<"$CHAIN" >/dev/null
jq -e '.quality.quote_coverage_pct == 100 and .quality.fresh_quote_coverage_pct == 100 and .quality.spot_age_ms == 0' <<<"$CHAIN" >/dev/null
jq -e '.provenance.forward_source and .provenance.exercise_model and (.tte_years > 0)' <<<"$CHAIN" >/dev/null

SURFACE="$(curl -fsS "$BASE_URL/api/surface?symbol=SPY&date=$DATE&minute=$MINUTE&max_dte=180")"
jq -e '(.grid | length > 0) and (.arbitrage.confidence_score >= 0) and .arbitrage.projection_model' <<<"$SURFACE" >/dev/null

VOLATILITY="$(curl -fsS "$BASE_URL/api/volatility-context?symbol=SPY&date=$DATE&minute=$MINUTE&expiration=$EXPIRATION")"
jq -e '(.history | length > 0) and .expected_move_basis and .iv_source and .rv_source' <<<"$VOLATILITY" >/dev/null

LEG="$(jq -c '.rows | map(select(.bid > 0 and .ask >= .bid and .quality_score >= 50))[0] | {symbol, strike, right, side:"BUY", ratio:1}' <<<"$CHAIN")"
STRATEGY_REQUEST="$(jq -nc --arg date "$DATE" --arg minute "$MINUTE" --arg expiration "$EXPIRATION" --argjson leg "$LEG" '{mode:"replay",symbol:"SPY",date:$date,minute:$minute,expiration:$expiration,pricing_mode:"micro",dealer_model:"classic",quantity:1,legs:[$leg]}')"
curl -fsS -X POST "$BASE_URL/api/strategy/analyze" -H 'Content-Type: application/json' --data-binary "$STRATEGY_REQUEST" | jq -e '.preview_id and (.payoff | length > 100) and (.scenarios | length == 45) and .pricing_basis' >/dev/null

curl -fsS "$BASE_URL/api/audit/records?limit=5" | jq -e 'type == "array"' >/dev/null
CONNECTION="$(curl -fsS "$BASE_URL/api/connection")"
jq -e '.credential_storage == "process_memory_only"' <<<"$CONNECTION" >/dev/null
if [[ "$(jq -r '.connected' <<<"$CONNECTION")" == "true" ]]; then
  STATUS="$(curl -sS -o /dev/null -w '%{http_code}' "$BASE_URL/api/live/snapshot")"
  [[ "$STATUS" == "200" || "$STATUS" == "409" ]]
  if [[ "$STATUS" == "200" ]]; then
    curl -fsS "$BASE_URL/api/live/volatility-context" | jq -e '.expected_move_basis and .rv_source' >/dev/null
  fi
  if [[ "$(jq -r '.trade_connected' <<<"$CONNECTION")" == "true" ]]; then
    curl -fsS "$BASE_URL/api/trade/account" | jq -e '.connected == true and has("execution_enabled")' >/dev/null
    curl -fsS "$BASE_URL/api/trade/orders" | jq -e 'type == "array"' >/dev/null
  fi
else
  STATUS="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$BASE_URL/api/live/session" -H 'Content-Type: application/json' --data '{"symbol":"SPY"}')"
  [[ "$STATUS" == "409" ]]
fi

printf 'Rust API smoke test passed for %s at %s\n' "$DATE" "$BASE_URL"
