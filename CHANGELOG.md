# Changelog

All notable changes to Option Workstation are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and releases use semantic versioning after the public API stabilizes.

## [Unreleased]

### Added

- Planned trading-workflow and option-factor visualization roadmap with
  priorities, acceptance criteria, DSL compatibility requirements, and staged
  delivery dependencies; this does not enable the planned features.
- Focused overview, volatility, trading, and records workspaces with readable
  labels, grouped toolbar controls, expandable model metadata, and responsive
  layouts that retain navigation and playback controls on narrow screens.
- Separate GEX/Vanna/Charm views with explicit units, quote observations drawn
  as scatter points against the SVI fit, and a default 2D volatility surface
  with optional 3D and automatic or fixed color ranges.

- Selectable ThetaData live market-data mode using one official Python SDK
  session, bounded snapshot polling, Rust analytics, and local WebSocket updates.
- Process-memory-only ThetaData connection API, in-app provider selector,
  provider-aware volatility/strategy/assistant contexts, and offline adapter
  protocol tests.

### Fixed

- Assistant replay and imported contexts exclude market bars after the frozen
  snapshot time; streamed Chinese and emoji text survives split UTF-8 chunks.
- Cached ThetaData quotes and connection status continue aging during stalled
  polls, with idle WebSocket freshness updates and explicit missing-timestamp
  blockers for live strategy previews.
- Guide smoke navigation follows the grouped toolbar menu.

- Vanna and Charm visualizations follow the snapshot's selected dealer
  convention, including all-long and all-short scenarios.
- Chart-specific number formats no longer get overwritten by shared axis
  settings; grid panels can scroll vertically and shrink after window resizing.

- Replay playback waits for each complete snapshot and commits the tape time,
  option chain, surface, and volatility context together. Slow requests reduce
  effective playback speed instead of leaving analytics on an old frame.
- Cached Longbridge snapshots refresh quote ages and freshness coverage on
  every read, so a stalled feed cannot keep an old paper preview executable;
  idle WebSocket clients receive freshness updates without new provider events.
- Historical OI coverage is measured against quoted contracts with valid OI
  observations, including known zeroes, instead of the existence of an OI file.
- Live expiration changes remain selected while the previous stream continues,
  then update the full workspace only after the provider confirms the new
  expiration.

### Performance

- Inactive workspaces do not mount their charts or chain tables. The 3D
  renderer loads only when requested; chart updates coalesce per animation
  frame and pause outside the viewport while preserving zoom and camera views.
- The mirrored option chain renders only the visible window and nearby rows,
  with keyboard navigation, filtering, and strategy-leg selection preserved.
  Browser measurements and their limits are recorded in `docs/PERFORMANCE.md`.

- Replay filters Parquet timestamps before decoding the selected minute's
  quote columns, including the requested New York trading date.
- Matched-DTE IV history uses the same ATM pricing rules without building
  unused Greeks, SVI fits, or dealer scenarios. SVI fitting reuses invariant
  sample weights and density grids.
- Playback counts request time towards each frame period while retaining
  complete snapshots, cancellation, and one active request.
- Added an isolated release-binary benchmark with complete JSON response
  comparison, separate sequential/repeated-frame timings, and local reports.

### Security

- Updated `rustls` to 0.23.45 for RUSTSEC-2026-0285 and replaced the
  yanked `chacha20` 0.10.1 lockfile entry with 0.10.2.
- Updated frontend browser tooling dependencies to remove the current npm
  audit findings while retaining the upstream React security updates.
- Server-side rejection of paper orders priced from the market-data-only
  ThetaData provider.
- ThetaData credentials are passed only to the local adapter process and are
  excluded from responses, browser persistence, logs, and audit records.
- Updated `h2` to 0.4.16 to address RUSTSEC-2026-0258.

## [0.1.0] - 2026-08-13

### Added

- Independent Option Workstation repository structure.
- Rust replay/live analytics server and React workstation.
- Chinese beginner guide and decision-support walkthrough.
- Chinese-first project README with a detailed capability map, operating
  workflow, local API reference, and maintainer contact.
- Reproducible local and container startup paths.
- Open-source governance, security, contribution, CI, and release standards.

### Security

- Loopback-only default binding.
- Process-memory-only Longbridge credential handling.
- Server-disabled paper execution with independent account, freshness, and
  typed-confirmation gates.
- Local Longbridge OAuth compatibility patch upgrades the transitive TLS stack
  past RUSTSEC-2026-0098, RUSTSEC-2026-0099, and RUSTSEC-2026-0104.
- RustSec CI now guards the reviewed lockfile-only `RUSTSEC-2026-0235`
  exception with an all-target dependency-reachability check.
- Updated the transitive `nanoid` dependency past `GHSA-2v37-7h3g-55p8`.
- Publication-time private-path, market-data, oversized-file, and secret scans.

Initial public preview. No compatibility guarantee is provided before 1.0.
