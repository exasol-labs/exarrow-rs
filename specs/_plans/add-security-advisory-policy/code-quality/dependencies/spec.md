# Feature: Dependency Security Policy

The project SHALL maintain a documented, auditable policy for dependency advisories
and version management so that every known vulnerability is either patched or
formally suppressed with a traceable rationale.

## Background

All advisory suppressions in `deny.toml` MUST include a structured rationale
comment stating: the affected advisory ID, the upstream fix status, the blocker
preventing an immediate fix, and the re-evaluation trigger.

Patch-level version bumps (`cargo update`) MAY be applied without a
breaking-change review cycle.  Minor or major version bumps MUST go through
explicit evaluation before merging.

`cargo deny check advisories` MUST be a required gate in CI and MUST run on
every pull request and push to `main`.

<!-- DELTA:NEW -->

## Scenarios

### Scenario: Advisory suppression documents rationale

* *GIVEN* a known security advisory affects a transitive dependency
* *AND* no patched version of that dependency is available on crates.io
* *WHEN* the advisory is suppressed in `deny.toml`
* *THEN* the suppression entry MUST include the advisory ID as the `id` field
* *AND* the suppression entry MUST include a `reason` field documenting the upstream fix status, the blocker preventing an upgrade, and the trigger for re-evaluation
* *AND* `cargo deny check advisories` MUST exit with code 0

### Scenario: Patch-level dep bump applied without breaking-change review

* *GIVEN* `cargo update` is run in the repository
* *WHEN* only patch-level version changes appear in `Cargo.lock`
* *THEN* the change MUST be mergeable without a breaking-change review
* *AND* all existing tests MUST continue to pass
* *AND* `cargo deny check advisories` MUST exit with code 0
* *AND* `cargo deny check licenses` MUST exit with code 0

### Scenario: Minor or major dep bump requires explicit evaluation

* *GIVEN* a dependency version bump changes the minor or major version component
* *WHEN* a pull request is opened with that change
* *THEN* the PR description MUST document the reason for the version change
* *AND* the PR MUST confirm no behavioral regressions via the full test suite
* *AND* `cargo deny check advisories` MUST exit with code 0

### Scenario: Advisory CI gate blocks merge on unacknowledged advisory

* *GIVEN* a new security advisory is published for a transitive dependency
* *AND* no suppression entry for that advisory exists in `deny.toml`
* *WHEN* CI runs `cargo deny check advisories`
* *THEN* the CI step MUST exit with a non-zero exit code
* *AND* the build MUST be marked as failed
* *AND* the pull request MUST NOT be mergeable until the advisory is patched or suppressed with rationale

### Scenario: GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift

* *GIVEN* `parquet 58.x` pulls in `thrift 0.17.0` as a transitive dependency
* *AND* `thrift 0.23.0` (which fixes GHSA-2f9f-gq7v-9h6m) is not published on crates.io
* *AND* the `parquet` semver constraint `^0.17` prevents a `[patch]` override to a non-existent version
* *AND* `parquet 59.x` (which removes the `thrift` dependency entirely) is not yet released
* *AND* `adbc_core 0.23` caps `arrow-schema` at `<59`, blocking an upgrade to `arrow/parquet 59.x`
* *WHEN* `cargo deny check advisories` is run
* *THEN* the check MUST exit with code 0
* *AND* the suppression MUST reference `GHSA-2f9f-gq7v-9h6m`
* *AND* the suppression `reason` MUST state that `thrift 0.23.0` is not on crates.io, that `^0.17` blocks a patch override, and that re-evaluation is triggered when `parquet 59.x` is released or `adbc_core` supports `arrow-schema >=59`

<!-- /DELTA:NEW -->
