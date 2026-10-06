# Feature: Dependency Security Policy

The project SHALL maintain a documented, auditable policy for dependency advisories
and version management so that every known vulnerability is either patched or
formally suppressed with a traceable rationale.

<!-- DELTA:CHANGED -->
## Background

All advisory suppressions in `deny.toml` MUST include a structured rationale comment stating: the affected advisory ID, the upstream fix status, the blocker preventing an immediate fix, and the re-evaluation trigger.

Patch-level version bumps (`cargo update`) MAY be applied without a breaking-change review cycle. Minor or major version bumps MUST go through explicit evaluation before merging.

`cargo deny --all-features check advisories` MUST be a required gate in CI and MUST run on every pull request and push to `main`. The gate covers the dependencies of every Cargo feature, including optional features such as `benchmark` and `ffi`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Advisory suppression documents rationale

* *GIVEN* a known security advisory affects a transitive dependency
* *AND* no patched version of that dependency is available on crates.io
* *WHEN* the advisory is suppressed in `deny.toml`
* *THEN* the suppression entry MUST include the advisory ID as the `id` field
* *AND* the suppression entry MUST include a `reason` field documenting the upstream fix status, the blocker preventing an upgrade, and the trigger for re-evaluation
* *AND* `cargo deny --all-features check advisories` MUST exit with code 0
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Patch-level dep bump applied without breaking-change review

* *GIVEN* `cargo update` is run in the repository
* *WHEN* only patch-level version changes appear in `Cargo.lock`
* *THEN* the change MUST be mergeable without a breaking-change review
* *AND* all existing tests MUST continue to pass
* *AND* `cargo deny --all-features check advisories` MUST exit with code 0
* *AND* `cargo deny check licenses` MUST exit with code 0
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Minor or major dep bump requires explicit evaluation

* *GIVEN* a dependency version bump changes the minor or major version component
* *WHEN* a pull request is opened with that change
* *THEN* the PR description MUST document the reason for the version change
* *AND* the PR MUST confirm no behavioral regressions via the full test suite
* *AND* `cargo deny --all-features check advisories` MUST exit with code 0
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Advisory CI gate blocks merge on unacknowledged advisory

* *GIVEN* a new security advisory is published for a dependency of any Cargo feature, including an optional feature
* *AND* no suppression entry for that advisory exists in `deny.toml`
* *WHEN* CI runs `cargo deny --all-features check advisories`
* *THEN* the CI step MUST exit with a non-zero exit code
* *AND* the build MUST be marked as failed
* *AND* the pull request MUST NOT be mergeable until the advisory is patched or suppressed with rationale
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift

* *GIVEN* `parquet 58.x` pulls in `thrift 0.17.0` as a transitive dependency
* *AND* the `parquet` semver constraint `^0.17` prevents an update to a `thrift` release that fixes GHSA-2f9f-gq7v-9h6m, and `parquet 59.x`, which drops `thrift`, needs `arrow 59` that the downstream users of exarrow-rs do not use yet
* *WHEN* `cargo deny --all-features check advisories` is run
* *THEN* the check MUST exit with code 0
* *AND* the suppression MUST reference `GHSA-2f9f-gq7v-9h6m`
* *AND* the suppression `reason` MUST state that `parquet 58.x` requires `thrift ^0.17`, that the fix is in `parquet 59.x`, and that re-evaluation is triggered when exarrow-rs and its downstream users move to `arrow 59`
<!-- /DELTA:CHANGED -->
