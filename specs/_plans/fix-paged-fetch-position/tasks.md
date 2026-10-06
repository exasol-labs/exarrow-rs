# Tasks: fix-paged-fetch-position

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1 Add a private async helper next to `paginate_remaining` that fetches the page after a given number of received rows (plan.md task 1.1)
- [x] 1.2 Rewrite `paginate_remaining` as a loop over the helper that passes `row_count_of(&collected)` and stops on `Ok(None)` (plan.md task 1.2)
- [x] 1.3 Rewrite `ResultSetIterator::fetch_next_batch` to call the helper with the row count of `self.batches` (plan.md task 1.3)
- [x] 1.4 Add unit tests with `MockTransport`: `test_next_batch_stops_at_total_rows_without_another_fetch` (one buffered row, total 3, exactly one fet (plan.md task 1.4)
- [x] 2.1 In `convert_and_cache_result`, bind `rows_received` and insert it into `fetch_positions` for every handle other than `SMALL_RESULTSET`, in t (plan.md task 2.1)
- [x] 2.2 Add the unit test `inline_rows_seed_the_fetch_position_of_a_large_result_set`: on a fresh `NativeTcpTransport`, a `NativeResponse::ResultSet (plan.md task 2.2)
- [x] 3.1 Add `fetch_positions: HashMap<i32, i64>` to `WebSocketTransport` and initialize it empty in `new`. (plan.md task 3.1)
- [x] 3.2 Change `query_result_from_response` to take `&mut self` (plan.md task 3.2)
- [x] 3.3 In `fetch_results`, read the handle's position (0 when absent), pass it as the start position to `FetchRequest::new`, and after a successful (plan.md task 3.3)
- [x] 3.4 In `close_result_set`, remove the handle's position before sending the close command. (plan.md task 3.4)
- [x] 3.5 In `src/transport/test_support.rs`, add a fake WebSocket server gated on the `websocket` feature (plan.md task 3.5)
- [x] 3.6 Add unit tests in `src/transport/websocket.rs` that connect a `WebSocketTransport` to the fake server with `use_tls = false`, then set the s (plan.md task 3.6)
- [x] 4.1 Add these tests to `tests/integration_tests.rs` (native transport): `test_fetch_all_multi_fetch_returns_every_row_once`, `test_fetch_all_par (plan.md task 4.1)
- [x] 4.2 Add the WebSocket counterparts to `tests/websocket_integration_tests.rs` with the `test_ws_` prefix and the file's `get_ws_connection` helpe (plan.md task 4.2)
- [x] 4.3 Add the `/// Scenario: Small result set retrieval` line to the existing `test_select_from_dual` in `tests/integration_tests.rs` and `test_ws (plan.md task 4.3)
- [x] 5.1 Add a `## [Unreleased]` section above `## 0.17.0` in `CHANGELOG.md` (`decision-log.md` entry [10]) (plan.md task 5.1)

## Phase 2: Implementation (Group C)
- [x] 8.1 In `src/import/parallel.rs`, next to `resolve_stream_task`, add a crate-visible async function that finishes an import from the IMPORT state (plan.md task 8.1)
- [x] 8.2 [expert] In `join_stream_handles` (`src/import/parallel.rs`), hold the handles in a guard that aborts every handle not yet joined when the guard is d (plan.md task 8.2)
- [x] 8.3 Replace the `execute_sql(sql).await` followed by `resolve_stream_task(stream_handle.await)` sequence with the function of task 8.1 in `impor (plan.md task 8.3)
- [x] 8.4 Add unit tests for the function of task 8.1 in `src/import/parallel.rs`: a failed statement with a tunnel task that never finishes returns ` (plan.md task 8.4)
- [x] 8.5 Add integration tests to `tests/import_export_tests.rs`, each with `skip_if_no_exasol!()`, a 60-second `tokio::time::timeout` around the imp (plan.md task 8.5)
- [x] 8.6 Keep the gate of `supports_native_parquet_import` in `src/connection/version.rs` unchanged (`>= (2025, 1, 11)`, `decision-log.md` entry [19] (plan.md task 8.6)
- [x] 8.7 Raise the CI image from `exasol/docker-db:2025.2.0` to `exasol/docker-db:2025.2.1` in the `integration-tests` job of `.github/workflows/ci.y (plan.md task 8.7)
- [x] 8.8 Add to the `## [Unreleased]` section of `CHANGELOG.md`: a `Fix:` line stating that a CSV or Parquet import whose IMPORT statement fails befo (plan.md task 8.8)
- [x] 8.9 In `tests/import_export_tests.rs`, replace the branch that prints "skip: server does not support native Parquet import (< 2025.1.11)" and re (plan.md task 8.9)
- [x] 8.10 In `docs/import-export.md` § Native Parquet Import, state that Exasol 2025.2.0 rejects HTTP Parquet sources with `ETL-2210` and is not suppo (plan.md task 8.10)

## Phase 2: Implementation (Group B)
- [x] 6.1 In `Cargo.toml`, set `indicatif = { version = "0.18", optional = true }` (plan.md task 6.1)
- [x] 6.2 In `deny.toml`, keep the ignore entries for `GHSA-2f9f-gq7v-9h6m` and `RUSTSEC-2024-0436`, and rewrite the `reason` and comment of `GHSA-2f9 (plan.md task 6.2)
- [x] 6.3 In `.github/workflows/ci.yml`, change the `licenses` job's advisory step to `cargo deny --all-features check advisories`. (plan.md task 6.3)
- [x] 6.4 Re-check advisories before the change is complete (`decision-log.md` entry [14]): run `cargo deny --all-features check advisories` and list  (plan.md task 6.4)
- [x] 6.5 Add to the `## [Unreleased]` section of `CHANGELOG.md` a `Security:` line naming the updates `xxhash-rust` 0.8.19 (GHSA-6g2r-675j-hx59), `cr (plan.md task 6.5)
- [x] 7.1 In the `integration-tests` job of `.github/workflows/ci.yml`, after the WebSocket step, add three steps with `REQUIRE_EXASOL: "1"`: `cargo t (plan.md task 7.1)
- [x] 7.2 Remove the bare `#[ignore]` attribute from every test in `tests/import_export_tests.rs` except `test_csv_export_runs_past_the_former_five_mi (plan.md task 7.2)
- [x] 7.3 Add `scripts/check_ci_test_targets.py`, using the Python standard library only (plan.md task 7.3)
- [x] 7.4 Add `scripts/test_check_ci_test_targets.py` (`unittest`, temporary directories as fixtures) with cases: every target present exits 0; a miss (plan.md task 7.4)
- [x] 7.5 Add a step to the `lint` job of `.github/workflows/ci.yml` that runs `python3 scripts/check_ci_test_targets.py`. (plan.md task 7.5)
- [x] 7.6 Update `scripts/run_all_tests.sh` so that its integration and import-export stages run the same `cargo test` targets with the same features  (plan.md task 7.6)
- [x] 7.7 Add a step to the `unit-tests` job of `.github/workflows/ci.yml` that runs `cargo test --lib --features websocket` without coverage instrume (plan.md task 7.7)
- [x] 7.8 Replace the import/export test command in `AGENTS.md` (§ Commands), in both places in `CONTRIBUTING.md`, and in `specs/mission.md` § Command (plan.md task 7.8)

## Phase 3: Verification
- [x] 9.1 Run the plan.md Verification checklist

## Phase 4: Review Fixes
- [x] 4.4 In `src/query/results.rs`, add the unit test `test_next_batch_fetches_again_after_a_transport_error` next to `test_next_batch_propagates_fetch_error`, asserting rows, then `QueryError::ExecutionFailed`, then a 1-row batch, then `None`
- [x] 4.5 In `src/query/results.rs`, document the row-count mismatch `QueryError::ExecutionFailed` in the `# Errors` section of `fetch_all`, and add a `# Errors` section to `ResultSetIterator::next_batch` stating the error and that later calls return `None`
- [x] 4.6 In `src/transport/websocket.rs`, rewrite `test_close_result_set_forgets_the_fetch_position` to assert `fetch_start_positions(&server) == vec![0]` after execute, close, and fetch, and delete its `/// Scenario: Fetch results command` line
- [x] 4.7 In `src/transport/websocket.rs`, delete the fetch start position banner comment in `mod tests` and move the `FakeWebSocketServer` and `serde_json` imports directly below `use super::*;`
- [x] 4.8 In `tests/driver_manager_tests.rs`, delete the local `skip_if_no_exasol` macro and its doc comment, and remove `is_exasol_available` from the `use common::{...}` list
- [x] 4.9 In `tests/common/mod.rs`, add `const MAX_NEXT_BATCH_CALLS: usize = 100;` above `drain_iterator` and use it in the loop bound, the panic message, and the doc comment
- [x] 4.10 In `tests/common/mod.rs`, add `assert_partly_inline` and `open_iterator` helpers, and use them in place of the repeated first-batch assertion and iterator setup in `tests/integration_tests.rs` and `tests/websocket_integration_tests.rs`
- [x] 4.11 In `tests/import_export_tests.rs`, add `const IMPORT_HANG_LIMIT` next to `MISSING_TABLE` and use it in the timeout, the panic message, and the doc comment of `assert_import_reports_missing_table`
- [x] 4.12 In `tests/import_export_tests.rs`, make the doc sentence and the `#[ignore]` reason of `test_csv_export_runs_past_the_former_five_minute_limit` name both `EXARROW_LONG_EXPORT_CHECK=1` and `-- --ignored`
- [x] 4.13 In `src/import/parallel.rs`, make `test_finish_import_returns_statement_error_over_a_tunnel_task_that_would_fail_later` wait for a `started` oneshot signal from the tunnel task before calling `finish_import`, so the abort reaches a running task [expert]
