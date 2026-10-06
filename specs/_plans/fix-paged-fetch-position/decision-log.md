# Decision Log: fix-paged-fetch-position

## Interview

Headless run. No live interview took place. The orchestrator brief is the only input, summarized here as Q/A pairs.

**Q:** What is in scope?
**A:** GitHub issue exasol-labs/exarrow-rs#80, "Paged result fetch returns duplicated/missing rows without error". The issue text is the intent. It names three causes: (1) the WebSocket transport sends fetch start position 0 on every fetch (`src/transport/websocket.rs`, `fetch_results`) and keeps no per-handle fetch position; (2) neither transport counts the rows delivered with the execute response (native `convert_and_cache_result` drops `rows_received`, and the WebSocket transport ignores `num_rows_in_message`), so the first fetch starts at row 0 instead of after those rows; (3) `ResultSetIterator::fetch_next_batch` (`src/query/results.rs`) stops only on an empty page and never compares against `metadata.total_rows`, unlike `paginate_remaining`.

**Q:** What fix does the issue suggest?
**A:** The WebSocket transport keeps a per-handle fetch position, seeds it with the rows delivered with the execute response, advances it by the rows of each fetch, and removes it in `close_result_set`. The native transport seeds `fetch_positions` with `rows_received` for handles other than `SMALL_RESULTSET`. The iterator marks the stream complete when the rows it has yielded reach `metadata.total_rows`.

**Q:** Which tests are required?
**A:** Unit tests that check the fetch start position, because the current mocks do not. Integration tests: WebSocket multi-chunk `fetch_all`, partial-inline `fetch_all`, and iterator end-of-stream; native partial-inline `fetch_all` and iterator end-of-stream.

**Q:** Are prepared statements in scope?
**A:** Yes, if they share the code path. They do: see entry [6].

**Q:** Is an architecture change expected?
**A:** Not for the paged-fetch fix. See entry [8]. The scope additions below change § Constraints, see entries [11], [13], and [15].

**Q:** (Scope addition 1) Which advisories must the plan fix?
**A:** The CVEs found in the project. Open Dependabot alerts: #20, `xxhash-rust` 0.8.15, GHSA-6g2r-675j-hx59 (low), fixed in 0.8.16, present only through `polars-core` 0.46 in the all-features tree; #19, `thrift` 0.17.0, CVE-2026-43868 / GHSA-2f9f-gq7v-9h6m (medium), fixed in 0.23.0, present through `parquet` 58, suppressed in `deny.toml` per ADR-003. If a real `thrift` fix is available, plan it. If `parquet`'s `^0.17` pin still blocks it, re-evaluate the trigger, update the `deny.toml` reason, and keep the suppression. Re-check `cargo deny check advisories` and the Dependabot alerts at implementation time. Follow `code-quality/dependencies`, add a changelog line, keep the plan name, and name the CVE scope in plan.md § Impact.

**Q:** (Scope addition 2) Which tests must CI run?
**A:** The new tests must run in CI. The `integration-tests` job runs `integration_tests`, `websocket_integration_tests`, `driver_manager_tests`, and the Python tests. It does not run `import_export_tests`, `native_protocol_tests`, or `native_transport_smoke_test`. Add a CI step for `import_export_tests` with `REQUIRE_EXASOL=1` and `--test-threads=1`, gating explicitly any test that cannot run in CI. Decide for `native_protocol_tests` and `native_transport_smoke_test`. Add a `code-quality/core` delta stating that every integration test file under `tests/` runs in the CI integration job. Add a check that fails when a `tests/*.rs` file is absent from `ci.yml`, if it is cheap. Confirm that the paged-fetch tests run with `--test-threads=1` and fit the 30-minute job timeout.

## Design Decisions

### [1] Each transport keeps its own per-handle fetch position

- **Decision:** `WebSocketTransport` gets a `fetch_positions: HashMap<i32, i64>` map, the same shape as the native transport's existing `fetch_positions`. Each transport seeds the position when an execute response returns a result set handle, reads it to build the next fetch request, advances it after each fetch, and removes it in `close_result_set`.
- **Alternatives:** (a) The query layer passes the start position to `TransportProtocol::fetch_results(handle, start)`. Rejected: `TransportProtocol` is a public trait, so a new parameter breaks every external implementor and every caller for a bug fix. The transports also already own the handle lifecycle (execute, fetch, close), and the native transport already tracks the position. (b) A shared cursor type in the transport core that both transports use. Rejected: it would wrap a `HashMap` with four one-line methods, a shallow module whose interface costs as much as the map it hides. Each transport's spec scenario and unit test pins its own start-position rule.
- **Rationale:** The fix matches the existing native design, so both transports hold the same state in the same shape. A per-handle map, not a single counter, keeps positions correct when several result sets are open on one connection and their fetches interleave.
- **Consequences:** The WebSocket transport removes the position before it sends the close command, as the native transport does, so a failed close does not leave a stale entry. A later execute response that reuses a handle number overwrites the old entry.
- **Promotes to ADR:** no

### [2] The WebSocket transport counts the rows it parsed, not the `numRowsInMessage` field

- **Decision:** `query_result_from_response` seeds the position with the number of rows parsed from the execute response's `data`. `fetch_results` advances the position by the number of rows parsed from the fetch response's `data`. The method takes `&mut self` so it can seed the map.
- **Alternatives:** (a) Seed with `numRowsInMessage` and advance by the fetch response's `numRows`, as the issue suggests. Rejected: `ResultSetData::num_rows_in_message` is an `Option`, so it needs a fallback that would be the parsed row count anyway. Two sources for the same number can disagree.
- **Rationale:** The parsed row count is the number of rows the caller receives. `ResultSet::from_transport_result` and the row-count check in entry [4] count the same rows, so the transport's position and the query layer's row count agree by construction. Exasol's WebSocket API documents `startPosition` as a 0-based row offset and `numRows` as the number of rows fetched, so on a correct server both counts are equal.
- **Promotes to ADR:** no

### [3] The native transport seeds the position in `convert_and_cache_result`

- **Decision:** `convert_and_cache_result` binds `rows_received` and inserts it into `fetch_positions` for every handle other than `SMALL_RESULTSET`, in the same branch that caches the column metadata.
- **Alternatives:** (a) Seed in `execute_query` and `execute_prepared_statement` separately. Rejected: both already route their response through `convert_and_cache_result`, so one insertion covers both, and two copies could drift apart.
- **Rationale:** `fetch_results` already reads the position from `fetch_positions` and advances it by `rows_received`. Only the seed is missing. `SMALL_RESULTSET` (-3) means the server closed the result set after the execute response, so it gets no position, the same rule that already skips its column cache.
- **Promotes to ADR:** no

### [4] One end-of-stream rule serves `fetch_all` and the iterator, and a row-count mismatch is an error

- **Decision:** A private async helper in `src/query/results.rs` fetches the page after a given number of received rows. `paginate_remaining` and `ResultSetIterator::fetch_next_batch` both call it. When the total row count is greater than zero, the helper returns "no further page" without a fetch once the rows received reach the total. It returns `QueryError::ExecutionFailed` when a fetch returns no rows before the total is reached, or when a page brings the rows received above the total. The error message states the rows received and the total. When the total row count is zero, the helper keeps today's rule and ends on the first empty page. After a mismatch error, the iterator marks itself complete, so the next call returns `None` and sends no further fetch.
- **Alternatives:** (a) Add the total check to `fetch_next_batch` only, as the issue suggests. Rejected: two read paths would keep two copies of the end-of-stream rule, and the divergence of these copies is cause (3) of issue #80. (b) Stop at the total without a mismatch error. Rejected: the issue's defect class is wrong data returned without an error. With the check, a remaining or future position defect fails loudly. Symptom 1 (1,309,444 rows for 1,000,000) and symptom 2 (135 rows for 70) both exceed the total and would have failed instead of returning wrong data. (c) Return a new `QueryError` variant. Rejected: `QueryError` is a public enum without `#[non_exhaustive]`, so a new variant breaks external exhaustive matches. `ExecutionFailed` already carries fetch failures on these paths.
- **Rationale:** Exasol reports the exact total row count with every result set: the native result set header carries total rows, and the WebSocket API documents `numRows` as the number of rows in the result set. The existing unit tests that use a total of zero keep their empty-page behavior.
- **Consequences:** `fetch_all` closes the result set handle and ignores a close error whether pagination succeeds or fails, then returns the pagination result. A mismatch is detected after a complete response, so the transport is in sync and the close releases the server's result set. The iterator sends no fetch after the last row, so the native "Invalid result set seek" error and the WebSocket endless loop both end. `paginate_remaining` keeps holding the transport lock for one fetch at a time, as today.
- **Promotes to ADR:** no

### [5] Unit tests check the start position on each transport, and the WebSocket check uses a fake server

- **Decision:** The native unit test calls `convert_and_cache_result` on a fresh `NativeTcpTransport` and asserts the recorded start position. The WebSocket unit tests run the real `execute_query`, `execute_prepared_statement`, and `fetch_results` against a local fake WebSocket server in `src/transport/test_support.rs`. The fake server accepts one plain `ws://` connection through `tokio_tungstenite::accept_async`, answers each request with the next scripted JSON response, and records each request. The test asserts the `startPosition` field of each recorded fetch request. The query-layer unit tests use `MockTransport`, as the existing ones do.
- **Alternatives:** (a) A fake native server. Rejected: it needs the native handshake, RSA login, and ChaCha20 framing, which costs more than the defect it guards. The native integration tests cover the wire behavior. (b) Test-only accessors on `WebSocketTransport` that expose the position map. Rejected: the fake server checks the field Exasol reads, without a seam that exists only for tests.
- **Rationale:** Issue #80 notes that the unit tests pass with and without the defect because no mock checks the start position. `test_support.rs` is the crate's home for shared wire fakes and stays out of the production coverage denominator.
- **Consequences:** The WebSocket unit tests compile only with the `websocket` feature. The CI unit job runs `cargo llvm-cov --lib` with default features, so plan.md task 7.7 adds a `unit-tests` step that runs `cargo test --lib --features websocket` without coverage instrumentation. CI then runs the WebSocket unit tests, including the only test that interleaves two open handles. `AGENTS.md` § Coverage keeps the `websocket` feature out of the coverage command, so the coverage numbers do not change.
- **Promotes to ADR:** no

### [6] Prepared statements need no separate code change

- **Decision:** The fix adds no prepared-statement code. One integration test per transport executes a prepared `SELECT` whose result is partly delivered with the execute response and checks every row.
- **Alternatives:** none
- **Rationale:** WebSocket `execute_prepared_statement` and `execute_query` both build their result through `query_result_from_response`. Native `execute_prepared_statement` and `execute_query` both go through `convert_and_cache_result`. `Connection::execute_prepared` builds its `ResultSet` with `ResultSet::from_transport_result`, as `Connection::execute` does. The ADBC FFI statement reads results through `ResultSet::fetch_all` (`src/adbc_ffi.rs`), so the FFI path needs no separate test.
- **Promotes to ADR:** no

### [7] Integration tests use the issue's server-generated queries and assert that the paged path ran

- **Decision:** The tests use `VALUES BETWEEN` queries, which need no table. The multi-fetch tests read 70,000 rows of about 1,000 bytes. The partial-inline tests read 70 rows of 1,000,000 bytes. The iterator end-of-stream tests read 5,000 short rows. Each multi-fetch test asserts at least two non-empty batches. Each partial-inline test asserts that the first batch holds between 1 and 69 rows. Each test asserts that the key column holds every value from 1 to N exactly once. The iterator tests are synchronous `#[test]` functions that build a multi-thread Tokio runtime, connect and execute inside `Runtime::block_on`, then call `next_batch()` while a `Runtime::enter()` guard is held. The loop stops after 100 calls and fails the test when it reaches that cap.
- **Alternatives:** (a) Lower the fetch size to shrink the test data. Rejected: the fetch size comes from the session's maximum data message size, which no connection parameter sets. The execute response's inline size is chosen by the server. (b) Call `next_batch()` inside `#[tokio::test]`. Rejected: `next_batch()` calls `Handle::block_on`, which panics inside an async context. On a current-thread runtime, `Handle::block_on` also cannot drive socket I/O.
- **Rationale:** The sizes are the smallest that cross the default 64 MiB data message size, which the CI image `exasol/docker-db:2025.2.0` uses. The batch-shape assertions make a test fail, not pass silently, if a server configuration stops exercising the paged path. Each partial-inline or multi-fetch test moves about 70 MB over the local connection.
- **Consequences:** Ten new integration tests, five per transport. They follow each file's `skip_if_no_exasol!()` convention and run with `REQUIRE_EXASOL=1` in CI.
- **Promotes to ADR:** no

### [8] The paged-fetch fix changes no architecture and needs no ADR

- **Decision:** Entries [1] to [7] add nothing to the architecture delta and promote nothing to an ADR.
- **Alternatives:** none
- **Rationale:** The fetch position is internal state of each transport. No component, boundary, interface, data flow, constraint, or external dependency changes for the paged-fetch fix, and `TransportProtocol` keeps its signatures. `speq decision-log show` lists ADR-001 (placeholder lexer), ADR-003 (Thrift advisory), ADR-004 (zero-row result schema), ADR-005 (batch arity check), ADR-006 (URI schema), the query-timeout and client-side give-up ADRs, and the transport Parquet export schema ADR. None covers result-set paging. The plan conforms to ADR-004: `fetch_all` and the iterator still emit the execute response's batch first, also when it holds zero rows. Entries [1] to [7] are bug-fix details or statements that live in a spec scenario, which `/speq:adr-rules` rule 3 excludes. The architecture delta in this plan comes from entries [11], [13], and [15].
- **Architecture:** no change: fetch positions are internal transport state, and no component, boundary, interface, data flow, constraint, or external dependency changes for the paged-fetch fix
- **Promotes to ADR:** no

### [9] Existing scenario wording follows the partly inline case

- **Decision:** "Small result set retrieval" applies only to results with fewer than 1,000 rows that fit within the server's maximum data message size, and states that no fetch request is sent. "Large result set pagination" loses the step "it SHALL provide mechanisms to retrieve subsequent batches" and gains the start-position and exactly-once steps.
- **Alternatives:** (a) Leave "Small result set retrieval" unchanged. Rejected: it requires every result below 1,000 rows to arrive in one request, which contradicts the new scenario "Result partly delivered with the execute response" and the server behavior in symptom 2.
- **Rationale:** The removed step repeats "it SHALL support fetching results in batches". Removing it keeps the scenario within the validator's recommended three AND steps.
- **Promotes to ADR:** no

### [10] Changelog entry under `[Unreleased]`

- **Decision:** `CHANGELOG.md` gets a `## [Unreleased]` section above `## 0.17.0` with `Breaking:`, `Fix:`, `Security:`, and `Changed:` entries for this plan. The release that carries this change is a 0.x minor bump over the latest published version. If tag `v0.17.0` does not exist when this PR merges, the implement step keeps `version = "0.17.0"` and folds `[Unreleased]` into `## 0.17.0`. If `v0.17.0` exists, it sets `version = "0.18.0"` and folds `[Unreleased]` into `## 0.18.0`. It never bumps the patch component.
- **Alternatives:** (a) Let the implement step bump by Conventional Commit type. Rejected: a plan named `fix-...` gets a patch bump, and a patch release on arrow 59 after 0.17.0 on arrow 58 would reach every dependent on `exarrow-rs = "0.17"` through `cargo update` and break code that passes Arrow values between the crates.
- **Rationale:** `AGENTS.md` requires the changelog update in the same PR as a user-facing change, and puts entries of a PR without a version bump under `## [Unreleased]`. Cargo treats versions with the same 0.x minor component as compatible, so the arrow 59 upgrade (entry [11]) needs a new 0.x minor version. `CONTRIBUTING.md` § Releasing requires a SemVer bump. Version 0.17.0 is in `Cargo.toml` and `CHANGELOG.md` but has no tag yet (latest tag `v0.16.0`), because it comes from the unmerged branch `feat/fix-export-parquet-transport-roundtrip`, on which this branch is stacked.
- **Promotes to ADR:** no

### [11] Remove the Apache Thrift advisory by upgrading parquet, not by suppressing it

- **Decision:** exarrow-rs removes the Apache Thrift advisory (GHSA-2f9f-gq7v-9h6m) by upgrading to the first parquet release line without a thrift dependency, together with the arrow and adbc releases that accept it, instead of suppressing the advisory. arrow and parquet stay on one major version that adbc_core accepts.
- **Alternatives:** (a) Keep the suppression. Rejected: a fixed release line exists, and the advisory would stay open. (b) Move arrow past the range that adbc_core accepts. Rejected: adbc_ffi passes Arrow arrays across the C ABI, so both must share one Arrow version. (c) Patch thrift under the current parquet. Rejected: parquet's thrift requirement excludes the fixed release.
- **Rationale:** `/speq:adr-rules` rule 2, criterion 3: a major dependency and security choice. Rule 5: the decision contradicts ADR-003, so it supersedes it. Search: `speq decision-log show` lists ADR-003 as the only ADR about dependencies. The re-evaluation trigger of ADR-003 has fired. Entry [18] holds the versions and the trial-build evidence.
- **Consequences:**
  - A downstream crate that exchanges Arrow values with exarrow-rs must move to the same Arrow major version.
  - An Arrow major upgrade waits until adbc_core accepts it.
- **Supersedes:** suppress-ghsa-2f9f-gq7v-9h6m-via-deny-toml
- **Architecture:** § Constraints
- **Promotes to ADR:** yes

### [12] Advisories in optional-feature dependencies are fixed by updates, not suppressions

- **Decision:** `Cargo.lock` moves `xxhash-rust` from 0.8.15 to 0.8.19 (GHSA-6g2r-675j-hx59, Dependabot alert #20) and `crossbeam-epoch` from 0.9.18 to 0.9.21 (RUSTSEC-2026-0204, invalid pointer dereference). `Cargo.toml` moves `indicatif` from 0.17 to 0.18, which replaces the unmaintained `number_prefix` (RUSTSEC-2025-0119) with `unit-prefix`.
- **Alternatives:** (a) Suppress these advisories, because only the optional `benchmark` feature pulls them in. Rejected: patched versions exist, and Dependabot reports the lockfile regardless of features. (b) Upgrade polars. Rejected: polars-core 0.46 requires `xxhash-rust ^0.8.6`, and rayon accepts crossbeam-epoch 0.9.21, so lockfile updates suffice.
- **Rationale:** `cargo deny --all-features check advisories` on the current tree fails on RUSTSEC-2026-0204 and RUSTSEC-2025-0119. The CI gate runs default features and passes, so CI did not report them. The trial build in entry [11] includes these three updates, and the `benchmark` binaries compile unchanged with indicatif 0.18.
- **Consequences:** The `xxhash-rust` and `crossbeam-epoch` changes are patch-level lockfile updates. The indicatif change is a minor bump of an optional dependency, so the PR description names it with its reason (scenario "Minor or major dep bump requires explicit evaluation").
- **Promotes to ADR:** no

### [13] The CI advisory gate checks the dependencies of every Cargo feature

- **Decision:** The `licenses` job runs `cargo deny --all-features check advisories` instead of `cargo deny check advisories`. `deny.toml` sets `unused-ignored-advisory = "deny"`, so an ignore entry that matches no crate fails the gate.
- **Alternatives:** (a) Keep the default-feature check. Rejected: Dependabot alert #20 and RUSTSEC-2026-0204 live only in optional-feature dependencies, so the gate passed while both were open. (b) Set `all-features = true` under `[graph]` in `deny.toml`. Rejected: it also widens the licenses check, which this plan does not need to change.
- **Rationale:** Dependabot scans `Cargo.lock`, which holds the dependencies of every feature. The gate now checks the same set.
- **Consequences:** An advisory in a benchmark-only or FFI-only dependency blocks a merge. The `code-quality/dependencies` Background and the scenario "Advisory CI gate blocks merge on unacknowledged advisory" state the command. The `unused-ignored-advisory` level enforces the scenario "Suppression is removed when its advisory no longer applies". Today the stale GHSA-2f9f-gq7v-9h6m ignore produces only warnings.
- **Promotes to ADR:** no

### [14] The implementer re-checks advisories before the change is complete

- **Decision:** The dependency task ends with `cargo deny --all-features check advisories` and a listing of open Dependabot alerts (`ghbrk gh api "repos/exasol-labs/exarrow-rs/dependabot/alerts?state=open"`). An advisory that appears after this plan is fixed by an update, or suppressed per `code-quality/dependencies`, in the same change.
- **Alternatives:** none
- **Rationale:** The advisory databases change between planning and implementation. Planning found two advisories that the Dependabot list did not show (RUSTSEC-2026-0204, RUSTSEC-2025-0119).
- **Consequences:** Dependabot closes alerts #19 and #20 only after the change reaches the default branch. Until then the alerts stay open.
- **Promotes to ADR:** no

### [15] CI runs every integration test target, and only a reasoned `#[ignore]` excludes a test

- **Decision:** The `integration-tests` job gains steps for `import_export_tests`, `native_protocol_tests`, and `native_transport_smoke_test`, each with `REQUIRE_EXASOL=1` and `--test-threads=1`. The bare `#[ignore]` attributes in `tests/import_export_tests.rs` and `tests/integration_tests.rs` are removed, because `skip_if_no_exasol!()` already gates these tests and `REQUIRE_EXASOL=1` turns a missing database into a failure. A test that cannot run in CI keeps `#[ignore = "<reason>"]`, as `test_terminated_export_session_is_reaped_server_side` does. `driver_manager_tests` keeps `--include-ignored`, and its one bare `#[ignore]` gets a reason.
- **Alternatives:** (a) Run `import_export_tests` with `--include-ignored`. Rejected: every test in the file carries a bare `#[ignore]`, so the flag would also run any future test ignored for a stated reason, and the bare attribute keeps hiding the tests from a plain `cargo test`. (b) Leave `native_protocol_tests` and `native_transport_smoke_test` out of CI. Rejected: both connect to Exasol on `localhost:8563`, which the CI container provides.
- **Rationale:** The import/export tests use the client-mode HTTP tunnel. The client opens every connection, and Exasol answers through the connection on port 8563, so the tests need no inbound connection to the runner. `native_protocol_tests` runs with `--features 'ffi websocket'` so its WebSocket comparison tests run and the step reuses the WebSocket step's build. `native_transport_smoke_test` and `import_export_tests` run with `--features ffi`. A planning run against a local `exasol/docker-db:2025.2.0` container, started as the CI job starts it, ran each target with `REQUIRE_EXASOL=1` and `--test-threads=1`. `native_protocol_tests` passed 14 of 14, `native_transport_smoke_test` 4 of 4, and the three URI-schema tests 3 of 3. In `import_export_tests` (all tests included), every CSV, Arrow, TLS, and Parquet export test passed, and the two forced-CSV Parquet import tests passed. Eleven tests that import Parquet through the native path hung until a 180-second timeout, over `localhost` and over `127.0.0.1`: `test_parallel_parquet_import_mixed_batch_sizes`, `test_parallel_parquet_import_native_path`, `test_parallel_parquet_import_two_files`, `test_parquet_import_auto_create_existing_table`, `test_parquet_import_auto_create_multi_file`, `test_parquet_import_auto_create_sanitized_names`, `test_parquet_import_auto_create_table`, `test_parquet_import_from_file`, `test_parquet_import_native_path_when_supported`, `test_parquet_round_trip`, and `test_parquet_stream_import_native_path`.
  - Cause check: `test_parquet_import_from_file` and `test_parquet_round_trip` ran with `REQUIRE_EXASOL=1` and a 150-second limit per test in five configurations. Each run reached the test.
    - CI flags of task 7.1 (`--features ffi`, `--test-threads=1`) on `exasol/docker-db:2025.2.0`: both hang.
    - No `--features ffi` and no `--test-threads=1` on 2025.2.0, both tests in parallel: both hang.
    - CI flags on `exasol/docker-db:2025.2.1`: both pass, in 17 and 4 seconds.
    - CI flags on `exasol/docker-db:2026.1.0`: both pass, in 3 and 2 seconds.
    - Commit `95819035` (built from `git archive`) with plan 002's command `cargo test --test import_export_tests -- --ignored` on 2025.2.0: both hang.
    - Recorded runs: plan 002 (`specs/_recorded/002-fix-thrift-cve-upgrade-arrow-58/verification-report.md`) reports 40 passed and 2 hung on 2025.2.0 at commit `95819035`. Plan 008 reports 51 of 51 passed without naming the image. `AGENTS.md` starts `exasol/docker-db:latest`, which is 2026.1.0 on the planning host.
    - Result: the hang depends on the server version. Neither a CI flag nor a driver change after commit `95819035` causes it, because the same commit hangs on 2025.2.0 today. The planning runs do not explain plan 002's 40 passes on 2025.2.0.
- **Consequences:**
  - The eleven hanging tests get `#[ignore = "native Parquet import hangs against Exasol 2025.2.0 (the CI image) and passes on 2025.2.1 and 2026.1.0, see #<issue>"]`. CI does not run them while it uses 2025.2.0. The orchestrator files the follow-up issue (plan.md task 7.0). Fixing the hang, or moving the CI image to a version on which the tests pass, belongs to that issue.
  - A user on Exasol 2025.2.0 who imports Parquet without forcing the CSV path waits without an error, because `supports_native_parquet_import` sends that version to the native path. plan.md § Impact states it.
  - `test_csv_export_runs_past_the_former_five_minute_limit` gets `#[ignore = "eight-minute opt-in check, run with EXARROW_LONG_EXPORT_CHECK=1"]` instead of losing its `#[ignore]`. It keeps its `EXARROW_LONG_EXPORT_CHECK` early return.
  - The `skip_if_no_exasol!` macro of `tests/driver_manager_tests.rs` panics under `REQUIRE_EXASOL=1`, as the macro in `tests/common/mod.rs` does, so that target also fails instead of skipping when Exasol is unavailable.
  - The import/export CI step gets `timeout-minutes: 10`, so a future hang fails that step instead of the whole 30-minute job.
  - The three URI-schema tests in `tests/integration_tests.rs` (`test_connect_with_nonexistent_uri_schema_succeeds`, `test_uri_schema_is_opened_on_connect`, `test_uri_schema_missing_is_best_effort_via_adbc`) run in CI for the first time.
  - `scripts/run_all_tests.sh` runs the same targets with the same flags, so a local run matches CI.
  - The module doc comments of both test files drop the instruction to run with `--ignored`.
- **Architecture:** § Constraints
- **Promotes to ADR:** no

### [16] A Python script checks that CI runs every test target

- **Decision:** `scripts/check_ci_test_targets.py` lists the top-level `.rs` files under `tests/`. It fails when `.github/workflows/ci.yml` has no `--test <file stem>` for one of them, or when a file under `tests/` holds an `#[ignore]` attribute without a reason. It prints each finding. `scripts/test_check_ci_test_targets.py` unit-tests it, and the `lint` job runs it.
- **Alternatives:** (a) A Rust test that reads `ci.yml`. Rejected: it would put a check of workflow text into the product's test suite. (b) An inline `grep` step in the workflow. Rejected: it cannot be unit-tested, and the repository already keeps CI helpers as Python scripts with `unittest` tests (`scripts/strip_test_coverage.py`), which the `unit-tests` job discovers.
- **Rationale:** The check costs one short script, needs neither Exasol nor a Rust build, and turns a target that CI forgot into a failed lint job.
- **Consequences:** Adding a test file under `tests/` requires a CI step in the same change.
- **Promotes to ADR:** no

### [17] The integration job keeps its 30-minute timeout

- **Decision:** The `integration-tests` job keeps `timeout-minutes: 30`, and every new step runs with `--test-threads=1`. The import/export step has its own `timeout-minutes: 10` (entry [15]). The paged-fetch tests run in `integration_tests` and `websocket_integration_tests`, which the job already runs with `--test-threads=1`.
- **Alternatives:** (a) Raise the timeout. Rejected: the measured total stays well below 30 minutes.
- **Rationale:** The last successful run of the job (run 30378579965, 2026-07-28) took 6 minutes 34 seconds, of which the Exasol container start and readiness wait took 2 minutes 33 seconds. In the planning run on a 4-CPU host, the import/export tests that pass took about 30 seconds of test time, `native_protocol_tests` 1.7 seconds, `native_transport_smoke_test` 0.2 seconds, and the three URI-schema tests 0.9 seconds. Each new target also compiles once, which took 20 to 30 seconds per target on that host. The ten paged-fetch tests each move at most about 70 MB over the local connection. Estimate, not measured on the CI runner: the job grows by 3 to 6 minutes and stays below 15 minutes.
- **Promotes to ADR:** no

### [18] Versions and lockfile steps of the parquet upgrade

- **Decision:** `Cargo.toml` requires arrow 59, parquet 59.2 or later within 59.x, and adbc_core, adbc_ffi, and adbc_driver_manager 0.24 (plan.md tasks 6.1 and 6.6). `deny.toml` drops the ignores for GHSA-2f9f-gq7v-9h6m and RUSTSEC-2024-0436 (task 6.3).
- **Alternatives:** (a) arrow and parquet 60. Rejected: adbc_core 0.24 accepts arrow-array and arrow-schema `>=58, <60`. (b) Keep the requirement `parquet = "59"`. Rejected: it admits 59.0.0 and 59.1.0, which still depend on `paste`, so the advisory gate would fail on RUSTSEC-2024-0436 if the lockfile resolved one of them.
- **Rationale:** The crates.io index lists no `thrift` dependency for any parquet 59.x release (apache/arrow-rs#9962, released in 59.0.0 on 2026-06-09), and `paste ^1.0` for 59.0.0 and 59.1.0 but not for 59.2.0 and 59.3.0. adbc_core 0.24.0 (2026-07-28) is the first release that accepts Arrow 59. parquet 58.4.0 still requires `thrift ^0.17`. A trial build in a scratch copy with arrow and parquet 59.3.0, adbc 0.24.0, indicatif 0.18, and the updated lockfile compiled every target with every feature without source changes, passed clippy without warnings, passed 1,596 unit tests (1,624 with `websocket`), and passed `cargo deny --all-features check advisories` and `check licenses`.
- **Consequences:**
  - After the manifest change, Cargo keeps adbc_core on the locked arrow 58.3.0 although adbc_core accepts 59. Task 6.1 unifies the Arrow sub-crates explicitly.
  - `specs/mission.md` § Tech Stack names arrow 59, parquet 59, and adbc 0.24 (task 6.6).
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] The native Parquet import hang was attributed without a cause check

- **Finding:** Round 2 `[UNSTATED_ASSUMPTION]`: the plan kept eleven tests out of CI on the belief that their hang was a separate defect. It did not establish the cause, and plan 002 reports nine of these tests passing on 2025.2.0.
- **Direction change:** The planner ran two of the tests on 2025.2.0 with and without the CI flags, on 2025.2.1, on 2026.1.0, and at commit `95819035` on 2025.2.0. The tests hang on 2025.2.0 in every configuration and pass on 2025.2.1 and 2026.1.0, so the hang depends on the server version. Entry [15] records the results. The `#[ignore]` reason in task 7.2 names Exasol 2025.2.0, plan.md § Impact states that native Parquet import hangs against Exasol 2025.2.0, and the orchestrator-owned task 7.0 files the follow-up issue.
- **Promotes to ADR:** no

### [2] [plan-review] The release version of the breaking Arrow upgrade was left open

- **Finding:** Round 2 `[NFR_IGNORED]`: a plan named `fix-...` gets a patch bump from `/speq:implement-pr`. A patch release on arrow 59 after 0.17.0 on arrow 58 would break dependents on `exarrow-rs = "0.17"` through `cargo update`.
- **Direction change:** Entry [10] states the rule: a 0.x minor bump over the latest published version, 0.17.0 if `v0.17.0` is not tagged at merge time, 0.18.0 if it is, never a patch bump. plan.md § Impact repeats the rule, and § Context states that this branch is stacked on the unmerged `feat/fix-export-parquet-transport-roundtrip`.
- **Promotes to ADR:** no

### [3] [plan-review] The eight-minute opt-in export test lost its `#[ignore]`

- **Finding:** Round 2 `[COMPLETENESS_GAP]`: task 7.2 removed the bare `#[ignore]` from `test_csv_export_runs_past_the_former_five_minute_limit`, so CI would run it, see no `EXARROW_LONG_EXPORT_CHECK`, and count the early return as a pass.
- **Direction change:** Task 7.2 excludes the test from the removal, gives it `#[ignore = "eight-minute opt-in check, run with EXARROW_LONG_EXPORT_CHECK=1"]`, and rewrites its doc-comment sentence about CI. Entry [15] names the test. The Checklist row expects 12 ignored tests, and the Manual Testing row names this test.
- **Promotes to ADR:** no

### [4] [plan-review] Three unchanged advisory scenarios named the default-feature command

- **Finding:** Round 2 `[REQUIREMENT_CONFLICT]`: "Advisory suppression documents rationale", "Patch-level dep bump applied without breaking-change review", and "Minor or major dep bump requires explicit evaluation" required `cargo deny check advisories` to exit 0. With `unused-ignored-advisory = "deny"`, that command fails on a permitted suppression for an optional-feature advisory.
- **Direction change:** The `code-quality/dependencies` delta has a `DELTA:CHANGED` block for each of the three scenarios that names `cargo deny --all-features check advisories`. `cargo deny check licenses` is unchanged.
- **Promotes to ADR:** no

### [5] [plan-review] `driver_manager_tests` skipped instead of failing under `REQUIRE_EXASOL`

- **Finding:** Round 2 `[TRACEABILITY_GAP]`: `tests/driver_manager_tests.rs` defines its own `skip_if_no_exasol!` macro, which skips even when `REQUIRE_EXASOL` is set, so the fail-not-skip step of "Every integration test target runs in the CI integration job" had no implementing task.
- **Direction change:** Task 7.2 changes that macro to panic with the message of `tests/common/mod.rs` when `REQUIRE_EXASOL` is set. The Scenario Coverage row of the scenario names the change, and entry [15] records it.
- **Promotes to ADR:** no

### [6] [plan-review] The ADR candidate pinned versions and held implementation detail

- **Finding:** Round 2 `[ADR_OVERPROMOTION]`: entry [11] must stay an ADR candidate because it supersedes ADR-003, but its Decision and Alternatives pinned versions and its Consequences held the lockfile step, the `paste` ignore removal, and the `specs/mission.md` edit (`/speq:adr-rules` rules 3 and 6).
- **Direction change:** Entry [11] states the policy without version numbers, keeps three version-free alternatives, and has two Consequences bullets. The new entry [18], which is not an ADR candidate, holds the versions, the trial-build evidence, the lockfile unification step, and the `specs/mission.md` edit.
- **Promotes to ADR:** no

### [7] [plan-review] Advisory findings applied with the round 2 revision

- **Finding:** Round 1 raised five ADVISORY findings. Round 2 raised six, two of which corrected statements of this plan: parquet 59.0.0 and 59.1.0 still depend on `paste`, and a stale ignore under `unused-ignored-advisory = "deny"` is an error, not a warning.
- **Direction change:**
  - Round 1, all five: plan.md § Impact states how to decline the mismatch error. `fetch_all` closes the handle after a mismatch error (task 1.2, entry [4]). The overshoot message counts the offending batch (spec step, tasks 1.1 and 1.4). A new unit test `test_fetch_all_small_result_set_sends_no_fetch` replaces the misleading tag (task 4.3). Task 7.7 runs the WebSocket unit tests in CI (entry [5]).
  - Round 2, two: plan.md § Context and entry [18] state that parquet 59.2.0 dropped `paste`, and task 6.1 requires `parquet` 59.2. The scenario "Suppression is removed when its advisory no longer applies" names an `advisory-not-detected` diagnostic.
  - Round 2 ADVISORY findings not applied: the group A commit for an arrow 58 backport, and the import/export commands in `AGENTS.md`, `CONTRIBUTING.md`, `specs/mission.md`, and the `tests/common/mod.rs` doc example. Task 7.0 covers the follow-up issue finding.
- **Promotes to ADR:** no
