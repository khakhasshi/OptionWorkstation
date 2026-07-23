#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE_URL="${1:-http://127.0.0.1:7311}"

(cd "$ROOT/rust-backend" && cargo fmt --check)
(cd "$ROOT/rust-backend" && cargo test --locked)
(cd "$ROOT/rust-backend" && cargo clippy --locked --all-targets -- -D warnings)
if [[ ! -d "$ROOT/frontend/node_modules" ]]; then
  (cd "$ROOT/frontend" && npm ci)
fi
(cd "$ROOT/frontend" && npm run build)

if curl -fsS "$BASE_URL/api/health" >/dev/null 2>&1; then
  "$ROOT/scripts/smoke-test.sh" "$BASE_URL"
  if [[ "${RUN_BROWSER_SMOKE:-0}" == "1" ]]; then
    (cd "$ROOT" && node scripts/browser-smoke.mjs "$BASE_URL")
  fi
fi

printf 'Option Workstation verification passed\n'
