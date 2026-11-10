# Tasks: add-streaming-result-sets

## Phase 2: Implementation (Group A: Async result set iterator)
- [x] 1.1 Write failing unit tests for the Result Sets scenarios (async iterator, drop-guard termination) in `src/query/results.rs` and `src/transport/test_support.rs`
- [x] 1.2 Replace `ResultSetIterator` with async `next_batch`, add the drop-guard request helper [expert]
- [x] 1.3 Change `ResultSetInner::Stream` to hold `first_batch: RecordBatch`
- [x] 1.4 Update the doc example in `src/query/mod.rs`, keep the re-exports
- [x] 1.5 Make `open_iterator` and `drain_iterator` async, rewrite the existing iterator integration tests
- [x] 1.6 Add the Result Sets integration tests (native and websocket)
- [x] 1.7 Add the "Streaming Results" docs section and the `stream_rows` example step
- [x] 1.8 Change `terminated_error` text and every copy of the old text

## Phase 2: Implementation (Group B: FFI streaming reader, after Group A)
- [x] 2.1 Write failing unit tests in `src/adbc_ffi.rs` for the FFI streaming reader
- [x] 2.2 Add the streaming reader to `src/adbc_ffi.rs` [expert]
- [x] 2.3 Route `FfiStatement::execute` for SQL with no bound RecordBatch through `Connection::execute`
- [x] 2.4 Add the driver manager integration tests (native and `_websocket`)
- [x] 2.5 Update `docs/driver-manager.md` and `examples/driver_manager_usage.rs`
- [x] 2.6 Bump `Cargo.toml` to 0.33.0 and add the `## 0.33.0` CHANGELOG entry

## Phase 3: Verification
- [ ] 3.1 Run the build and test checklist
- [ ] 3.2 Run fmt and clippy (default, `ffi`, websocket-only)
- [ ] 3.3 Scenario coverage audit and manual testing

## Phase 4: Review Fixes
- [x] 4.1 In `src/adbc_ffi.rs` `StreamingRecordBatchReader::open`, fail with `QueryError::InvalidState` when the iterator returns no first batch instead of falling back to `Schema::empty()`, and take the schema from that batch
- [x] 4.2 In `src/adbc_ffi.rs` tests, make `transport_answering` and `query_transport` return a non-panicking termination counter and assert zero terminations in the four fetch-error, row-count, early-release, and failed-close streaming reader tests
- [x] 4.3 In `src/query/results.rs`, extend the `InFlightRequest` doc comment: a panic also terminates the transport, and `TransportProtocol::terminate` must not panic
- [x] 4.4 In `src/query/results.rs` tests, delete `test_next_batch_reports_exhausted_when_handle_was_already_released`
- [x] 4.5 In `tests/common/mod.rs`, make `assert_dropped_next_batch_terminates_connection` take `connect: impl Fn() -> F`, kill the abandoned session from a second connection, update the doc comment, and update both callers
- [x] 4.6 Document the termination rule for `Connection::query`, `fetch_all`, `next_batch`, and `close` in `docs/queries.md`, the `Connection::query` doc comment, and the `## 0.33.0` CHANGELOG entry
- [x] 4.7 In `src/adbc_ffi.rs` `StreamingRecordBatchReader::next`, fail a read whose fetched batch differs from the reader schema in field count, name, data type, or metadata; rewrite the struct doc comment; add `streaming_reader_read_fails_when_a_fetched_batch_differs_from_the_reader_schema`; replace the comment in `streaming_reader_schema_is_set_before_the_first_read` [expert]
