# Plan: fix-ffi-bound-execute

> **Status:** blocked: see open-questions.md

## Summary

ADBC `execute` and `execute_update` with a bound RecordBatch send all rows to Exasol in one prepared-statement execution, so a prepared INSERT through `execute` no longer fails after writing, writes every bound row, and a bound batch takes effect for all of its rows or for none. Over the native protocol, a batch whose parameter values exceed the maximum data message size that Exasol reports at login runs as consecutive executions instead of breaking the connection. The ADBC crates move to 0.24, so `rowcount` after `execute` reads -1 (unknown) instead of 0. Fixes #78, fixes #67.

## Context

- Issue exasol-labs/exarrow-rs#78: a prepared DML statement run through ADBC `execute`, such as Python `cursor.execute("INSERT ... VALUES (?, ?)", params)`, writes the row and then fails with `Cannot fetch batches from row count result`. A caller that retries writes duplicates. With a three-row bound batch, only the first row is written.
- `FfiStatement::execute_bound_batch` (`src/adbc_ffi.rs`) runs the prepared statement once per bound row and calls `ResultSet::fetch_all()` on each result. For a row-count result, `fetch_all()` returns `QueryError::NoResultSet` (`src/query/results.rs`). `FfiStatement::query_sql` already turns that error into no batches.
- `rowcount` after `execute` is always 0, also for DML that changed rows. adbc_ffi 0.23 never writes `rows_affected` when the caller requests a result stream, so Python reads back its own start value (apache/arrow-adbc#4468). adbc_ffi 0.24.0 writes -1 there (apache/arrow-adbc#4469). The Rust `adbc_core::Statement::execute` method returns only a reader, so a real count from `execute` is not possible.
- Issue exasol-labs/exarrow-rs#67: `execute_bound_batch` and `execute_bound_batch_update` take the connection lock, block on the runtime, and execute on the server once per row. 100,000 rows cost 100,000 round trips.
- `Connection::execute_batch_update` and `Connection::execute_batch` (`src/adbc/connection.rs`) already send all rows in one prepared-statement execution through `PreparedStatement::build_batch_parameters_data`, and `tests/integration_tests.rs` covers them. Their only callers are tests and `examples/prepared_statements.rs`.
- The spec gap of #67: `prepared-statements/batch-execution` requires one execution for the `Connection` methods, and the FFI scenarios in `prepared-statements/binding-and-execution` do not state a round-trip count.
- Exasol behavior, probed against the local `exasol/docker-db:latest` container (`decision-log.md` entries [2], [4], [8]). A multi-row parameter set for an INSERT runs as one statement and reports the summed row count. A multi-row parameter set for a SELECT fails with `Feature not supported: Prepared statement with multiple result tables`. A batch with one rejected row stores none of its rows. Over the WebSocket transport, a 3,000,000-row batch (80 MB of JSON) runs in one execution. Over the native transport, a batch of about 108 MB runs in one execution, and a batch of about 135 MB fails with `Broken pipe` and leaves the connection unusable.
- Exasol reports its maximum data message size at login and documents it as the "maximum size of a data message in bytes". The local container reports 67,108,864 bytes (64 MiB), which equals the crate's `MAX_DATA_MESSAGE_SIZE` fallback. `NativeTcpTransport::build_execute_prepared_payload` (`src/transport/native/mod.rs`) writes the whole parameter set as one message, and `write_parameter_rows` writes the values row by row after the column metadata.
- The prepare response carries result-set column metadata on both transports. `PreparedStatementHandle::result_columns` is empty for a statement that returns an affected-row count.
- adbc_core, adbc_ffi, and adbc_driver_manager 0.24.0 require arrow `>=58, <60` and Rust 1.85. The CI toolchain is 1.92.0, and the crate pins arrow and parquet 58.
- PR #89 (plan `fix-transport-deadlines`) changes the native and HTTP transports. Its plan leaves `src/adbc/connection.rs` and `src/adbc_ffi.rs` unchanged. The source overlap with this plan is `src/transport/native/mod.rs`: task 4.2 of PR #89 replaces the guard at the top of `execute_prepared_statement` and changes the native test module, and tasks 3.2 and 3.3 of this plan change the same function and test module. The two plans also share § Constraints of `specs/architecture.md` and `CHANGELOG.md` (`decision-log.md` entry [11]).
- The plan changes one bullet of § Constraints in `specs/architecture.md` (`architecture.md` delta, `decision-log.md` entry [11]).

## Features

| Feature | Status | Spec |
|---------|--------|------|
| FFI Statement Execution | NEW | `adbc-driver/ffi-statement-execution/spec.md` |
| Batch Execution | CHANGED | `prepared-statements/batch-execution/spec.md` |
| Binding and Execution | CHANGED | `prepared-statements/binding-and-execution/spec.md` |
| Driver Interface | CHANGED | `adbc-driver/driver-interface/spec.md` |
| Code Quality | CHANGED | `code-quality/core/spec.md` |

## Impact

- Fix: a prepared INSERT, UPDATE, DELETE, or MERGE run through ADBC `execute` with bound parameters returns no error after it wrote data, and writes every row of a bound batch. This is the Python DB-API `cursor.execute(sql, params)` path.
- Fix: ADBC `execute_update` and `execute` send all rows of a bound batch in one prepared-statement execution. Python `executemany` takes one round trip instead of one per row. A SELECT with two or more bound rows still runs once per row, because Exasol rejects a multi-row parameter set for a statement that returns rows.
- Fix: over the native protocol, a prepared batch whose parameter values exceed the maximum data message size that Exasol reports at login (64 MiB on the tested `exasol/docker-db` 2026.1.0 image) runs as consecutive executions instead of breaking the connection. This applies to ADBC `execute_update` and `execute`, Python `executemany`, and `Connection::execute_batch_update` in the Rust API.
- Changed: a bound batch whose parameter values fit in one data message takes effect for all of its rows or for none. A batch with one row that Exasol rejects, or with one value that cannot be converted, stores no row. The per-row loop used to keep the rows ahead of the failing row. A larger batch runs as several executions, and with autocommit on a failing execution leaves the rows of the earlier executions committed.
- Changed: `rowcount` after ADBC `execute` is -1 (unknown) instead of 0. `execute_update` and Python `executemany` report the affected-row count.
- Breaking (Rust users of the `ffi` feature only): the `ffi` feature depends on `adbc_core` and `adbc_ffi` 0.24. Rust code that uses `adbc_core` types together with `exarrow_rs::adbc_ffi::FfiDriver` must move to `adbc_core` 0.24. Python, Polars, and other driver manager users load the cdylib and are not affected.
- The memory use of a bound execution grows with the batch. The driver holds three converted copies of the whole batch at once: the row-major `Parameter` rows of task 2.1, the column-major JSON parameter data that `Connection::execute_batch_update` and `Connection::execute_batch` build, and the rows that the native transport encodes. A split batch also holds the data message of one range, up to the maximum data message size. A bound batch therefore needs several times its Arrow size in driver memory, and a large batch that the per-row loop completes today can exceed the available memory.
- `Cargo.toml` keeps version 0.18.0, and the changelog entries go under `## [Unreleased]`.

## Dependencies

- `adbc_core` and `adbc_ffi` 0.23.0 to 0.24.0 (optional, `ffi` feature), `adbc_core` and `adbc_driver_manager` 0.23 to 0.24 (dev). Per `code-quality/dependencies`, this minor bump needs its reason in the PR description, the full test suite, and a passing `cargo deny --all-features check advisories` (`decision-log.md` entries [5], [6]).
- New dev-dependencies `adbc_ffi` 0.24 and `libloading` 0.8, and `arrow` 58 with the `ffi` feature under `[dev-dependencies]`, for the test at the C ABI. `adbc_ffi` and `libloading` are already in `Cargo.lock` through `adbc_driver_manager`, and the `ffi` feature adds no package (`decision-log.md` entry [7]).

## Implementation Tasks

1. ADBC 0.24 (`Cargo.toml`, `Cargo.lock`, `specs/mission.md`)

- [ ] 1.1 In `Cargo.toml`, set `adbc_core` and `adbc_ffi` under `[dependencies]` to `version = "0.24.0"`, keeping `optional = true`. Set `adbc_driver_manager` and `adbc_core` under `[dev-dependencies]` to `"0.24"`, and add `adbc_ffi = "0.24"`, `libloading = "0.8"`, and `arrow = { version = "58", features = ["ffi"] }` there for task 4.2. Keep `version = "0.18.0"` (`decision-log.md` entries [5], [7]).
- [ ] 1.2 Run `cargo update -p adbc_core -p adbc_ffi -p adbc_driver_manager`. `git diff Cargo.lock` shows that `adbc_core`, `adbc_ffi`, and `adbc_driver_manager` move from 0.23.0 to 0.24.0, that `libloading` joins the dependency list of the `exarrow-rs` entry, and that no other package is added, removed, or changed. `cargo build --release --features ffi` and `cargo test --lib --features ffi` pass without a source change, because the 0.24 traits match 0.23 (entry [5]).
- [ ] 1.3 In `specs/mission.md` § Tech Stack, change the ADBC FFI row to `adbc_core / adbc_ffi 0.24 (optional)`.
- [ ] 1.4 Run `cargo deny check licenses` and `cargo deny --all-features check advisories` with `deny.toml` unchanged. Both exit 0 (entry [6]).

2. Bound execution (`src/adbc_ffi.rs`)

- [ ] 2.1 Add a private function that converts every row of a bound RecordBatch into row-major `Vec<Vec<Parameter>>` with `arrow_value_to_parameter`. It returns the `AdbcError` of the first failing value unchanged, so a conversion error keeps status `InvalidArguments` (entry [4]).
- [ ] 2.2 Rewrite `FfiStatement::execute_bound_batch_update`. Call `prepared_with_connection()` first, as today. Convert all rows with task 2.1. For zero rows, return `Ok(0)` with no execution request (entry [10]). Otherwise call `Connection::execute_batch_update(prepared, &rows)` once inside one `get_runtime().block_on` and one connection lock, and map its error with `to_adbc_error` (entry [1]).
- [ ] 2.3 Rewrite `FfiStatement::execute_bound_batch`. Call `prepared_with_connection()` first, convert all rows with task 2.1, and return no batches for zero rows with no execution request. When the prepared statement returns a result set (`handle_ref().result_columns` is not empty) and there are two or more rows, call `Connection::execute_batch` once per row with a one-row slice, in bound-row order, and collect the batches. Otherwise call `Connection::execute_batch` once with all rows. Turn each `ResultSet` into batches in one private function: a result with `row_count()` set gives no batches, and a result set gives `fetch_all()` (entries [2], [3]).
- [ ] 2.4 Rewrite the doc comments of `execute_bound_batch` and `execute_bound_batch_update`. They state that a bound batch runs as one execution when its parameter values fit in one data message, that the native transport runs a larger batch as consecutive executions, that a statement returning a result set runs once per row because Exasol rejects a multi-row parameter set for it, and that a conversion error sends nothing. Delete `bind_row_as_parameters`.
- [ ] 2.5 Add unit tests to `mod tests` in `src/adbc_ffi.rs`. Each test builds its `FfiStatement` with `FfiStatement::with_connection` on the `inner` connection of `connected_connection(transport)`, binds a RecordBatch, and sets the SQL. The mock accepts `connect`, `authenticate`, and `close`, as the tests of the `Connection options` section do. The mock answers `create_prepared_statement` with a `PreparedStatementHandle`, built with `with_result_columns` where the test needs a result-set statement. For a result-set answer, follow `single_decimal_result_set` in the test module of `src/adbc/connection.rs`. Each test carries its `/// Scenario:` line.
  - `bound_execute_update_sends_every_row_in_one_request`: a handle with 2 parameters and no result columns. `execute_prepared_statement` is expected once, with 2 columns of 3 values each, and answers `QueryResult::RowCount { count: 3 }`. `execute_update` returns `Some(3)` (Scenario: execute_update sends every bound row in one execution).
  - `bound_execute_of_a_row_count_statement_returns_no_batches`: the same mock. `execute()` succeeds, and the reader yields no batches (Scenario: execute runs a row-count statement once for the whole bound batch).
  - `bound_execute_of_a_result_set_statement_sends_one_request_per_row`: a handle with 1 parameter and one result column, and a bound batch with the values 3 and 1. `execute_prepared_statement` is expected twice, first with the single value 3 and then with the single value 1, and each call answers a one-row result set. The reader yields the two rows in that order (Scenario: execute runs a result-set statement once per bound row).
  - `zero_row_bound_batch_sends_no_execution_request`: `execute_prepared_statement` is expected `.never()`. `execute_update` returns `Some(0)`, and `execute()` returns a reader with no batches (Scenario: A zero-row bound batch runs no execution).
  - `unconvertible_bound_value_sends_no_execution_request`: a Date32 column with the values 0 and 2932897. `execute_prepared_statement` is expected `.never()`. `execute_update` and `execute()` each fail with status `InvalidArguments` (Scenario: A bound value that cannot be converted fails the batch before execution).

3. Native transport message size (`src/transport/native/mod.rs`, `docs/prepared-statements.md`)

- [ ] 3.1 Add a private associated function to `NativeTcpTransport`, next to `build_execute_prepared_payload`, that splits a parameter set into consecutive row ranges for a byte limit (`decision-log.md` entry [8]). Infer the wire type of every column once from the whole parameter set, as `write_parameter_rows` does, and use those wire types for every range. Encode the rows once and record where each row ends, because `write_parameter_rows` writes the values row by row. Add consecutive rows to a range while the full message of that range stays within the limit: the message header, the attributes, the payload prefix with the column metadata, and the row bytes. A row whose message alone exceeds the limit forms a range of its own. The payload of each range declares the row count of that range in both `total_rows` and `rows_in_msg`. The ranges cover every row once, in input order.
- [ ] 3.2 Change `execute_prepared_statement` below its authentication guard, and leave the guard unchanged. Read the limit from `max_data_message_size` of `self.session`, with `MAX_DATA_MESSAGE_SIZE` as the fallback. When the handle has no result columns and the parameter set does not fit in one message, send one `CMD_EXECUTE_PREPARED` per range of task 3.1 in order, add up the row counts, and return `QueryResult::RowCount` with the sum. Return the first error unchanged and send no later range. Return `TransportError::ProtocolError` when a range answers with anything other than a row count. Every other call keeps the single message of today. The doc comment states the split in one or two lines and names the documented maximum data message size as its reason.
- [ ] 3.3 Add unit tests to the test module of `src/transport/native/mod.rs`, next to `prepared_payload_interleaves_parameter_values_row_by_row`. Each test builds a `PreparedStatementHandle` and string parameters and calls the function of task 3.1 with a small limit computed from the single-message payload. Each test carries `/// Scenario: Batch update larger than one data message over the native protocol`.
  - `prepared_payload_ranges_keep_each_message_within_the_limit`: ten rows of 100-character strings and a limit that fits three rows. The ranges hold rows 0 to 2, 3 to 5, 6 to 8, and 9. Each message is at most the limit, each payload declares the row count of its range, and the row bytes of the ranges, joined in order, equal the row bytes of the single-message payload.
  - `prepared_payload_that_fits_forms_one_range`: the same rows and a limit above the single-message size give one range whose payload equals `build_execute_prepared_payload` for all rows.
  - `prepared_payload_row_above_the_limit_forms_its_own_range`: a 1,000-character row between two 10-character rows, with a limit that fits two 10-character rows, gives three ranges, and the middle range holds only the long row.
  - `prepared_payload_ranges_reuse_the_wire_types_of_the_whole_batch`: a handle without parameter types, so that `infer_wire_type` reads the first value of each column, and one column whose values make the first value of a later range infer a different wire type than the first value of the column, while the single-message payload still encodes. Every range payload carries the same column metadata as the single-message payload.
- [ ] 3.4 In `docs/prepared-statements.md` § Batch Execution, after the `execute_batch` example, state that `execute_batch` takes one row for a statement that returns rows, because Exasol rejects two or more rows with `Feature not supported: Prepared statement with multiple result tables`. State that over the native protocol a batch update whose parameter values exceed the server's maximum data message size (64 MiB on the tested `exasol/docker-db` 2026.1.0 image) runs as consecutive executions, and that with autocommit on a failing execution leaves the rows of the earlier executions committed.

4. Driver manager tests (`tests/driver_manager_tests.rs`)

- [ ] 4.1 Add a `// FFI Statement Execution Tests` section with two helpers: `current_statement(conn) -> i64`, which runs `SELECT CURRENT_STATEMENT` on a new statement of the same connection and reads the value with `integers_in`; and a builder for a two-column RecordBatch of `Int32` ids and `Utf8` names. Exasol's `CURRENT_STATEMENT` counts the statements of the session, and a probe showed that one execution advances it by the same amount for 1 and for 50 bound rows.
- [ ] 4.2 Add `test_execute_query_reports_unknown_rows_affected`. Create a schema and a one-column table through the driver manager connection. Load `get_library_path()` with `libloading`, resolve `AdbcDriverExasolInit`, and fill an `adbc_ffi::FFI_AdbcDriver` with `adbc_core::constants::ADBC_VERSION_1_1_0`. Through the vtable, create a database with the `uri` option `get_test_uri()`, a connection, and a statement with SQL `INSERT INTO <schema>.T VALUES (1)`. Call `StatementExecuteQuery` with an `arrow::ffi_stream::FFI_ArrowArrayStream` from `FFI_ArrowArrayStream::empty()` as the result stream and `rows_affected` set to 42, and assert status OK and `rows_affected == -1`. Release the stream, the statement, the connection, the database, and the driver. Follow `test_statement_execute_query_sets_rows_affected` from apache/arrow-adbc#4469 (entry [7]) (Scenario: ExecuteQuery with a result stream reports an unknown affected-row count).
- [ ] 4.3 Change `test_bind_execute_update` to assert that `execute_update` returns `Some(3)`, and give it the line `/// Scenario: execute_update sends every bound row in one execution`.
- [ ] 4.4 Add `test_bind_execute_update_runs_one_execution_for_any_row_count`. Prepare an INSERT into a two-column table. Read `current_statement` as c0, bind a 1-row batch and call `execute_update`, read c1, bind a 1,000-row batch and call `execute_update`, read c2. Assert `c2 - c1 == c1 - c0`, the returned counts 1 and 1,000, and 1,001 rows in the table (Scenario: execute_update sends every bound row in one execution).
- [ ] 4.5 Add `test_bind_execute_writes_every_row_of_a_dml_batch`. Prepare `INSERT INTO <schema>.BIND_TABLE (id, name) VALUES (?, ?)`, bind `bind_table_batch()`, and call `execute()`. It returns `Ok`, and the reader yields no batches. `read_bind_table` returns ids 10, 20, 30 and names alpha, beta, gamma. After that check, repeat the statement-count check of task 4.4 with `execute()` and batches of 1 and 50 rows (Scenario: execute runs a row-count statement once for the whole bound batch).
- [ ] 4.6 Give `test_bind_execute_query` the line `/// Scenario: execute runs a result-set statement once for a one-row bound batch`.
- [ ] 4.7 Add `test_bind_execute_query_runs_once_per_bound_row`. Fill `QUERY_TABLE` with (1, 100), (2, 200), and (3, 300), prepare `SELECT val FROM <schema>.QUERY_TABLE WHERE id = ?`, bind the ids 3 and 1, and call `execute()`. The reader yields the values 300 and 100 in that order (Scenario: execute runs a result-set statement once per bound row).
- [ ] 4.8 Add `test_bind_failed_batch_stores_no_row`. Create `(id INTEGER, name VARCHAR(3))`, prepare an INSERT, and bind ids 1, 2, 3 with names `a`, `TOOLONG`, `c`. `execute_update` fails with a message that contains `right truncation`, and the table holds no row. Bind the same batch again and call `execute()`: it fails the same way, and the table still holds no row (Scenario: A failed bound batch stores none of its rows).
- [ ] 4.9 Add `test_bind_unconvertible_value_stores_no_row`. Create `(id INTEGER, d DATE)`, prepare an INSERT, and bind ids 1, 2 with the Date32 values 0 and 2932897. `execute_update` and `execute()` each fail with status `InvalidArguments`, and the table holds no row (Scenario: A bound value that cannot be converted fails the batch before execution).
- [ ] 4.10 Add `test_bind_zero_row_batch_runs_no_execution`. Prepare an INSERT and bind a zero-row batch. `execute_update` returns `Some(0)`, `execute()` returns a reader with no batches, and the table holds no row (Scenario: A zero-row bound batch runs no execution).
- [ ] 4.11 Add `test_bind_batch_above_the_data_message_size_is_stored_in_full`. Create `(id INTEGER, s VARCHAR(2000))` and prepare an INSERT. Read `current_statement` as c0, run a 1-row batch with `execute_update`, and read c1. Bind 40,000 rows with ids 1 to 40,000 and a 2,000-character ASCII string each, about 80 MB of parameter values, call `execute_update`, and read c2. Assert the count 40,000 and `c2 - c1 > c1 - c0`, which shows more than one execution. Bind 40,000 more rows with new ids and call `execute()`: the reader yields no batches. The table then holds 80,001 rows, and `SELECT 1` on the same connection returns 1. The sizes assume the 64 MiB that `exasol/docker-db` reports, and a server that reports more than about 80 MB fails the statement-count assertion (Scenarios: A bound batch larger than one data message is stored in full over the native protocol; Batch update larger than one data message over the native protocol).
- [ ] 4.12 Add `test_bind_split_batch_stops_at_the_failing_execution`. With autocommit on, the ADBC default, create `(id INTEGER, s VARCHAR(2000))` and prepare an INSERT. Bind 70,000 rows with ids 0 to 69,999 and a 2,000-character string each, about 141 MB, except row 42,000, whose string has 2,001 characters. `execute_update` fails with a message that contains `right truncation`. The stored ids form one contiguous range that starts at 0 and holds at least one row, no stored id is 42,000 or higher, and `SELECT 1` on the same connection returns 1. A batch sent as one message breaks the connection instead, so the test fails without the split of task 3.2. The sizes assume the 64 MiB of task 4.11, so that row 42,000 falls outside the first execution (Scenarios: A failed execution of a split bound batch keeps the rows of the earlier executions; Batch update larger than one data message over the native protocol).
- [ ] 4.13 Every test of tasks 4.2 to 4.12 starts with `skip_if_no_library!()` and `skip_if_no_exasol!()`, uses `generate_unique_test_name` for its schema, and drops the schema with `drop_test_schema` at the end.

5. Documentation and changelog

- [ ] 5.1 In `docs/driver-manager.md`, add `### Parameters and row counts` after the `### Python (adbc-driver-manager)` subsection. Show `cursor.execute("INSERT INTO t VALUES (?, ?)", (1, "a"))` with `cursor.rowcount` -1, and `cursor.executemany(...)` with `cursor.rowcount` equal to the number of affected rows. State that a bound batch runs as one execution and stores all of its rows or none when its parameter values fit in the server's maximum data message size (64 MiB on the tested `exasol/docker-db` 2026.1.0 image), that a larger batch runs as consecutive executions and, with autocommit on, keeps the rows of the executions before a failing one, that a SELECT with several bound rows runs once per row and returns the rows in bound order, that a bound batch needs several times its Arrow size in driver memory, and that bulk ingestion (`cursor.adbc_ingest`) is the path for large loads. Replace the paragraph that sends DDL and DML to the low-level statement API: DDL and DML run through `cursor.execute` with `rowcount` -1, and the low-level `execute_update` returns the affected-row count. In the Rust `Cargo.toml` example, use `adbc_core = "0.24"`, `adbc_driver_manager = "0.24"`, and `arrow = "58"`.
- [ ] 5.2 In `CHANGELOG.md`, add these entries under the existing `## [Unreleased]` heading, after its current entries. Leave `## 0.18.0` and the `Cargo.toml` version unchanged.
  - `Fix:` a prepared INSERT, UPDATE, DELETE, or MERGE run through ADBC `execute` with bound parameters, such as Python `cursor.execute(sql, params)`, no longer fails with `Cannot fetch batches from row count result` after it wrote data, and it writes every row of a bound batch. Fixes #78.
  - `Fix:` ADBC `execute_update` and `execute` send all rows of a bound batch to Exasol in one prepared-statement execution instead of one execution per row. A SELECT with several bound rows still runs once per row, because Exasol rejects a multi-row parameter set for a statement that returns rows. Fixes #67.
  - `Fix:` over the native protocol, a prepared batch whose parameter values exceed the server's maximum data message size (64 MiB on the tested `exasol/docker-db` 2026.1.0 image) runs as consecutive executions instead of breaking the connection. This applies to `Connection::execute_batch_update`, ADBC `execute_update` and `execute`, and Python `executemany`.
  - `Changed:` a bound batch that fits in one data message stores all of its rows or none. A batch with a value that cannot be converted, or with a row that Exasol rejects, stores no row. Before, the rows ahead of the failing row stayed written. A larger batch runs as several executions, and with autocommit on a failing execution leaves the rows of the earlier executions committed.
  - `Changed:` `rowcount` after ADBC `execute` is -1 (unknown) instead of 0. `execute_update` and Python `executemany` report the affected-row count.
  - `Breaking:` the `ffi` feature uses `adbc_core` and `adbc_ffi` 0.24. Rust code that uses `adbc_core` types with the exarrow-rs FFI driver must move to `adbc_core` 0.24.

6. Verification

- [ ] 6.1 Start Exasol if it is not running (`docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`) and wait until `exapump sql 'select 1'` returns `1`.
- [ ] 6.2 Run every Checklist step. Build the release cdylib before the driver manager tests.
- [ ] 6.3 For the PR description, as `code-quality/dependencies` requires: state the reason for the adbc 0.24 bump (the -1 row count of apache/arrow-adbc#4469), that `Cargo.lock` moves `adbc_core`, `adbc_ffi`, and `adbc_driver_manager` from 0.23.0 to 0.24.0, adds `libloading` to the dependency list of the `exarrow-rs` entry, and adds, removes, or changes no other package, that the full test suite passed, and that the ADR-003 re-evaluation keeps the GHSA-2f9f-gq7v-9h6m suppression (entry [6]).

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| B: Native transport message size | 3.1-3.4 | none | spec delta `prepared-statements/batch-execution`; architecture delta `architecture.md` (§ Constraints); `src/transport/native/mod.rs` (`build_execute_prepared_payload`, `write_column_headers`, `write_parameter_rows`, `infer_wire_type`, `execute_prepared_statement`, `send_and_receive`, test module), `src/transport/native/constants.rs` (read only: `MAX_DATA_MESSAGE_SIZE`, `HEADER_SIZE`), `src/connection/auth.rs` (read only: `SessionInfo::max_data_message_size`), `docs/prepared-statements.md` |
| A: FFI bound execution and ADBC 0.24 | 1.1-2.5, 4.1-6.3 | B | spec deltas `adbc-driver/ffi-statement-execution`, `prepared-statements/binding-and-execution`, `adbc-driver/driver-interface`, `code-quality/core`; `src/adbc_ffi.rs`, `src/adbc/connection.rs` (read only: `execute_batch`, `execute_batch_update`, test helper `single_decimal_result_set`), `src/query/prepared.rs` and `src/query/results.rs` (read only), `src/transport/test_support.rs`, `tests/driver_manager_tests.rs`, `Cargo.toml`, `Cargo.lock`, `deny.toml` (read only), `docs/driver-manager.md`, `specs/mission.md`, `CHANGELOG.md` |

- Group B owns the message-size rule, its spec delta, and its unit tests in the native transport. Group A depends on B because the integration tests of tasks 4.11 and 4.12 need the split of task 3.2.
- Group A stays one group, because the bound-execution code and its tests live in `src/adbc_ffi.rs` and `tests/driver_manager_tests.rs`, and the test of task 4.2 needs the dev-dependencies of task 1.1.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/adbc_ffi.rs` `bind_row_as_parameters` | Replaced by the conversion of task 2.1; the FFI path no longer binds parameters one row at a time |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| execute_update sends every bound row in one execution | Integration | `tests/driver_manager_tests.rs` | `test_bind_execute_update` |
| execute_update sends every bound row in one execution | Integration | `tests/driver_manager_tests.rs` | `test_bind_execute_update_runs_one_execution_for_any_row_count` |
| execute_update sends every bound row in one execution | Unit | `src/adbc_ffi.rs` | `bound_execute_update_sends_every_row_in_one_request` |
| execute runs a row-count statement once for the whole bound batch | Integration | `tests/driver_manager_tests.rs` | `test_bind_execute_writes_every_row_of_a_dml_batch` |
| execute runs a row-count statement once for the whole bound batch | Unit | `src/adbc_ffi.rs` | `bound_execute_of_a_row_count_statement_returns_no_batches` |
| execute runs a result-set statement once for a one-row bound batch | Integration | `tests/driver_manager_tests.rs` | `test_bind_execute_query` |
| execute runs a result-set statement once per bound row | Integration | `tests/driver_manager_tests.rs` | `test_bind_execute_query_runs_once_per_bound_row` |
| execute runs a result-set statement once per bound row | Unit | `src/adbc_ffi.rs` | `bound_execute_of_a_result_set_statement_sends_one_request_per_row` |
| A failed bound batch stores none of its rows | Integration | `tests/driver_manager_tests.rs` | `test_bind_failed_batch_stores_no_row` |
| A bound batch larger than one data message is stored in full over the native protocol | Integration | `tests/driver_manager_tests.rs` | `test_bind_batch_above_the_data_message_size_is_stored_in_full` |
| A failed execution of a split bound batch keeps the rows of the earlier executions | Integration | `tests/driver_manager_tests.rs` | `test_bind_split_batch_stops_at_the_failing_execution` |
| A bound value that cannot be converted fails the batch before execution | Integration | `tests/driver_manager_tests.rs` | `test_bind_unconvertible_value_stores_no_row` |
| A bound value that cannot be converted fails the batch before execution | Unit | `src/adbc_ffi.rs` | `unconvertible_bound_value_sends_no_execution_request` |
| A zero-row bound batch runs no execution | Integration | `tests/driver_manager_tests.rs` | `test_bind_zero_row_batch_runs_no_execution` |
| A zero-row bound batch runs no execution | Unit | `src/adbc_ffi.rs` | `zero_row_bound_batch_sends_no_execution_request` |
| ExecuteQuery with a result stream reports an unknown affected-row count | Integration | `tests/driver_manager_tests.rs` | `test_execute_query_reports_unknown_rows_affected` |
| Batch update execution with affected row count | Integration | `tests/integration_tests.rs` | `test_execute_batch_update` |
| Batch update larger than one data message over the native protocol | Unit | `src/transport/native/mod.rs` | `prepared_payload_ranges_keep_each_message_within_the_limit` |
| Batch update larger than one data message over the native protocol | Unit | `src/transport/native/mod.rs` | `prepared_payload_that_fits_forms_one_range` |
| Batch update larger than one data message over the native protocol | Unit | `src/transport/native/mod.rs` | `prepared_payload_row_above_the_limit_forms_its_own_range` |
| Batch update larger than one data message over the native protocol | Unit | `src/transport/native/mod.rs` | `prepared_payload_ranges_reuse_the_wire_types_of_the_whole_batch` |
| Batch update larger than one data message over the native protocol | Integration | `tests/driver_manager_tests.rs` | `test_bind_batch_above_the_data_message_size_is_stored_in_full` |
| Batch update larger than one data message over the native protocol | Integration | `tests/driver_manager_tests.rs` | `test_bind_split_batch_stops_at_the_failing_execution` |
| Batch query execution returning a result set | Integration | `tests/integration_tests.rs` | `test_execute_batch_select_single_row` |
| Driver registration | Integration | `tests/driver_manager_tests.rs` | `test_driver_manager_loads_driver` |
| Arrow and Parquet dependencies resolve to version 58 or above with no duplicate sub-crate versions | Integration | `tests/integration_tests.rs` | `test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates` |

- The three scenarios that `prepared-statements/binding-and-execution` removes are restated in `adbc-driver/ffi-statement-execution` and covered above.
- The unit tests prove the absence or the count of execution requests and the size of each data message, which the database cannot show directly. The integration tests prove the outcome in Exasol.
- The driver manager tests reach `Connection::execute_batch_update` and the native transport through the FFI path, so they also cover the `prepared-statements/batch-execution` scenario for a batch larger than one data message. This plan adds no test to `tests/integration_tests.rs` (`decision-log.md` entry [11]).

### Manual Testing

The Python commands need `pip install adbc-driver-manager pyarrow`, a running Exasol, and `cargo build --release --features ffi`. Run them in one shell after `CONNECT="import time, adbc_driver_manager.dbapi as d, pyarrow as pa; c = d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://sys:exasol@localhost:8563?validateservercertificate=0'}); cur = c.cursor()"`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| FFI Statement Execution | `python3 -c "$CONNECT; cur.execute('CREATE SCHEMA IF NOT EXISTS ZZ_B4'); cur.execute('CREATE OR REPLACE TABLE ZZ_B4.T(a INT, b VARCHAR(10))'); cur.execute('INSERT INTO ZZ_B4.T VALUES (?, ?)', (1, 'a')); cur.execute('INSERT INTO ZZ_B4.T VALUES (?, ?)', pa.record_batch([pa.array([2, 3, 4]), pa.array(['b', 'c', 'd'])], names=['a', 'b'])); print(cur.rowcount); cur.execute('SELECT a FROM ZZ_B4.T ORDER BY a'); print(cur.fetchall()); c.commit()"` | Prints `-1`, then `[(1,), (2,), (3,), (4,)]`, with no exception |
| FFI Statement Execution | `python3 -c "$CONNECT; cur.execute('CREATE OR REPLACE TABLE ZZ_B4.U(a INT)'); t = time.time(); cur.executemany('INSERT INTO ZZ_B4.U VALUES (?)', [(i,) for i in range(100000)]); print(cur.rowcount, round(time.time() - t, 1)); c.commit()"` | Prints `100000` and a duration of a few seconds. The per-row loop needed 100,000 round trips |
| FFI Statement Execution, Batch Execution | `python3 -c "$CONNECT; cur.execute('CREATE OR REPLACE TABLE ZZ_B4.W(a INT, s VARCHAR(2000))'); cur.executemany('INSERT INTO ZZ_B4.W VALUES (?, ?)', [(i, 'x' * 2000) for i in range(40000)]); print(cur.rowcount); cur.execute('SELECT COUNT(*) FROM ZZ_B4.W'); print(cur.fetchone()); c.commit()"` | Prints `40000`, then `(40000,)`: the batch of about 80 MB ran as consecutive executions, and the connection stayed usable |
| FFI Statement Execution | `python3 -c "$CONNECT; cur.execute('CREATE OR REPLACE TABLE ZZ_B4.V(a INT, b VARCHAR(3))'); c.commit(); cur.executemany('INSERT INTO ZZ_B4.V VALUES (?, ?)', [(1, 'a'), (2, 'TOOLONG'), (3, 'c')])"` | Raises an exception whose message contains `right truncation` |
| FFI Statement Execution | `python3 -c "$CONNECT; cur.execute('SELECT COUNT(*) FROM ZZ_B4.V'); print(cur.fetchone()); cur.execute('DROP SCHEMA ZZ_B4 CASCADE'); c.commit()"` | Prints `(0,)`: the failed batch stored no row |
| Driver Interface | `cargo tree -i adbc_ffi --features ffi --depth 0` | Prints `adbc_ffi v0.24.0` |
| Binding and Execution | `cargo run --example prepared_statements` against the running Exasol | Runs a single-row insert, a batch insert, and a one-row batch select, and exits 0 |
| Code Quality | `cargo tree -d --features ffi -e normal \| grep -E '^arrow-(array\|schema)'` | Prints nothing: no duplicate `arrow-array` or `arrow-schema` version |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Format | `cargo fmt --all -- --check` | No changes |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Build | `cargo build` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Build (WebSocket only) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit test | `cargo test --lib` | 0 failures |
| Unit test (FFI) | `cargo test --lib --features ffi` | 0 failures |
| Unit test (WebSocket) | `cargo test --lib --features websocket` | 0 failures |
| CI guard | `python3 scripts/check_ci_test_targets.py` | Exit 0 |
| Integration test | `REQUIRE_EXASOL=1 cargo test --test integration_tests -- --test-threads=1` | 0 failures, no skips |
| Driver manager test | `REQUIRE_EXASOL=1 cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures, no skips, run after the FFI build |
| Import/export test | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Licenses | `cargo deny check licenses` | Exit 0 |
| Advisories | `cargo deny --all-features check advisories` | Exit 0, with `deny.toml` unchanged |
| Lockfile | `git diff main -- Cargo.lock` | `adbc_core`, `adbc_ffi`, and `adbc_driver_manager` move from 0.23.0 to 0.24.0, `libloading` joins the dependency list of the `exarrow-rs` entry, and no other package is added, removed, or changed |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Check passes: total production coverage at least 80%, every file at least 50% |
| Changelog | `git diff main -- CHANGELOG.md Cargo.toml` | New entries under `## [Unreleased]`, no change to `## 0.18.0` or to the `Cargo.toml` version |
