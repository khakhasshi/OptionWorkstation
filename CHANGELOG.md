# Changelog

All notable changes to Option Workstation are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and releases use semantic versioning after the public API stabilizes.

## [Unreleased]

### Added

- Selectable ThetaData live market-data mode using one official Python SDK
  session, bounded snapshot polling, Rust analytics, and local WebSocket updates.
- Process-memory-only ThetaData connection API, in-app provider selector,
  provider-aware volatility/strategy/assistant contexts, and offline adapter
  protocol tests.
- Independent Option Workstation repository structure.
- Rust replay/live analytics server and React workstation.
- Chinese beginner guide and decision-support walkthrough.
- Chinese-first project README with a detailed capability map, operating
  workflow, local API reference, and maintainer contact.
- Reproducible local and container startup paths.
- Open-source governance, security, contribution, CI, and release standards.

### Fixed

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

- Server-side rejection of paper orders priced from the market-data-only
  ThetaData provider.
- ThetaData credentials are passed only to the local adapter process and are
  excluded from responses, browser persistence, logs, and audit records.
- Loopback-only default binding.
- Process-memory-only Longbridge credential handling.
- Server-disabled paper execution with independent account, freshness, and
  typed-confirmation gates.
- Local Longbridge OAuth compatibility patch upgrades the transitive TLS stack
  past RUSTSEC-2026-0098, RUSTSEC-2026-0099, and RUSTSEC-2026-0104.
- Updated `h2` to 0.4.16 to address RUSTSEC-2026-0258.
- Updated the locked `nanoid` transitive dependency to 3.3.18 for
  GHSA-2v37-7h3g-55p8.
- Publication-time private-path, market-data, oversized-file, and secret scans.

## [0.1.0] - Unreleased

Initial public preview. No compatibility guarantee is provided before 1.0.
