# Engineering Standards

## Correctness

- Domain calculations are deterministic for identical inputs.
- Timezones are explicit at acquisition and conversion boundaries.
- Floating-point tolerances are named and tested.
- Missing inputs produce explicit unavailable states.
- Point-in-time reads never use future observations.

## Reliability

- Network and provider failures preserve the last valid state when safe.
- Retries are bounded and respect provider limits.
- Live universe transitions are serialized and rollback-aware.
- Mutation requests are idempotent or carry deterministic request IDs.
- Shutdown clears in-memory credentials and provider contexts.

## Security

- Default network binding is loopback-only.
- Secrets are never logged, audited, serialized to the browser, or committed.
- Input length and option-universe size are bounded.
- Real-money order mutation is rejected.
- Paper mutation requires independent server and user gates.

## Observability

Every derived panel should expose enough context to answer:

- which provider supplied the inputs;
- when the inputs were observed;
- how fresh and complete they are;
- which model and assumptions were used;
- why the result is unavailable or research-only.

Logs may contain symbols, route names, counts, and timings. They must not contain
tokens, secrets, account identifiers, full order payloads, or licensed rows.

## Testing

- Pure math: unit tests with deterministic fixtures and tolerances.
- Parsers/data contracts: schema and malformed-input tests.
- API behavior: request validation, error status, and redaction tests.
- UI: empty, loading, stale, partial, ready, and error states.
- Execution: paper-only mocks or explicitly controlled paper accounts.
- Regressions: a test that fails before the fix and passes after it.

## Review

Reviewers prioritize:

1. account and credential safety;
2. point-in-time and data-provenance correctness;
3. behavioral regressions;
4. model-claim accuracy;
5. test sufficiency;
6. maintainability and performance.

Style-only preferences should not obscure correctness findings.

## Release

A release is not ready until source, lockfiles, docs, container build, tests,
secret scans, license review, and manual safety checks all pass. Follow
`docs/RELEASE_CHECKLIST.md`.
