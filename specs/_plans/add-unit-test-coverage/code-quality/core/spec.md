# Feature: Code Quality

Code quality standards for the exarrow-rs codebase, ensuring zero clippy warnings, clean builds across all targets and features, and comprehensive test coverage for all changes.

## Background

The codebase SHALL maintain zero clippy warnings when built with all targets and features. ALL code changes MUST pass the complete test suite before being considered complete. Library unit-test coverage measured by `cargo llvm-cov --lib` MUST NOT fall below 82 percent total and MUST NOT fall below the per-file minimum recorded in `.github/workflows/ci.yml`, and the unit-test suite MUST pass with no Exasol instance reachable. The reported total counts lines inside `#[cfg(test)]` modules, so it rises when test code is added and MUST NOT be read as production-code coverage.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Unit test coverage floor enforced in CI

* *GIVEN* the CI `unit-tests` job measures library coverage with `cargo llvm-cov --lib`
* *WHEN* total line coverage falls below 82 percent
* *THEN* the job MUST exit with a non-zero status
* *AND* the threshold MUST be enforced by cargo-llvm-cov's `--fail-under-lines` option rather than by inspecting job logs
* *AND* the measured total line coverage MUST be at least 82 percent
* *WHEN* the line coverage of any single source file falls below the per-file minimum configured in the job
* *THEN* the job MUST exit with a non-zero status
* *AND* the per-file minimum MUST be enforced by cargo-llvm-cov's `--fail-under-file-lines` option
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Unit tests pass without a reachable database

* *GIVEN* no Exasol instance is reachable
* *WHEN* running `cargo test --lib`
* *THEN* every unit test MUST pass
* *AND* no unit test SHALL open a network connection to any address other than a loopback address
* *AND* a unit test that exercises a transport failure path SHALL target a closed loopback port, a loopback `TcpListener`, or an in-process stream
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Reported coverage figure records its composition

* *GIVEN* `cargo llvm-cov --lib` instruments every line compiled into the library test binary, including lines inside `#[cfg(test)]` modules
* *WHEN* a reader interprets the reported coverage percentage
* *THEN* `AGENTS.md` MUST state that the reported figure counts in-file test-module lines and therefore exceeds production-code coverage
* *AND* `AGENTS.md` MUST state that the total floor does not block a new untested module, and MUST name `--fail-under-file-lines` as the guard that does
* *AND* `AGENTS.md` MUST record the count of uncovered lines in `lcov-unit.info` together with the command that produces the count, so a later reader can measure progress without the reported percentage
* *AND* `AGENTS.md` MUST name the source files excluded from the measurement because their Cargo features are off by default (`src/adbc_ffi.rs` behind `ffi`, `src/transport/websocket.rs` behind `websocket`)
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: TLS certificate verification is unit-tested

* *GIVEN* `src/transport/tls.rs` implements the `NoVerifier` and `FingerprintVerifier` certificate verifiers
* *WHEN* running `cargo test --lib`
* *THEN* a unit test MUST assert that `FingerprintVerifier` accepts a certificate whose SHA-256 fingerprint matches the expected value
* *AND* a unit test MUST assert that `FingerprintVerifier` rejects a mismatched fingerprint with an error naming both the expected and the actual fingerprint
* *AND* a unit test MUST assert that `NoVerifier` accepts an arbitrary certificate, pinning the documented `validateservercertificate=0` behavior
* *AND* a unit test MUST assert that both verifiers report the schemes returned by `all_supported_verify_schemes`
<!-- /DELTA:NEW -->
