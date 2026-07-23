# Local security patch

This directory contains the source of `longbridge-oauth` 4.4.1 from the
Longbridge OpenAPI Rust SDK.

Option Workstation carries one narrow patch:

- upgrade `oauth2` from 4.4 to 5.0;
- adapt the OAuth client builder and async HTTP client calls to the 5.0 API;
- disable HTTP redirects for OAuth token requests, as recommended by the
  `oauth2` crate.

The patch removes the transitive `rustls-webpki` 0.101 dependency affected by
RUSTSEC-2026-0098, RUSTSEC-2026-0099, and RUSTSEC-2026-0104.

No Longbridge endpoint, token storage, callback, or refresh behavior is
intentionally changed. Remove this patch when an upstream Longbridge release
uses a non-vulnerable OAuth stack, then run the full release checklist.

Upstream: <https://github.com/longbridge/openapi>

License: MIT OR Apache-2.0
