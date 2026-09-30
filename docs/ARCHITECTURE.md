# Architecture

## Goals

Option Workstation is a local-first research application. Its architecture is
designed to keep market-data provenance, calculation ownership, and order
safety observable.

The core goals are:

- one deterministic Rust implementation for replay and live analytics;
- no provider credentials in browser storage or repository files;
- point-in-time replay over operator-owned local files;
- explicit quality gates instead of fabricated fallback metrics;
- no real-money order submission;
- UI refreshes that do not destroy the analyst's working context.

## Components

```mermaid
flowchart TB
  subgraph Browser
    UI["React workstation"]
    CHART["ECharts / ECharts-GL"]
    GUIDE["Static beginner guide"]
  end

  subgraph Local Rust Process
    API["Axum HTTP + WebSocket API"]
    REPLAY["ReplayStore"]
    LIVE["Longbridge LiveManager"]
    THETALIVE["ThetaLiveManager"]
    ANALYTICS["BSM / Greeks / SVI / surface / exposure"]
    STRATEGY["Strategy risk and paper gates"]
    AUDIT["Redacted hash-chain ledger"]
  end

  THETA["Licensed ThetaData Parquet"] --> REPLAY
  LB["Longbridge Rust SDK"] --> LIVE
  THETASDK["ThetaData Python SDK adapter"] --> THETALIVE
  REPLAY --> ANALYTICS
  LIVE --> ANALYTICS
  THETALIVE --> ANALYTICS
  ANALYTICS --> API
  STRATEGY --> API
  AUDIT --> API
  API <--> UI
  UI --> CHART
  GUIDE --> Browser
```

### Frontend

`frontend/` is a React 19 and Vite application. It renders the underlying tape,
chain, smile, SVI diagnostics, term structure, dealer exposure, constrained
surface, strategy builder, risk matrix, audit records, and account/order
monitor. The snapshot assistant is a lazy-loaded floating module; it sends only
server-resolvable context references and user text, not browser-assembled raw
chains.

The browser receives normalized JSON snapshots. It does not parse Parquet,
calculate authoritative analytics, or own execution gates.

### Rust Server

`rust-backend/` is the production application:

- `main.rs`: API routes, configuration, static serving, and process lifecycle;
- `replay.rs`: partition discovery, Parquet decoding, and point-in-time reads;
- `live.rs`: Longbridge contexts, subscriptions, rate limits, cache, and paper
  account operations;
- `theta_live.rs`: one ThetaData SDK child process, serialized snapshot polling,
  cache/freshness state, and local WebSocket broadcast;
- `analytics.rs`: chain construction, IV/Greeks, quality, SVI, and exposure;
- `volatility.rs`: IV history, realized volatility, VRP, and expected move;
- `strategy.rs`: executable multi-leg risk and order-plan validation;
- `assistant.rs`: bounded context compaction, in-memory conversations, and
  OpenAI-compatible SSE streaming without trade tools;
- `audit.rs`: credential-rejecting append-only hash-chain records;
- `models.rs`: transport and domain structures.

### Legacy Python Reference

`backend/` is a migration parity reference. It is not started by
`scripts/start.sh`, is not used by the React application, and must never receive
provider credentials. It may be removed after parity fixtures become fully
redistributable.

## Historical Request Flow

```mermaid
sequenceDiagram
  participant B as Browser
  participant A as Axum API
  participant R as ReplayStore
  participant P as Parquet
  participant M as Analytics

  B->>A: GET /api/chain?symbol&date&minute&expiration
  A->>R: validate partition and timestamp
  R->>P: read underlying, quote, and OI batches
  P-->>R: typed Arrow arrays
  R->>M: spot, quote rows, metadata coverage, as-of
  M-->>A: chain, quality, SVI, Greeks, exposure
  A-->>B: normalized snapshot with provenance
```

Replay caches are keyed by symbol, date, expiration, and minute. Cache entries
contain decoded market observations, not provider credentials or account data.

## Live Request Flow

```mermaid
sequenceDiagram
  participant B as Browser
  participant A as Axum API
  participant L as LiveManager
  participant S as Longbridge SDK

  B->>A: POST /api/live/session with credentials and universe
  A->>L: validate bounded request
  L->>S: create quote/trade contexts
  S-->>L: account type, chain, subscriptions
  L-->>A: redacted connection status
  A-->>B: session status
  S-->>L: quote/depth push events
  L-->>B: WS /api/live/stream normalized snapshot
```

Credentials are moved into SDK contexts and are not returned in status
responses. Live option-universe changes are serialized and rate-limited.
Failed switches attempt subscription rollback while preserving the previous
stream.

### ThetaData live request flow

```mermaid
sequenceDiagram
  participant B as Browser
  participant A as Axum API
  participant T as ThetaLiveManager
  participant P as Python adapter
  participant S as ThetaData SDK

  B->>A: POST /api/thetadata/connection
  A->>T: validate and move credentials
  T->>P: spawn once; connect over NDJSON stdin
  P->>S: create one authenticated SDK session
  S-->>P: redacted package status
  P-->>T: connection status without credentials
  B->>A: POST /api/live/session provider=thetadata
  A->>T: configure bounded universe
  loop configured poll interval
    T->>P: serialized snapshot request
    P->>S: stock/option quote and cached metadata calls
    S-->>P: observations
    P-->>T: normalized inputs
    T->>T: Rust analytics and quality gates
    T-->>B: WS /api/live/stream?provider=thetadata
  end
```

The SDK transport is polling, even though the last hop to the browser is a
local WebSocket. The adapter does not calculate authoritative analytics and
does not expose an order operation. Invalid-session recovery performs one
reconnect and retry while preserving serialized access.

## Strategy and Paper-Order Flow

1. The browser selects legs from a server-produced chain.
2. `POST /api/strategy/analyze` recalculates entry cash, liquidation value,
   payoff, break-even, margin approximation, POP approximation, and scenarios.
3. The server binds the preview to current quote provenance and freshness.
4. Paper submission revalidates account type, server enablement, preview, and
   exact confirmation.
5. Legs are submitted sequentially with deterministic request IDs.
6. A later-leg failure triggers cancellation requests for earlier submitted
   legs.

The workflow is not atomic. The UI and documentation must continue to describe
legging and partial-fill risk.

## State and Persistence

| State | Location | Lifetime |
| --- | --- | --- |
| Longbridge credentials and SDK contexts | Rust process memory | until disconnect/process exit |
| ThetaData credentials and SDK child session | Rust/Python process memory | until disconnect/process exit |
| Live quote/depth cache | Rust process memory | active process |
| Replay decoded cache | Rust process memory | active process |
| UI layout and collapsed panels | browser local storage | browser profile |
| Audit records | configured JSONL path | operator-managed |
| Historical data | configured local/mounted root | operator-managed |

## Compatibility

The local API is pre-1.0 and may change. Changes to routes, field semantics,
quality thresholds, or execution gates require tests, changelog notes, and
documentation updates.
