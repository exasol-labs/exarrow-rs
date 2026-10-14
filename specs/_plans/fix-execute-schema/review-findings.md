# Code Review Findings: fix-execute-schema

## Summary
- Files reviewed: 6
- Total findings: 4 (standard: 4, expert: 0)

The production change in `execute_schema`, `set_sql_query`, and `ResultSet::build_schema` matches the plan and has no findings. `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -W clippy::all` report nothing, and the 8 new and changed FFI unit tests pass. All 4 findings are in test code. `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md` have no findings.

## Standard fixes

### src/adbc_ffi.rs

#### [SHRINKABLE] `connected_transport` copies the existing `schema_transport` and adds an expectation that never fires
- Location: lines 4563-4576 (`connected_transport`), call sites in `transport_that_never_executes` (line 4580) and `bound_execute_after_set_sql_query_runs_the_new_sql`
- Issue: `connected_transport` sets the same `connect`, `authenticate`, and `close` expectations as `schema_transport` (line 3960) in the same `tests` module. It also sets `expect_close_prepared_statement`. No FFI code path calls `close_prepared_statement`. Only `Connection::close_prepared` and the Parquet export call it, and no FFI code calls `close_prepared`. A copy of the worktree with those 3 lines removed passed all 7 `execute_schema` and `set_sql_query` unit tests, so the expectation is dead. Its doc comment also says the transport has "no prepare ... behavior", although it sets a close-prepared expectation.
- Fix: In src/adbc_ffi.rs, delete the function `connected_transport` and its doc comment. Replace the call `connected_transport()` with `schema_transport()` in `transport_that_never_executes` and in `bound_execute_after_set_sql_query_runs_the_new_sql`. Do not rename or edit `schema_transport`. Run `cargo test --lib --features ffi -- execute_schema set_sql_query` and confirm 7 passed.

#### [REDUNDANT_COMMENT] Doc comment on `transport_that_never_executes` restates the function name
- Location: line 4578
- Issue: `/// A transport that fails the test on any statement execution.` says what the name `transport_that_never_executes` already says. AGENTS.md allows a comment only for a non-obvious why and forbids restating the code.
- Fix: In src/adbc_ffi.rs, delete the doc comment line above `fn transport_that_never_executes`.

### tests/driver_manager_tests.rs

#### [SHRINKABLE] `open_connection` repeats the driver-loading block that `load_driver()` already provides
- Location: lines 3992-3997 (`open_connection`)
- Issue: `open_connection` calls `ManagedDriver::load_dynamic_from_filename(get_library_path(), Some(b"ExarrowDriverInit"), AdbcVersion::V110).expect("Failed to load driver")`. The helper `load_driver()` at line 3747 in the same file runs exactly this expression and returns the `ManagedDriver`.
- Fix: In tests/driver_manager_tests.rs, in `open_connection`, replace the `ManagedDriver::load_dynamic_from_filename(...)` expression and its `.expect("Failed to load driver")` with `load_driver()`, so the line reads `let mut driver = load_driver();`.

#### [REDUNDANT_COMMENT] Section banner added above the execute_schema tests
- Location: line 3979
- Issue: The line `// Statement Schema Lookup: execute_schema` is a section banner. It names the functions that follow and states no why. AGENTS.md forbids adding banners.
- Fix: In tests/driver_manager_tests.rs, delete the line `// Statement Schema Lookup: execute_schema` and the blank line that follows it.

## Expert fixes
[none]
