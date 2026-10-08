# Feature: Code Quality

Code quality standards for the exarrow-rs codebase, ensuring zero clippy warnings, clean builds across all targets and features, and comprehensive test coverage for all changes.

## Background

The codebase SHALL maintain zero clippy warnings when built with all targets and features. ALL code changes MUST pass the complete test suite before being considered complete.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Arrow and Parquet dependencies resolve to version 58 or above with no duplicate sub-crate versions

* *GIVEN* the resolved dependency tree in `Cargo.lock`
* *WHEN* inspecting the `[[package]]` entries
* *THEN* the `arrow` crate MUST resolve to a version `>= 58.0.0`
* *AND* the `parquet` crate MUST resolve to a version `>= 58.0.0`
* *AND* the `arrow-array` and `arrow-schema` sub-crates MUST NOT appear at both a 57.x and a 58.x version simultaneously (unified resolution required by `adbc_core`)
<!-- /DELTA:CHANGED -->
