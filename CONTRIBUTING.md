# Contributing

Thank you for improving Option Workstation. The project sits close to market
data and order APIs, so correctness, provenance, and conservative failure modes
matter more than feature count.

## Before You Start

- Search existing issues before opening a new one.
- Use the issue forms for bugs, data-quality problems, and feature requests.
- Discuss large UI, API, data-contract, model, or execution changes before
  implementation.
- Never include real credentials, account identifiers, licensed market data,
  audit ledgers, or screenshots that expose them.
- Security vulnerabilities follow `SECURITY.md`, not the public issue tracker.

## Development Setup

```bash
cp .env.example .env
make setup
make check
```

Historical integration tests require a separately licensed dataset. Unit tests
and the no-data service smoke test must remain runnable without that dataset.
See `docs/DEVELOPMENT.md` for detailed workflows.

## Branches and Commits

- Branch from `main`.
- Use focused branches such as `feat/surface-confidence` or
  `fix/live-session-rollback`.
- Prefer Conventional Commit subjects: `feat:`, `fix:`, `docs:`, `test:`,
  `refactor:`, `perf:`, `build:`, `ci:`, and `chore:`.
- Keep generated files, unrelated formatting, and local artifacts out of a
  functional change.
- Rebase or merge the latest `main` before requesting final review.

## Engineering Requirements

### Data and Models

- Preserve point-in-time semantics. Replay code must not read observations that
  occur after the active timestamp.
- Report the source, timestamp, sample count, and freshness of derived metrics.
- Return `null` or an explicit unavailable state when required inputs fail a
  gate. Do not synthesize confidence.
- Keep model assumptions visible. A dealer-sign convention, European pricing
  approximation, or surface projection must not be described as observed fact.
- Add deterministic fixtures for model math whenever possible.

### Execution and Risk

- Do not add real-money order submission.
- Do not bypass paper-account detection, server enablement, preview freshness,
  or typed confirmation.
- Treat a multi-leg sequence as non-atomic unless the broker confirms an atomic
  complex order.
- Default executable modeling is buy at ask and sell at bid. Midpoint studies
  require an explicit separate label.
- Mutation routes require idempotency, bounded inputs, and audit-safe payloads.

### Frontend

- Keep data quality and stale states visible.
- Charts must not throw on empty, partial, or rapidly changing datasets.
- Preserve user camera/layout state across refreshes unless the semantic context
  changes.
- Verify desktop and mobile overflow for any layout change.
- Controls must have labels, keyboard access, focus states, and non-color-only
  status cues.

### Security and Privacy

- Credentials remain process-memory-only and must never be logged or audited.
- Bind locally by default.
- Avoid adding analytics, telemetry, remote fonts, or third-party browser calls
  without prior discussion and an opt-in design.
- New dependencies need a maintained upstream, a compatible license, and a
  concrete reason.

## Testing

Run before every pull request:

```bash
make check
make security
```

For UI or chart changes, also run:

```bash
./scripts/start.sh
RUN_BROWSER_SMOKE=1 ./scripts/verify.sh http://127.0.0.1:7311
node scripts/guide-smoke.mjs http://127.0.0.1:7311
```

For live-session changes, use a paper account and run:

```bash
node scripts/live-switch-smoke.mjs http://127.0.0.1:7311
```

Automated tests must not submit, modify, or cancel orders.

## Pull Requests

Complete the pull request template. A reviewable change includes:

- the user-visible or API behavior being changed;
- model and data assumptions;
- risk and rollback notes;
- tests run and their results;
- screenshots for UI changes with all sensitive data removed;
- documentation and changelog updates when behavior changes.

Maintainers may ask for smaller commits or a separate research branch when a
proposal mixes multiple model assumptions.

## Licensing

By submitting a contribution, you agree that it is licensed under Apache-2.0
and that you have the right to submit it. Do not contribute code or data copied
from a source whose license is incompatible with this repository.
