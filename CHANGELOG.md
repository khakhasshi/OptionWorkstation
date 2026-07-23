# Changelog

All notable changes to Option Workstation are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and releases use semantic versioning after the public API stabilizes.

## [Unreleased]

### Added

- Independent Option Workstation repository structure.
- Rust replay/live analytics server and React workstation.
- Chinese beginner guide and decision-support walkthrough.
- Reproducible local and container startup paths.
- Open-source governance, security, contribution, CI, and release standards.

### Security

- Loopback-only default binding.
- Process-memory-only Longbridge credential handling.
- Server-disabled paper execution with independent account, freshness, and
  typed-confirmation gates.
- Local Longbridge OAuth compatibility patch upgrades the transitive TLS stack
  past RUSTSEC-2026-0098, RUSTSEC-2026-0099, and RUSTSEC-2026-0104.
- Publication-time private-path, market-data, oversized-file, and secret scans.

## [0.1.0] - Unreleased

Initial public preview. No compatibility guarantee is provided before 1.0.
