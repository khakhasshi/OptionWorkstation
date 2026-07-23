# Governance

Option Workstation currently uses a maintainer-led governance model.

Maintainers are responsible for releases, repository settings, security
response, roadmap priorities, and final decisions on changes affecting data
contracts, model claims, or order safety.

Routine fixes can be merged after review and passing required checks. Changes
to execution gates, credential handling, public APIs, model semantics, or
licensing require explicit maintainer approval and corresponding documentation.

When consensus is not immediate, decisions should favor:

1. user and account safety;
2. observable data provenance;
3. deterministic and reproducible behavior;
4. compatibility with existing replay evidence;
5. implementation simplicity.

The maintainer list and escalation contact should be added when the public
repository owner is finalized. Until then, no contributor should claim to speak
for the project without maintainer approval.
