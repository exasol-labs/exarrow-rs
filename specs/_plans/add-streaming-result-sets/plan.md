# Plan: add-streaming-result-sets

## Summary

Make query results stream one batch at a time with bounded memory: replace the blocking `ResultSetIterator` with an async `next_batch` that keeps no yielded batch and closes its handle at the end, and return a streaming RecordBatchReader from ADBC FFI `execute` for SQL with no bound RecordBatch. Source: GitHub issue exasol-labs/exarrow-rs#66.

## Context

- `ResultSetIterator::next_batch` calls `Handle::try_current().block_on`. That call panics when async code on a Tokio runtime calls it, and outside any runtime the iterator returns an `InvalidState` error. Only a synchronous caller that has entered a runtime, as `tests/common/mod.rs` `drain_iterator` does, can fetch.
- The iterator pushes a clone of every yielded batch into its `batches` list, so its memory grows with the result. It has no production caller.
- The iterator never closes its result set handle, while `fetch_all` closes it and `native-client/result-sets` "Large result set (multi-fetch)" requires the close after all rows are fetched. Each transport keeps a fetch position per handle until `close_result_set` removes it.
- `Connection::query` and FFI `FfiStatement::query_sql` (`src/adbc_ffi.rs`) both call `fetch_all`, and FFI `execute` wraps the collected batches in `VecRecordBatchReader`. A driver-manager caller therefore holds the whole result in memory before it reads the first batch.
- `ResultSet::from_transport_result` always stores exactly one batch from the execute response, and a zero-row batch built from the column metadata when no rows arrive (ADR `zero-row-result-sets-carry-column-schema`). A reader can take its schema from that batch without a fetch.
- The FFI runtime is a process-global multi-thread Tokio runtime with 2 workers. Every FFI entry point blocks the calling thread on it with `block_on`.
- `src/export/arrow.rs` already exposes the same shape for export results: `async fn next_batch(&mut self) -> Result<Option<RecordBatch>, ExportError>`.
- ADR `client-give-up-terminates-connection` requires that a client that gives up on an in-flight request terminates the connection. A caller can drop an async `next_batch` future in the middle of a fetch, for example through `tokio::time::timeout` or `tokio::select!`, which the blocking iterator did not allow. The export timeout already conforms: `src/export/csv.rs` calls `TransportProtocol::terminate()`. The terminated-transport error text in `src/transport/lifecycle.rs` and the `connection-management/session-and-lifecycle` scenario "Operations after an export timeout name the termination" name only the export as the cause.
- Issue #66 names `adbc-driver/statement-and-results` as silent on streaming. This plan specifies streaming where result reading lives: the Rust API in `query-execution/result-sets`, and the C ABI reader in `adbc-driver/ffi-statement-execution` and `adbc-driver/result-schema`.
- The plan changes the Data Flow and Constraints sections of `specs/architecture.md`: see `architecture.md` in this plan directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Result Sets | CHANGED | `query-execution/result-sets/spec.md` |
| FFI Statement Execution | CHANGED | `adbc-driver/ffi-statement-execution/spec.md` |
| Result Schema | CHANGED | `adbc-driver/result-schema/spec.md` |
| Session and Lifecycle | CHANGED | `connection-management/session-and-lifecycle/spec.md` |

## Impact

- Breaking (Rust API): `ResultSetIterator::next_batch` becomes `async fn next_batch(&mut self) -> Result<Option<RecordBatch>, QueryError>`, and `ResultSetIterator` no longer implements `Iterator`. Code that writes `for batch in result_set.into_iterator()?` must change to `while let Some(batch) = iterator.next_batch().await?`. The version moves from 0.32.0 to 0.33.0, and `CHANGELOG.md` gets a `## 0.33.0` entry.
- The iterator now closes the result set handle at the end of the result set. A Rust caller that stops early awaits `close`. Dropping the iterator before its end leaves the handle open until the session ends, as dropping a `ResultSet` does today.
- A Rust caller that drops a `next_batch`, `close`, or `fetch_all` future after its fetch or close request has started, for example through `tokio::time::timeout`, now terminates the connection. Every later operation on that connection except `close` fails with the terminated-transport error. Before this plan, such a drop left the transport out of step with the server, and the next statement read the stale response.
- The terminated-transport error text changes from `Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation` to `Transport was terminated because an in-flight response was abandoned; reconnect before the next operation`. A caller that matches the old text must update it.
- Driver-manager callers (Python, Polars, Go, Java): `execute` of SQL with no bound RecordBatch returns a reader that fetches one batch per read, so peak memory no longer grows with the result size. Releasing the reader early closes the result set handle. The reader yields the same rows and schema as before.
- A driver-manager caller that runs another statement on the same connection before it reads a streaming reader to its end now interleaves that statement with the reader's fetches. Each transport keeps a fetch position per handle, but the `fetch` and `closeResultSet` pages of the Exasol WebSocket API documentation do not state whether a handle stays valid while the session runs another statement, and this plan does not specify that case.
- Unchanged: `Connection::query` and `fetch_all` return every batch in memory. FFI `execute` with a bound RecordBatch, bulk ingest, and metadata readers such as GetObjects keep their in-memory readers.

## Implementation Tasks

1. Async result set iterator (Group A)
   - [ ] 1.1 Write failing unit tests in `src/query/results.rs` for the six new Result Sets scenarios, with `MockTransport` and `#[tokio::test]` on a current-thread and a multi-thread runtime: the first `next_batch` sends no fetch and later calls send one fetch each; after the caller drops a yielded batch, `Arc::strong_count` of each of its column arrays is 1; the handle is closed at the end and after a row count error, a failed close is ignored, and later `next_batch` and `close` calls send no request; `close` before the end sends `close_result_set` and no fetch. For the dropped-request scenario, add constructors to `GatedQueryTransport` in `src/transport/test_support.rs` that keep `fetch_results` or `close_result_set` pending, as `stalled` keeps `execute_query` pending; `terminate` already delegates to the inner `MockTransport`. With that double, a `next_batch`, a `close`, and a `fetch_all` future dropped through `tokio::time::timeout` each cause exactly one `terminate` call and no later request, and a `next_batch` future dropped while the test holds the transport lock causes no `terminate` call. Rewrite the existing `test_next_batch_*` and `test_iterator_close_*` tests to await `next_batch` and expect the end-of-stream close.
   - [ ] 1.2 Replace `ResultSetIterator` in `src/query/results.rs`: `pub async fn next_batch(&mut self) -> Result<Option<RecordBatch>, QueryError>`; fields `handle`, `transport`, `metadata`, `first_batch: Option<RecordBatch>` (moved out on the first call), `rows_received: usize` (starts at the rows of the execute response batch), and `complete`. Pass `rows_received` to `fetch_page_after`, so the end rule and the row count errors stay shared with `fetch_all`. Close the handle and ignore the close error when the stream ends or a row count error occurs. Keep the retry after a transport error. Remove `batches`, `current_index`, `fetch_next_batch`, the `block_on` branch, and `impl Iterator`. Add one private helper that locks the transport and runs one result set request while a drop guard holds the lock guard; if the future is dropped before the response is read, the guard calls `TransportProtocol::terminate()`, so every later operation on the connection fails with the terminated-transport error. Run the `fetch_results` call in `fetch_page_after` and every `close_result_set` call in `src/query/results.rs` (`fetch_all`, the end-of-stream close in `next_batch`, and `close`) through that helper, so the Rust iterator and `fetch_all` conform to ADR `client-give-up-terminates-connection`. State in the doc comments of `next_batch`, `close`, and `fetch_all` that dropping the future during a request terminates the connection. [expert]
   - [ ] 1.3 Change `ResultSetInner::Stream` in `src/query/results.rs` to hold the execute response batch as `first_batch: RecordBatch` instead of `batches: Vec<RecordBatch>`, so `into_iterator` moves the one batch that `from_transport_result` always stores, and adapt `fetch_all` and `check_buffered_rows`.
   - [ ] 1.4 Update the doc example in `src/query/mod.rs` to `while let Some(batch) = iterator.next_batch().await?`, and keep the `ResultSetIterator` re-exports in `src/query/mod.rs` and `src/lib.rs`.
   - [ ] 1.5 Make `open_iterator` and `drain_iterator` in `tests/common/mod.rs` async and runtime-free. Rewrite `test_iterator_ends_after_last_row`, `test_iterator_partial_inline_result_returns_every_row_once`, and `test_every_iterated_batch_carries_the_result_set_field_metadata` in `tests/integration_tests.rs`, and their `test_ws_` counterparts in `tests/websocket_integration_tests.rs`, as `#[tokio::test]` tests.
   - [ ] 1.6 Add the integration tests that Scenario Coverage lists for the Result Sets scenarios to `tests/integration_tests.rs` and `tests/websocket_integration_tests.rs`.
   - [ ] 1.7 Add a "Streaming Results" section to `docs/queries.md` with an `execute`, `into_iterator`, `next_batch` example and the early `close`. Add a `stream_rows` step to `examples/basic_usage.rs` that reads `SELECT t.v AS v, RPAD(TO_CHAR(t.v), 1000, 'x') AS s FROM VALUES BETWEEN 1 AND 70000 AS t(v)` with `next_batch` and prints `Streamed <rows> rows in <batches> batches`.
   - [ ] 1.8 Change `terminated_error` in `src/transport/lifecycle.rs` to the text `Transport was terminated because an in-flight response was abandoned; reconnect before the next operation`, and rewrite its doc comment so it no longer names the export timeout as the only caller of `terminate()`. Update the copies of the old text in `TERMINATED_CAUSE` in `src/transport/test_support.rs`, `tests/integration_tests.rs` (L3574), `tests/websocket_integration_tests.rs` (L267), and `docs/import-export.md` (L315), and the `Connection::is_closed` doc comment in `src/adbc/connection.rs` (L1051-1054), which names only the export timeout.

2. FFI streaming reader (Group B, after Group A)
   - [ ] 2.1 Write failing unit tests in `src/adbc_ffi.rs` with `connected_connection(MockTransport)` for the four new FFI Statement Execution scenarios and the Result Schema scenario: `execute` sends no fetch; each `next()` sends at most one fetch; a returned batch's column arrays have a strong count of 1 after the caller drops it; the reader closes the handle at its end and its later drop sends no request; dropping the reader before its end sends `close_result_set` with the handle once and no fetch; a fetch error and a row count error make `next()` return `Some(Err(_))` whose message carries the failure; `schema()` before the first `next()` equals the first batch's schema, also for a zero-row result; the reader type is `Send`.
   - [ ] 2.2 Add the streaming reader to `src/adbc_ffi.rs`: it holds the schema of the execute response batch and an `Option<ResultSetIterator>`; `next()` blocks on `get_runtime()` for one `next_batch` and maps `QueryError` to `ArrowError::ExternalError`; `Drop` blocks on `get_runtime()` for `ResultSetIterator::close` and ignores its error. [expert]
   - [ ] 2.3 Route `FfiStatement::execute` for SQL with no bound RecordBatch through `Connection::execute`: a result set returns the streaming reader, and an affected-row count returns the existing empty reader. The bound-batch, bulk-ingest, and metadata paths keep `reader_over_batches`.
   - [ ] 2.4 Add the driver manager integration tests that Scenario Coverage lists to `tests/driver_manager_tests.rs`, each with a native test and a `#[cfg(feature = "websocket")]` `_websocket` variant, as the existing reader schema tests do.
   - [ ] 2.5 State in `docs/driver-manager.md` that `execute` streams a query result one batch per read and that releasing the reader early closes the result. Add a streaming select to `examples/driver_manager_usage.rs` that runs the same 70,000-row query as `examples/basic_usage.rs`, prints `Schema before first read: <field names>` from `reader.schema()` before the first read, and then prints `Streamed <rows> rows in <batches> batches`.
   - [ ] 2.6 Bump `Cargo.toml` to 0.33.0 and add a `## 0.33.0` entry to `CHANGELOG.md`: a `Breaking:` line for the async `next_batch`, the removed `Iterator` impl, the end-of-stream close, the terminated connection after a `next_batch`, `close`, or `fetch_all` future is dropped during a request, and the new terminated-transport error text, and a `Changed:` line for the streaming FFI reader and its early-release close, both ending in `Fixes #66.`

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Async result set iterator | 1.1-1.8 | none | spec deltas `query-execution/result-sets`, `connection-management/session-and-lifecycle`; ADR `client-give-up-terminates-connection` (`specs/_decision/008-remove-query-timeout.md`); `src/query/results.rs`, `src/query/mod.rs`, `src/transport/lifecycle.rs`, `src/transport/test_support.rs`, `src/adbc/connection.rs` (`is_closed` doc comment), `tests/common/mod.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `docs/queries.md`, `docs/import-export.md`, `examples/basic_usage.rs` |
| B: FFI streaming reader | 2.1-2.6 | A (uses the async `ResultSetIterator` API) | spec deltas `adbc-driver/ffi-statement-execution`, `adbc-driver/result-schema`; `src/adbc_ffi.rs`, `tests/driver_manager_tests.rs`, `docs/driver-manager.md`, `examples/driver_manager_usage.rs`, `Cargo.toml`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Trait impl | `impl Iterator for ResultSetIterator` in `src/query/results.rs` | Replaced by async `next_batch` |
| Function | `ResultSetIterator::fetch_next_batch` in `src/query/results.rs` | Merged into async `next_batch` |
| Fields | `ResultSetIterator::batches`, `ResultSetIterator::current_index` in `src/query/results.rs` | Replaced by `first_batch` and `rows_received` |
| Code path | `Handle::try_current().block_on` and the "No async runtime available" error in `ResultSetIterator::next_batch` | The iterator awaits on the caller's runtime |
| Test | `test_next_batch_without_async_runtime_reports_invalid_state` in `src/query/results.rs` | Tests the removed no-runtime path |
| Test | `test_iterator_trait_yields_every_page_until_exhausted` in `src/query/results.rs` | Tests the removed `Iterator` impl |
| Test | `test_fetch_next_batch_short_circuits_once_the_stream_is_complete` in `src/query/results.rs` | Tests the removed private function; the rewritten end-of-stream tests cover the behavior |
| Test helper | `entered_runtime` in `src/query/results.rs` tests | Only the blocking iterator needed an entered runtime |
| Test helper code | The runtime construction in `open_iterator` and the `runtime.enter()` in `drain_iterator`, `tests/common/mod.rs` | The helpers become async |
| Function | `FfiStatement::query_sql` returning `Vec<RecordBatch>` in `src/adbc_ffi.rs` | Replaced by the streaming path of task 2.3 |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Result set iterator fetches one batch per call | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_iterator_reads_multi_fetch_result_on_current_thread_runtime`, `test_ws_iterator_reads_multi_fetch_result_on_current_thread_runtime` |
| Result set iterator fetches one batch per call | Unit | `src/query/results.rs` | `test_next_batch_serves_execute_response_then_one_fetch_per_call` |
| Result set iterator keeps no batch that it has yielded | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_iterator_keeps_no_yielded_batch`, `test_ws_iterator_keeps_no_yielded_batch` |
| Result set iterator keeps no batch that it has yielded | Unit | `src/query/results.rs` | `test_next_batch_keeps_no_yielded_batch` |
| Result set iterator reads inside an async runtime | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_iterator_reads_multi_fetch_result_on_current_thread_runtime`, `test_iterator_reads_multi_fetch_result_on_multi_thread_runtime`, and their `test_ws_` counterparts |
| Result set iterator closes the result set handle when it ends | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_iterator_end_closes_handle_and_connection_stays_usable`, `test_ws_iterator_end_closes_handle_and_connection_stays_usable` |
| Result set iterator closes the result set handle when it ends | Unit | `src/query/results.rs` | `test_next_batch_closes_handle_at_end_and_ignores_close_error`, `test_next_batch_closes_handle_after_row_count_error`, `test_close_after_end_sends_no_request` |
| Result set iterator closed before its end releases the result set handle | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_iterator_close_before_end_keeps_connection_usable`, `test_ws_iterator_close_before_end_keeps_connection_usable` |
| Result set iterator closed before its end releases the result set handle | Unit | `src/query/results.rs` | `test_close_before_end_closes_handle_without_fetch` |
| Result set request dropped before its response terminates the connection | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_dropped_next_batch_terminates_connection`, `test_ws_dropped_next_batch_terminates_connection`: after the first `next_batch` of `multi_fetch_query`, `tokio::time::timeout(Duration::ZERO, iterator.next_batch())` returns `Elapsed`, because tokio polls the fetch once and the server cannot answer within that poll; the next `execute` on the connection fails with an error that contains `Transport was terminated because an in-flight response was abandoned` |
| Result set request dropped before its response terminates the connection | Unit | `src/query/results.rs` | `test_dropped_next_batch_during_fetch_terminates_transport`, `test_dropped_close_during_close_terminates_transport`, `test_dropped_fetch_all_during_fetch_terminates_transport`, `test_next_batch_dropped_while_waiting_for_transport_does_not_terminate` |
| Operations after an export timeout name the termination (changed error text, retested) | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection`, `test_terminate_marks_websocket_transport_disconnected`, with the new text |
| Operations after an export timeout name the termination (changed error text, retested) | Unit | `src/transport/lifecycle.rs`, `src/transport/native/mod.rs`, `src/transport/websocket.rs` | `every_guard_names_the_termination_on_a_terminated_transport`, `close_after_terminate_succeeds_without_io_and_keeps_reporting_termination`, `close_disconnects_an_open_transport_and_records_closed_not_terminated`, and both `operations_after_terminate_report_the_terminated_transport` tests, through the updated `TERMINATED_CAUSE` |
| Result set iterator ends after the last row (existing, retested) | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_iterator_ends_after_last_row`, `test_iterator_partial_inline_result_returns_every_row_once`, and their `test_ws_` counterparts, rewritten as `#[tokio::test]` |
| Result set that ends before its total row count fails (existing, retested) | Unit | `src/query/results.rs` | `test_next_batch_fails_when_the_stream_ends_before_the_total`, rewritten async |
| Result set that exceeds its total row count fails (existing, retested) | Unit | `src/query/results.rs` | `test_next_batch_fails_when_a_page_exceeds_the_total`, rewritten async |
| Every batch carries the field metadata of the result set metadata schema (existing, retested) | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_every_iterated_batch_carries_the_result_set_field_metadata`, `test_ws_every_iterated_batch_carries_the_result_set_field_metadata`, rewritten as `#[tokio::test]` |
| execute streams a query result one batch per read | Integration | `tests/driver_manager_tests.rs` | `test_execute_streams_multi_fetch_result`, `test_execute_streams_multi_fetch_result_websocket` |
| execute streams a query result one batch per read | Unit | `src/adbc_ffi.rs` | `streaming_reader_fetches_nothing_on_execute_and_once_per_read`, `streaming_reader_keeps_no_returned_batch` |
| A streaming reader closes the result set handle at its end | Integration | `tests/driver_manager_tests.rs` | `test_streaming_reader_end_keeps_connection_usable`, `test_streaming_reader_end_keeps_connection_usable_websocket` |
| A streaming reader closes the result set handle at its end | Unit | `src/adbc_ffi.rs` | `streaming_reader_closes_handle_at_end_and_drop_sends_nothing` |
| Releasing a streaming reader before its end closes the result set handle | Integration | `tests/driver_manager_tests.rs` | `test_releasing_streaming_reader_before_end_keeps_connection_usable`, `test_releasing_streaming_reader_before_end_keeps_connection_usable_websocket` |
| Releasing a streaming reader before its end closes the result set handle | Unit | `src/adbc_ffi.rs` | `releasing_streaming_reader_before_end_closes_handle_without_fetch` |
| A failed fetch fails the read of a streaming reader | Integration | `tests/driver_manager_tests.rs` | `test_streaming_reader_read_fails_after_session_is_killed`, `test_streaming_reader_read_fails_after_session_is_killed_websocket`: after the first read of `multi_fetch_query`, a second connection runs `KILL SESSION <id>` on the reader's session; the test reads until a read fails, asserts that the failure arrives before all `MULTI_FETCH_ROWS` rows and carries a non-empty message, then releases the reader without a panic |
| A failed fetch fails the read of a streaming reader | Unit | `src/adbc_ffi.rs` | `streaming_reader_read_fails_with_fetch_error_message`, `streaming_reader_read_fails_on_row_count_mismatch` |
| Reader schema is available before the first read | Integration | `tests/driver_manager_tests.rs` | `test_reader_schema_is_available_before_the_first_read`, `test_reader_schema_is_available_before_the_first_read_websocket` (zero-row query and `multi_fetch_query`) |
| Reader schema is available before the first read | Unit | `src/adbc_ffi.rs` | `streaming_reader_schema_is_set_before_the_first_read` |

- The integration tests assert the observable result: every row once (`assert_every_key_once`), at least two batches for `multi_fetch_query`, a first batch with fewer rows than the total, `Arc::strong_count` of 1 on yielded Rust column arrays, and a usable connection afterwards. The unit tests with `MockTransport` assert the fetch and close requests, which Exasol does not expose.
- The `test_ws_` and `_websocket` tests run only with the `websocket` feature. The driver manager `_websocket` tests need the release library built with `--features 'ffi websocket'`.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Result Sets | `cargo run --example basic_usage` (Exasol on `localhost:8563`) | A line `Streamed 70000 rows in <n> batches` with `n` of 2 or more, then `Done` |
| FFI Statement Execution | `cargo build --release --features ffi && cargo run --example driver_manager_usage` | A line `Streamed 70000 rows in <n> batches` with `n` of 2 or more, and exit code 0 |
| Result Schema | `cargo build --release --features ffi && cargo run --example driver_manager_usage` | A line `Schema before first read: V, S` before the `Streamed` line |

### Checklist

Commands come from `specs/mission.md` § Commands, `AGENTS.md`, and `.github/workflows/ci.yml`. Integration tests need the local Exasol container that `AGENTS.md` § Testing describes.

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build FFI library | `cargo build --release --features 'ffi websocket'` | Exit 0 |
| Unit tests | `cargo test --lib --features parquet` | 0 failures |
| Unit tests (websocket) | `cargo test --lib --features websocket` | 0 failures |
| Unit tests (ffi) | `cargo test --lib --features ffi` | 0 failures |
| Integration (native) | `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| Integration (websocket) | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Integration (driver manager) | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Websocket-only build | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| CI test targets | `python3 scripts/check_ci_test_targets.py` | Exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings`, `cargo clippy --all-targets --features ffi -- -D warnings`, `cargo clippy --all-targets --no-default-features --features websocket -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --features parquet --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip` and `python3 scripts/strip_test_coverage.py check` | Production line coverage at least 80 percent, every file at least 50 percent |
