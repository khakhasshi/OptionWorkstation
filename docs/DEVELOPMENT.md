# Development

## Toolchain

- Rust stable, edition 2024
- Node.js 22
- Python 3.12+ for the optional ThetaData adapter and legacy parity
- Chromium or Chrome for browser smoke tests

Use the committed Cargo and npm lockfiles. Dependency changes must update the
appropriate lockfile in the same pull request.

## Setup

```bash
cp .env.example .env
make setup
```

The default data root is `./data`, which is ignored by Git. The application can
start without it.

For ThetaData live-mode development, install its isolated SDK environment:

```bash
./scripts/setup-thetadata.sh
```

The Rust service discovers `.venv-thetadata/bin/python` automatically. Override
it with `OPTION_WORKSTATION_THETADATA_PYTHON` when reusing another environment.
Credentials may be submitted through the loopback UI or read from an ignored,
mode-`600` `.env`; never add them to fixtures or test output.

## Run

Production-style local run:

```bash
make run
```

Frontend-only development:

```bash
cd frontend
npm run dev
```

In another terminal:

```bash
cargo run --locked --manifest-path rust-backend/Cargo.toml
```

Vite proxies `/api` and WebSocket traffic to `127.0.0.1:7311`.

## Test Layers

### Rust Unit Tests

```bash
cargo test --locked --manifest-path rust-backend/Cargo.toml
```

These cover model calculations, safety gates, redaction, and point-in-time
helpers without provider data.

### Replay Playback Tests

```bash
cd frontend
npm test
```

These use a controlled clock and deferred responses to exercise fast playback,
slow requests, cancellation, and snapshot consistency without licensed data or
provider credentials. They also run in CI and `make check`.

### Legacy Python Tests

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r backend/requirements.txt pytest httpx
python -m pytest -q
```

The no-data smoke test always runs. Licensed ThetaData parity tests skip unless
`OPTION_WORKSTATION_DATA_ROOT` points to the expected fixture.

### ThetaData Adapter Tests

```bash
python3 -m pytest -q tests/test_thetadata_adapter.py
```

These tests use a fake SDK and verify protocol normalization and invalid-session
recovery without credentials or network access. A live provider smoke test is
manual and must report only redacted counts, timestamps, quality, and source
metadata. Do not record raw licensed rows.

### Browser Tests

Start the server, then:

```bash
RUN_BROWSER_SMOKE=1 ./scripts/verify.sh http://127.0.0.1:7311
node scripts/guide-smoke.mjs http://127.0.0.1:7311
node scripts/assistant-browser-smoke.mjs http://127.0.0.1:7311
```

Start the Rust service with `OPTION_WORKSTATION_LLM_MOCK=1` before the assistant
checks. To resolve a full frozen context for every replay symbol, verify
two-context comparison, favorite it, and restore it without a trading call:

```bash
node scripts/assistant-smoke.mjs http://127.0.0.1:7311
```

Live switching additionally requires an in-memory Longbridge paper session:

```bash
node scripts/live-switch-smoke.mjs http://127.0.0.1:7311
```

ThetaData live switching can be exercised from the same workstation after its
connection status is ready. It must never invoke an order endpoint; the server
also rejects ThetaData-priced paper submissions independently of the browser.

```bash
RUN_THETADATA_LIVE_SMOKE=1 ./scripts/verify.sh http://127.0.0.1:7311
```

Browser automation must never call order mutation routes.

## Quality Gate

```bash
make check
make security
```

The gate includes:

- `cargo fmt --check`;
- `cargo test --locked`;
- `cargo clippy --locked --all-targets -- -D warnings`;
- deterministic frontend replay tests;
- frontend production build;
- running-server API smoke checks when available;
- private path, market-data, oversized artifact, and secret scans.

## Adding a Dependency

1. Explain why the standard library or current dependencies are insufficient.
2. Check maintenance activity and the transitive dependency impact.
3. Confirm license compatibility.
4. Use the narrowest version range that matches ecosystem conventions.
5. Update lockfiles and `THIRD_PARTY_NOTICES.md` when attribution is required.
6. Run the complete quality gate and dependency audit.

## API Changes

The API is pre-1.0, but silent semantic changes are still prohibited. Update:

- Rust request/response types;
- frontend client and empty/error states;
- unit or integration tests;
- README API table;
- `CHANGELOG.md`;
- model/data documentation when interpretation changes.

## Performance Work

Benchmark before and after. Keep separate measurements for:

- Parquet decode;
- chain construction;
- SVI fit;
- surface projection;
- live snapshot serialization;
- browser chart rendering.

Do not trade away freshness or quality checks for headline latency without
showing the behavioral effect.
