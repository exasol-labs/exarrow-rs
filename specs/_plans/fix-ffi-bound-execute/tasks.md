# Tasks: fix-ffi-bound-execute

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [x] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group B)
- [x] 3.1 Add two private associated functions to `NativeTcpTransport` in `src/transport/native/mod.rs`, next to `build_ (plan.md task 3.1)
- [x] 3.2 Change `execute_prepared_statement` below its guard `self.lifecycle.require(ConnectionState::Authenticated, .. (plan.md task 3.2)
- [x] 3.3 Add unit tests to the test module of `src/transport/native/mod.rs`, next to `prepared_payload_interleaves_para (plan.md task 3.3)
- [x] 3.4 In `docs/prepared-statements.md` § Batch Execution, after the `execute_batch` example, state that `execute_bat (plan.md task 3.4)

## Phase 2: Implementation (Group A)
- [x] 1.1 In `Cargo.toml`, set `adbc_core` and `adbc_ffi` under `[dependencies]` to `version = "0.24.0"`, keeping `optio (plan.md task 1.1)
- [x] 1.2 Run `cargo update -p adbc_core -p adbc_ffi -p adbc_driver_manager`. (plan.md task 1.2)
- [x] 1.3 In `specs/mission.md` § Tech Stack, change the ADBC FFI row to `adbc_core / adbc_ffi 0.24 (optional)`. (plan.md task 1.3)
- [x] 1.4 Run `cargo deny check licenses` and `cargo deny --all-features check advisories` with `deny.toml` unchanged. (plan.md task 1.4)
- [x] 2.1 Add a private function that converts every row of a bound RecordBatch into row-major `Vec<Vec<Parameter>>` wit (plan.md task 2.1)
- [x] 2.2 Rewrite `FfiStatement::execute_bound_batch_update`. (plan.md task 2.2)
- [x] 2.3 Rewrite `FfiStatement::execute_bound_batch`. (plan.md task 2.3)
- [x] 2.4 Rewrite the doc comments of `execute_bound_batch` and `execute_bound_batch_update`. (plan.md task 2.4)
- [x] 2.5 Add unit tests to `mod tests` in `src/adbc_ffi.rs`. (plan.md task 2.5)
- [x] 4.1 Add a `// FFI Statement Execution Tests` section with two helpers: (plan.md task 4.1)
- [x] 4.2 Add `test_execute_query_reports_unknown_rows_affected`. (plan.md task 4.2)
- [x] 4.3 Change `test_bind_execute_update` to assert that `execute_update` returns `Some(3)`, and give it the line `/// (plan.md task 4.3)
- [x] 4.4 Add `test_bind_execute_update_runs_one_execution_for_any_row_count`. (plan.md task 4.4)
- [x] 4.5 Add `test_bind_execute_writes_every_row_of_a_dml_batch`. (plan.md task 4.5)
- [x] 4.6 Give `test_bind_execute_query`, which binds one row, the line `/// Scenario: (plan.md task 4.6)
- [x] 4.7 Add `test_bind_execute_query_runs_once_per_bound_row`. (plan.md task 4.7)
- [x] 4.8 Add `test_bind_failed_batch_stores_no_row`. (plan.md task 4.8)
- [x] 4.9 Add `test_bind_unconvertible_value_stores_no_row`. (plan.md task 4.9)
- [x] 4.10 Add `test_bind_zero_row_batch_runs_no_execution`. (plan.md task 4.10)
- [x] 4.11 Add `test_bind_batch_above_the_data_message_size_is_stored_in_full`. (plan.md task 4.11)
- [x] 4.12 Add `test_bind_split_batch_stops_at_the_failing_execution`. (plan.md task 4.12)
- [x] 4.13 Add `test_bind_wrong_column_count_stores_no_row`. (plan.md task 4.13)
- [x] 4.14 Every test of tasks 4.2 to 4.13 starts with `skip_if_no_library!()` and `skip_if_no_exasol!()`, uses `generate (plan.md task 4.14)
- [x] 4.15 In `tests/integration_tests.rs`, add the line `/// Scenario: (plan.md task 4.15)
- [x] 5.1 In `docs/driver-manager.md`, add `### Parameters and row counts` after the `### Python (adbc-driver-manager)` (plan.md task 5.1)
- [x] 5.2 In `CHANGELOG.md`, add a `## [Unreleased]` heading between `# Changelog` and `## 0.19.0`, and put these entrie (plan.md task 5.2)

## Phase 3: Verification
- [x] 6.1 Start Exasol if it is not running (`docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-d (plan.md task 6.1)
- [x] 6.2 Run every Checklist step. (plan.md task 6.2)
- [x] 6.3 For the PR description, as `code-quality/dependencies` requires: (plan.md task 6.3)

## Phase 4: Review Fixes
- [x] 4.1 In `src/transport/native/mod.rs`, add private `NativeTcpTransport::execute_prepared_attributes() -> AttributeSet` and use it in `run_execute_prepared`, `split_parameter_rows`, and the test module's `message_size`; delete `NO_ATTRIBUTE_BYTES`; compute `limit` in two split tests from `message_size(&single[..prefix_len])`.
- [x] 4.2 In `execute_prepared_statement`, replace `.unwrap_or(0)` with `.unwrap_or(MAX_DATA_MESSAGE_SIZE as usize)`.
- [x] 4.3 Extract the range loop of `execute_prepared_statement` into private `execute_prepared_ranges` and rewrite the body below the guard as the two-way choice.
- [x] 4.4 Replace `write_payload_prefix` with `payload_prefix(handle, columns, num_rows) -> Vec<u8>` and update `build_execute_prepared_payload`, `build_range_payload`, and `split_parameter_rows`.
- [x] 4.5 Add constants `PARAMETER_TABLE_COUNT` and `IS_TABLE` and use them in `payload_prefix`; delete the two inline comments.
- [x] 4.6 Change the `build_execute_prepared_payload` doc line to `/// For each row, for each column: [null_marker:1] [value (type-specific)]`.
- [x] 4.7 Replace the history-narrating tail of the `write_parameter_rows` doc comment with `/// Column-major encoding causes the server to drop the connection for num_rows > 1.`
- [x] 4.8 In the split tests, add `TOTAL_ROWS_BYTES`, `ROWS_IN_MSG_BYTES`, `FIXED_PREFIX_LEN`, `DECIMAL_VALUE_LEN`, `string_value_len` and replace the bare offsets and sizes and `ROW_100`; delete the `Bytes 29..` comment.
- [x] 4.9 Add test `prepared_payload_without_rows_forms_no_range` asserting an empty `encoded.ranges`.
- [x] 4.10 In `src/adbc_ffi.rs`, make `FfiStatement::prepared_with_connection` return `&PreparedStatement` via `as_ref()` and delete the two `let prepared = &*prepared;` lines.
- [x] 4.11 In `tests/driver_manager_tests.rs`, split the two `/// Scenarios:` lines into one `/// Scenario:` line per scenario.
- [x] 4.12 Correct the `prepare_insert` doc comment.
- [x] 4.13 Add `WIDE_VALUE_CHARS` and `FAILING_ROW` constants and use them in `wide_batch`, the column definitions, the split test, the `expect_err` message, and the stored-id assertion.
