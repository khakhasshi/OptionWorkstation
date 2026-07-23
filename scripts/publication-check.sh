#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf 'Checking for private paths and credential material...\n'
users_prefix="/""Users/"
token_prefix="hk_""m_"
private_key_marker="PRIVATE"" KEY"
publication_pattern="(${users_prefix}[^/[:space:]]+|${token_prefix}[A-Za-z0-9._-]{20,}|-----BEGIN [A-Z ]*${private_key_marker}-----)"
if rg --hidden \
  --glob '!.git/**' \
  --glob '!frontend/node_modules/**' \
  --glob '!frontend/dist/**' \
  --glob '!**/target/**' \
  --glob '!artifacts/**' \
  "$publication_pattern" \
  .; then
  printf 'Publication check failed: private path or credential-like material found.\n' >&2
  exit 1
fi

printf 'Checking for excluded market data and oversized files...\n'
publication_file_failure=0
while IFS= read -r -d '' file; do
  case "$file" in
    *.parquet|*.arrow|*.feather)
      printf '%s\n' "$file"
      publication_file_failure=1
      continue
      ;;
  esac
  if [[ -f "$file" ]] && (( $(wc -c <"$file") > 10 * 1024 * 1024 )); then
    printf '%s\n' "$file"
    publication_file_failure=1
  fi
done < <(git ls-files --cached --others --exclude-standard -z)

if (( publication_file_failure != 0 )); then
  printf 'Publication check failed: market data or oversized files found.\n' >&2
  exit 1
fi

git diff --check

if command -v gitleaks >/dev/null 2>&1; then
  gitleaks_failure=0
  while IFS= read -r -d '' file; do
    if ! gitleaks dir \
      --log-level error \
      --max-target-megabytes 10 \
      --redact \
      --no-banner \
      "$file"; then
      gitleaks_failure=1
    fi
  done < <(git ls-files --cached --others --exclude-standard -z)
  if (( gitleaks_failure != 0 )); then
    printf 'Publication check failed: gitleaks found credential-like material.\n' >&2
    exit 1
  fi
else
  printf 'gitleaks is not installed; skipping heuristic secret scan.\n' >&2
fi

node scripts/license-inventory.mjs
(cd frontend && npm audit --audit-level=high)

if cargo deny --version >/dev/null 2>&1; then
  cargo deny --log-level error \
    --manifest-path rust-backend/Cargo.toml \
    check bans licenses sources
else
  printf 'cargo-deny is not installed; CI will run the Rust license policy.\n' >&2
fi

if cargo audit --version >/dev/null 2>&1; then
  cargo audit --file rust-backend/Cargo.lock
else
  printf 'cargo-audit is not installed; CI will run the RustSec audit.\n' >&2
fi

printf 'Publication checks passed.\n'
