# Security Policy

## Supported Versions

Option Workstation is pre-1.0. Security fixes target the latest `main` branch
and the most recent tagged release only.

| Version | Supported |
| --- | --- |
| latest release | yes |
| `main` | best effort |
| older releases | no |

## Reporting a Vulnerability

Do not open a public issue for a vulnerability.

After the repository is published, use GitHub Private Vulnerability Reporting
from the repository Security tab. Before that feature is enabled, email
<jiangjingzhe2004@gmail.com> with the subject `Option Workstation Security`.

Include:

- affected commit or release;
- component and route;
- reproducible steps using fake credentials and a paper account only;
- impact and prerequisites;
- any suggested mitigation.

Do not include a real token, account number, licensed market-data sample, or
order history. If a credential was exposed while investigating, revoke it
before sending the report.

We aim to acknowledge a complete report within seven days and provide an
initial assessment within fourteen days. Timelines may change for a
maintainer-run project, but reporters will receive status updates when a private
contact channel is available.

## Security Boundaries

- The default server binding is `127.0.0.1`.
- The application has no built-in multi-user authentication or TLS.
- Longbridge credentials are expected to remain in process memory only.
- Real-money order submission is intentionally unsupported.
- Paper-order submission is server-disabled by default and separately gated.
- Historical market data and audit ledgers are local operator assets.

Running the server on a shared host, reverse proxy, or public network without
additional authentication and isolation is outside the supported threat model.
See `docs/THREAT_MODEL.md`.

## Out of Scope

The following are not security vulnerabilities by themselves:

- market losses, missed fills, slippage, or inaccurate investment decisions;
- stale or unavailable third-party market data that is visibly reported;
- model approximation error described in `docs/MODEL_LIMITATIONS.md`;
- attacks requiring prior control of the local operating-system account;
- provider outages, entitlement limits, or rate limiting.

Data-quality failures that are hidden or mislabeled are still important; report
those with the data-quality issue form.
