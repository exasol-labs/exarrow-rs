# Feature: Code Quality

Code quality standards for the exarrow-rs codebase, ensuring zero clippy warnings, clean builds across all targets and features, and comprehensive test coverage for all changes.

## Background

The codebase SHALL maintain zero clippy warnings when built with all targets and features. ALL code changes MUST pass the complete test suite before being considered complete.

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Arrow and Parquet dependencies resolve to version 58 or above with no duplicate sub-crate versions

* *GIVEN* the resolved dependency tree in `Cargo.lock`
* *WHEN* inspecting the `[[package]]` entries
* *THEN* the `arrow` crate MUST resolve to a version `>= 58.0.0`
* *AND* the `parquet` crate MUST resolve to a version `>= 58.0.0`
* *AND* the `arrow-array` and `arrow-schema` sub-crates MUST NOT appear at both a 57.x and a 58.x version simultaneously (unified resolution required by `adbc_core 0.23.0`)
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Arrow and Parquet dependencies resolve to version 59 or above with one version of each Arrow sub-crate

* *GIVEN* the resolved dependency tree in `Cargo.lock`
* *WHEN* inspecting the `[[package]]` entries
* *THEN* the `arrow` and `parquet` crates MUST resolve to a version `>= 59.0.0`
* *AND* each of the `arrow-array`, `arrow-buffer`, `arrow-data`, and `arrow-schema` crates MUST appear in exactly one `[[package]]` entry, so that `adbc_core` and `arrow` share one Arrow version
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Every integration test target runs in the CI integration job

* *GIVEN* a top-level `.rs` file under `tests/` defines a Rust integration test target
* *WHEN* the CI `integration-tests` job runs against the Exasol container
* *THEN* the job SHALL run that target with `REQUIRE_EXASOL=1` and `--test-threads=1`
* *AND* every test in that target SHALL run, except a test whose `#[ignore = "<reason>"]` attribute states why it cannot run in CI
* *AND* a test that needs Exasol MUST fail, not skip, when Exasol is unavailable
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: CI rejects an integration test target that the integration job does not run

* *GIVEN* a top-level `.rs` file under `tests/` has no `--test <file stem>` invocation in `.github/workflows/ci.yml`, or a test under `tests/` carries an `#[ignore]` attribute without a reason
* *WHEN* CI runs `python3 scripts/check_ci_test_targets.py`
* *THEN* the check MUST exit with a non-zero exit code
* *AND* the output MUST name each missing test target and each `#[ignore]` without a reason
<!-- /DELTA:NEW -->
