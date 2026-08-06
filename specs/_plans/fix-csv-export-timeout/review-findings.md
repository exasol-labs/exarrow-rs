# Code Review Findings: fix-csv-export-timeout

## Summary
- Files reviewed: 20
- Total findings: 10 (standard: 8, expert: 2)

Verification run during review (evidence for the findings below):
- `cargo clippy --all-targets --all-features -- -D warnings` — exit 0, no warnings
- `cargo test --lib` — 1549 passed, 0 failed
- `cargo llvm-cov --lib` + `scripts/strip_test_coverage.py check` — 83.72% production, `src/export/csv.rs` at 50.44%, floors met

Deliberate, plan-authorized choices NOT raised as findings: the `#[allow(dead_code)]` markers on the two promoted `tests/common` helpers (task 1.1), the env-gated early return in `test_csv_export_runs_past_the_former_five_minute_limit` (task 4.5), the runtime-assertion-free `test_export_to_callback_future_is_send` (task 3.1), extracting `shared_csv_export_options` at the second duplication rather than the third (task 3.2), and keeping the `src/export/csv.rs` per-file coverage-floor exemption (plan § Non-Goals).

## Standard fixes

### src/transport/native/mod.rs

#### [MISSING_BOUNDARY_TEST] The terminate unit test cannot fail if the socket and session are not dropped
- Location: lines 1326-1334 (`test_terminate_marks_transport_disconnected`)
- Issue: The test arranges `NativeTcpTransport::new()` (whose `stream` and `session` are already `None`) and only writes `state = ConnectionState::Authenticated`, then asserts `is_connected()` is `false`. `is_connected()` is derived from `state` alone (line 1219), so the assertion holds even if `terminate()` were reduced to `self.state = ConnectionState::Closed;`. The spec delta `connection-management/session-and-lifecycle` requires "the transport SHALL drop its socket and SHALL report itself as not connected"; the socket/session half of that requirement has no executed unit coverage. The only thing exercising it end-to-end is the `SYS.EXA_ALL_SESSIONS` poll in `tests/integration_tests.rs`, which needs a live container.
- Fix: In src/transport/native/mod.rs, extend `test_terminate_marks_transport_disconnected` so it also proves the teardown clears state that `terminate()` is responsible for: set `transport.session = Some(<a constructed session value>)` in the arrange step alongside `transport.state = ConnectionState::Authenticated`, and after `transport.terminate()` assert both `!transport.is_connected()` and `transport.session.is_none()`. Rename the test to `terminate_drops_the_session_and_reports_the_transport_disconnected` so the name states both post-conditions.

### src/transport/websocket.rs

#### [MISSING_BOUNDARY_TEST] The terminate unit test cannot fail if the socket and session are not dropped
- Location: lines 826-834 (`test_terminate_marks_transport_disconnected`)
- Issue: Same defect as the native transport test. `WebSocketTransport::new()` starts with `ws_stream: None` and `session_info: None`, the test only sets `state`, and `is_connected()` (lines 838-843) reads `state` alone. `terminate()` (lines 752-756) also clears `ws_stream` and `session_info`, and neither is asserted, so the test passes against a `terminate()` that forgets them.
- Fix: In src/transport/websocket.rs, extend `test_terminate_marks_transport_disconnected` to set `transport.session_info = Some(<a constructed session info value>)` in the arrange step and assert both `!transport.is_connected()` and `transport.session_info.is_none()` after `transport.terminate()`. Rename the test to `terminate_drops_the_session_and_reports_the_transport_disconnected` to match the native transport's test name.

### src/adbc/connection.rs

#### [OUTDATED_COMMENT] The `is_closed()` doc comment still describes session state only
- Location: lines 1048-1055
- Issue: The doc reads "Check if the connection is closed. / # Returns / `true` if the connection is closed, `false` otherwise." The body now also returns `true` when the transport reports `is_connected() == false` while the session still believes it is open — the plan's § Impact lists this as one of six breaking changes ("Code that treated `is_closed() == false` as proof of a live session sees a changed answer after a terminating export timeout"). A caller reading this doc cannot learn that a terminated transport now flips the answer, nor that the method acquires the transport mutex.
- Fix: In src/adbc/connection.rs, rewrite the doc comment on `is_closed()` (lines 1048-1052) to state that it reports closed when either the session is closed or the transport has been terminated, so one fact about liveness has one answer, and to note that a terminating export timeout (`ExportError::Timeout { transport_terminated: true }`) is what makes the second half fire. Keep the `# Returns` section.

#### [MISSING_DESIGN_INTENT] The blocking export wrappers do not state that they can now block indefinitely
- Location: lines 2044-2065 (`blocking_export_csv_to_file`), 2074-2095 (`blocking_export_to_parquet`), 2104-2124 (`blocking_export_to_record_batches`)
- Issue: All three docs say only "This is a synchronous wrapper around [...] for use in non-async contexts." With `CsvExportOptions::timeout_ms` now defaulting to `None` and `src/transport/http_transport.rs` carrying no read timeout, a stalled tunnel blocks the calling OS thread forever with no way for the caller to cancel it. The plan's § Impact names exactly this ("a stalled tunnel now blocks the calling OS thread indefinitely, which the caller cannot cancel"), but the risk is recorded nowhere a caller of these three methods will see it. `blocking_export_to_record_batches` and `blocking_export_to_parquet` are worse than the CSV one, because `ArrowExportOptions`/`ParquetExportOptions` expose no `timeout_ms` at all.
- Fix: In src/adbc/connection.rs, add a `# Blocking behavior` doc section to `blocking_export_csv_to_file`, `blocking_export_to_parquet`, and `blocking_export_to_record_batches` stating that no client-side timeout is armed by default, that a stalled HTTP tunnel therefore blocks the calling thread with no cancellation path, and that the bound must come from either `CsvExportOptions::timeout_ms` (CSV only) or the server-enforced `query_timeout` connection parameter.

### src/export/parquet.rs

#### [REDUNDANT_COMMENT] Inline comment restates the helper's own doc comment
- Location: lines 756-758
- Issue: The comment block reads "Get the data as CSV via the existing export function. / `shared_csv_export_options` always disables column names, because we don't want header rows mixed with data rows." The second sentence duplicates the doc comment already on `shared_csv_export_options` (`src/export/csv.rs:246-252`: "Column headers are always disabled, because the caller reinterprets rows positionally rather than by name"). One decision now has two descriptions that must be edited together. The first sentence describes what the next line plainly does. Line 767's "// Get the CSV data as a list of rows" above `export_to_list(...)` is the same defect.
- Fix: In src/export/parquet.rs, delete the inline comment block at lines 756-758 and the inline comment at line 767 in `export_to_parquet_via_transport`. The rationale already lives in the `shared_csv_export_options` doc comment; do not duplicate it here.

### src/export/arrow.rs

#### [INLINE_COMMENT] Leftover inline comments in the reworked block
- Location: lines 739, 748
- Issue: Line 739's "// First, get the data as CSV via the existing export function" now sits above the options-construction call rather than the export call, so it describes the wrong statement after the task 3.2 edit. Line 748's "// Get the CSV data as a list of rows" restates `let rows = export_to_list(...)`. Neither states a "why"; both are inline comments the guardrails forbid.
- Fix: In src/export/arrow.rs, delete the inline comments at lines 739 and 748 in `export_to_record_batches`.

### tests/common/mod.rs

#### [OUTDATED_COMMENT] The promoted helper's doc comment states a claim that is false for two of its callers
- Location: lines 350-352
- Issue: `long_running_count_query`'s doc says the query is "large enough to force a genuinely multi-second, server-side-only query — there is no client-side timer left to race against." That rationale was written for the query-timeout tests it came from. After task 1.1 promoted it to shared scope, two of its callers do exactly the opposite: `test_csv_export_explicit_timeout_terminates_connection` (`tests/integration_tests.rs:3172`) arms `.timeout_ms(1_000)` and depends on the client-side timer winning the race, and `test_csv_export_timeout_during_callback_keeps_connection_usable` depends on timer placement too. The doc now misdescribes the helper's purpose for its shared audience.
- Fix: In tests/common/mod.rs, rewrite the doc comment on `long_running_count_query` (lines 350-352) to state only what the helper guarantees to every caller: a `COUNT(*)` over a cartesian-product `VALUES BETWEEN` join whose server-side runtime grows with the square of `side_rows`, used to hold a statement open long enough for a test to observe timeout behavior. Drop the "no client-side timer left to race against" clause.

### tests/integration_tests.rs

#### [INLINE_COMMENT] Rationale for the new export tests lives in inline comments instead of the test doc comments
- Location: lines 3204-3218 and 3314-3315
- Issue: Both new tests already carry doc comments, and the "why" is then continued in inline comment blocks inside the bodies: the `SYS.EXA_ALL_SESSIONS` poll rationale plus the measured 2.1s reap latency (lines 3204-3218), and the tunnel warm-up rationale (lines 3314-3315). The guardrails apply to test code the same as production code: no inline comments, rationale belongs in the doc comment. The measured-latency evidence is worth keeping — it just belongs above the `#[tokio::test]`.
- Fix: In tests/integration_tests.rs, move the content of the inline comment blocks at lines 3204-3218 into the doc comment of `test_csv_export_explicit_timeout_terminates_connection`, and the content at lines 3314-3315 into the doc comment of `test_csv_export_timeout_during_callback_keeps_connection_usable`, then delete the inline comments. Keep the measured "about 2.1s reap latency, hence a polled 15s bound" evidence verbatim in the relocated text.

### docs/import-export.md

#### [MISSING_DESIGN_INTENT] The new export-timeout section omits the blocking-API hazard
- Location: lines 256-279 (`### Export Timeout`, `### Server-Enforced Alternative`)
- Issue: The section correctly documents the `None` default, the `transport_terminated` branch, and the `query_timeout=` alternative. It does not mention that the synchronous wrappers (`blocking_export_csv_to_file`, `blocking_export_to_parquet`, `blocking_export_to_record_batches`) now hold an OS thread for the whole unbounded export with no cancellation path, and that Arrow and Parquet exports cannot opt in to a client-side bound at all because their option types expose no `timeout_ms`. Those are the two consequences a reader most needs from this section, and the plan's § Impact identifies both.
- Fix: In docs/import-export.md, add a short paragraph to `### Export Timeout` stating that the `blocking_export_*` methods block the calling OS thread for the full duration of an unbounded export and cannot be cancelled, and that `ArrowExportOptions` and `ParquetExportOptions` expose no `timeout_ms`, so `query_timeout=` is the only bound available on those two paths.

## Expert fixes

### tests/integration_tests.rs

#### [UNTESTED_ERROR_PATH] The terminating-branch test never checks that the connection is unusable afterwards
- Location: lines 3159-3202 (`test_csv_export_explicit_timeout_terminates_connection`)
- Issue: This is the only executed test of the terminating branch, and it stops at asserting `ExportError::Timeout { timeout_ms: 1000, transport_terminated: true }` before going straight to `drop(conn)`. Three normative requirements go unverified:
  - `import-export/csv-export` delta, "Explicit export timeout stops the export": "a subsequent operation on the same connection MUST fail instead of returning data from the abandoned response". This is the stale-response corruption the whole plan exists to fix, and nothing asserts it.
  - `connection-management/session-and-lifecycle` delta: "`Connection::is_closed()` SHALL report the connection as closed once its transport is terminated". The only executed coverage is `is_closed_reports_true_when_the_transport_is_terminated` (`src/adbc/connection.rs:3979-3992`), a `mockall` double whose `is_connected()` is hardcoded to `false` — it proves the OR expression, not that a real terminating export drives it.
  - plan.md task 4.3, verbatim: "then assert a follow-up `conn.query("SELECT 1")` fails".

  The failure mode is a passing test over wrong behavior: if a future change made `terminate()` a no-op on the state machine, or made `Connection::query` transparently recover, this test would stay green.
- Fix: In tests/integration_tests.rs, insert assertions into `test_csv_export_explicit_timeout_terminates_connection` between the `match result { ... }` block (ends line 3196) and the connection teardown: assert `conn.is_closed().await` is `true` while nothing has closed the session, and assert `conn.query("SELECT 1").await` returns `Err` with a message naming the failure rather than returning rows. Both must run before the connection is released, so they need to precede whatever teardown the `[OUTDATED_COMMENT]` finding below settles on. Do not weaken the existing `SYS.EXA_ALL_SESSIONS` poll — keep it after these two assertions.

#### [OUTDATED_COMMENT] The stated reason for skipping `close()` is false
- Location: lines 3198-3202
- Issue: The comment asserts "The transport is already terminated, so `close()` would return an error and turn the test that proves the fix into a false red." Both halves of `Connection::close()` succeed on a terminated connection: `Session::close()` (`src/connection/session.rs:277-289`) only mutates in-memory state and unconditionally returns `Ok(())`, and `NativeTcpTransport::close()` (`src/transport/native/mod.rs:1199-1202`) returns `Ok(())` immediately when `state == ConnectionState::Closed`, which is exactly what `terminate()` set. So `conn.close().await` returns `Ok(())` here. The comment (inherited from plan.md task 4.3) documents a hazard that does not exist, and the test forgoes a real observable assertion — that a terminated connection closes cleanly and idempotently without a protocol round-trip — on the strength of it.
- Fix: In tests/integration_tests.rs, delete the inline comment at lines 3198-3202 and replace `drop(conn)` at line 3202 with `conn.close().await.expect("close() on a terminated connection must succeed without a protocol round-trip");`, matching every other test in the Query Timeout Tests section. Record the corrected reasoning in the test's doc comment: `Session::close()` and `NativeTcpTransport::close()` both short-circuit to `Ok(())` once the transport is terminated, so no round-trip is attempted. Verify with `REQUIRE_EXASOL=1 cargo test --test integration_tests test_csv_export_explicit_timeout_terminates_connection -- --nocapture` before reporting done; if `close()` does return an error against the live container, keep `drop(conn)` and replace the comment with the observed error instead of the current claim.
