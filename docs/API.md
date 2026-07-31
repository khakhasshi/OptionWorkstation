# API Contract

Option Workstation exposes a local HTTP API on the configured loopback port.
The API is intended for the bundled frontend and local research automation; it
does not provide application-user authentication. Longbridge OAuth below is
provider authorization for the local live-data connection, not public user
login.

## Longbridge OAuth 2.0

`POST /api/oauth/start` starts a browser-based Longbridge authorization flow.
The request body is:

```json
{"client_id":"your-longbridge-oauth-client-id"}
```

The response contains a flow status and, once the local callback server is
ready, an `authorization_url`. It never contains an access token. The browser
should open that URL in a new tab and poll the status endpoint:

`GET /api/oauth/status`

The status is one of `idle`, `pending`, `connecting`, `connected`, or `error`.
The OAuth callback listens on `127.0.0.1:60355` and the token is held only by
the in-process SDK configuration. Tokens are not written to the OAuth crate's
default file storage. Starting a new flow or disconnecting cancels a pending
flow. This mechanism does not add authentication or authorization to the
HTTP API, so public or multi-user deployments remain unsupported.

## Point-in-time replay snapshot

`GET /api/v1/replay/snapshot` (the unversioned `/api/replay/snapshot` alias is
kept for the bundled frontend and pre-1.0 compatibility)

Query parameters:

| Parameter | Required | Description |
| --- | --- | --- |
| `symbol` | yes | Underlying symbol, for example `SPY` |
| `date` | yes | Trading date in `YYYY-MM-DD` |
| `minute` | yes | ET replay time in `HH:MM` |
| `expiration` | yes | Option expiration in `YYYY-MM-DD` |
| `pricing_mode` | no | `micro`, `mid`, or `ask`; defaults to `micro` |
| `dealer_model` | no | `classic`, `short_all`, or `long_all` |
| `max_dte` | no | Surface horizon from 1 to 1000 days; defaults to 180 |

The response is the authoritative replay unit:

```json
{
  "kind": "replay_snapshot",
  "snapshot_id": "replay:<chain-snapshot-id>",
  "symbol": "SPY",
  "date": "2026-07-10",
  "minute": "10:30",
  "expiration": "2026-07-17",
  "as_of": "2026-07-10T14:30:00Z",
  "model_version": "BSM+SVI-v1",
  "chain": {},
  "surface": {},
  "volatility": {}
}
```

The `chain`, `surface`, and `volatility` objects are computed from the same
symbol, date, minute and expiration request. Clients should display the
top-level `snapshot_id`, `as_of`, and `model_version` alongside derived panels.
The older `/api/chain`, `/api/surface`, and `/api/volatility-context` routes
remain available for compatibility.

## Error contract

Non-2xx responses use:

```json
{
  "detail": "human-readable explanation",
  "retry_after_ms": null
}
```

`409` means the requested state is not currently available, `429` means a
provider rate-limit window is active, and `502` means the upstream broker
operation failed. The frontend must preserve the last valid live snapshot for
these transient states and show the current connection state.

## Provenance expectations

Every derived response should expose, directly or through its parent snapshot:

- provider and quote interval;
- observation timestamp and timezone conversion;
- freshness and coverage;
- model and risk-free-rate assumptions;
- explicit unavailable, partial, or research-only reasons.

This contract intentionally does not expose licensed raw market data through a
public endpoint.
