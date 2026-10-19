# AGENTS.md

@specs/mission.md
@specs/architecture.md

## Specs

- `specs/` holds functional and behavioral requirements only.
- Integration and e2e testing have no specs. Their rules live in this file.
- Feature specs never pin dependency, toolchain, or image versions. Those live in `Cargo.toml`, the toolchain file, and CI. Exasol and protocol version thresholds that change behavior stay.
- A test implementing a spec scenario carries one `/// Scenario: <title>` line per scenario, quoting the title verbatim.

## Testing

- Integration and e2e tests run against a local Exasol Docker database. Start the container yourself, do not ask the user: `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`.
- Credentials are `sys` / `exasol` on `localhost:8563`. The database is ready when `exapump sql 'select 1'` returns `1`.
- Connection strings set `validateservercertificate=0`, because the Docker image uses a self-signed certificate.
- Tests fail, never skip, when Exasol is unavailable.
- Every top-level file under `tests/` is an integration test target. The CI integration job runs it with `REQUIRE_EXASOL=1` and `--test-threads=1`, and `scripts/check_ci_test_targets.py` enforces that. An `#[ignore]` states its reason.
- Tests never call `std::env::set_var` or `std::env::remove_var`. Pass values to pure helpers such as `common::connection_string`, so results do not depend on test order or `--test-threads`.
- The driver manager tests load `target/release/libexarrow_rs.so` (`.dylib` on macOS). `cargo test` does not rebuild it, so run `cargo build --release --features ffi` first.
- Never run tests or coverage with the `ffi` feature under `cargo-llvm-cov`.
- `cargo test` captures output. Use `--nocapture` for `eprintln!` traces.

## Code quality

- `cargo fmt --all` and `cargo clippy --all-targets --all-features -- -D warnings` pass with zero warnings before committing.
- Keep the local Rust toolchain on the version pinned in `.github/workflows/ci.yml`.
- Coverage counts production code only. Run `cargo llvm-cov --lib --lcov --output-path lcov-unit.info` with default features, then `scripts/strip_test_coverage.py strip` and `check`.
- Waive the per-file coverage floor only by naming the file in `PER_FILE_FLOOR_EXEMPTIONS` with a reason. Delete the exemption once the file no longer needs it.

## Code style

- A comment states a non-obvious why: an invariant, an external-system quirk, or a spec or issue constraint. Keep it to 1 or 2 lines. Never restate the code, narrate history, or add banners. Update or delete comments when behavior changes.

## Debugging CI hangs

- Confirm the fix takes effect first. If a test loads a dynamic library, check that the library was rebuilt.
- Add one minimal trace and confirm it appears in the CI log before adding more.
- If two fix iterations fail, the root cause analysis is probably wrong. Investigate from scratch.

## Changelog

- Bump the version in `Cargo.toml` in every PR.
- Update `CHANGELOG.md` in the same PR as any user-facing change. Entries are concise, user-facing, and sit under a `## X.Y.Z` header matching the new version.
- Merging a PR makes the CI `release` job tag, release, and publish to crates.io.
