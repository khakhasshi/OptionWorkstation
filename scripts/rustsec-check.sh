#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/rust-backend/Cargo.toml"
LOCKFILE="$ROOT/rust-backend/Cargo.lock"
REVIEWED_ADVISORY="RUSTSEC-2026-0235"

# rust_decimal declares rkyv 0.7 as an optional dependency, so Cargo.lock records
# it even though no Option Workstation target enables it. Never allow the audit
# exception to survive if that changes.
active_tree="$(cargo tree --manifest-path "$MANIFEST" --locked --target all -i rkyv 2>/dev/null)"
if [[ -n "$active_tree" ]]; then
  printf 'RustSec check failed: rkyv entered the active dependency tree:\n%s\n' "$active_tree" >&2
  exit 1
fi

printf 'Verified that rkyv is absent from the active dependency tree on all targets.\n'
cargo audit --file "$LOCKFILE" --ignore "$REVIEWED_ADVISORY"
