# Tasks: fix-csv-export-timeout

## Phase 0: Prerequisite (alone, against an unmodified tree)
- [x] 0.1 Confirm an HTTP-tunnel CSV export succeeds against CI container image `exasol/docker-db:2025.2.0`, not only `exasol/docker-db:latest`. Run one existing export test from `tests/import_export_tests.rs` against a container started from the pinned image. If it fails, stop and escalate to the human before task 4.1 — do not silently relocate tests (fallback is task 6.1, human-approved only).

## Phase 2: Implementation (Group A)
- [x] 1.1 Move `long_running_count_query` and `disable_query_cache` from `tests/integration_tests.rs` into `tests/common/mod.rs` as public items, `#[allow(dead_code)]`; update imports.
- [x] 2.1 Add `fn terminate(&mut self)` to `TransportProtocol`, implement on the shared mock, native transport, and WebSocket transport, with unit tests asserting `is_connected()` is `false` afterwards. [expert]

## Phase 2: Implementation (Group B)
- [x] 2.2 Add `test_terminate_marks_websocket_transport_disconnected` to `tests/websocket_integration_tests.rs`.
- [x] 2.3 Change `Connection::is_closed()` to also check `!self.transport.lock().await.is_connected()`. Add `expect_is_connected()` to the two existing mock tests and a new sibling test `is_closed_reports_true_when_the_transport_is_terminated` for the `false` case.
- [x] 3.1 Change `CsvExportOptions::timeout_ms` to `Option<u64>` (default `None`) and rework `export_to_callback`'s timeout handling: no timer when `None`; short-circuit a failed `sql_task`; hold the HTTP `JoinHandle` outside the timed block and await/abort it on every exit path; track SQL-completion via `AtomicBool` (not `Cell<bool>`) and call `terminate()` only when the elapse happens before the SQL response is read; add `transport_terminated: bool` to `ExportError::Timeout` with two distinct display texts; add `test_export_to_callback_future_is_send` pinning the `Send` guarantee. [expert]

## Phase 2: Implementation (Group C)
- [x] 3.2 Deduplicate the CSV-option construction shared by Arrow and Parquet export paths into one `pub(crate)` function in `src/export/csv.rs`, taking a named-field struct parameter. Add a unit test asserting no export timeout is set.
- [x] 5.1 Document the export timeout behavior in `docs/import-export.md`.

## Phase 2: Implementation (Group D, sequential)
- [x] 4.1 Add `test_csv_export_default_arms_no_client_side_timer` to `tests/integration_tests.rs`.
- [x] 4.2 Add `test_csv_export_server_timeout_aborts_and_keeps_connection_usable` to `tests/integration_tests.rs`.
- [x] 4.3 Add `test_csv_export_explicit_timeout_terminates_connection` to `tests/integration_tests.rs`.
- [x] 4.4 Add `test_csv_export_timeout_during_callback_keeps_connection_usable` to `tests/integration_tests.rs`.

## Phase 2: Implementation (Group E)
- [x] 4.5 Add `test_csv_export_runs_past_the_former_five_minute_limit` to `tests/import_export_tests.rs`, `#[ignore]`, gated on `EXARROW_LONG_EXPORT_CHECK`.

## Phase 2: Conditional (only if 0.1 failed and human approved)
- [ ] 6.1 Add a CI step running the five new `import_export_tests` names; move tasks 4.1-4.4 into that file.

## Phase 4: Review Fixes
- [x] 4.1 In `src/transport/native/mod.rs`, extend `test_terminate_marks_transport_disconnected` to set `transport.session = Some(<a constructed session value>)` in the arrange step alongside `transport.state = ConnectionState::Authenticated`, and after `transport.terminate()` assert both `!transport.is_connected()` and `transport.session.is_none()`. Rename the test to `terminate_drops_the_session_and_reports_the_transport_disconnected`.
- [x] 4.2 In `src/transport/websocket.rs`, extend `test_terminate_marks_transport_disconnected` to set `transport.session_info = Some(<a constructed session info value>)` in the arrange step, and after `transport.terminate()` assert both `!transport.is_connected()` and `transport.session_info.is_none()`. Rename the test to `terminate_drops_the_session_and_reports_the_transport_disconnected`.
- [x] 4.3 In `src/adbc/connection.rs`, rewrite `is_closed()`'s doc comment (lines ~1048-1052) to state that it reports closed when either the session is closed or the transport has been terminated, noting that a terminating export timeout (`ExportError::Timeout { transport_terminated: true }`) is what makes the second half fire. Keep the `# Returns` section.
- [x] 4.4 In `src/adbc/connection.rs`, add a `# Blocking behavior` doc section to `blocking_export_csv_to_file`, `blocking_export_to_parquet`, and `blocking_export_to_record_batches` stating that no client-side timeout is armed by default, that a stalled HTTP tunnel therefore blocks the calling thread with no cancellation path, and that the bound must come from either `CsvExportOptions::timeout_ms` (CSV only) or the server-enforced `query_timeout` connection parameter.
- [x] 4.5 In `src/export/parquet.rs`, delete the inline comment block in `export_to_parquet_via_transport` that duplicates `shared_csv_export_options`'s own doc comment, and the "Get the CSV data as a list of rows" inline comment above `export_to_list(...)`.
- [x] 4.6 In `src/export/arrow.rs`, delete the inline comments in `export_to_record_batches` that restate the export call and the `export_to_list` call.
- [x] 4.7 In `tests/common/mod.rs`, rewrite the doc comment on `long_running_count_query` to state only what the helper guarantees to every caller: a `COUNT(*)` over a cartesian-product `VALUES BETWEEN` join whose server-side runtime grows with the square of `side_rows`, used to hold a statement open long enough for a test to observe timeout behavior. Drop the "no client-side timer left to race against" clause.
- [x] 4.8 In `tests/integration_tests.rs`, move the `SYS.EXA_ALL_SESSIONS` poll rationale (with the measured ~2.1s reap latency) into `test_csv_export_explicit_timeout_terminates_connection`'s doc comment (merging with whatever the expert-fix pass already wrote there, not overwriting it), and the tunnel-warmup rationale into `test_csv_export_timeout_during_callback_keeps_connection_usable`'s doc comment, then delete the inline comments.

## Phase 4: Review Fixes (Expert)
- [x] 4.9 In `tests/integration_tests.rs`, insert assertions into `test_csv_export_explicit_timeout_terminates_connection` between the `match result { ... }` block and the connection teardown: assert `conn.is_closed().await` is `true` while nothing has closed the session, and assert `conn.query("SELECT 1").await` returns `Err` naming the failure rather than returning rows. Keep the existing `SYS.EXA_ALL_SESSIONS` poll after them. [expert]
- [x] 4.10 In `tests/integration_tests.rs`, delete the inline comment claiming `close()` would error on a terminated connection and replace `drop(conn)` with `conn.close().await.expect(...)`, recording the corrected reasoning in the test's doc comment. Verify against the live container; if `close()` does error, keep `drop(conn)` and document the observed error instead. [expert]

## Phase 5: Verification
- [x] 5a.1 Run Checklist commands (build, unit tests, integration tests, export tests, websocket build check, lint, format, coverage floors, spec validation)
- [x] 5b.1 Scenario Coverage Audit against plan's Scenario Coverage table
- [x] 5c.1 Manual Testing steps from plan's Manual Testing table

## Phase 6: Report
- [x] 6.2 Generate verification-report.md
