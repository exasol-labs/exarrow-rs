# AGENTS.md

## Prerequisites

**Before running integration tests or examples**, start the Exasol Docker container yourself (don't ask the user):
```bash
docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest
```

Default credentials: `sys` / `exasol` on `localhost:8563`

**Check Exasol is ready** before running tests:
```bash
exapump sql 'select 1'   # Uses default profile; should return "1"
```

## Build & Test Commands

```bash
# Build
cargo build                              # Debug build
cargo build --release --features ffi     # Release with FFI for driver manager

# Linting (MUST pass before committing)
cargo fmt --all                          # Format code
cargo fmt --all -- --check               # Check formatting
cargo clippy --all-targets --all-features -- -W clippy::all  # Lint (zero warnings required)

# Tests
cargo test --lib                         # Unit tests only
cargo test --test integration_tests      # Integration tests (requires Exasol)
cargo test --test driver_manager_tests   # Driver manager tests
cargo test --test import_export_tests -- --ignored  # Import/export tests (requires Exasol)

# Run single test
cargo test test_name                     # Run specific test by name
cargo test --test integration_tests test_select_from_dual  # Single integration test
```

## Repository Structure

```
benches/           # Rust benchmarks (feature-gated behind "benchmark")
docs/              # User-facing documentation (connection, queries, import/export, type mapping, driver manager)
examples/          # Runnable usage examples (basic_usage, driver_manager_usage, import_export)
scripts/           # CI helper scripts
specs/             # Feature specifications (speq format: specs/<domain>/<feature>/spec.md)
src/               # Library source code (ADBC driver, transport, import/export, Arrow conversion)
tests/             # Integration test suites (integration_tests, driver_manager_tests, import_export_tests)
```

## Specifications

Specifications live in `specs/` using a `specs/<domain>/<feature>/spec.md` structure. Use the `speq` CLI to explore, search, and validate specs. Use Context7 MCP tools for third-party library research before implementing.

## Key Design Patterns

- **ADBC Driver Hierarchy:** Driver → Database → Connection → Statement
- Async-first: All I/O via Tokio
- Connection owns transport exclusively (no `Arc<Mutex<>>`)
- Statement is pure data; execution goes through Connection
- Import/Export uses HTTP tunneling with EXA protocol handshake

## Feature Flags

- `ffi` - Enable ADBC FFI/cdylib for driver manager integration
- `benchmark` - Enable benchmarking tools

## Important Constraints

- TLS is enabled by default in both the public connection-string/builder API and the low-level transport struct, matching Exasol 7.1+ (which requires TLS on port 8563) and all official Exasol drivers (pyexasol, JDBC, Go, ODBC).
- Certificate validation is on by default. Exasol Docker containers ship a self-signed certificate — test connection strings must set `?validateservercertificate=0` to accept it.
- Never log or expose connection passwords
- Integration tests require running Exasol instance

## Driver Manager Tests & FFI

Driver manager tests (`tests/driver_manager_tests.rs`) load `target/release/libexarrow_rs.so` (or `.dylib`) as a **dynamic library** at runtime. This has critical implications:

- **`cargo test` does NOT rebuild the cdylib.** Always run `cargo build --release --features ffi` before driver manager tests, otherwise tests run against a stale `.so`.
- **The FFI runtime is `multi_thread(2)`** (`src/adbc_ffi.rs`). This is required for import operations that need concurrent WebSocket + HTTP I/O inside `block_on`.
- **Do not run driver manager tests under `cargo-llvm-cov`.** The coverage instrumentation's atexit handlers conflict with the FFI static runtime, causing hangs.
- The FFI `OnceLock<Runtime>` is a process-global static inside the `.so` — it persists across all test invocations in the same process.

## CI Pipeline

- **Rust toolchain is pinned to 1.92.0** in `.github/workflows/ci.yml`. Keep CI and local toolchains in sync to avoid formatting drift.
- **GitHub Actions cache is immutable** — once a cache entry exists for a key based on `Cargo.lock`, it cannot be updated. The integration tests job explicitly rebuilds the release cdylib before driver manager tests to avoid stale artifacts.
- **`cargo test` captures stdout/stderr by default.** When adding diagnostic `eprintln!` traces for CI debugging, use `--nocapture`. Always verify diagnostic output is visible before adding more instrumentation.
- Integration tests job has `timeout-minutes: 30` to prevent runaway hangs.
- **SonarQube Cloud static analysis** runs via the `Sonar Analysis` job (config in `sonar-project.properties`), consuming the `unit-tests` job's lcov output for coverage. Its Quality Gate is intended to become a required, PR-blocking check (alongside Build/Lint/License Check/Unit Tests/Integration Tests) once rolled out. Integration-test coverage is deliberately not fed into Sonar — same FFI/`cargo-llvm-cov` atexit-hang reason as the driver manager tests above.
- **Coverage is measured on production code only** — see the next section. The `unit-tests` job enforces the floors; Sonar reads the same stripped report.

## Coverage Measurement

`cargo llvm-cov` instruments `#[cfg(test)]` modules like any other code, so the raw report counts the unit tests in their own denominator. On this crate the test code is roughly two thirds of the instrumented lines, which inflated the reported figure by about 8 percentage points (91.7% raw vs. 83.7% production-only at the time of writing). Rust's `#[coverage(off)]` attribute would exclude test modules at the source level, but it is behind the nightly `coverage_attribute` feature gate and this crate pins **stable 1.92.0**, so the exclusion happens after the fact instead.

`scripts/strip_test_coverage.py` (stdlib-only Python, no dependencies) does the stripping:

```bash
cargo llvm-cov --lib --lcov --output-path lcov-unit.info

python3 scripts/strip_test_coverage.py strip \
  --input lcov-unit.info \
  --output lcov-unit-production.info \
  --summary coverage-summary.json

python3 scripts/strip_test_coverage.py check --summary coverage-summary.json
```

- `strip` removes every `DA:`/`BRDA:`/`FN:`/`FNDA:` entry falling inside a `#[cfg(test)] mod name { … }` block, drops whole records for out-of-line test modules (`#[cfg(test)] mod name;` in a sibling file — e.g. `src/transport/test_support.rs`), recomputes `LF`/`LH`/`BRF`/`BRH`/`FNF`/`FNH` per file, and writes `coverage-summary.json` with the totals plus a per-file list sorted ascending by percentage.
- Module bodies are brace-matched through a small Rust lexer state (comments, nested block comments, strings, raw strings with hash delimiters, char literals vs. lifetimes), because a naive brace counter miscounts on `"}"`, `'}'`, and `r#"}"#`. `#[cfg(not(test))]` and `#[cfg(any(test, unix))]` also guard production code and are deliberately **not** stripped.
- `check` fails the job when total production line coverage is below **80.0%**, or when any single file is below **50.0%**.
- The script's own unit tests (`scripts/test_strip_test_coverage.py`) run in the `unit-tests` job before the coverage step: `python3 -m unittest discover --start-directory scripts --pattern 'test_*.py'`.
- The `Upload unit coverage` step is `if: always()`, so a run that trips a floor still publishes the report needed to diagnose it.

**Per-file floor exemptions** live in `PER_FILE_FLOOR_EXEMPTIONS` in the script. Lowering the global 50% floor to accommodate one file is not acceptable; name the file instead, with a reason. Currently exempt:

- `src/export/csv.rs` (48.4%) — remove the exemption once its uncovered write paths are unit-tested.

Re-check the list whenever coverage work lands: an exemption that is no longer needed is stale and should be deleted, since exemptions only waive the floor and never cap a file.

**Feature flags in the coverage command:** the command runs with **default features only**.

- **`websocket` is deliberately not enabled.** `src/transport/websocket.rs` has 26 unit tests that never run in CI today. Enabling the feature runs them (1,541 → 1,568 unit tests) but *lowers* production-only coverage from **83.70% to 82.95%**, because the file adds 278 production lines at only 56.99% coverage — below the crate average — so it drags the denominator down faster than the numerator. Its build is still checked (`cargo test --no-default-features --features websocket --tests --no-run`) and its behavior is covered by `websocket_integration_tests` in the integration job. Revisit if `websocket.rs` unit coverage rises above the crate average.
- **`ffi` must never be enabled** in a coverage command: the instrumentation's atexit handlers deadlock against the FFI `OnceLock<Runtime>` (see the driver manager section above).

## Debugging CI Hangs

When a CI test hangs:
1. **Verify your fix takes effect first** — if the test loads a dynamic library, confirm the library is rebuilt with your changes (not served from cache).
2. **Add ONE minimal trace and confirm it appears** in the CI log before adding elaborate instrumentation. Remember `--nocapture`.
3. **Don't trust a root cause analysis blindly** — verify the hypothesis by observing the actual behavior change. If 2 iterations of a fix don't work, the RCA is likely wrong; investigate from scratch.

## Changelog

- `CHANGELOG.md` must be updated with every version bump
- Format: `## <version>` header followed by bullet points describing changes
- Entries should be concise, user-facing descriptions (not internal implementation details)
