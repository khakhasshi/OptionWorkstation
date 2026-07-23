# Release and Public-Launch Checklist

## Source and Identity

- [ ] Product name, version, changelog, and release notes agree.
- [ ] Repository owner and maintainer contact are final.
- [ ] Remote URL is added where appropriate after the repository exists.
- [ ] No copied project history, unrelated files, or private branches remain.
- [ ] `git status` is clean and the release commit is signed if available.

## Legal and Data

- [ ] Apache-2.0 license choice is confirmed by the copyright owner.
- [ ] `NOTICE` and `THIRD_PARTY_NOTICES.md` are current.
- [ ] Dependency licenses pass review.
- [ ] `cargo audit` reports zero vulnerabilities; all informational warnings
      and vendored patches have been reviewed.
- [ ] The local Longbridge OAuth patch is still necessary, or has been removed
      in favor of an equivalent secure upstream release.
- [ ] No provider market data or restricted screenshots are tracked.
- [ ] Documentation does not imply rights to redistribute provider data.
- [ ] Brand names are used descriptively and not presented as endorsements.

## Security

- [ ] `make security` passes from a clean checkout.
- [ ] Full Git history is scanned for secrets before publication.
- [ ] Any credential ever shown in development logs is rotated.
- [ ] GitHub Private Vulnerability Reporting is enabled.
- [ ] Secret scanning and push protection are enabled when available.
- [ ] The default deployment remains loopback-only.
- [ ] Real-money order submission is rejected in code and tested.
- [ ] Paper-order gates are manually reviewed.

## Build and Test

- [ ] `make setup` succeeds from a clean clone.
- [ ] `make check` succeeds.
- [ ] `docker build -t option-workstation:release .` succeeds.
- [ ] Container health check passes with an empty data directory.
- [ ] Replay smoke test passes with a licensed local fixture.
- [ ] Desktop and mobile browser smoke tests pass.
- [ ] Live switching is tested with a paper account.
- [ ] No automated test mutates or cancels an order.

## Documentation

- [ ] English and Chinese quick starts match current commands.
- [ ] Environment variable table matches code.
- [ ] Data schema and timezone assumptions are current.
- [ ] Model limitations match implementation.
- [ ] Security and threat-model boundaries are current.
- [ ] Beginner guide screenshots contain no credentials or account identifiers.

## GitHub Repository Settings

- [ ] Default branch is `main`.
- [ ] Branch protection requires CI and review.
- [ ] Force pushes and branch deletion are disabled for `main`.
- [ ] Required status checks include Rust, frontend, Python smoke, secret scan,
      and dependency review where supported.
- [ ] Dependabot security and version updates are enabled.
- [ ] Discussions, Issues, and templates are configured intentionally.
- [ ] Repository description and topics avoid performance claims.
- [ ] Sponsorship or funding links are omitted unless intentionally configured.

## Release Artifact

- [ ] Tag follows `vMAJOR.MINOR.PATCH`.
- [ ] Release notes include breaking changes, data/model changes, and safety
      changes.
- [ ] Source archive contains no local state or market data.
- [ ] Container image, if published, has an immutable digest.
- [ ] Software bill of materials and vulnerability results are attached when
      binary artifacts are distributed.
- [ ] Release artifacts are signed or provenance-attested when infrastructure is
      available.

## Final Manual Gate

- [ ] A maintainer has used the clean build as a new user would.
- [ ] A maintainer has confirmed that public publication is intentional.
- [ ] The public repository is created only after every blocking item above is
      complete or explicitly documented as deferred.
