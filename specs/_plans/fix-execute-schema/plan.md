# Plan: fix-execute-schema

## Summary

ADBC `execute_schema` returns the result schema of a statement from the metadata of its prepare and no longer executes the statement (#62). The fix ships as a patch release, expected 0.21.3 if PR #93 is the only release that merges before it.

## Context

- `FfiStatement::execute_schema` (`src/adbc_ffi.rs:2326`) prepares the statement and then runs its SQL with `query_batches` (`src/adbc_ffi.rs:521`). `query_batches` calls `Connection::query`, which executes the SQL and fetches every batch. The method returns the schema of the first batch.
- The code therefore executes the statement, which is worse than #62 states. This follows from reading the code and has not been run against Exasol:
  - An INSERT, UPDATE, or DELETE changes the table. The call then fails with status `Internal` and the message `Cannot fetch batches from row count result`, because `ResultSet::fetch_all` rejects a row count.
  - A SELECT with parameter markers runs as plain SQL without parameters, so the call fails.
  - A SELECT runs in full and fetches every row.
- The `adbc_core` 0.24 trait doc defines `Statement::execute_schema` as "Get the schema of the result set of a query without executing it" and adds "Depending on the driver, this may require first executing [Statement::prepare]". It does not define the case of a statement without a result set.
- A prepared statement handle holds `PreparedStatementHandle::result_columns` (`src/transport/protocol.rs:113`), a list of `ColumnInfo` (`src/transport/messages.rs:369`). Both transports fill it from the prepare reply, and it is empty for a statement that returns an affected-row count (`prepared-statements/result-columns`).
- `ResultSet::build_schema` (`src/query/results.rs:378`) builds the schema of every zero-row result and of every WebSocket result from the same column metadata.
- Over the native transport, a result with rows can carry other Arrow types than its zero-row schema for integer-coded DECIMAL, TIMESTAMP WITH LOCAL TIME ZONE, INTERVAL, GEOMETRY, and HASHTYPE columns. This plan does not change that. Decision-log entry [1] lists the types.
- `set_sql_query` (`src/adbc_ffi.rs:2400`) replaces the SQL text and keeps the prepared handle. A later `get_parameter_schema` or bound `execute` uses the handle of the earlier SQL, and a fixed `execute_schema` would too.
- No spec, document, example, or test mentions `execute_schema`. The table of supported ADBC features in the module doc of `src/adbc_ffi.rs` (lines 43-55) does not list it.
- Merging a version bump to `main` makes the CI `release` job tag the version, create the GitHub release from the matching `CHANGELOG.md` section, and publish to crates.io (`CONTRIBUTING.md`, Releasing).
- Architecture: no change. `execute_schema` stays inside `adbc_ffi` and uses the existing prepare path and the existing `query` result-set mapping. No component, boundary, interface, data flow, constraint, or external dependency in `specs/architecture.md` changes.
- Follow-up plans:
  - align-native-result-batch-types: make the Arrow types of native result batches with rows agree with the column schema that zero-row results and `execute_schema` report.
  - Rename the spec directory `adbc-driver/get-parameter-schema` to match its new title, Statement Schema Lookup. This is a library reorganization and needs a user decision (decision-log entry [4]).
  - Make `transport_datatype_to_exasol` (`src/adbc_ffi.rs:325`), which `get_parameter_schema` uses, delegate to `exasol_type_of`, so that one parser reads Exasol type names.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Statement Schema Lookup | CHANGED | `adbc-driver/get-parameter-schema/spec.md` |

## Impact

- ADBC `execute_schema`, which Python reaches through `cursor.adbc_execute_schema`, no longer runs the statement. An INSERT, UPDATE, or DELETE no longer changes data, a SELECT no longer fetches its rows, and a SELECT with parameter markers works without bound parameters.
- `execute_schema` on a statement without a result set returns a schema with zero fields instead of an `Internal` error.
- After `set_sql_query` sets different SQL on a prepared statement, `get_parameter_schema` returns `InvalidState` until the statement is prepared for the new SQL, and `execute` or `execute_update` with a bound RecordBatch prepares and runs the new SQL. Before, both used the prepared statement of the earlier SQL.
- Over the native transport, `execute_schema` of a query whose result has rows reports the types of the column schema. These are Decimal128(18,0) and Decimal128(9,0) for DECIMAL columns that the native protocol sends as 64-bit or 32-bit integers, Timestamp with time zone UTC for TIMESTAMP WITH LOCAL TIME ZONE, Interval for INTERVAL, and Binary for GEOMETRY and HASHTYPE. It used to report the types of the first fetched batch: Int64, Int32, Timestamp without time zone, and Utf8. The batches that `execute` returns over the native transport keep those types until the follow-up plan align-native-result-batch-types.
- No breaking change. The Rust public API is unchanged, because `ResultSet::build_schema` becomes `pub(crate)` only. The earlier `execute_schema` types came from running the statement. The new schema equals the schema of a zero-row result and the schema over the WebSocket transport.
- Patch release, expected 0.21.3 if PR #93 is the only release in between.

## Dependencies

- This branch is stacked on `feat/fix-tls-import-handshake` (PR #93), which merges first and releases 0.21.2. When PR #93 squash-merges, the orchestrator rebases this branch onto `origin/main`. Task 4.1 reads the `Cargo.toml` version of the base again at implement time and after each rebase.
- The unit fix-connection-parameters edits the connection-parameter code of `src/adbc_ffi.rs` in parallel. The later unit fix-result-set-iterator (#66) edits `src/adbc_ffi.rs` after this plan merges. The diff in `src/adbc_ffi.rs` stays within `execute_schema`, `set_sql_query`, one row of the module doc table, new tests, and the added assertions of `test_get_parameter_schema_not_prepared` (task 2.2), and it moves or reformats no other code.
- A local Exasol Docker container named `exasol-test` for tasks 1.7 and 3.5.
- No new crate dependency.

## Implementation Tasks

1. Driver-manager tests that show the defect (group A)
- [ ] 1.1 Reuse a running `exasol-test` container (`docker ps --filter name=exasol-test`), start it with `docker start exasol-test` if it exists and is stopped, or create it with `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`. Run only one Exasol container on the machine. Wait until `exapump sql 'select 1'` returns `1`.
- [ ] 1.2 In `tests/driver_manager_tests.rs`, add the helper `assert_execute_schema_reports_columns_without_running(uri: String)`, which opens the driver, database, and connection for `uri` the way `assert_bound_temporal_values_are_stored` does (line 2911). It creates a schema named with `generate_unique_test_name("TEST_EXEC_SCHEMA")`, a table `T (TXT VARCHAR(10))` holding the row `'x'`, and an empty table `E (ID DECIMAL(18,0), NAME VARCHAR(50), AMOUNT DECIMAL(10,2), D DATE, TS TIMESTAMP, F DOUBLE, B BOOLEAN)`. It then checks two cases and drops the schema with `drop_test_schema`:
  - Run-time failure: on an unprepared statement with `SELECT CAST(TXT AS DECIMAL(9,0)) AS N FROM <schema>.T`, `execute_schema` returns exactly one field, `N`, nullable, of type `Decimal128(9, 0)`. A second statement with the same SQL fails on `execute()`, which proves that the query fails when it runs.
  - Zero-row parity: on a statement with `SELECT ID*2 AS DOUBLED, UPPER(NAME), AMOUNT, D, TS, F, B FROM <schema>.E` and `prepare()`, `execute_schema` returns 7 fields, the first named `DOUBLED` and the second with a non-empty name. The schema equals `reader.schema()` of `execute()` for the same SQL on another statement, compared with `assert_eq!` on the whole schema.
  Add `test_execute_schema_reports_columns_without_running_the_query`, which calls the helper with `get_test_uri()`, and `test_execute_schema_reports_columns_without_running_the_query_websocket` with `#[cfg(feature = "websocket")]`, which calls it with `format!("{}&transport=websocket", get_test_uri())`. Both carry `/// Scenario: Execute schema returns the result columns of a query without executing it`.
- [ ] 1.3 Add `test_execute_schema_of_parameterized_select_needs_no_binding`: a table `T (ID DECIMAL(18,0), NAME VARCHAR(50))`, a statement with `SELECT ID, NAME FROM <schema>.T WHERE ID = ?`, `prepare()`, no `bind()`, and `execute_schema` returns the fields `ID` and `NAME`. Scenario line: `Execute schema of a parameterized query needs no bound parameters`.
- [ ] 1.4 Add `test_execute_schema_of_row_count_statement_is_empty_and_runs_nothing`: a table `T (ID DECIMAL(18,0))` holding the row `1`. On one statement with `INSERT INTO <schema>.T VALUES (2)` and on another with `DELETE FROM <schema>.T`, `execute_schema` returns a schema with zero fields. Afterwards `integers_from(&mut conn, "SELECT ID FROM <schema>.T")` returns `[1]`. Scenario line: `Execute schema of a statement without a result set returns an empty schema`.
- [ ] 1.5 Add `test_execute_schema_follows_sql_set_after_prepare`: a table `T (ID DECIMAL(18,0), NAME VARCHAR(50))`.
  - Statement one: set `SELECT ID FROM <schema>.T WHERE ID = ?` and call `prepare()`. Set `SELECT NAME, ID FROM <schema>.T`. `get_parameter_schema` then returns an error with status `InvalidState`, and after it `execute_schema` returns the fields `NAME` and `ID`. The order matters, because `execute_schema` prepares the new SQL.
  - Statement two: set `SELECT ID FROM <schema>.T WHERE ID = ?` and call `prepare()`. Set `INSERT INTO <schema>.T (ID) VALUES (?)`, bind an Int32 batch holding `7`, and call `execute_update`. Afterwards `T` holds the row with `ID` 7.
  Scenario line: `Setting new SQL discards the earlier prepare`.
- [ ] 1.6 Add `test_execute_schema_without_sql_fails_with_invalid_state`. A statement with no SQL fails with status `InvalidState`. A statement with only the option `OptionStatement::Other("adbc.ingest.target_table")` set to `T` also fails with status `InvalidState`. Scenario line: `Execute schema without SQL fails`.
  Rules for tasks 1.2 to 1.6:
  - Each new test starts with `skip_if_no_library!();` and does not call `skip_if_no_exasol!()`. An unreachable Exasol then panics in `setup_driver_manager_conn!` or in the helper's `.expect` and fails the test, as `AGENTS.md` requires.
  - `skip_if_no_library!` stays, because every test in the file uses it, and the Checklist builds the library before the run.
  - Each test creates its own schema and drops it.
- [ ] 1.7 On the unchanged driver code, run `cargo build --release --features 'ffi websocket'` and then `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests execute_schema -- --test-threads=1 --nocapture`. Confirm that every new test fails except `test_execute_schema_without_sql_fails_with_invalid_state`, which pins existing behavior. Note each failure message for the verification report.

2. Unit tests (group A)
- [ ] 2.1 In the `tests` module of `src/adbc_ffi.rs`, next to `prepared_transport`, `row_count_handle`, `result_set_handle`, and `statement_over` (line 4507), add the tests below. Each test except `bound_execute_after_set_sql_query_runs_the_new_sql` sets `expect_execute_query().never()` and `expect_execute_prepared_statement().never()` on its `MockTransport`. Each test carries the `/// Scenario:` lines named here:
  - `execute_schema_maps_result_columns_without_executing`: a handle with one parameter and the columns `N DECIMAL(18,0)` and `NAME VARCHAR(50)`, a statement that is not prepared, and no bound batch. One `create_prepared_statement` call happens, and the schema is `N: Decimal128(18, 0)` and `NAME: Utf8`, both nullable. Scenario lines: `Execute schema returns the result columns of a query without executing it` and `Execute schema of a parameterized query needs no bound parameters`.
  - `execute_schema_of_row_count_statement_is_empty`: `row_count_handle(2)` gives a schema with zero fields. Scenario line: `Execute schema of a statement without a result set returns an empty schema`.
  - `set_sql_query_discards_the_prepared_statement`: `create_prepared_statement` runs twice and returns a handle with the column `A` for SQL A and one with the column `B` for SQL B, matched on the SQL argument. After `prepare()` for A and `set_sql_query` with B, `get_parameter_schema` fails with status `InvalidState` and `execute_schema` returns the field `B`. Scenario line: `Setting new SQL discards the earlier prepare`.
  - `bound_execute_after_set_sql_query_runs_the_new_sql`: `create_prepared_statement` returns a handle with id 17 and one parameter for SQL A and a row-count handle with id 18 and one parameter for SQL B. After `prepare()` for A, `set_sql_query` with B, and `bind` of a one-row batch, `execute()` sends `execute_prepared_statement` once with handle id 18 and never with handle id 17, matched on the handle argument. Scenario line: `Setting new SQL discards the earlier prepare`.
  - `set_sql_query_with_unchanged_sql_keeps_the_prepared_statement`: after `prepare()` and `set_sql_query` with the same text, `get_parameter_schema` succeeds and `execute_schema` runs no second `create_prepared_statement` (`times(1)`). It implements no scenario and pins decision-log entry [3].
  - `execute_schema_without_sql_fails_with_invalid_state`: a connected statement without SQL and with the option `adbc.ingest.target_table`, with `expect_create_prepared_statement().never()`, fails with status `InvalidState`. Scenario line: `Execute schema without SQL fails`.
  - `execute_schema_reports_an_unmappable_result_column_type`: a handle with a column of type name `BINARY`, which `exasol_type_of` rejects. The call fails with status `Internal` and a message that contains `BINARY`. Scenario line: `Execute schema reports a result-set column type that has no Arrow mapping`.
- [ ] 2.2 Add `/// Scenario: Get parameter schema after prepare` to `test_parameter_schema_names` (`tests/driver_manager_tests.rs:3321`) and `/// Scenario: Get parameter schema without prepare` to `test_get_parameter_schema_not_prepared` (`src/adbc_ffi.rs:2671`) where the line is missing. `test_get_parameter_schema_not_prepared` asserts only `is_err()` today. Add assertions that the status is `AdbcStatus::InvalidState` and that the message contains `prepare`.
- [ ] 2.3 Run `cargo test --lib --features ffi execute_schema` and `cargo test --lib --features ffi set_sql_query`. Confirm that the new tests fail on the unchanged code, except `execute_schema_without_sql_fails_with_invalid_state` and `set_sql_query_with_unchanged_sql_keeps_the_prepared_statement`, which pin existing behavior.

3. Fix (group A)
- [ ] 3.1 In `src/query/results.rs`, make `ResultSet::build_schema` `pub(crate)`. Its doc comment states only the invariant: query results and ADBC `execute_schema` must report the same column types, so both build the schema with this function. Nothing else in the file changes.
- [ ] 3.2 In `src/adbc_ffi.rs`, replace the body of `execute_schema`. It prepares the statement when `self.prepared` is none, as today, and returns `InvalidState` without SQL through `prepare`. It passes `prepared.handle_ref().result_columns` to `ResultSet::build_schema`, maps a `ConversionError` with `to_adbc_error` (status `Internal`), and returns the schema. It sends no execution request and leaves `bound_data` untouched. Empty `result_columns` give a schema with zero fields.
- [ ] 3.3 In `set_sql_query`, set `self.prepared = None` when the new SQL text differs from `self.sql`, and keep the prepared statement when the text is unchanged. Add a one-line comment that a prepare belongs to the SQL text it ran for (decision-log entry [3]).
- [ ] 3.4 Add the row `` | `execute_schema` | Supported | `` to the module doc table of `src/adbc_ffi.rs`, after the `get_parameter_schema` row.
- [ ] 3.5 Run `cargo test --lib --features ffi` and confirm 0 failures, including the existing bound-execution and parameter-schema tests. Rebuild with `cargo build --release --features 'ffi websocket'` and run `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --include-ignored --test-threads=1` with 0 failures.

4. Release (group B)
- [ ] 4.1 Set `version` in `Cargo.toml` to the next patch version above both the `Cargo.toml` version of the base at implement time and the version of every release PR that merges before this one. PR #93 releases 0.21.2, so the expected version is 0.21.3. If PR #93 has not set its version yet, count its planned 0.21.2 anyway. The number changes only if PR #93 or another release PR that merges first sets a higher version. Check it again after each rebase and before merge. Run `cargo check` so the `exarrow-rs` entry in `Cargo.lock` takes the version.
- [ ] 4.2 Add a `## <version>` section to `CHANGELOG.md` above the newest section, and fold in the entries of a `## [Unreleased]` section if one exists. Entries:
  - A `Fix:` entry stating that ADBC `execute_schema`, which Python reaches through `cursor.adbc_execute_schema`, returns the result schema of a statement without running it. It used to run the statement, so an INSERT, UPDATE, or DELETE changed data before the call failed, and a SELECT with parameter markers failed. A statement without a result set returns a schema with no fields. The entry ends with `Fixes #62.`.
  - A `Fix:` entry stating that after `set_sql_query` sets different SQL on a prepared ADBC statement, `execute` and `execute_update` with bound parameters prepare and run the new SQL instead of the earlier prepared statement, and `get_parameter_schema` fails with `InvalidState` until the statement is prepared again.
  - A `Changed:` entry stating that over the native transport, `execute_schema` of a query that returns rows reports the types of a zero-row result, which are also the types over the WebSocket transport. DECIMAL columns that Exasol sends as integers are Decimal128 instead of Int64 or Int32, TIMESTAMP WITH LOCAL TIME ZONE is a Timestamp with time zone UTC, INTERVAL is Interval instead of Utf8, and GEOMETRY and HASHTYPE are Binary instead of Utf8. The batches that `execute` returns over the native transport keep their types.
- [ ] 4.3 Run the Checklist below and confirm every expected result.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: execute_schema and statement schema lookup | 1.1-3.5 | none | spec delta `adbc-driver/get-parameter-schema`; `src/adbc_ffi.rs` (`execute_schema`, `set_sql_query`, `prepare`, `get_parameter_schema`, `prepared_with_connection`, the module doc table, the bound-execution unit-test helpers), `src/query/results.rs` (`build_schema`, `exasol_type_of`), `tests/driver_manager_tests.rs`, `tests/common/mod.rs` (read only) |
| B: release | 4.1-4.3 | A (the CHANGELOG describes A's result) | `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `CONTRIBUTING.md` (Releasing, read only) |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None | none | `query_batches` stays because `get_objects` and `get_table_schema` call it. `transport_datatype_to_exasol` stays because `get_parameter_schema` calls it. |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Execute schema returns the result columns of a query without executing it | Integration | `tests/driver_manager_tests.rs` | `test_execute_schema_reports_columns_without_running_the_query`, `test_execute_schema_reports_columns_without_running_the_query_websocket` |
| Execute schema returns the result columns of a query without executing it | Unit | `src/adbc_ffi.rs` | `execute_schema_maps_result_columns_without_executing` |
| Execute schema of a parameterized query needs no bound parameters | Integration | `tests/driver_manager_tests.rs` | `test_execute_schema_of_parameterized_select_needs_no_binding` |
| Execute schema of a parameterized query needs no bound parameters | Unit | `src/adbc_ffi.rs` | `execute_schema_maps_result_columns_without_executing` |
| Execute schema of a statement without a result set returns an empty schema | Integration | `tests/driver_manager_tests.rs` | `test_execute_schema_of_row_count_statement_is_empty_and_runs_nothing` |
| Execute schema of a statement without a result set returns an empty schema | Unit | `src/adbc_ffi.rs` | `execute_schema_of_row_count_statement_is_empty` |
| Setting new SQL discards the earlier prepare | Integration | `tests/driver_manager_tests.rs` | `test_execute_schema_follows_sql_set_after_prepare` |
| Setting new SQL discards the earlier prepare | Unit | `src/adbc_ffi.rs` | `set_sql_query_discards_the_prepared_statement`, `bound_execute_after_set_sql_query_runs_the_new_sql` |
| Execute schema without SQL fails | Integration | `tests/driver_manager_tests.rs` | `test_execute_schema_without_sql_fails_with_invalid_state` |
| Execute schema without SQL fails | Unit | `src/adbc_ffi.rs` | `execute_schema_without_sql_fails_with_invalid_state` |
| Execute schema reports a result-set column type that has no Arrow mapping | Unit | `src/adbc_ffi.rs` | `execute_schema_reports_an_unmappable_result_column_type` |
| Get parameter schema after prepare | Integration | `tests/driver_manager_tests.rs` | `test_parameter_schema_names` (existing) |
| Get parameter schema without prepare | Unit | `src/adbc_ffi.rs` | `test_get_parameter_schema_not_prepared` (existing) |

No known SQL statement makes the Docker image report a result-set column type without an Arrow mapping, so a mock transport supplies the `BINARY` column for that scenario.

### Manual Testing

Requires the Python packages `adbc-driver-manager` and `pyarrow`, the release build from the Checklist, and the `exasol-test` container.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Statement Schema Lookup | `exapump sql 'CREATE SCHEMA IF NOT EXISTS ES_MANUAL'`, `exapump sql 'CREATE OR REPLACE TABLE ES_MANUAL.T (TXT VARCHAR(10))'`, and `exapump sql "INSERT INTO ES_MANUAL.T VALUES ('x')"` | Each command exits 0 |
| Statement Schema Lookup | `python3 -c "import adbc_driver_manager.dbapi as d; c = d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://sys:exasol@localhost:8563?tls=true&validateservercertificate=0'}, autocommit=True); cur = c.cursor(); print(cur.adbc_execute_schema('SELECT CAST(TXT AS DECIMAL(9,0)) AS N FROM ES_MANUAL.T')); print(len(cur.adbc_execute_schema(\"INSERT INTO ES_MANUAL.T VALUES ('y')\")))"` | Prints `N: decimal128(9, 0)`, then `0` |
| Statement Schema Lookup | `exapump sql 'SELECT COUNT(*) FROM ES_MANUAL.T'`, then `exapump sql 'DROP SCHEMA ES_MANUAL CASCADE'` | Returns `1`, so the INSERT did not run, and the schema is dropped |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Format | `cargo fmt --all -- --check` | No changes |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Unit tests | `cargo test --lib` | 0 failures |
| FFI unit tests | `cargo test --lib --features ffi` | 0 failures |
| WebSocket build | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json`, then `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json`, with default features only and never `ffi` | `check` passes: total at least 80.0% and every file at least 50.0%. `src/adbc_ffi.rs` is not in this report, because it compiles only with `ffi`, and `PER_FILE_FLOOR_EXEMPTIONS` is empty |
| Release build | `cargo build --release --features 'ffi websocket'` | Exit 0 |
| Driver-manager tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| Plan validation | `speq plan validate fix-execute-schema` | Pass |
