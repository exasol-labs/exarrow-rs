# AGENTS.md

Spec-driven development with mission in: @specs/mission.md

## Testing

- Integration and E2E tests run against a local Exasol Docker database. Start the container yourself, do not ask the user.
- Tests must fail, not skip, when Exasol is unavailable.
- Connection strings must set `validateservercertificate=0`, because the Docker image uses a self-signed certificate.

Project specifics:

- Start command: `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`
- Default credentials: `sys` / `exasol` on `localhost:8563`.
- Check readiness with `exapump sql 'select 1'`, which must return `1`.

## Code quality

- `cargo fmt --all` and `cargo clippy --all-targets` must pass with zero warnings before committing.
- Run clippy with `--all-features -- -W clippy::all`, as CI does.

## Code style

- A comment states a non-obvious why: an invariant, an external-system quirk, or a spec or issue constraint. Keep it to 1 or 2 lines. Never restate the code, narrate history, or add banners. Update or delete comments when behavior changes.
- A test implementing a spec scenario carries one `/// Scenario: <title>` line per scenario, quoting the title verbatim.

## Commands

```bash
cargo build --release --features ffi                 # cdylib for the driver manager
cargo test --lib                                     # unit tests
cargo test --test integration_tests                  # needs Exasol
cargo test --test driver_manager_tests               # needs the release cdylib
REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1  # needs Exasol
```

## Design

- ADBC hierarchy: Driver → Database → Connection → Statement. A Statement is pure data, and execution goes through the Connection.
- All I/O is async on Tokio. The Connection holds its transport as `Arc<Mutex<dyn TransportProtocol>>`, shared with `ResultSet` for lazy fetching.
- Import and export use HTTP tunneling with the EXA protocol handshake.
- TLS is on by default in the connection-string API, the builder, and the transport struct. Certificate validation is on by default.
- Never log or expose connection passwords.

## Driver manager tests and FFI

The tests in `tests/driver_manager_tests.rs` load `target/release/libexarrow_rs.so` (`.dylib` on macOS) at runtime.

- `cargo test` does not rebuild the cdylib. Run `cargo build --release --features ffi` first, or the tests run against a stale library.
- The FFI runtime is `multi_thread(2)` (`src/adbc_ffi.rs`). Import needs concurrent WebSocket and HTTP I/O inside `block_on`.
- Never run these tests, or any coverage command, with the `ffi` feature under `cargo-llvm-cov`. Its atexit handlers deadlock against the FFI `OnceLock<Runtime>`.

## CI

- The Rust toolchain is pinned in `.github/workflows/ci.yml`. Keep the local toolchain on the same version to avoid formatting drift.
- The GitHub Actions cache is immutable per `Cargo.lock` key. The integration job rebuilds the release cdylib so driver manager tests never use a stale artifact.
- `cargo test` captures output. Use `--nocapture` for diagnostic `eprintln!` traces.
- SonarQube Cloud analysis reads the unit-test lcov output. Integration coverage is not fed to Sonar, for the same FFI reason.

## Coverage

Coverage counts production code only. `scripts/strip_test_coverage.py` removes `#[cfg(test)]` code from the lcov report, because `cargo llvm-cov` otherwise counts unit tests in their own denominator.

```bash
cargo llvm-cov --lib --lcov --output-path lcov-unit.info
python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json
python3 scripts/strip_test_coverage.py check --summary coverage-summary.json
```

- `check` fails when total production line coverage is below 80.0% or any file is below 50.0%.
- Waive the per-file floor only by naming the file in `PER_FILE_FLOOR_EXEMPTIONS` with a reason. Delete an exemption once the file no longer needs it.
- Run the coverage command with default features only. The `websocket` feature lowers production coverage, and its build is checked separately with `cargo test --no-default-features --features websocket --tests --no-run`.

## Debugging CI hangs

1. Confirm the fix takes effect first. If a test loads a dynamic library, check that the library was rebuilt.
2. Add one minimal trace and confirm it appears in the CI log before adding more.
3. If two fix iterations fail, the root cause analysis is probably wrong. Investigate from scratch.

## Changelog

- Update `CHANGELOG.md` in the same PR as any user-facing change. Entries are concise and user-facing.
- A PR without a version bump adds entries under `## [Unreleased]`. Merging it does not release.
- A PR that bumps the `Cargo.toml` version puts entries under a `## X.Y.Z` header matching the version and folds in `[Unreleased]`. Merging it makes the CI `release` job tag, release, and publish to crates.io.
- The full procedure is in `CONTRIBUTING.md` under Releasing.
