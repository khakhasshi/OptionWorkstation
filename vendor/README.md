# Vendored Source Policy

Vendored source is exceptional in Option Workstation. It is allowed only when
a required upstream dependency has no released fix for a blocking security or
correctness issue.

Every vendored component must include:

- the exact upstream package and version;
- upstream license files;
- a narrow patch description and removal condition;
- tests covering the changed compatibility surface;
- dependency, license, and vulnerability checks in the release workflow.

Do not make unrelated refactors in a vendored component. Prefer an upstream
release as soon as it provides an equivalent fix, and record the replacement in
the changelog.

Current components:

- [`longbridge-oauth`](longbridge-oauth/PATCHES.md)
