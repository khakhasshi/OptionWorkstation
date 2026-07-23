## Summary

Describe the user-visible or API behavior and why it is needed.

## Data and Model Assumptions

List provider, timestamps, point-in-time behavior, quality gates, model
assumptions, and any changed metric semantics.

## Risk and Rollback

Describe failure states, credential/account impact, execution impact, and how to
revert safely.

## Verification

- [ ] `make check`
- [ ] `make security`
- [ ] UI/browser smoke test, when applicable
- [ ] Live paper-account smoke test, when applicable
- [ ] No automated test submitted, modified, or canceled an order

Commands and results:

```text
paste redacted results here
```

## Documentation

- [ ] README/API documentation updated
- [ ] Model or data limitations updated
- [ ] `CHANGELOG.md` updated for user-visible behavior
- [ ] Screenshots are redacted and contain no licensed rows or account data

## Final Safety Check

- [ ] No credentials, account identifiers, audit ledgers, local paths, or market-data files are included
- [ ] Real-money order submission remains rejected
- [ ] Paper-order gates were not weakened
