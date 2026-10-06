# Code Review Findings: fix-paged-fetch-position

## Summary
- Files reviewed: 26
- Total findings: 10 (standard: 9, expert: 1)
- Baseline checks run during review: `cargo fmt --all -- --check` passes, `cargo clippy --all-targets --all-features -- -W clippy::all` reports no issues, `python3 -m unittest discover --start-directory scripts --pattern 'test_check_ci*.py'` passes 8 tests, `python3 scripts/check_ci_test_targets.py` exits 0, and the 33 new or changed unit tests in `query::results`, `import::parallel`, `import::parquet`, `transport::native`, and `transport::websocket` pass with `cargo test --lib --features websocket`.

## Standard fixes

### src/query/results.rs

#### [UNTESTED_ERROR_PATH] Iterator retry after a transport error is unasserted
- Location: `ResultSetIterator::fetch_next_batch`, line 806; `test_next_batch_propagates_fetch_error`, line 2566
- Issue: `fetch_next_batch` sets `complete` after `PageFetchError::RowCountMismatch` and leaves it unset after `PageFetchError::Fetch`, so a caller can retry after a transport error (plan task 1.3). No test asserts the retry. `test_next_batch_propagates_fetch_error` checks only the first error. A change that also set `complete` after a transport error would pass every test, although this distinction is the reason `PageFetchError` exists.
- Fix: In src/query/results.rs, add the unit test `test_next_batch_fetches_again_after_a_transport_error` next to `test_next_batch_propagates_fetch_error`. Enter `entered_runtime()` as the neighboring tests do. Give the `MockTransport` one `expect_fetch_results().times(2)` expectation whose `returning` closure captures a call counter and returns `Err(TransportError::ReceiveError("no page".to_string()))` on the first call and `Ok(single_column_result_data(&[2], 2))` on the second. Build the iterator with `streaming_iterator(transport, &[1], 2, Some(ResultSetHandle::new(1)))`. Assert that the first `next_batch()` yields 1 row, the second yields `QueryError::ExecutionFailed`, the third yields a batch of 1 row, and the fourth returns `None`.

#### [OUTDATED_COMMENT] Public docs omit the new row-count mismatch error
- Location: `ResultSet::fetch_all` doc comment, lines 258-261; `ResultSetIterator::next_batch` doc comment, lines 810-812
- Issue: `fetch_all` and `next_batch` now return `QueryError::ExecutionFailed` when a result set delivers fewer or more rows than its total row count. CHANGELOG.md lists this as a `Changed:` behavior. The `# Errors` section of `fetch_all` names only a failed fetch and a non-streaming result, and `next_batch` documents no errors, so the public API docs do not state the new failure.
- Fix: In src/query/results.rs, add to the `# Errors` section of `fetch_all` the line "Returns `QueryError::ExecutionFailed` when the result set delivers fewer or more rows than its total row count." Add a `# Errors` section with the same sentence to the doc comment of `ResultSetIterator::next_batch`, and state there that the iterator returns `None` on every later call after that error.

### src/transport/websocket.rs

#### [IMPLEMENTATION_COUPLED_TEST] Close test asserts the private position map
- Location: `test_close_result_set_forgets_the_fetch_position`, lines 1583-1600
- Issue: the test asserts `transport.fetch_positions.is_empty()`, a private field. Decision-log entry [5] rejected test access to the position map and chose to assert the `startPosition` field that Exasol reads. The test also carries `/// Scenario: Fetch results command`, a scenario that says nothing about closing a result set.
- Fix: In src/transport/websocket.rs, rewrite `test_close_result_set_forgets_the_fetch_position` to observe the wire. Script the responses `execute_response(7, 5, &[1, 2])`, `json!({"status": "ok"})`, and `fetch_response(&[1])`. Call `execute_query("SELECT 7")`, then `close_result_set(ResultSetHandle::new(7))`, then `fetch_results(ResultSetHandle::new(7))`. Replace the map assertion with `assert_eq!(fetch_start_positions(&server), vec![0]);`. Delete the test's `/// Scenario: Fetch results command` line.

#### [REDUNDANT_COMMENT] Banner comment and mid-module imports in the test module
- Location: lines 1470-1473
- Issue: `// --- Fetch start position, against a scripted fake server ---` is a banner. AGENTS.md § Code style forbids banners, and the file has no other banner of this style. The two `use` lines below it sit in the middle of `mod tests` instead of next to `use super::*;` at line 817.
- Fix: In src/transport/websocket.rs, delete the banner line at line 1470. Move `use crate::transport::test_support::FakeWebSocketServer;` and `use serde_json::{json, Value};` to directly below `use super::*;` at the top of `mod tests`.

### tests/driver_manager_tests.rs

#### [INFORMATION_LEAKAGE] Local skip macro duplicates the exported macro
- Location: `macro_rules! skip_if_no_exasol` and its doc comment, lines 87-106; `use common::{...}`, lines 48-50
- Issue: the local macro is now a line-for-line copy of the `#[macro_export]` macro `skip_if_no_exasol!` in tests/common/mod.rs. Both hold the same `REQUIRE_EXASOL` check, the same panic message, the same skip message, and the same `common` function calls. The fail-not-skip rule therefore lives in two macros that must change together. The file already declares `mod common;`, so the exported macro resolves here as it does in tests/integration_tests.rs.
- Fix: In tests/driver_manager_tests.rs, delete the local `skip_if_no_exasol` macro and its doc comment. Remove `is_exasol_available` from the `use common::{...}` list, and keep `get_host` and `get_port`, which `get_test_uri` and other code use. Confirm with `cargo clippy --all-targets --all-features -- -W clippy::all` and `cargo test --features ffi --test driver_manager_tests --no-run`.

### tests/common/mod.rs

#### [MAGIC_NUMBER] Iterator call cap is a bare literal
- Location: `drain_iterator`, lines 439, 451, and 463
- Issue: the cap of 100 `next_batch` calls appears as the literal `100` in the loop bound, in the panic message, and in the doc comment.
- Fix: In tests/common/mod.rs, add `const MAX_NEXT_BATCH_CALLS: usize = 100;` directly above `drain_iterator`. Use it as the loop bound `0..MAX_NEXT_BATCH_CALLS`. Change the panic to `panic!("the iterator did not end within {MAX_NEXT_BATCH_CALLS} next_batch calls")`. Change the doc sentence to "Fails after `MAX_NEXT_BATCH_CALLS` calls, so a missing end of stream cannot hang the test." If clippy reports the constant as unused in a test target, add `#[allow(dead_code)]` to it, as on the neighboring helpers.

### tests/integration_tests.rs, tests/websocket_integration_tests.rs

#### [SHRINKABLE] Partial-inline assertion and iterator setup repeat across both transport files
- Location: tests/integration_tests.rs lines 3753, 3768-3779, 3797-3808, 3814, and 3845; tests/websocket_integration_tests.rs lines 2039, 2054-2065, 2083-2094, 2100, and 2131
- Issue: the first-batch assertion `let first = batches[0].num_rows(); assert!((1..PARTIAL_INLINE_ROWS).contains(&first), ...)` appears six times across the two files. The block that builds a multi-thread runtime, connects, executes the query, and calls `into_iterator` appears four times and differs only in the connect function and the query. Both exceed the Rule of Three, so a change to either check needs six or four identical edits.
- Fix: In tests/common/mod.rs, add two `#[allow(dead_code)]` helpers with doc comments. First, `pub fn assert_partly_inline(batches: &[RecordBatch])`, which holds the first-batch assertion against `PARTIAL_INLINE_ROWS` with its current message. Second, `pub fn open_iterator<F>(connect: F, sql: String) -> (tokio::runtime::Runtime, Connection, ResultSetIterator) where F: std::future::Future<Output = Result<Connection, exarrow_rs::error::ExasolError>>`, which builds the multi-thread runtime with `enable_all()`, awaits `connect` and `execute(sql)` inside `block_on`, and returns the runtime, the connection, and the result of `into_iterator()`, keeping the current `expect` messages. In tests/integration_tests.rs, replace the three assertion copies with `assert_partly_inline(&batches);` and the two setup blocks with `let (runtime, conn, mut iterator) = open_iterator(get_test_connection(), <query>);`. In tests/websocket_integration_tests.rs, make the same replacements with `open_iterator(get_ws_connection(), <query>)`. Update the `use common::{...}` lists of both files.

### tests/import_export_tests.rs

#### [MAGIC_NUMBER] Import hang limit is a bare literal
- Location: `assert_import_reports_missing_table`, lines 2683-2690
- Issue: the 60-second limit appears as `std::time::Duration::from_secs(60)`, again as text in the `expect` message, and again in the doc comment.
- Fix: In tests/import_export_tests.rs, add `const IMPORT_HANG_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);` next to `MISSING_TABLE`. Pass it to `tokio::time::timeout`. Replace the `expect` call with `.unwrap_or_else(|_| panic!("an import into a missing table must return within {IMPORT_HANG_LIMIT:?}"))`. Change the doc comment to say "within `IMPORT_HANG_LIMIT`" instead of "within 60 seconds".

#### [OUTDATED_COMMENT] Long export test instructions name only half of the opt-in
- Location: `test_csv_export_runs_past_the_former_five_minute_limit`, lines 2830 and 2832
- Issue: the doc sentence "Select it with `-- --ignored` when you want it." does not match the code. With `--ignored` alone, the `EXARROW_LONG_EXPORT_CHECK` check at line 2834 returns early, and the test passes without running. The `#[ignore]` reason names only `EXARROW_LONG_EXPORT_CHECK=1`, which alone leaves the test ignored. Each text omits the other half of the opt-in.
- Fix: In tests/import_export_tests.rs, change the doc sentence to "Run it with `EXARROW_LONG_EXPORT_CHECK=1` and `-- --ignored`." Change the attribute to `#[ignore = "eight-minute opt-in check, run with EXARROW_LONG_EXPORT_CHECK=1 and -- --ignored"]`.

## Expert fixes

### src/import/parallel.rs

#### [DUPLICATE_TEST] The "would fail later" tunnel task never runs
- Location: `test_finish_import_returns_statement_error_over_a_tunnel_task_that_would_fail_later`, line 790
- Issue: `#[tokio::test]` runs on a current-thread runtime, and the test calls `finish_import` without yielding after `tokio::spawn`. `finish_import` therefore aborts the task before the runtime first polls it, and tokio drops the task's future without running it. The test exercises the same state as `test_finish_import_returns_statement_error_without_waiting_for_a_silent_tunnel` at line 746: a tunnel task that is never polled. The condition that the test name and plan task 8.4 describe, a tunnel task that is still running, is never reached, so the test passes whether or not `finish_import` handles a running task correctly.
- Fix: In src/import/parallel.rs, add a second oneshot channel `let (started, has_started) = oneshot::channel::<()>();` to the test. Inside the spawned task, call `let _ = started.send(());` before `let _ = released.await;`. Before calling `finish_import`, run `tokio::time::timeout(HANG_LIMIT, has_started).await.expect("the tunnel task must start").expect("the tunnel task must signal its start");`, so the abort reaches a task suspended on `released`. Keep the `_release` sender alive for the whole test and keep the assertion that the result is `ImportError::SqlError`.
