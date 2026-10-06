# Plan: fix-paged-fetch-position

## Summary

Paged result-set fetches start at the first row the client has not received, on both transports and for both read paths, and the result set iterator ends after the last row. The plan also fixes the open dependency advisories by moving to arrow and parquet 59 and adbc 0.24, which removes Apache Thrift, and by updating the affected optional-feature dependencies. CI then runs every integration test target and checks advisories across all Cargo features.

## Context

- Issue exasol-labs/exarrow-rs#80 reports that paged result fetches return duplicated and missing rows without an error. Versions 0.12.7, 0.13.0, and 0.16.0 are affected.
- Symptom 1: over WebSocket, `fetch_all()` on a result larger than one fetch message returns the first chunk repeatedly. 1,000,000 rows of 200 bytes come back as 1,309,444 rows with 327,361 distinct values.
- Symptom 2: on both transports, a result with fewer than 1,000 rows and more than about 64 MiB comes back wrong. 70 rows of 1 MB come back as 135 rows with 68 distinct values.
- Symptom 3: `ResultSetIterator` never ends over WebSocket and yields the first chunk on every call. Over native it yields every row, then fails with `Invalid result set seek` instead of returning `None`.
- `WebSocketTransport::fetch_results` (`src/transport/websocket.rs`) builds every request with `FetchRequest::new(handle.as_i32(), 0, max_bytes)` and keeps no per-handle position. `query_result_from_response` ignores the rows that the execute response delivers.
- `NativeTcpTransport::fetch_results` (`src/transport/native/mod.rs`) reads the start position from `fetch_positions` and advances it by `rows_received`. `convert_and_cache_result` drops the execute response's `rows_received`, so the first `CMD_FETCH2` starts at 0.
- `ResultSetIterator::fetch_next_batch` (`src/query/results.rs`) ends only on an empty page. `ResultSet::paginate_remaining` also ends once the rows reach `metadata.total_rows`.
- Exasol sends a result with fewer than 1,000 rows inline with the execute response. When that result exceeds the maximum data message size (session attribute `maxDataMessageSize`, default 64 MiB), the execute response carries the first rows and a result set handle. A result with 1,000 rows or more returns a handle, and the issue observed zero inline rows for it. The WebSocket API documents the fetch field `startPosition` as a 0-based row offset.
- `execute_query` and `execute_prepared_statement` share `query_result_from_response` on WebSocket and `convert_and_cache_result` on native. The ADBC FFI statement reads results through `ResultSet::fetch_all`.
- The unit tests pass with and without the defect, because `MockTransport` does not model start positions. The WebSocket unit tests compile only with the `websocket` feature, and the CI unit job runs default features only.
- Dependabot reports two open alerts. Alert #19: `thrift` 0.17.0, GHSA-2f9f-gq7v-9h6m / CVE-2026-43868 (medium), fixed in 0.23.0, pulled in by `parquet` 58.3.0, which requires `thrift ^0.17`. Alert #20: `xxhash-rust` 0.8.15, GHSA-6g2r-675j-hx59 (low), fixed in 0.8.16, pulled in by `polars-core` 0.46.0 under the optional `benchmark` feature.
- `deny.toml` suppresses GHSA-2f9f-gq7v-9h6m per ADR-003. Its trigger has fired: `parquet` 59.0.0 (2026-06-09) dropped `thrift`, `parquet` 59.2.0 dropped `paste`, and `adbc_core` 0.24.0 (2026-07-28) accepts `arrow-schema >=58, <60`. crates.io also lists `thrift` 0.25.0, but `parquet` 58.4.0 still requires `^0.17`.
- This branch is stacked on the unmerged branch `feat/fix-export-parquet-transport-roundtrip`, which sets `version = "0.17.0"`. No `v0.17.0` tag exists yet. The latest tag is `v0.16.0`.
- cargo-deny does not know the ID `GHSA-2f9f-gq7v-9h6m` (`unknown-advisory` warning), so the suppression never matched. Only Dependabot reports the thrift advisory.
- `cargo deny check advisories` passes in CI because it checks default features only. `cargo deny --all-features check advisories` fails on RUSTSEC-2026-0204 (`crossbeam-epoch` 0.9.18, fixed in 0.9.20, through rayon and polars) and RUSTSEC-2025-0119 (`number_prefix` unmaintained, through `indicatif` 0.17). Both crates come in only through the `benchmark` feature.
- A trial build in a scratch copy with arrow and parquet 59.3.0, adbc 0.24.0, indicatif 0.18, and the updated lockfile compiled every target with every feature without source changes. Clippy reported no warnings, 1,596 unit tests passed (1,624 with `websocket`), and `cargo deny --all-features check advisories` and `check licenses` passed. Cargo kept adbc_core on arrow 58.3.0 until the arrow sub-crates were unified explicitly.
- The CI `integration-tests` job (timeout 30 minutes, last successful run 6 minutes 34 seconds) runs `integration_tests`, `websocket_integration_tests`, `driver_manager_tests`, and the Python tests. It does not run `import_export_tests`, `native_protocol_tests`, or `native_transport_smoke_test`. Every test in `import_export_tests` carries a bare `#[ignore]`, and three URI-schema tests in `integration_tests` do too, so CI never runs them.
- A planning run against a local `exasol/docker-db:2025.2.0` container passed `native_protocol_tests` (14 tests), `native_transport_smoke_test` (4 tests), the three ignored URI-schema tests, and every CSV, Arrow, TLS, Parquet export, and forced-CSV Parquet import test in `import_export_tests`. Eleven tests that import Parquet through the native path hung until a 180-second timeout. The same native Parquet import tests pass against `exasol/docker-db:2025.2.1` and `2026.1.0` and hang against 2025.2.0 also at commit `95819035`, so the hang depends on the server version (`decision-log.md` entry [15]).
- The paged-fetch fix changes no architecture (`decision-log.md` entry [8]). The dependency upgrade and the CI rules change § Constraints, see the architecture delta, `architecture.md` in this plan directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| query-execution/results-and-transactions | CHANGED | `query-execution/results-and-transactions/spec.md` |
| websocket-client/protocol | CHANGED | `websocket-client/protocol/spec.md` |
| native-client/result-sets | CHANGED | `native-client/result-sets/spec.md` |
| code-quality/dependencies | CHANGED | `code-quality/dependencies/spec.md` |
| code-quality/core | CHANGED | `code-quality/core/spec.md` |

## Impact

- `fetch_all()` over WebSocket returns every row exactly once for results larger than one fetch message.
- `fetch_all()` on both transports returns every row exactly once for results that are partly delivered with the execute response.
- `ResultSetIterator` returns `None` after the last row on both transports. It no longer loops over WebSocket or fails with `Invalid result set seek` over native.
- `Connection::execute_prepared`, `Connection::query`, and the ADBC FFI statement pick up the fix, because they read results through the same code.
- Behavior change: `fetch_all()` and the iterator return `QueryError::ExecutionFailed` when a result set with a known total row count delivers fewer or more rows than that total. They used to return the rows without an error.
- The row-count mismatch error goes beyond the fix that issue #80 suggests (`decision-log.md` entry [4]). Declining it means deleting the two mismatch scenarios in `query-execution/results-and-transactions`, the two error branches of task 1.1, the four mismatch unit tests of task 1.4, and the `Changed:` line of task 5.1.
- The iterator sends one fetch request fewer at the end of a result set, because it stops at the total row count instead of fetching an empty page.
- `TransportProtocol` and `QueryError` keep their shape.
- Security: the dependency tree no longer contains `thrift`, so GHSA-2f9f-gq7v-9h6m / CVE-2026-43868 (Dependabot alert #19) no longer applies. `xxhash-rust` 0.8.19 fixes GHSA-6g2r-675j-hx59 (alert #20). `crossbeam-epoch` 0.9.21 fixes RUSTSEC-2026-0204. `indicatif` 0.18 removes the unmaintained `number_prefix` (RUSTSEC-2025-0119). Dependabot closes the alerts once the change reaches the default branch.
- Breaking change: exarrow-rs moves from arrow and parquet 58 to 59 and from adbc 0.23 to 0.24. Its public API passes Arrow types (`RecordBatch`, `Schema`), so a downstream crate that exchanges Arrow values with exarrow-rs must use arrow 59. exarrow-rs has no source change for the upgrade.
- Release version: the release that carries this change is a 0.x minor bump over the latest published version, never a patch bump (`decision-log.md` entry [10]). If tag `v0.17.0` does not exist when this PR merges, the version stays 0.17.0 and `[Unreleased]` folds into `## 0.17.0`. If `v0.17.0` exists, the version becomes 0.18.0 and `[Unreleased]` folds into `## 0.18.0`.
- CI runs `import_export_tests`, `native_protocol_tests`, and `native_transport_smoke_test`, and the formerly ignored import/export and URI-schema tests. The integration job takes longer. `decision-log.md` entry [17] records the measured durations.
- Native Parquet import hangs against Exasol 2025.2.0. The driver sends that server version to the native path, so a user on Exasol 2025.2.0 who imports Parquet without forcing the CSV path waits without an error. The same tests pass against Exasol 2025.2.1 and 2026.1.0. This plan does not change native Parquet import. A follow-up issue tracks the hang (task 7.0).
- Eleven native Parquet import tests stay out of CI with a stated `#[ignore]` reason, because CI uses `exasol/docker-db:2025.2.0`. `test_csv_export_runs_past_the_former_five_minute_limit` also stays out of CI with a stated reason, because it is an eight-minute opt-in check.
- CI checks advisories across all Cargo features, so an advisory in a `benchmark`-only or `ffi`-only dependency blocks a merge. A test file under `tests/` that the CI workflow does not run fails the `lint` job.
- exapump picks up the fixes when it upgrades its exarrow-rs dependency, and it needs arrow 59 to do so.

## Dependencies

The PR description lists these version bumps with their reasons, as the `code-quality/dependencies` scenario "Minor or major dep bump requires explicit evaluation" requires.

| Crate | From | To | Kind | Reason |
|-------|------|----|------|--------|
| `arrow` | 58.3.0 | 59.3.0 | major | parquet 59 must share one Arrow version with arrow (`decision-log.md` entry [11]) |
| `parquet` | 58.3.0 | 59.3.0 (requirement `59.2`) | major | 59.0.0 drops `thrift` (GHSA-2f9f-gq7v-9h6m), and 59.2.0 drops `paste` |
| `adbc_core`, `adbc_ffi`, `adbc_driver_manager` (dev) | 0.23.0 | 0.24.0 | minor (0.x) | 0.23 accepts only arrow below 59 |
| `indicatif` (optional, `benchmark`) | 0.17.11 | 0.18.x | minor (0.x) | Replaces the unmaintained `number_prefix` (RUSTSEC-2025-0119) |
| `xxhash-rust` (lockfile, `benchmark`) | 0.8.15 | 0.8.19 | patch | GHSA-6g2r-675j-hx59 |
| `crossbeam-epoch` (lockfile, `benchmark`) | 0.9.18 | 0.9.21 | patch | RUSTSEC-2026-0204 |

## Implementation Tasks

1. Query layer: one end-of-stream rule (`src/query/results.rs`)

- [ ] 1.1 Add a private async helper next to `paginate_remaining` that fetches the page after a given number of received rows. Inputs: the transport, the handle, the `QueryMetadata`, and the rows received so far. When `metadata.total_rows` is greater than zero and the rows received reach it, return `Ok(None)` without calling `fetch_results`. Otherwise lock the transport, call `fetch_results`, and map a transport error to `QueryError::ExecutionFailed` as today. On an empty page, return `Ok(None)` when the total is zero, and `QueryError::ExecutionFailed` when the rows received are below a total greater than zero. Convert the page with `payload_to_record_batch`. Return `QueryError::ExecutionFailed` when the rows received plus the page's rows exceed a total greater than zero. Each mismatch message states the rows received and the total row count. The early-end message uses the rows received before the empty page. The overshoot message uses the rows received plus the page's rows. The doc comment states that both read paths share this rule (`decision-log.md` entry [4]).
- [ ] 1.2 Rewrite `paginate_remaining` as a loop over the helper that passes `row_count_of(&collected)` and stops on `Ok(None)`. Keep its doc comment on why the per-page `total_rows` is ignored. Make `fetch_all` call `close_result_set` and ignore its error, as it already does on success, whether pagination succeeds or fails, and then return the pagination result. The existing `fetch_all` tests stay unchanged and pass.
- [ ] 1.3 Rewrite `ResultSetIterator::fetch_next_batch` to call the helper with the row count of `self.batches`. Set `complete` on `Ok(None)` and after a row-count mismatch error, so the next call returns `None` without a fetch. A transport error leaves `complete` unchanged, as today. The existing iterator tests stay unchanged and pass.
- [ ] 1.4 Add unit tests with `MockTransport`: `test_next_batch_stops_at_total_rows_without_another_fetch` (one buffered row, total 3, exactly one fetch that returns two rows, then `None` twice), `test_fetch_all_fails_when_the_stream_ends_before_the_total` (one buffered row, total 3, an empty page), `test_fetch_all_fails_when_a_page_exceeds_the_total` (one buffered row, total 2, a page of two rows), `test_next_batch_fails_when_the_stream_ends_before_the_total`, and `test_next_batch_fails_when_a_page_exceeds_the_total`. Each error test asserts `QueryError::ExecutionFailed`. The early-end tests assert a message that contains "1" and "3". The overshoot tests assert a message that contains "3" and "2". The two `fetch_all` error tests also expect `close_result_set` exactly once. Each test carries its `/// Scenario:` lines.

2. Native transport: seed the fetch position (`src/transport/native/mod.rs`)

- [ ] 2.1 In `convert_and_cache_result`, bind `rows_received` and insert it into `fetch_positions` for every handle other than `SMALL_RESULTSET`, in the branch that caches the column metadata. Update the method's doc comment.
- [ ] 2.2 Add the unit test `inline_rows_seed_the_fetch_position_of_a_large_result_set`: on a fresh `NativeTcpTransport`, a `NativeResponse::ResultSet` with handle 42, total 70, and 67 rows received records start position 67 for handle 42. A response with handle `SMALL_RESULTSET` records no position. The test carries the `/// Scenario: Large result set (multi-fetch)` line.

3. WebSocket transport: track the fetch position (`src/transport/websocket.rs`)

- [ ] 3.1 Add `fetch_positions: HashMap<i32, i64>` to `WebSocketTransport` and initialize it empty in `new`.
- [ ] 3.2 Change `query_result_from_response` to take `&mut self`. When the result set carries a handle, insert the number of rows parsed from `data` as the handle's position (`decision-log.md` entry [2]).
- [ ] 3.3 In `fetch_results`, read the handle's position (0 when absent), pass it as the start position to `FetchRequest::new`, and after a successful response store the position plus the number of rows parsed from the response's `data`.
- [ ] 3.4 In `close_result_set`, remove the handle's position before sending the close command.
- [ ] 3.5 In `src/transport/test_support.rs`, add a fake WebSocket server gated on the `websocket` feature. It binds `127.0.0.1:0`, accepts one connection with `tokio_tungstenite::accept_async`, answers each text frame with the next scripted JSON response, and records each request as a `serde_json::Value` that the test reads after the exchange. The module doc comment names it next to the existing doubles.
- [ ] 3.6 Add unit tests in `src/transport/websocket.rs` that connect a `WebSocketTransport` to the fake server with `use_tls = false`, then set the state to `Authenticated` and a `SessionInfo`. `test_fetch_results_starts_after_the_inline_rows_and_advances_per_page`: an `execute` response with handle 7, `numRows` 5, and 2 inline rows, then a second `execute` response with handle 8 and 0 inline rows. The fetches for handle 7, handle 8, and handle 7 again carry `startPosition` 2, 0, and 4 when the fetch responses deliver 2 rows each. `test_prepared_statement_fetch_starts_after_the_inline_rows`: an `executePreparedStatement` response with handle 9 and 3 inline rows makes the first fetch carry `startPosition` 3. Each test carries the `/// Scenario: Fetch results command` line.

4. Integration tests

- [ ] 4.1 Add these tests to `tests/integration_tests.rs` (native transport): `test_fetch_all_multi_fetch_returns_every_row_once`, `test_fetch_all_partial_inline_result_returns_every_row_once`, `test_iterator_ends_after_last_row`, `test_iterator_partial_inline_result_returns_every_row_once`, and `test_prepared_partial_inline_result_returns_every_row_once`. Use the queries, batch-shape assertions, and iterator harness of `decision-log.md` entry [7]: `SELECT t.v AS v, RPAD(TO_CHAR(t.v), 1000, 'x') AS s FROM VALUES BETWEEN 1 AND 70000 AS t(v)` for multi-fetch, `SELECT t.v AS v, RPAD(TO_CHAR(t.v), 1000000, 'x') AS s FROM VALUES BETWEEN 1 AND 70 AS t(v)` for partial-inline, and `SELECT t.v AS v FROM VALUES BETWEEN 1 AND 5000 AS t(v)` for end-of-stream. The iterator tests assert that `next_batch()` returns `None` twice after the last row. The prepared test prepares the partial-inline query, executes it with `Connection::execute_prepared`, reads it with `fetch_all`, and closes the statement with `Connection::close_prepared`. Put the key-column check (cast `V` to `Int64` with Arrow's `cast` kernel, then assert each value from 1 to N exactly once) in one helper in the file. Each test carries its `/// Scenario:` lines and starts with `skip_if_no_exasol!()`.
- [ ] 4.2 Add the WebSocket counterparts to `tests/websocket_integration_tests.rs` with the `test_ws_` prefix and the file's `get_ws_connection` helper: `test_ws_fetch_all_multi_fetch_returns_every_row_once`, `test_ws_fetch_all_partial_inline_result_returns_every_row_once`, `test_ws_iterator_ends_after_last_row`, `test_ws_iterator_partial_inline_result_returns_every_row_once`, and `test_ws_prepared_partial_inline_result_returns_every_row_once`. Same queries, assertions, and harness as task 4.1.
- [ ] 4.3 Add the `/// Scenario: Small result set retrieval` line to the existing `test_select_from_dual` in `tests/integration_tests.rs` and `test_ws_select_from_dual` in `tests/websocket_integration_tests.rs`. Add the unit test `test_fetch_all_small_result_set_sends_no_fetch` to `src/query/results.rs`: a result with no handle and a total equal to the buffered rows, a `MockTransport` that expects zero fetches and zero closes, and the `/// Scenario: Small result set retrieval` line.

5. Changelog

- [ ] 5.1 Add a `## [Unreleased]` section above `## 0.17.0` in `CHANGELOG.md` (`decision-log.md` entry [10]). Entries: a `Fix:` line stating that `fetch_all()` and prepared-statement results return every row exactly once for WebSocket results larger than one fetch message and for results on both transports that are partly delivered with the execute response, with "Fixes #80". A `Fix:` line stating that `ResultSetIterator` ends after the last row on both transports. A `Changed:` line stating that `fetch_all()` and the iterator return `QueryError::ExecutionFailed` when a result set delivers fewer or more rows than its total row count.

6. Dependency advisories (`Cargo.toml`, `Cargo.lock`, `deny.toml`)

- [ ] 6.1 In `Cargo.toml`, set `arrow = "59"`, `parquet = { version = "59.2", features = ["async"] }`, `adbc_core` and `adbc_ffi` to `"0.24.0"`, and the dev-dependencies `adbc_driver_manager` and `adbc_core` to `"0.24"`. Run `cargo update -p arrow -p parquet -p adbc_core -p adbc_ffi -p adbc_driver_manager`. For each of `arrow-array`, `arrow-buffer`, `arrow-data`, and `arrow-schema` that `Cargo.lock` still holds at a 58.x version, run `cargo update -p <crate>@<58.x version> --precise <the locked 59.x version of arrow>`, until `cargo tree --all-features -d` lists no `arrow-*` crate (`decision-log.md` entry [11]). Confirm that `cargo tree --all-features -i thrift` and `cargo tree --all-features -i paste` match no package, and that `cargo check --all-targets --all-features` compiles.
- [ ] 6.2 In `Cargo.toml`, set `indicatif = { version = "0.18", optional = true }`. Run `cargo update -p indicatif -p xxhash-rust -p crossbeam-epoch` (`decision-log.md` entry [12]). Confirm that `cargo tree --all-features -i xxhash-rust` shows 0.8.16 or later, that `cargo tree --all-features -i crossbeam-epoch` shows 0.9.20 or later, that `cargo tree --all-features -i number_prefix` matches no package, and that `cargo build --features benchmark --bins` compiles.
- [ ] 6.3 In `deny.toml`, remove the ignore entries and their comments for `GHSA-2f9f-gq7v-9h6m` and `RUSTSEC-2024-0436`, and set `unused-ignored-advisory = "deny"` under `[advisories]` so that a stale ignore fails the gate (`decision-log.md` entry [13]). Confirm that `cargo deny --all-features check advisories` exits 0 without warnings, and that `cargo deny check licenses` and `cargo deny --all-features check licenses` exit 0.
- [ ] 6.4 In `tests/integration_tests.rs`, replace `test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates` with `test_arrow_and_parquet_resolve_to_59_with_one_version_per_arrow_crate`. It reads `Cargo.lock` and asserts that `arrow` and `parquet` resolve to a major version of 59 or above, that `arrow-array`, `arrow-buffer`, `arrow-data`, and `arrow-schema` each have exactly one `[[package]]` entry, and that no `[[package]]` entry is named `thrift`. It carries the `/// Scenario:` lines for "Arrow and Parquet dependencies resolve to version 59 or above with one version of each Arrow sub-crate" and "Apache Thrift is absent from the dependency tree".
- [ ] 6.5 In `.github/workflows/ci.yml`, change the `licenses` job's advisory step to `cargo deny --all-features check advisories`.
- [ ] 6.6 In `specs/mission.md` § Tech Stack, change the Arrow row to `arrow 59 / parquet 59` and the ADBC FFI row to `adbc_core / adbc_ffi 0.24 (optional)`.
- [ ] 6.7 Re-check advisories before the change is complete (`decision-log.md` entry [14]): run `cargo deny --all-features check advisories` and list open Dependabot alerts with `ghbrk gh api "repos/exasol-labs/exarrow-rs/dependabot/alerts?state=open" --jq '.[] | [.number, .dependency.package.name, .security_advisory.ghsa_id] | @tsv'`. Fix each advisory that is not alert #19 or #20 and not covered by tasks 6.1 to 6.2 with an update. Suppress it per `code-quality/dependencies` only when no patched version exists. Add each one to the changelog entry of task 6.8 and to plan.md § Dependencies.
- [ ] 6.8 Add to the `## [Unreleased]` section of `CHANGELOG.md`: a `Breaking:` line stating that exarrow-rs now uses arrow and parquet 59 and adbc 0.24, so a crate that exchanges Arrow values with it must use arrow 59. A `Security:` line stating that the dependency tree no longer contains `thrift`, which resolves GHSA-2f9f-gq7v-9h6m / CVE-2026-43868. A `Security:` line naming the updates `xxhash-rust` 0.8.19 (GHSA-6g2r-675j-hx59), `crossbeam-epoch` 0.9.21 (RUSTSEC-2026-0204), and `indicatif` 0.18, which removes the unmaintained `number_prefix` (RUSTSEC-2025-0119), all in dependencies of the optional `benchmark` feature.

7. CI runs every integration test target (`.github/workflows/ci.yml`, `tests/`, `scripts/`)

- [ ] 7.1 In the `integration-tests` job of `.github/workflows/ci.yml`, after the WebSocket step, add three steps with `REQUIRE_EXASOL: "1"`: `cargo test --features ffi --test import_export_tests -- --test-threads=1` with `timeout-minutes: 10`, `cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1`, and `cargo test --features ffi --test native_transport_smoke_test -- --test-threads=1` (`decision-log.md` entry [15]). Keep the job's `timeout-minutes: 30` (entry [17]).
- [ ] 7.0 Owned by the orchestrator, before group B starts: create the follow-up issue with `ghbrk gh issue create --repo exasol-labs/exarrow-rs`. It names the hang of native Parquet import against Exasol 2025.2.0, the eleven tests of `decision-log.md` entry [15], the CI flags of task 7.1, and the cause check of entry [15] (hang on 2025.2.0 at this branch and at commit `95819035`, pass on 2025.2.1 and 2026.1.0). Pass the issue number to task 7.2.
- [ ] 7.2 Remove the bare `#[ignore]` attribute from every test in `tests/import_export_tests.rs` except `test_csv_export_runs_past_the_former_five_minute_limit`, and from `test_connect_with_nonexistent_uri_schema_succeeds`, `test_uri_schema_is_opened_on_connect`, and `test_uri_schema_missing_is_best_effort_via_adbc` in `tests/integration_tests.rs`.
  - Give the eleven native Parquet import tests that `decision-log.md` entry [15] lists `#[ignore = "native Parquet import hangs against Exasol 2025.2.0 (the CI image) and passes on 2025.2.1 and 2026.1.0, see #<issue from task 7.0>"]`.
  - Give `test_csv_export_runs_past_the_former_five_minute_limit` `#[ignore = "eight-minute opt-in check, run with EXARROW_LONG_EXPORT_CHECK=1"]`. Keep its `EXARROW_LONG_EXPORT_CHECK` early return. Rewrite its doc-comment sentence that says `import_export_tests` is not part of the CI job, so it states that CI runs the file and that the reasoned `#[ignore]` keeps this test out.
  - Update the module doc comments of both files, and the test doc comment that says "run with `--ignored`", so they no longer tell the reader to pass `--ignored`.
  - In `tests/driver_manager_tests.rs`, give the `#[ignore]` on `test_ffi_uri_schema_is_opened_on_connect` a reason that names the release cdylib it loads. Change the file's own `skip_if_no_exasol!` macro so that it panics with the message of the macro in `tests/common/mod.rs` ("REQUIRE_EXASOL is set but Exasol is not available at {host}:{port}") when `REQUIRE_EXASOL` is set, and skips otherwise.
  - Keep the reasoned `#[ignore]` on `test_terminated_export_session_is_reaped_server_side`.
  - Then run `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` against `exasol/docker-db:2025.2.0`. Every test that is not ignored passes, and no test runs longer than 60 seconds. If another test hangs or fails for a reason outside this plan, give it the same kind of reasoned `#[ignore]` and add it to entry [15].
- [ ] 7.3 Add `scripts/check_ci_test_targets.py`, using the Python standard library only. It takes the tests directory and the workflow file as arguments, with `tests` and `.github/workflows/ci.yml` as defaults. It reports each top-level `.rs` file under the tests directory whose stem does not appear in the workflow as `--test <stem>` followed by whitespace or the end of the line. It reports each line under the tests directory, subdirectories included, whose content without surrounding whitespace is exactly `#[ignore]`, as `<path>:<line>`. It prints one line per finding and exits 1 when it finds any, 0 otherwise (`decision-log.md` entry [16]).
- [ ] 7.4 Add `scripts/test_check_ci_test_targets.py` (`unittest`, temporary directories as fixtures) with cases: every target present exits 0; a missing target exits 1 and names the target; `--test integration_tests_extra` does not count as `--test integration_tests`; a bare `#[ignore]` exits 1 and names the file and line; `#[ignore = "reason"]` exits 0. The `unit-tests` job's existing `unittest discover` step runs it. Each test carries a docstring line `Scenario: CI rejects an integration test target that the integration job does not run`.
- [ ] 7.5 Add a step to the `lint` job of `.github/workflows/ci.yml` that runs `python3 scripts/check_ci_test_targets.py`.
- [ ] 7.6 Update `scripts/run_all_tests.sh` so that its integration and import-export stages run the same `cargo test` targets with the same features and flags as the CI `integration-tests` job, with `REQUIRE_EXASOL=1` and without `--ignored`.
- [ ] 7.7 Add a step to the `unit-tests` job of `.github/workflows/ci.yml` that runs `cargo test --lib --features websocket` without coverage instrumentation, because `AGENTS.md` § Coverage keeps the `websocket` feature out of the coverage command. CI then runs the WebSocket unit tests of task 3.6 (`decision-log.md` entry [5]).

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Paged fetch position and end of stream | 1.1-1.4, 2.1-2.2, 3.1-3.6, 4.1-4.3, 5.1 | none | spec deltas `query-execution/results-and-transactions`, `websocket-client/protocol`, `native-client/result-sets`; `src/query/results.rs` (`paginate_remaining`, `ResultSetIterator`, `from_transport_result`), `src/transport/websocket.rs` (`query_result_from_response`, `fetch_results`, `close_result_set`), `src/transport/native/mod.rs` (`convert_and_cache_result`, `fetch_results`), `src/transport/messages.rs` (`ResultSetData`, `FetchRequest`, `FetchResponseData`), `src/transport/test_support.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `tests/common/mod.rs`, `CHANGELOG.md` |
| B: Dependency advisories and CI test coverage | 6.1-6.8, 7.1-7.7 (7.0 by the orchestrator) | A (shares `tests/integration_tests.rs` and `CHANGELOG.md`) | spec deltas `code-quality/dependencies` and `code-quality/core`; architecture delta `architecture.md` (§ Constraints); `Cargo.toml`, `Cargo.lock`, `deny.toml`, `.github/workflows/ci.yml` (`lint`, `licenses`, `unit-tests`, `integration-tests` jobs), `scripts/check_ci_test_targets.py`, `scripts/test_check_ci_test_targets.py`, `scripts/run_all_tests.sh`, `tests/integration_tests.rs` (`test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates`, the three URI-schema tests), `tests/import_export_tests.rs`, `tests/driver_manager_tests.rs`, `tests/native_protocol_tests.rs`, `tests/native_transport_smoke_test.rs`, `benches/rust/generate_data.rs` (indicatif), `specs/mission.md` (§ Tech Stack), `CHANGELOG.md` |

- Group A: the three paged-fetch spec deltas describe one rule, where the next fetch starts and when the stream ends. The integration tests of each transport need the query-layer fix and that transport's fix, so a split would give several agents the `query-execution/results-and-transactions` delta and the same test files.
- Order inside group A: tasks 1.x, 2.x, and 3.x in any order with their unit tests, then 4.x, then 5.1.
- Group B: the dependency tasks and the CI tasks share `.github/workflows/ci.yml` and `tests/integration_tests.rs`, so one agent owns both. Group B runs after group A because both edit `tests/integration_tests.rs` and `CHANGELOG.md`, and because group B's final integration run then covers group A's tests under arrow 59.
- Order inside group B: the orchestrator runs task 7.0 first. Then 6.1 to 6.6, then 7.1 to 7.7, then 6.7 and 6.8 last, so the advisory re-check sees the final lockfile.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Code block | `src/query/results.rs` `paginate_remaining`: the inline empty-page and `known_total` checks | Moved into the shared end-of-stream helper (task 1.1) |
| Code block | `src/query/results.rs` `ResultSetIterator::fetch_next_batch`: the inline fetch and empty-page check | Replaced by the shared end-of-stream helper (task 1.3) |
| Code block | `src/transport/websocket.rs` `fetch_results`: the hardcoded start position `0` | Replaced by the per-handle position (task 3.3) |
| Code block | `src/transport/native/mod.rs` `convert_and_cache_result`: the ignored `rows_received: _` binding | Replaced by the position seed (task 2.1) |
| Config | `deny.toml`: the ignore entries and comments for `GHSA-2f9f-gq7v-9h6m` and `RUSTSEC-2024-0436` | `thrift` and `paste` leave the dependency tree with parquet 59 (task 6.3) |
| Test | `tests/integration_tests.rs` `test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates` | Replaced by the version 59 test (task 6.4) |
| Attribute | Bare `#[ignore]` on every test in `tests/import_export_tests.rs` and on three URI-schema tests in `tests/integration_tests.rs` | `skip_if_no_exasol!()` with `REQUIRE_EXASOL=1` gates them, and CI runs them (task 7.2) |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Small result set retrieval | Integration | `tests/integration_tests.rs` | `test_select_from_dual` (existing) |
| Small result set retrieval | Integration | `tests/websocket_integration_tests.rs` | `test_ws_select_from_dual` (existing) |
| Small result set retrieval (no fetch request clause) | Unit | `src/query/results.rs` | `test_fetch_all_small_result_set_sends_no_fetch` |
| Large result set pagination | Integration | `tests/integration_tests.rs` | `test_fetch_all_multi_fetch_returns_every_row_once` |
| Large result set pagination | Integration | `tests/websocket_integration_tests.rs` | `test_ws_fetch_all_multi_fetch_returns_every_row_once` |
| Result partly delivered with the execute response | Integration | `tests/integration_tests.rs` | `test_fetch_all_partial_inline_result_returns_every_row_once` |
| Result partly delivered with the execute response | Integration | `tests/integration_tests.rs` | `test_iterator_partial_inline_result_returns_every_row_once` |
| Result partly delivered with the execute response | Integration | `tests/websocket_integration_tests.rs` | `test_ws_fetch_all_partial_inline_result_returns_every_row_once` |
| Result partly delivered with the execute response | Integration | `tests/websocket_integration_tests.rs` | `test_ws_iterator_partial_inline_result_returns_every_row_once` |
| Result set iterator ends after the last row | Integration | `tests/integration_tests.rs` | `test_iterator_ends_after_last_row` |
| Result set iterator ends after the last row | Integration | `tests/websocket_integration_tests.rs` | `test_ws_iterator_ends_after_last_row` |
| Result set iterator ends after the last row (no fetch after the last row clause) | Unit | `src/query/results.rs` | `test_next_batch_stops_at_total_rows_without_another_fetch` |
| Prepared statement result is paged like a query result | Integration | `tests/integration_tests.rs` | `test_prepared_partial_inline_result_returns_every_row_once` |
| Prepared statement result is paged like a query result | Integration | `tests/websocket_integration_tests.rs` | `test_ws_prepared_partial_inline_result_returns_every_row_once` |
| Result set that ends before its total row count fails | Unit | `src/query/results.rs` | `test_fetch_all_fails_when_the_stream_ends_before_the_total` |
| Result set that ends before its total row count fails | Unit | `src/query/results.rs` | `test_next_batch_fails_when_the_stream_ends_before_the_total` |
| Result set that exceeds its total row count fails | Unit | `src/query/results.rs` | `test_fetch_all_fails_when_a_page_exceeds_the_total` |
| Result set that exceeds its total row count fails | Unit | `src/query/results.rs` | `test_next_batch_fails_when_a_page_exceeds_the_total` |
| Fetch results command | Unit | `src/transport/websocket.rs` | `test_fetch_results_starts_after_the_inline_rows_and_advances_per_page` |
| Fetch results command | Unit | `src/transport/websocket.rs` | `test_prepared_statement_fetch_starts_after_the_inline_rows` |
| Fetch results command | Integration | `tests/websocket_integration_tests.rs` | `test_ws_fetch_all_multi_fetch_returns_every_row_once` |
| Large result set (multi-fetch) | Unit | `src/transport/native/mod.rs` | `inline_rows_seed_the_fetch_position_of_a_large_result_set` |
| Large result set (multi-fetch) | Integration | `tests/integration_tests.rs` | `test_fetch_all_partial_inline_result_returns_every_row_once` |
| Large result set (multi-fetch) | Integration | `tests/integration_tests.rs` | `test_fetch_all_multi_fetch_returns_every_row_once` |
| Advisory CI gate blocks merge on unacknowledged advisory | Integration | `.github/workflows/ci.yml` (`licenses` job) | step running `cargo deny --all-features check advisories` (task 6.5) |
| Suppression is removed when its advisory no longer applies | Integration | `.github/workflows/ci.yml` (`licenses` job) and `deny.toml` | `cargo deny --all-features check advisories` with `unused-ignored-advisory = "deny"` (task 6.3) |
| Apache Thrift is absent from the dependency tree | Integration | `tests/integration_tests.rs` | `test_arrow_and_parquet_resolve_to_59_with_one_version_per_arrow_crate` |
| Arrow and Parquet dependencies resolve to version 59 or above with one version of each Arrow sub-crate | Integration | `tests/integration_tests.rs` | `test_arrow_and_parquet_resolve_to_59_with_one_version_per_arrow_crate` |
| Every integration test target runs in the CI integration job | Integration | `.github/workflows/ci.yml` (`integration-tests` job) | the job's `cargo test --test <target>` steps (tasks 7.1, 7.2), checked by `scripts/check_ci_test_targets.py`; the fail-not-skip step through the `skip_if_no_exasol!` macros of `tests/common/mod.rs` and `tests/driver_manager_tests.rs`, which panic under `REQUIRE_EXASOL=1` (task 7.2) |
| CI rejects an integration test target that the integration job does not run | Unit | `scripts/test_check_ci_test_targets.py` | the five cases of task 7.4 |
| CI rejects an integration test target that the integration job does not run | Integration | `.github/workflows/ci.yml` (`lint` job) | step running `python3 scripts/check_ci_test_targets.py` (task 7.5) |

- The CI and dependency scenarios are checked by commands that CI runs, as the earlier advisory-policy plan did. `test_arrow_and_parquet_resolve_to_59_with_one_version_per_arrow_crate` is a plain `#[test]` that reads `Cargo.lock` and needs no Exasol.
- The scenario "Minor or major dep bump requires explicit evaluation" is unchanged. plan.md § Dependencies gives the reasons that the PR description lists.
- The two row-count mismatch scenarios use unit tests only. A correct Exasol server does not deliver a row count that differs from its total, so only a transport double can produce the condition.
- The start-position clauses of "Fetch results command" and "Large result set (multi-fetch)" also have unit tests, because the start position is not observable through the public API. The integration tests check the result of a correct start position: every row exactly once.
- "Small result set retrieval" keeps its existing integration tests. Its no-fetch clause is checked by a new unit test whose `MockTransport` expects zero fetches.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| query-execution/results-and-transactions | `REQUIRE_EXASOL=1 cargo test --test integration_tests test_iterator_partial_inline_result_returns_every_row_once -- --nocapture` | The iterator yields 70 rows with the values 1 to 70 once each, then `None`, and the test passes |
| websocket-client/protocol | `REQUIRE_EXASOL=1 cargo test --features websocket --test websocket_integration_tests test_ws_fetch_all_multi_fetch_returns_every_row_once -- --nocapture --test-threads=1` | `fetch_all` returns 70,000 rows in at least two non-empty batches with 70,000 distinct values, and the test passes |
| native-client/result-sets | `REQUIRE_EXASOL=1 cargo test --test integration_tests test_fetch_all_partial_inline_result_returns_every_row_once -- --nocapture` | The first batch holds between 1 and 69 rows, `fetch_all` returns 70 rows with 70 distinct values, and the test passes |
| code-quality/dependencies | `cargo deny --all-features check advisories && cargo tree --all-features -i thrift` | `advisories ok` with no warning, then `error: package ID specification 'thrift' did not match any packages` |
| code-quality/core | `python3 scripts/check_ci_test_targets.py; echo $?` | No finding lines, then `0` |
| code-quality/core | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures, and only the tests with a stated `#[ignore]` reason (eleven native Parquet import tests and `test_csv_export_runs_past_the_former_five_minute_limit`) are ignored |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (WebSocket only) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Unit test | `cargo test --lib` | 0 failures |
| Unit test (WebSocket) | `cargo test --lib --features websocket` | 0 failures |
| Integration test (native) | `REQUIRE_EXASOL=1 cargo test --test integration_tests -- --test-threads=1` | 0 failures, no skips |
| Integration test (WebSocket) | `REQUIRE_EXASOL=1 cargo test --features websocket --test websocket_integration_tests -- --test-threads=1` | 0 failures, no skips |
| Driver manager test | `REQUIRE_EXASOL=1 cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures, run after the FFI build |
| Import/export test | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures, 12 ignored with a stated reason, finishes within the step timeout of 10 minutes |
| Native protocol test | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1` | 0 failures |
| Native smoke test | `REQUIRE_EXASOL=1 cargo test --features ffi --test native_transport_smoke_test -- --test-threads=1` | 0 failures |
| Benchmark build | `cargo build --features benchmark --bins` | Exit 0 |
| Advisories | `cargo deny --all-features check advisories` | `advisories ok`, no warnings |
| Licenses | `cargo deny check licenses && cargo deny --all-features check licenses` | `licenses ok` twice |
| Duplicate Arrow crates | `cargo tree --all-features -d` | No `arrow-*` crate listed |
| CI test targets | `python3 scripts/check_ci_test_targets.py` | Exit 0, no output |
| Script tests | `python3 -m unittest discover --start-directory scripts --pattern 'test_*.py'` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Check passes: total production coverage at least 80%, every file at least 50% |
