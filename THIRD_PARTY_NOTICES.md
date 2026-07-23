# Third-Party Notices

Option Workstation depends on open-source packages listed in
`rust-backend/Cargo.lock` and `frontend/package-lock.json`. Each package remains
under its own license. The lockfiles are the authoritative dependency inventory
for a given release.

Notable bundled browser dependency:

- `claygl` 1.3.0, Copyright (c) 2014 Yi Shen, licensed under the BSD 2-Clause
  License. Source and license: <https://github.com/pissang/claygl>.
- `longbridge-oauth` 4.4.1, sourced from the Longbridge OpenAPI Rust SDK and
  licensed under MIT OR Apache-2.0. The copy in `vendor/longbridge-oauth`
  upgrades the `oauth2` dependency to 5.0 and contains the corresponding API
  migration. See `vendor/longbridge-oauth/PATCHES.md` for the exact scope.

Apache ECharts, React, Vite, the Longbridge Rust SDK, and the Rust crates used by
the analytics server retain their respective upstream licenses. Before each
release, run `./scripts/publication-check.sh`, `npm audit`, and the dependency
license checks in CI, then review any new package with missing or nonstandard
license metadata.

No ThetaData or Longbridge market data is distributed with this repository.
Access to those services is governed by the user's separate agreement with the
provider.
