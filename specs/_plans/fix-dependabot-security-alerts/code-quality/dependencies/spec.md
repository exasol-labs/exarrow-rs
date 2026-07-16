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

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift

* *GIVEN* `parquet 58.x` pulls in the vulnerable `thrift 0.17.0` as a transitive dependency
* *AND* `parquet 59.x` removes `thrift` but requires `arrow-schema` 59.x, while `adbc_core 0.23.0` requires `arrow-schema <59`, so adopting `parquet` 59.x forces two incompatible major versions of `arrow-schema` into the build graph and the advisory is not fixable by upgrading
* *WHEN* `cargo deny check advisories` is run
* *THEN* the check MUST exit with code 0
* *AND* the suppression MUST reference `GHSA-2f9f-gq7v-9h6m`, and its `reason` MUST state that `parquet 59.x` is released and removes `thrift`, that `adbc_core 0.23.0` caps `arrow-schema` at `<59` and blocks the `arrow`/`parquet` 59.x upgrade, and that re-evaluation is triggered when `adbc_core` lifts the `arrow-schema <59` cap
* *AND* the corresponding GitHub Dependabot alert MUST be `dismissed` with reason `tolerable_risk` and a dismissal comment referencing the `deny.toml` suppression rationale
<!-- /DELTA:CHANGED -->
