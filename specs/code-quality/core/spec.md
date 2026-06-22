# Feature: Code Quality

Code quality standards for the exarrow-rs codebase, ensuring zero clippy warnings, clean builds across all targets and features, and comprehensive test coverage for all changes.

## Background

The codebase SHALL maintain zero clippy warnings when built with all targets and features. ALL code changes MUST pass the complete test suite before being considered complete.

## Scenarios

### Scenario: Clippy validation

* *GIVEN* code changes have been made to the codebase
* *WHEN* running `cargo clippy --all-targets --all-features -- -W clippy::all`
* *THEN* it SHALL produce zero warnings
* *AND* it SHALL pass all lint checks

### Scenario: Test code lint exceptions

* *GIVEN* test code uses approximate values for constants
* *WHEN* test code uses approximate values for constants (e.g., 3.14 for PI)
* *THEN* it SHALL use `#[allow(clippy::approx_constant)]` attribute
* *AND* the lint exception SHALL be scoped to the test module only

### Scenario: Unit test validation

* *GIVEN* clippy fixes have been applied to the codebase
* *WHEN* clippy fixes are applied
* *THEN* `cargo test` MUST pass with zero failures
* *AND* no existing functionality SHALL be broken

### Scenario: Integration test validation

* *GIVEN* clippy fixes have been applied to the codebase
* *WHEN* clippy fixes are applied
* *THEN* integration tests against Exasol database MUST pass
* *AND* driver functionality SHALL remain intact

### Scenario: Tests do not mutate shared process environment

* *GIVEN* the test suite under `tests/`
* *WHEN* any test exercises environment-derived configuration (e.g. the `EXASOL_HOST`/`EXASOL_PORT`/`EXASOL_USER`/`EXASOL_PASSWORD` connection parameters)
* *THEN* it SHALL NOT call `std::env::set_var` or `std::env::remove_var`
* *AND* logic that depends on those values SHALL be exercised through pure helpers that take the values as arguments (e.g. `common::connection_string`)
* *AND* the integration suite MUST produce identical results regardless of test execution order or `--test-threads` count

### Scenario: Arrow and Parquet dependencies resolve to version 58 or above with no duplicate sub-crate versions

* *GIVEN* the resolved dependency tree in `Cargo.lock`
* *WHEN* inspecting the `[[package]]` entries
* *THEN* the `arrow` crate MUST resolve to a version `>= 58.0.0`
* *AND* the `parquet` crate MUST resolve to a version `>= 58.0.0`
* *AND* the `arrow-array` and `arrow-schema` sub-crates MUST NOT appear at both a 57.x and a 58.x version simultaneously (unified resolution required by `adbc_core 0.23.0`)
