# Verification Report: add-streaming-result-sets

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All 21 tasks are done. Build, unit, integration, driver manager, lint, format, coverage, and the manual runs pass. |
| Code review | 7 findings (6 standard, 1 expert) — 7 fixed |

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
| Unit (production code only, `cargo llvm-cov --lib --features parquet`) | 91.47 (floor 80, per-file floor 50: met) |
| Integration | not measured (CI measures unit coverage only) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit | `--features parquet` | 1804 | 0 |
| Unit | `--features websocket` | 1671 | 0 |
| Unit | `--features ffi` | 1766 | 0 |
| Integration | native, `integration_tests` | 102 | 1 (existing reap test) |
| Integration | `websocket_integration_tests` | 77 | 0 |
| Integration | `driver_manager_tests` (`--include-ignored`) | 92 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `cargo run --example basic_usage` prints `Streamed 70000 rows in 3 batches`, then `Done` | ✓ |
| `cargo run --example driver_manager_usage` prints `Schema before first read: V, S`, then `Streamed 70000 rows in 3 batches` | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings                       exit 0
cargo clippy --all-targets --features ffi -- -D warnings                       exit 0
cargo clippy --all-targets --no-default-features --features websocket -- -D warnings   exit 0
scripts/check_ci_test_targets.py                                               exit 0
cargo test --no-default-features --features websocket --tests --no-run         exit 0
```

### Formatter

```
cargo fmt --all -- --check    exit 0
```

## Scenario Coverage

The 49 test names that the plan's Scenario Coverage table lists all exist in `src/` or `tests/`, and each passed in the runs above. The table's `test_ws_` entry is a name prefix for the WebSocket counterparts, not a test.

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| query-execution | result-sets | Result set iterator fetches one batch per call | `src/query/results.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_next_batch_serves_execute_response_then_one_fetch_per_call`, `test_iterator_reads_multi_fetch_result_on_current_thread_runtime` | Pass |
| query-execution | result-sets | Result set iterator keeps no batch that it has yielded | same | `test_next_batch_keeps_no_yielded_batch`, `test_iterator_keeps_no_yielded_batch` | Pass |
| query-execution | result-sets | Result set iterator reads inside an async runtime | integration targets | `test_iterator_reads_multi_fetch_result_on_multi_thread_runtime` and the current-thread test | Pass |
| query-execution | result-sets | Result set iterator closes the result set handle when it ends | `src/query/results.rs`, integration targets | `test_next_batch_closes_handle_at_end_and_ignores_close_error`, `test_iterator_end_closes_handle_and_connection_stays_usable` | Pass |
| query-execution | result-sets | Result set iterator closed before its end releases the result set handle | `src/query/results.rs`, integration targets | `test_close_before_end_closes_handle_without_fetch`, `test_iterator_close_before_end_keeps_connection_usable` | Pass |
| query-execution | result-sets | Result set request dropped before its response terminates the connection | `src/query/results.rs`, integration targets | `test_dropped_next_batch_during_fetch_terminates_transport`, `test_dropped_next_batch_terminates_connection`, `test_ws_dropped_next_batch_terminates_connection` | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `src/transport/lifecycle.rs`, integration targets | `every_guard_names_the_termination_on_a_terminated_transport`, `test_csv_export_explicit_timeout_terminates_connection` | Pass |
| adbc-driver | ffi-statement-execution | execute streams a query result one batch per read | `src/adbc_ffi.rs`, `tests/driver_manager_tests.rs` | `streaming_reader_fetches_nothing_on_execute_and_once_per_read`, `test_execute_streams_multi_fetch_result` | Pass |
| adbc-driver | ffi-statement-execution | A streaming reader closes the result set handle at its end | same | `streaming_reader_closes_handle_at_end_and_drop_sends_nothing`, `test_streaming_reader_end_keeps_connection_usable` | Pass |
| adbc-driver | ffi-statement-execution | Releasing a streaming reader before its end closes the result set handle | same | `releasing_streaming_reader_before_end_closes_handle_without_fetch`, `test_releasing_streaming_reader_before_end_keeps_connection_usable` | Pass |
| adbc-driver | ffi-statement-execution | A failed fetch fails the read of a streaming reader | same | `streaming_reader_read_fails_with_fetch_error_message`, `test_streaming_reader_read_fails_after_session_is_killed` | Pass |
| adbc-driver | result-schema | Reader schema is available before the first read | same | `streaming_reader_schema_is_set_before_the_first_read`, `test_reader_schema_is_available_before_the_first_read` | Pass |

## Notes

- The first run of `websocket_integration_tests` failed 3 tests (`test_ws_every_fetched_batch_carries_the_result_set_field_metadata`, `test_ws_every_iterated_batch_carries_the_result_set_field_metadata`, `test_ws_fetch_all_multi_fetch_returns_every_row_once`). Each failed at execute with a closed connection. They passed on a freshly created container, and the native, driver manager, and WebSocket suites then passed in sequence on one container. The cause is most likely database memory held by sessions that earlier runs of the dropped-fetch tests left behind; the cause was not confirmed.
- On the local `exasol/docker-db` 2026.1.0, a client that disconnects while a fetch is pending leaves the server session behind. Its process and temp RAM stay until the database restarts, and `KILL SESSION` fails. The two dropped-`next_batch` integration tests end their sessions with `KILL SESSION` from a second connection. CI uses 2025.2.1, which was not tested for this behavior.
- Exasol answers a large query's execute call with no rows, so the FFI reader's schema comes from the type mapping, and later native batches carry the decoded types. The reader checks each fetched batch against its schema and fails the read on a difference. No tested type differs.
- Running another statement on the same connection before a streaming reader is drained is not specified and not tested. `docs/driver-manager.md` tells callers to finish or release a reader first.
- `cargo doc -D rustdoc::broken_intra_doc_links` fails on two `[col_idx]` links in `src/transport/messages.rs`. That file is unchanged.
- One doctest in `src/transport/mod.rs` fails with only `--features parquet`. That file is unchanged.
