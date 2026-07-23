#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_ROOT="${1:-${HOME}/MarketData/thetadata-options-2026-last-50d}"
TARGET_ROOT="${2:-${ROOT}/data/example}"

SYMBOLS=(${OPTION_WORKSTATION_EXAMPLE_SYMBOLS:-AAPL AMZN GOOGL META MSFT NVDA QQQ SPY TSLA})
DATES=(${OPTION_WORKSTATION_EXAMPLE_DATES:-2026-07-09 2026-07-10})

if [[ ! -d "$SOURCE_ROOT" ]]; then
  printf 'Source data root does not exist: %s\n' "$SOURCE_ROOT" >&2
  exit 1
fi

mkdir -p "$TARGET_ROOT"/underlying "$TARGET_ROOT"/options

printf 'Materializing example replay data\n'
printf '  source: %s\n' "$SOURCE_ROOT"
printf '  target: %s\n' "$TARGET_ROOT"
printf '  symbols: %s\n' "${SYMBOLS[*]}"
printf '  dates: %s\n' "${DATES[*]}"

for symbol in "${SYMBOLS[@]}"; do
  for date in "${DATES[@]}"; do
    underlying_src="$SOURCE_ROOT/underlying/symbol=$symbol/date=$date"
    options_src="$SOURCE_ROOT/options/symbol=$symbol/date=$date"
    underlying_dst="$TARGET_ROOT/underlying/symbol=$symbol/date=$date"
    options_dst="$TARGET_ROOT/options/symbol=$symbol/date=$date"

    if [[ ! -f "$underlying_src/ohlc.parquet" ]]; then
      printf 'Missing underlying partition: %s\n' "$underlying_src" >&2
      exit 1
    fi
    if [[ ! -d "$options_src" ]]; then
      printf 'Missing options partition: %s\n' "$options_src" >&2
      exit 1
    fi

    mkdir -p "$(dirname "$underlying_dst")" "$(dirname "$options_dst")"
    rsync -a --delete "$underlying_src/" "$underlying_dst/"
    rsync -a --delete "$options_src/" "$options_dst/"
  done
done

printf '\nExample data ready. Start the workstation with:\n'
printf '  OPTION_WORKSTATION_DATA_ROOT=%q ./scripts/start.sh\n' "$TARGET_ROOT"
