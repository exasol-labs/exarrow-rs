# Feature: Code Quality

Code quality standards for the exarrow-rs codebase, ensuring zero clippy warnings, clean builds across all targets and features, and comprehensive test coverage for all changes.

## Background

The codebase SHALL maintain zero clippy warnings when built with all targets and features, without the `parquet` feature, and with the `websocket` feature alone, because code that only one feature uses passes an all-features build even when another build never uses it. ALL code changes MUST pass the complete test suite before being considered complete.

## Scenarios

### Scenario: Clippy validation

* *GIVEN* code changes have been made to the codebase
* *WHEN* running `cargo clippy --all-targets --all-features -- -D warnings`
* *THEN* it SHALL produce zero warnings
* *AND* it SHALL pass all lint checks
* *AND* `cargo clippy --all-targets --features ffi -- -D warnings` SHALL produce zero warnings
* *AND* `cargo clippy --all-targets --no-default-features --features websocket -- -D warnings` SHALL produce zero warnings

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

### Scenario: Arrow and Parquet dependencies resolve to one version of each Arrow sub-crate

* *GIVEN* the resolved dependency tree in `Cargo.lock`
* *WHEN* inspecting the `[[package]]` entries
* *THEN* the `arrow` crate MUST resolve to the release line that `Cargo.toml` requires
* *AND* the `parquet` crate MUST resolve to the release line that `Cargo.toml` requires
* *AND* the `arrow-array` and `arrow-schema` sub-crates MUST each appear at exactly one version (unified resolution required by `adbc_core`)
