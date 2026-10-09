# Verification Report: fix-ffi-bound-execute

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All checklist steps pass on the final code. Prepared DML through `execute` writes every bound row without an error, a bound batch runs in one execution, and `rowcount` after `execute` reads -1. |
| Code review | 13 findings, 13 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | 87.87 production lines (floor 80.0, check passes) |
| Integration | Not measured, per AGENTS.md (the `ffi` feature deadlocks under cargo-llvm-cov) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`--lib`) | 1644 | 1644 | 0 |
| Unit (`--lib --features ffi`) | 1754 | 1754 | 0 |
| Unit (`--lib --features websocket`) | 1680 | 1680 | 0 |
| Integration | 81 | 80 | 1 |
| Driver manager (`--include-ignored`) | 59 | 59 | 0 |
| Import/export | 62 | 61 | 1 |
| Native protocol | 12 | 12 | 0 |

The ignored tests are reasoned `#[ignore]` attributes that predate this change: the Exasol session reap latency measurement in `tests/integration_tests.rs` and the eight-minute export check in `tests/import_export_tests.rs`.

### Manual Tests

| Test | Result |
|------|--------|
| `cursor.execute` with bound parameters inserts the row, and `rowcount` prints -1 | ✓ |
| `executemany` of 100,000 rows takes one execution and prints 100000 | ✓ |
| `executemany` of 40,000 rows (about 80 MB) is stored in full | ✓ |
| A failing batch with autocommit on raises an error and leaves the count at 0 | ✓ |
| `cargo run --example prepared_statements` exits 0 | ✓ |

The implementer ran the manual checks with adbc-driver-manager 1.12.0 and pyarrow 26.0.0 in a throwaway virtual environment. They ran against the cdylib built before the review fixes. The driver manager tests rebuilt the cdylib and passed after the review fixes.

## Tool Evidence

### Linter

```
cargo +1.92 clippy --all-targets --all-features -- -W clippy::all: exit 0, 0 warnings
cargo deny check licenses: exit 0
cargo deny --all-features check advisories: exit 0 (deny.toml unchanged)
```

### Formatter

```
cargo +1.92 fmt --all -- --check: exit 0
```

### Lockfile and version

```
git diff main -- Cargo.lock: adbc_core, adbc_ffi, adbc_driver_manager 0.23.0 to 0.24.0, libloading added to the exarrow-rs entry, nothing else
Cargo.toml package version: 0.19.0 (unchanged)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| adbc-driver | ffi-statement-execution | execute_update sends every bound row in one execution | `tests/driver_manager_tests.rs` | `test_bind_execute_update` | Pass |
| adbc-driver | ffi-statement-execution | execute_update sends every bound row in one execution | `tests/driver_manager_tests.rs` | `test_bind_execute_update_runs_one_execution_for_any_row_count` | Pass |
| adbc-driver | ffi-statement-execution | execute_update sends every bound row in one execution | `src/adbc_ffi.rs` | `bound_execute_update_sends_every_row_in_one_request` | Pass |
| adbc-driver | ffi-statement-execution | execute runs a row-count statement once for the whole bound batch | `tests/driver_manager_tests.rs` | `test_bind_execute_writes_every_row_of_a_dml_batch` | Pass |
| adbc-driver | ffi-statement-execution | execute runs a row-count statement once for the whole bound batch | `src/adbc_ffi.rs` | `bound_execute_of_a_row_count_statement_returns_no_batches` | Pass |
| adbc-driver | ffi-statement-execution | execute runs a result-set statement once per bound row | `tests/driver_manager_tests.rs` | `test_bind_execute_query` | Pass |
| adbc-driver | ffi-statement-execution | execute runs a result-set statement once per bound row | `tests/driver_manager_tests.rs` | `test_bind_execute_query_runs_once_per_bound_row` | Pass |
| adbc-driver | ffi-statement-execution | execute runs a result-set statement once per bound row | `src/adbc_ffi.rs` | `bound_execute_of_a_result_set_statement_sends_one_request_per_row` | Pass |
| adbc-driver | ffi-statement-execution | A failed bound batch stores none of its rows | `tests/driver_manager_tests.rs` | `test_bind_failed_batch_stores_no_row` | Pass |
| adbc-driver | ffi-statement-execution | A bound batch larger than one data message is stored in full over the native protocol | `tests/driver_manager_tests.rs` | `test_bind_batch_above_the_data_message_size_is_stored_in_full` | Pass |
| adbc-driver | ffi-statement-execution | A failed execution of a split bound batch keeps the rows of the earlier executions | `tests/driver_manager_tests.rs` | `test_bind_split_batch_stops_at_the_failing_execution` | Pass |
| adbc-driver | ffi-statement-execution | A bound value that cannot be converted fails the batch before execution | `tests/driver_manager_tests.rs` | `test_bind_unconvertible_value_stores_no_row` | Pass |
| adbc-driver | ffi-statement-execution | A bound value that cannot be converted fails the batch before execution | `src/adbc_ffi.rs` | `unconvertible_bound_value_sends_no_execution_request` | Pass |
| adbc-driver | ffi-statement-execution | A bound batch with the wrong column count fails before execution | `tests/driver_manager_tests.rs` | `test_bind_wrong_column_count_stores_no_row` | Pass |
| adbc-driver | ffi-statement-execution | A bound batch with the wrong column count fails before execution | `src/adbc_ffi.rs` | `bound_batch_with_wrong_column_count_sends_no_execution_request` | Pass |
| adbc-driver | ffi-statement-execution | A zero-row bound batch runs no execution | `tests/driver_manager_tests.rs` | `test_bind_zero_row_batch_runs_no_execution` | Pass |
| adbc-driver | ffi-statement-execution | A zero-row bound batch runs no execution | `src/adbc_ffi.rs` | `zero_row_bound_batch_sends_no_execution_request` | Pass |
| adbc-driver | ffi-statement-execution | ExecuteQuery with a result stream reports an unknown affected-row count | `tests/driver_manager_tests.rs` | `test_execute_query_reports_unknown_rows_affected` | Pass |
| prepared-statements | batch-execution | Batch update execution with affected row count | `tests/integration_tests.rs` | `test_execute_batch_update` | Pass |
| prepared-statements | batch-execution | Batch update larger than one data message over the native protocol | `src/transport/native/mod.rs` | `prepared_payload_ranges_keep_each_message_within_the_limit` | Pass |
| prepared-statements | batch-execution | Batch update larger than one data message over the native protocol | `src/transport/native/mod.rs` | `prepared_payload_that_fits_forms_one_range` | Pass |
| prepared-statements | batch-execution | Batch update larger than one data message over the native protocol | `src/transport/native/mod.rs` | `prepared_payload_row_above_the_limit_forms_its_own_range` | Pass |
| prepared-statements | batch-execution | Batch update larger than one data message over the native protocol | `src/transport/native/mod.rs` | `prepared_payload_ranges_reuse_the_wire_types_of_the_whole_batch` | Pass |
| prepared-statements | batch-execution | Batch update larger than one data message over the native protocol | `tests/driver_manager_tests.rs` | `test_bind_batch_above_the_data_message_size_is_stored_in_full` | Pass |
| prepared-statements | batch-execution | Batch update larger than one data message over the native protocol | `tests/driver_manager_tests.rs` | `test_bind_split_batch_stops_at_the_failing_execution` | Pass |
| prepared-statements | batch-execution | Batch query execution returning a result set | `tests/integration_tests.rs` | `test_execute_batch_select_single_row` | Pass |
| adbc-driver | driver-interface | Driver registration | `tests/driver_manager_tests.rs` | `test_driver_manager_loads_driver` | Pass |
| code-quality | core | Arrow and Parquet dependencies resolve to version 58 or above with no duplicate sub-crate versions | `tests/integration_tests.rs` | `test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates` | Pass |

## Notes

- The wrong-column-count and unconvertible-value cases report `InvalidArguments`. A failing batch that Exasol rejects reaches Python as `InternalError`, because the Exasol error maps to status `Internal`.
- The test of the -1 row count had no RED run. The cdylib used for the RED runs already carried adbc_ffi 0.24, so a failing run against 0.23 would need a rebuild on the old dependency. The other new integration tests failed against the per-row loop before the rewrite.
- The tests above the 64 MiB message size assume that the server reports 64 MiB. The implementer verified this on the `exasol/docker-db` 2026.1.0 image and the plan's review verified it on the 2025.2.1 image.
- The Python manual checks and the split behavior above 64 MiB were not run over the WebSocket protocol. WebSocket executes a batch update as one execution, as in decision-log entry [8].
