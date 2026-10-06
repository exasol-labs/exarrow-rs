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
- **Rationale:** The sizes are the smallest that cross the default 64 MiB data message size, the server default. The batch-shape assertions make a test fail, not pass silently, if a server configuration stops exercising the paged path. Each partial-inline or multi-fetch test moves about 70 MB over the local connection.
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

- **Decision:** `CHANGELOG.md` gets a `## [Unreleased]` section above `## 0.17.0` with `Breaking:`, `Fix:`, `Security:`, and `Changed:` entries for this plan. The release that carries this change is a 0.x minor bump over the latest published version, never a patch bump. Tag `v0.17.0` exists, so the implement step sets `version = "0.18.0"` and folds `[Unreleased]` into `## 0.18.0` (plan.md § Parallelization › Commits, checkpoint 4).
- **Alternatives:** (a) Let the implement step bump by Conventional Commit type. Rejected: a plan named `fix-...` gets a patch bump, and a patch release on arrow 59 after 0.17.0 on arrow 58 would reach every dependent on `exarrow-rs = "0.17"` through `cargo update` and break code that passes Arrow values between the crates.
- **Rationale:** `AGENTS.md` requires the changelog update in the same PR as a user-facing change, and puts entries of a PR without a version bump under `## [Unreleased]`. Cargo treats versions with the same 0.x minor component as compatible, so the arrow 59 upgrade (entry [11]) needs a new 0.x minor version. `CONTRIBUTING.md` § Releasing requires a SemVer bump. This branch is based on `main`, and tag `v0.17.0` exists, so the release that carries this plan is 0.18.0.
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
    - Result: the hang depends on the server version, and entry [19] gives its cause. Neither a CI flag nor a driver change after commit `95819035` makes 2025.2.0 hang, because the same commit hangs on 2025.2.0 today.
- **Consequences:**
  - No native Parquet import test is ignored. Entry [20] fixes the driver cause of the hang, entry [19] declares Exasol 2025.2.0 unsupported for native Parquet import, and entry [21] moves CI to `exasol/docker-db:2025.2.1`, on which every import/export test passes.
  - A test that hangs or fails in CI gets its cause fixed. A reasoned `#[ignore]` is for a test that cannot run in CI by design, such as the eight-minute opt-in check.
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
- **Rationale:** The last successful run of the job (run 30378579965, 2026-07-28) took 6 minutes 34 seconds, of which the Exasol container start and readiness wait took 2 minutes 33 seconds. In the planning run on a 4-CPU host, the import/export tests that pass took about 30 seconds of test time, `native_protocol_tests` 1.7 seconds, `native_transport_smoke_test` 0.2 seconds, and the three URI-schema tests 0.9 seconds. Each new target also compiles once, which took 20 to 30 seconds per target on that host. The ten paged-fetch tests each move at most about 70 MB over the local connection. On `exasol/docker-db:2025.2.1`, the CI-flag runs took, including compilation on the planning host: `import_export_tests` 76 seconds for 54 tests, `integration_tests` 64 seconds, `websocket_integration_tests` 51 seconds, `native_protocol_tests` 16 seconds, and `native_transport_smoke_test` 21 seconds. Estimate, not measured on the CI runner: the job grows by 3 to 6 minutes and stays below 15 minutes.
- **Promotes to ADR:** no

### [18] Versions and lockfile steps of the parquet upgrade

- **Decision:** `Cargo.toml` requires arrow 59, parquet 59.2 or later within 59.x, and adbc_core, adbc_ffi, and adbc_driver_manager 0.24 (plan.md tasks 6.1 and 6.6). `deny.toml` drops the ignores for GHSA-2f9f-gq7v-9h6m and RUSTSEC-2024-0436 (task 6.3).
- **Alternatives:** (a) arrow and parquet 60. Rejected: adbc_core 0.24 accepts arrow-array and arrow-schema `>=58, <60`. (b) Keep the requirement `parquet = "59"`. Rejected: it admits 59.0.0 and 59.1.0, which still depend on `paste`, so the advisory gate would fail on RUSTSEC-2024-0436 if the lockfile resolved one of them.
- **Rationale:** The crates.io index lists no `thrift` dependency for any parquet 59.x release (apache/arrow-rs#9962, released in 59.0.0 on 2026-06-09), and `paste ^1.0` for 59.0.0 and 59.1.0 but not for 59.2.0 and 59.3.0. adbc_core 0.24.0 (2026-07-28) is the first release that accepts Arrow 59. parquet 58.4.0 still requires `thrift ^0.17`. A trial build in a scratch copy with arrow and parquet 59.3.0, adbc 0.24.0, indicatif 0.18, and the updated lockfile compiled every target with every feature without source changes, passed clippy without warnings, passed 1,596 unit tests (1,624 with `websocket`), and passed `cargo deny --all-features check advisories` and `check licenses`.
- **Consequences:**
  - After the manifest change, Cargo keeps adbc_core on the locked arrow 58.3.0 although adbc_core accepts 59. Task 6.1 unifies the Arrow sub-crates explicitly.
  - `specs/mission.md` § Tech Stack names arrow 59, parquet 59, and adbc 0.24 (task 6.6).
- **Promotes to ADR:** no

### [19] Exasol 2025.2.0 is not supported for native Parquet import, and nothing pins it

- **Decision:** `supports_native_parquet_import` keeps its gate `>= (2025, 1, 11)` with no exception for any single version. Exasol 2025.2.0 is not supported for native Parquet import: Parquet import there returns Exasol's `ETL-2210` error (entry [20]), and users upgrade to 2025.2.1 or later or set `with_native_parquet(Some(false))`. Tests, scripts, CI, and specs no longer name 2025.2.0. `docs/import-export.md` § Native Parquet Import, `CHANGELOG.md`, and plan.md § Impact state the limitation. Dropping 2025.2.0 support means that no code, test, script, CI step, or spec names 2025.2.0, and only `docs/import-export.md` and `CHANGELOG.md` state the native Parquet import limitation. The PR description repeats this sentence so the requester can confirm the interpretation.
- **Alternatives:** (a) Exclude `(2025, 2, 0)` in the gate so that 2025.2.0 converts Parquet to CSV. Rejected by the user: the driver would keep code and tests for an outdated server version. (b) Raise the gate to `(2025, 2, 1)`. Rejected: it would also turn off native import on Exasol 2025.1.11 and later 2025.1.x releases.
- **Rationale:** User decision. Root-cause runs against local containers show why 2025.2.0 fails. Sent directly through `exapump` without the driver, `IMPORT INTO RC_PROBE.T FROM PARQUET AT 'http://127.0.0.1:9' FILE 'x.parquet'` fails on 2025.2.0 with `ETL-2210: AWS URL is invalid: Provided URL is not a valid AWS S3 URL (http://127.0.0.1:9/x.parquet)`. On 2025.2.1 the same statement fails only because nothing listens on the address (`ETL-2238`). On 2025.2.0 the statement with `FROM CSV` also fails only on the connection (`ETL-5105`). A traced driver run on 2025.2.0 shows the IMPORT statement failing with `ETL-2210` after 0.2 seconds while the tunnel task waits. On 2025.2.1 the trace shows range `GET`s and a `HEAD`, the server closing the tunnel, and 3 imported rows. Exasol 2025.2.0 therefore accepts only S3 URLs as Parquet sources, and 2025.2.1 and 2026.1.0 accept HTTP sources. The repository states no minimum Exasol version overall. The only version requirement for a feature is the one for native Parquet import, in `docs/import-export.md` and `specs/architecture.md` § Constraints, so the 2025.2.0 limitation belongs in `docs/import-export.md`.
- **Consequences:**
  - The recorded scenario "Native Parquet import threshold" uses `(2025,2,1)` instead of `(2025,2,0)` as a boundary case. The gate and its other cases are unchanged.
  - The Backgrounds of `connection-management/version-capability` and `import-export/parquet-io` state that the driver selects native Parquet import from Exasol 2025.1.11 onward, instead of stating that the feature is available from that release.
  - The three native Parquet import tests assert native Parquet support instead of skipping, because CI and the local runner use 2025.2.1.
  - `scripts/run_all_tests.sh` drops 2025.2.0 from its default and its help example. `specs/architecture.md` names 2025.2.1 as the CI image. Every other Docker reference in the repository already uses `exasol/docker-db:latest`.
- **Architecture:** no change: § Constraints states Exasol 2025.1.11 as a requirement for native Parquet import, which stays true, and the server-side limitation lives in `docs/import-export.md`
- **Promotes to ADR:** no

### [20] A failed IMPORT statement stops the tunnel tasks and returns its error

- **Decision:** One crate-visible function in `src/import/parallel.rs` finishes an import from the IMPORT statement's result and its tunnel task. When the statement fails, the function aborts a tunnel task that has not finished and returns the statement's error, or the task's own error if the task failed first. Per-connection tasks stop together with their parent task. The five import paths that awaited the tunnel task after the statement use it. The single-file CSV path keeps its `tokio::select!`.
- **Alternatives:** (a) Convert the five paths to the `tokio::select!` shape of the single-file CSV path. Rejected: the multi-file paths join a set of spawned per-connection tasks, so the change would rewrite each path instead of one shared ending. (b) Add a timeout to the tunnel task. Rejected: a fixed limit either cuts off a slow, healthy import or keeps a failed import waiting until it expires.
- **Rationale:** Exasol never requests data after it rejects the statement, and it keeps the client's tunnel socket open, so a task that waits for the next HTTP request never finishes. Probes on 2025.2.0 and 2025.2.1 reproduce the hang for a missing target table: multi-file CSV import and native Parquet import never return within 30 seconds, while single-file CSV import returns `object NO_SUCH_SCHEMA_XYZ.NO_SUCH_TABLE not found`. The recorded scenario "Native Parquet import option overrides the server-version probe" already requires that the Exasol error reach the caller. One shared function gives the five paths one rule for ending an import, next to `resolve_stream_task`, which already collapses a tunnel task's outcome.
- **Consequences:** An import that fails before Exasol requests data returns within the statement's own round trip. A tunnel task that failed before the statement returned still reports its own error. A tunnel task that is still running when the statement fails is aborted, and the statement's error is returned. Before this change, a tunnel error that arrived after the statement's error took precedence. Now the statement's error, which explains the failure, takes precedence. `import-export/http-transport` states the behavior in a new scenario.
- **Architecture:** § Data Flow
- **Promotes to ADR:** no

### [21] CI runs on `exasol/docker-db:2025.2.1`

- **Decision:** The `integration-tests` job and the `EXASOL_TAG` default and help example of `scripts/run_all_tests.sh` use `exasol/docker-db:2025.2.1` instead of 2025.2.0.
- **Alternatives:** (a) Keep 2025.2.0. Rejected: 2025.2.0 rejects HTTP Parquet sources, so every native Parquet import test would fail there (entry [19]). (b) Move to 2026.1.0. Rejected: the planning runs on 2026.1.0 covered only two tests, and 2025.2.1 is the closest image to the current one.
- **Rationale:** On a local 2025.2.1 container, every integration target passes with the CI flags: `import_export_tests` 54 of 54 including the formerly ignored tests, `integration_tests` 69 passed and 4 ignored, `websocket_integration_tests` 44, `native_protocol_tests` 14, and `native_transport_smoke_test` 4. `driver_manager_tests` and the Python tests were not run on 2025.2.1 during planning.
- **Consequences:** No CI step or local script runs Exasol 2025.2.0 any longer.
- **Architecture:** § Constraints
- **Promotes to ADR:** no

### [22] Group A lands as its own commit for a possible arrow 58 backport

- **Decision:** The paged-fetch fix (group A) lands as the first commit of this plan. It holds only group A's source, tests, and `CHANGELOG.md` lines, so a maintainer can cherry-pick it onto an arrow 58 release. Group C follows as the second commit and group B as the third.
- **Alternatives:** (a) Split the plan into two PRs. Rejected: the user keeps the arrow 59 upgrade in this plan.
- **Rationale:** User decision. Group A has no arrow 59 dependency, and a dependent on arrow 58, such as exapump today, cannot take the issue #80 fix from a release on arrow 59.
- **Consequences:**
  - The `/speq:implement-pr` orchestrator, the only actor with git write authority, commits after group A, after group C, and at step A4 for group B. Implementer agents never commit. Code-review fixes go into a separate commit per group. plan.md § Parallelization › Commits lists the checkpoints.
  - `main` squash-merges pull requests, which drops the group boundaries from `main`. The PR description lists the backport commits by SHA and states that a maintainer who wants a backport keeps the feature branch or merges without squashing.
  - This plan ships no backport release. A human decides whether one is needed. Group C also has no arrow dependency, so its commits can join a backport as well.
- **Promotes to ADR:** no

### [23] Contributor docs name the new import/export test command

- **Decision:** `AGENTS.md`, `CONTRIBUTING.md`, and `specs/mission.md` § Commands name `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1`. The `tests/common/mod.rs` doc example drops `#[ignore]`, and the `tests/driver_manager_tests.rs` module doc names `--include-ignored`.
- **Alternatives:** none
- **Rationale:** After task 7.2, `-- --ignored` selects only the eight-minute opt-in test of `import_export_tests`, and the lint check of task 7.3 rejects a bare `#[ignore]`, so the old instructions would mislead contributors and agents.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] The native Parquet import hang was attributed without a cause check

- **Finding:** Round 2 `[UNSTATED_ASSUMPTION]`: the plan kept eleven tests out of CI on the belief that their hang was a separate defect. It did not establish the cause, and plan 002 reports nine of these tests passing on 2025.2.0.
- **Direction change:** The planner ran two of the tests on 2025.2.0 with and without the CI flags, on 2025.2.1, on 2026.1.0, and at commit `95819035` on 2025.2.0. The tests hang on 2025.2.0 in every configuration and pass on 2025.2.1 and 2026.1.0, so the hang depends on the server version. Entry [15] records the results. The `#[ignore]` reason in task 7.2 names Exasol 2025.2.0, plan.md § Impact states that native Parquet import hangs against Exasol 2025.2.0, and the orchestrator-owned task 7.0 files the follow-up issue.
- **Promotes to ADR:** no

### [2] [plan-review] The release version of the breaking Arrow upgrade was left open

- **Finding:** Round 2 `[NFR_IGNORED]`: a plan named `fix-...` gets a patch bump from `/speq:implement-pr`. A patch release on arrow 59 after 0.17.0 on arrow 58 would break dependents on `exarrow-rs = "0.17"` through `cargo update`.
- **Direction change:** Entry [10] states the rule: a 0.x minor bump over the latest published version, never a patch bump, which is 0.18.0 because `v0.17.0` is tagged. plan.md § Impact repeats the rule, and § Context states that the branch is based on `main`.
- **Promotes to ADR:** no

### [3] [plan-review] The eight-minute opt-in export test lost its `#[ignore]`

- **Finding:** Round 2 `[COMPLETENESS_GAP]`: task 7.2 removed the bare `#[ignore]` from `test_csv_export_runs_past_the_former_five_minute_limit`, so CI would run it, see no `EXARROW_LONG_EXPORT_CHECK`, and count the early return as a pass.
- **Direction change:** Task 7.2 excludes the test from the removal, gives it `#[ignore = "eight-minute opt-in check, run with EXARROW_LONG_EXPORT_CHECK=1"]`, and rewrites its doc-comment sentence about CI. Entry [15] names the test. The Checklist row expects 1 ignored test, and the Manual Testing row names this test.
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
  - Round 2 ADVISORY findings not applied: the group A commit for an arrow 58 backport, and the import/export commands in `AGENTS.md`, `CONTRIBUTING.md`, `specs/mission.md`, and the `tests/common/mod.rs` doc example. Review finding [8] applies both and drops the follow-up issue task.
- **Promotes to ADR:** no

### [8] [plan-review] User decisions after review round 2

- **Finding:** The user reviewed the plan after round 2. (1) Keep the row-count mismatch error of entry [4]. (2) Keep the arrow and parquet 59 and adbc 0.24 upgrade in this plan. (3) Land group A as its own commit for a possible arrow 58 backport. (4) Do not ignore the native Parquet import tests in CI; find the root cause of the hang on `exasol/docker-db:2025.2.0` and fix it, or keep the tests running in CI. (5) Update the import/export test command in the contributor docs. The branch is now based on `main`, and tag `v0.17.0` exists.
- **Direction change:**
  - (1) and (2): entries [4] and [11] stay unchanged.
  - (3): entry [22] and plan.md § Parallelization make group A the first, self-contained commit.
  - (4): root-cause runs found two causes. Exasol 2025.2.0 rejects HTTP Parquet sources with `ETL-2210` (entry [19]). The driver hangs whenever an IMPORT statement fails before Exasol requests data, on every server version (entry [20]). New group C (tasks 8.1 to 8.10) fixes the driver cause, adds the scenario "Failed IMPORT statement returns its error without waiting for the tunnel", drops Exasol 2025.2.0 (entry [19], superseded exclusion by review finding [9]), and moves CI to 2025.2.1 (entry [21]). Task 7.0 and the eleven `#[ignore]` reasons are removed. Only the eight-minute opt-in test keeps a reasoned `#[ignore]`.
  - (5): task 7.8 and entry [23].
  - Entry [10] states 0.18.0 as the release version. Review finding [1] above records the earlier ignore-based direction that this entry replaces.
- **Promotes to ADR:** no

### [9] [plan-review] User decision: drop Exasol 2025.2.0 support instead of excluding it in the gate

- **Finding:** The user does not keep tests or code that pin outdated Exasol versions. Since 2025.2.1 works, support for 2025.2.0 is dropped entirely, the driver-side fix for the IMPORT hang stays, and the CI image move to 2025.2.1 stays.
- **Direction change:**
  - Entry [19] now keeps the gate `>= (2025, 1, 11)` and declares 2025.2.0 unsupported for native Parquet import, instead of excluding `(2025, 2, 0)`.
  - The deltas for `import-export/parquet-io` and `import-export/parallel-import` are removed, together with the scenario "Exasol 2025.2.0 receives Parquet converted to CSV", the manual 2025.2.0 run, the two CSV-path multi-file tests that covered the changed parallel-import scenarios, and the 2025.2.0 lines of the architecture delta. A fifth missing-table test covers the CSV-converted multi-file Parquet path.
  - The `connection-management/version-capability` delta now changes only the boundary case `(2025,2,0)` to `(2025,2,1)` in "Native Parquet import threshold".
  - Tasks 8.6, 8.7, 8.9, and 8.10 move every remaining 2025.2.0 reference in tests, scripts, docs, and the gate's doc comment to 2025.2.1 or remove it. plan.md § Impact and task 8.8 state that users on 2025.2.0 get `ETL-2210` instead of a hang and upgrade to 2025.2.1 or force the CSV path.
- **Promotes to ADR:** no

### [10] [plan-review] No actor produced the commit order A, C, B or the 0.18.0 version

- **Finding:** Round 3 `[HIDDEN_DEPENDENCY]`: `/speq:implement-pr` commits once at step A4, implementer agents are read-only, code review runs after all groups, `main` squash-merges, and step A3 bumps by Conventional Commits. User decision (3) and the 0.18.0 rule of entry [10] would fail without an error.
- **Direction change:** plan.md § Parallelization has a `### Commits` subsection. It names the `/speq:implement-pr` orchestrator as the only committer, with commits after group A, after group C, and at step A4 for group B, separate review-fix commits per group, a review base before implementation, step A3 setting 0.18.0 per entry [10], and a PR description that lists the backport commits by SHA and states the squash-merge caveat. Entry [22] Consequences name the same actor and the caveat.
- **Promotes to ADR:** no

### [11] [plan-review] No test checked that per-connection tunnel tasks stop

- **Finding:** Round 3 `[TRACEABILITY_GAP]`: task 8.2 had no test, so a wrong or skipped task 8.2 would leave detached per-connection tasks holding tunnel sockets while every planned test passed. The `JoinSet` option broke the signature of the existing `join_stream_handles` tests, and task 8.2 placed `stream_parquet_files_parallel` in the wrong file.
- **Direction change:** Task 8.2 puts an abort-on-drop guard inside `join_stream_handles`, which also stops the remaining tasks after the first failure, and drops the `JoinSet` option. Task 8.4 adds a unit test that aborts a parent of two never-finishing children and asserts that both `oneshot` receivers return `RecvError` within 5 seconds, and extends the `serve_parquet_bytes` test to assert end of stream on the fake server's connection through a new `FakeExasolServer` method. The Scenario Coverage rows and the group C Knowledge column name `src/import/parallel.rs` for `stream_parquet_files_parallel`.
- **Promotes to ADR:** no

### [12] [plan-review] Source code still named Exasol 2025.2.0

- **Finding:** Round 3 `[INTENT_DRIFT]` ADVISORY: task 8.6 wrote 2025.2.0 into the doc comment of `supports_native_parquet_import`, and entry [19] did not define what dropping 2025.2.0 support means.
- **Direction change:** Task 8.6 keeps the doc comment free of 2025.2.0 and points to `docs/import-export.md`. Entry [19] defines the term, and plan.md § Parallelization › Commits puts the definition into the PR description. This agrees with the user decision of review finding [9].
- **Promotes to ADR:** no

### [13] [plan-review] The changed error precedence was described as unchanged

- **Finding:** Round 3 `[COMPLETENESS_GAP]` ADVISORY: the http-transport delta and entry [20] said the tunnel error wins "as before", but a tunnel task still running when the statement fails is now aborted and the statement's error wins.
- **Direction change:** The delta step reads "when a tunnel task has failed before the IMPORT statement returns its error, the system SHALL return that task's error". Entry [20] Consequences state the new precedence. Task 8.4 adds a fourth case: a failed statement with a tunnel task that is still running returns `ImportError::SqlError`.
- **Promotes to ADR:** no

### [14] [plan-review] Two Backgrounds claimed native Parquet availability from 2025.1.11 without exception

- **Finding:** Round 3 `[REQUIREMENT_CONFLICT]` ADVISORY: the recorded Backgrounds of `connection-management/version-capability` and `import-export/parquet-io` state availability from 2025.1.11 onward, which entry [19] disproves for 2025.2.0, and entry [19] had no `Architecture:` line.
- **Direction change:** Both deltas change their Background to say that the driver selects native Parquet import from 2025.1.11 onward. The `import-export/parquet-io` delta returns for this Background change only. Entry [19] has an `Architecture: no change` line. Neither Background names 2025.2.0, which agrees with review finding [9].
- **Promotes to ADR:** no

### [15] [plan-review] Missing-table tests and test ordering did not pin the paths they cover

- **Finding:** Round 3 `[TRACEABILITY_GAP]` ADVISORY: the two native-path missing-table tests relied on the server version to take the native path, the Manual Testing row expected four tests instead of five, the group C order put the reproducing tests after the fix, and task 8.9 updated one of three stale doc comments.
- **Direction change:** Task 8.5 sets `with_native_parquet(Some(true))` on the two native-path tests. The Manual Testing row expects five tests. The group C order runs the tests of 8.5 and 8.4 first. Task 8.9 updates the doc comments of all three native-path tests.
- **Promotes to ADR:** no

### [16] [plan-review] Several log statements described superseded states

- **Finding:** Round 3 `[PROSE_BLOAT]` ADVISORY: entries [7] and [10] and review findings [2], [3], [7], and [8] described the 2025.2.0 CI image, the untagged `v0.17.0` branch, the stacked branch, 12 ignored tests, task 7.0, and the 2025.2.0 exclusion.
- **Direction change:** Each statement now describes the current plan or names the review finding that supersedes it.
- **Promotes to ADR:** no
