# Decision Log: fix-ffi-bound-execute

## Interview

The plan ran in headless mode. The orchestrator passed the content of issues exasol-labs/exarrow-rs#78 and #67 and the user's constraints. Each pair below paraphrases one part of that brief.

**Q:** What is the defect in issue #78?
**A:** A prepared DML statement run through ADBC `execute`, such as Python `cursor.execute(sql, params)`, writes the row and then fails with `Cannot fetch batches from row count result`. With a multi-row bound batch, only the first row is written. `FfiStatement::execute_bound_batch` runs the statement once per row and calls `fetch_all()` on a row-count result. `query_sql` already turns `NoResultSet` into no batches. Separately, `rowcount` after `execute` is always 0, because adbc_ffi 0.23 never writes `rows_affected` when a result stream is requested (apache/arrow-adbc#4468). adbc_ffi 0.24.0 fixes this with apache/arrow-adbc#4469 and writes -1, meaning unknown.

**Q:** What does issue #78 propose?
**A:** First, treat a row-count result as no batches, and test that a multi-row prepared INSERT through `execute` writes all rows with no error. Second, bump `adbc_core`, `adbc_ffi`, and `adbc_driver_manager` from 0.23 to 0.24; the author's spike changed only `Cargo.toml` and `Cargo.lock`, and `rowcount` after `execute` became -1. Third, document that `execute_update` and `executemany` report the row count and that `execute` reports -1. The proposed changelog has a fix entry, a change entry for the -1 row count, and a dependency entry for adbc 0.24.

**Q:** What is issue #67?
**A:** `execute_bound_batch_update` and `execute_bound_batch` take the lock, block on the runtime, and execute on the server once per row, so N rows cost N round trips. `Connection::execute_batch_update` and `Connection::execute_batch` already send all rows in one prepared-statement execution and have integration tests, but the FFI path does not use them. The `prepared-statements/batch-execution` requirements apply to the `Connection` methods, and the FFI scenario in `prepared-statements/binding-and-execution` is silent on the round-trip count. A spec decision is needed. The brief says to assume the conventional choice: the FFI path batches through the existing `Connection` methods, with a new requirement and scenarios, recorded here.

**Q:** Which scope and release constraints apply?
**A:** One plan and one PR that fix #78 and #67. Do not bump the crate version in `Cargo.toml`. Changelog entries go under `## [Unreleased]`. The plan writes only under `specs/_plans/fix-ffi-bound-execute/`. The triage note (`~/code/_scratches/exarrow-rs-bug-triage.md`, item 4) groups both issues in one PR because both live in the same function.

**Q:** Which parallel work must the plan avoid?
**A:** PR #89 (plan `fix-transport-deadlines`, branch `fix/transport-deadlines`) changes `src/transport/native/mod.rs`, the HTTP transport, and the login path in `src/adbc/connection.rs`. Keep this plan out of those files and areas, and note any unavoidable overlap here.

**Q:** Which decisions does the brief leave to the planner?
**A:** Whether the adbc 0.24 bump belongs in this plan, given the dependency policy in `code-quality/dependencies` and ADR-003, which mentions an `adbc_core 0.23` cap on `arrow-schema <59`. How a bound batch behaves when `execute` gets a row-count result. The partial-failure semantics of a batched execution compared with the per-row loop. How the one-shot batch interacts with `execute` returning a reader for a SELECT with a bound batch.

After review round 2, the requester answered the open question and two review findings. The orchestrator passed these answers verbatim or as close paraphrase.

**Q:** The split of an oversize native batch changes `src/transport/native/mod.rs`, which PR #89 also changes. Approve the overlap, split in `src/adbc_ffi.rs` instead, or hold this PR until PR #89 merges?
**A:** "Base on #89 as this will be merged soon as stacked PR". The plan builds on the implementation and recording of PR #89, and the split stays in the native transport.

**Q:** Which PR merges first?
**A:** Stack on PR #89. PR #89 merges first, and this PR then retargets `main`.

**Q:** Where does a multi-row bound SELECT get routed to one execution per row?
**A:** In the FFI layer only, as the plan has it. Decision [2] records the alternative "route a multi-row result-set batch inside `Connection::execute_batch`" and the reason it lost: it would change the behavior of the public Rust API and `src/adbc/connection.rs`, which this plan leaves unchanged.

**Q:** What does the split path return when a range answers with a result set?
**A:** It returns that result unchanged, so that `Connection::execute_batch_update` rejects it with `QueryError::UnexpectedResultSet`, as the recorded batch-execution scenario requires.

After PR #89 merged, the orchestrator passed the new state of the branch and the versioning constraint.

**Q:** PR #89 is merged into `main` and released as 0.19.0. What does the plan build on, and where do the changelog entries go?
**A:** The base of PR #90 is `main`. The branch has `origin/main` merged in, so its code and Cargo files equal `main`. `Cargo.toml` has version 0.19.0, and `CHANGELOG.md` starts with `## 0.19.0` and has no `## [Unreleased]` heading. This PR does not bump the `Cargo.toml` version. Its changelog entries go under a new `## [Unreleased]` heading above `## 0.19.0`, as the Changelog rules of `AGENTS.md` require. Every design decision stays as it is.

## Design Decisions

### [1] The FFI path sends a bound batch through the existing Connection batch methods

- **Decision:** `execute_update` with a bound batch calls `Connection::execute_batch_update` once with every bound row. `execute` with a bound batch calls `Connection::execute_batch`, either once for all rows or once per row, as entry [2] states. The FFI layer converts the RecordBatch into row-major `Parameter` rows and passes them to these methods unchanged. The new feature `adbc-driver/ffi-statement-execution` requires one execution per bound batch that fits in one data message, which settles the spec question of #67.
- **Alternatives:** Document the per-row loop as intentional: rejected, because N rows cost N round trips and a failure leaves the earlier rows written. Build column-major wire data from the Arrow arrays inside `src/adbc_ffi.rs`: rejected, because it copies the wire-format knowledge of `PreparedStatement::build_batch_parameters_data` into a second module and bypasses the arity check of ADR-005. Split the batch into chunks in the FFI layer: rejected in entry [8].
- **Rationale:** The `Connection` batch methods already own column-major assembly, the arity check, and the session-state updates, and integration tests cover them. Reusing them keeps one execution path for the Rust API and the FFI. The FFI layer keeps its one job, translating Arrow values into `Parameter` values. The row-major detour costs one in-memory transpose, which is small compared with one network round trip per row.
- **Consequences:** The FFI path conforms to ADR-005 (`fail-fast-per-row-arity-check-in-batch-execution`): a bound batch whose column count differs from the parameter count fails with a parameter binding error before any execution request, and the FFI layer reports it with status `InvalidArguments` (entry [12]). `bind_row_as_parameters` in `src/adbc_ffi.rs` becomes dead code and is deleted. `src/adbc/connection.rs` and `src/query/prepared.rs` stay unchanged.
- **Promotes to ADR:** no

### [2] `execute` runs a multi-row bound batch once per row only for a statement that returns a result set

- **Decision:** For `execute` with a bound batch, a statement that returns a result set and has two or more bound rows runs once per bound row. Each run calls `Connection::execute_batch` with one row, and the reader returns the batches of all runs in bound-row order. Every other bound batch runs as one `Connection::execute_batch` call with all rows. The FFI layer reads whether a statement returns a result set from the result-set column metadata of the prepared statement: `PreparedStatementHandle::result_columns` is empty for a statement that returns an affected-row count.
- **Alternatives:**
  - Send every batch as one execution and fall back to per-row execution on SQLSTATE `0A000`: rejected, because it depends on error matching and spends one failing round trip on every multi-row query.
  - Reject a multi-row bound batch for a statement that returns a result set: rejected, because it removes behavior that works today.
  - Run every `execute` once per row: rejected, because the Python `cursor.execute(sql, batch)` path of #78 is a DML path and would keep N round trips.
  - Route a multi-row result-set batch inside `Connection::execute_batch`: rejected by the requester, because it would change the behavior of the public Rust API, where `Connection::execute_batch` passes the Exasol error through today, and it would change `src/adbc/connection.rs`, which this plan leaves unchanged.
- **Rationale:** A probe against the local `exasol/docker-db:latest` container sent a three-row parameter set for `SELECT a, b FROM t WHERE a = ?`. Exasol rejected it with `Feature not supported: Prepared statement with multiple result tables` (SQLSTATE `0A000`). The same probe sent a three-row parameter set for an INSERT, and Exasol answered with `rowCount` 3. The prepare response already carries the result-set columns on both transports, so the routing costs no extra round trip.
- **Consequences:** A multi-row bound SELECT keeps one round trip per row, because Exasol requires it. A one-row bound SELECT and every bound DML statement take one round trip. `execute_update` does not route: it always sends one `Connection::execute_batch_update` call. For a statement that returns a result set, that call still fails, as it does today: one bound row gets `QueryError::UnexpectedResultSet` from `Connection::execute_batch_update`, and two or more rows get the Exasol `0A000` error. The plan specifies neither error text.
  - `Connection::execute_batch` passes the Exasol `0A000` error through unchanged for two or more rows of a statement that returns a result set. The `prepared-statements/batch-execution` scenario "Batch query execution returning a result set" therefore specifies one bound row, and `test_execute_batch_select_single_row` covers it.
  - A Rust API caller of `Connection::execute_batch` and an FFI caller get different results for a multi-row result-set batch: the Rust API caller gets the Exasol error, and the FFI caller gets the rows of one execution per bound row.
  - Two modules read `PreparedStatementHandle::result_columns`: the FFI layer to run a multi-row result-set batch once per row, and the native transport to split only a row-count batch (entry [8]). The handle owns the fact, and neither module derives it from the SQL text.
  - The `adbc-driver/ffi-statement-execution` scenario "execute runs a result-set statement once per bound row" covers one and several bound rows, so one scenario states the rule.
- **Promotes to ADR:** no

### [3] A row-count result from `execute` yields a reader with no batches

- **Decision:** When the execution behind `execute` returns an affected-row count, the FFI layer returns a RecordBatchReader with no batches and an empty schema, and no error. The FFI layer drops the count, because the Rust `adbc_core::Statement::execute` method returns only a reader. adbc_ffi 0.24 then writes -1 to `rows_affected` (entry [5]).
- **Alternatives:** Return an error for DML through `execute`: rejected, because ADBC callers such as Python DB-API `cursor.execute` route parameterized DML through `execute`. Return a one-column batch that holds the row count: rejected, because no ADBC driver manager reads a count from the stream, and the batch would invent a result set.
- **Rationale:** `FfiStatement::query_sql` already returns no batches for a statement without a result set. The fix applies the same rule to the bound path. ADR-004 (`zero-row-result-sets-carry-column-schema`) governs a result set with zero rows, which has columns. An affected-row count has no columns, so the empty reader conforms to it.
- **Promotes to ADR:** no

### [4] A bound batch takes effect for all of its rows or for none

- **Decision:** The FFI layer converts every bound value to a `Parameter` before it sends an execution request. A conversion error returns status `InvalidArguments` and sends nothing. A server error on the single batched execution returns the Exasol error, and Exasol stores none of the bound rows. On the per-row path of entry [2], the first failing run returns its error and the FFI layer discards the batches of earlier runs. A SELECT writes nothing, so that path has no partial write.
- **Alternatives:** Keep the semantics of the per-row loop, in which the rows before the failing row stay written: rejected, because the caller cannot tell which rows were written, and a retry duplicates them (#78).
- **Rationale:** A probe against the local container showed that Exasol treats a batched INSERT as one statement. Three rows in which the second string exceeded `VARCHAR(3)` failed with `data exception - string data, right truncation` (SQLSTATE `40001`), and the table held none of the three rows afterwards. With autocommit off, the same error rolled back the open transaction, both for a one-row and for a two-row execution. Batching therefore does not change what Exasol does with earlier work in the transaction.
- **Consequences:** The changelog states the all-or-nothing behavior. The session-state defect of #72, in which a failed execution leaves the session in `Executing`, is unchanged. `SessionState::is_active` counts `Executing` as active, so later statements on the connection still run.
  - The all-or-nothing rule covers a bound batch whose parameter values fit in one data message. Over the native protocol, a larger batch runs as consecutive executions, and with autocommit on a failing execution leaves the rows of the earlier executions committed (entry [8]).
  - The conversion error keeps the status of `arrow_value_to_parameter`, and its message starts with the zero-based column index and row index of the failing value, as the recorded `adbc-driver/driver-interface` scenario "Type conversion errors" requires.
- **Promotes to ADR:** no

### [5] The plan bumps adbc_core, adbc_ffi, and adbc_driver_manager to 0.24

- **Decision:** `Cargo.toml` requires `adbc_core` and `adbc_ffi` 0.24.0 as optional dependencies and `adbc_core` and `adbc_driver_manager` 0.24 as dev-dependencies. `Cargo.lock` moves `adbc_core`, `adbc_ffi`, and `adbc_driver_manager` from 0.23.0 to 0.24.0, adds `libloading` to the dependency list of the `exarrow-rs` entry, and adds, removes, or changes no other package. The crate version stays 0.19.0.
- **Alternatives:** Leave the bump to a separate PR: rejected, because #78 lists the -1 row count as part of its fix and the brief bundles it.
- **Rationale:** adbc_ffi 0.24.0 writes -1 to `rows_affected` on the query path of `AdbcStatementExecuteQuery` (apache/arrow-adbc#4469). A diff of the 0.23.0 and 0.24.0 sources in the local cargo registry found no change to the `Driver`, `Database`, `Connection`, `Statement`, or `Optionable` traits. `InfoCode` gains an `Other(u32)` variant, but the enum was already `#[non_exhaustive]`, and `src/adbc_ffi.rs` names no `InfoCode` variant. The public functions of `adbc_driver_manager` are unchanged. All three 0.24.0 crates require `arrow-array` and `arrow-schema` `>=58, <60`, which admits the pinned arrow 58, and Rust 1.85, below the CI toolchain 1.92.0. On `main`, `Cargo.toml` requires the 0.23 crates, and `Cargo.lock` holds `adbc_core`, `adbc_ffi`, and `adbc_driver_manager` 0.23.0.
- **Consequences:**
  - Rust code that uses `exarrow_rs::adbc_ffi::FfiDriver` with `adbc_core` 0.23 types no longer compiles and must move to `adbc_core` 0.24. The changelog marks this `Breaking:`, limited to Rust users of the `ffi` feature.
  - `code-quality/dependencies` treats 0.23 to 0.24 as a minor bump: the PR description documents the reason, the full test suite runs, and `cargo deny --all-features check advisories` exits 0.
  - The `adbc-driver/driver-interface` scenario "Driver registration" names 0.24. The `code-quality/core` dependency scenario names `adbc_core` without a version. The Tech Stack row of `specs/mission.md` names 0.24.
  - `adbc-driver/driver-interface` holds 13 scenarios before and after this plan, above the threshold of 10. The plan changes one scenario and adds none, so `/speq:record` reports a count that already exists on `main`.
- **Promotes to ADR:** no

### [6] ADR-003's re-evaluation trigger fires, and the advisory suppression stays

- **Decision:** `deny.toml` stays unchanged. The suppression of GHSA-2f9f-gq7v-9h6m remains.
- **Alternatives:** Move to arrow and parquet 59 now: rejected, because it is out of scope and the downstream users of exarrow-rs do not use arrow 59 yet. Supersede ADR-003 to restate its trigger: rejected, because the decision of ADR-003, a documented suppression with a re-evaluation trigger, is unchanged, and the recorded `code-quality/dependencies` spec already states the current trigger.
- **Rationale:** ADR-003 (`suppress-ghsa-2f9f-gq7v-9h6m-via-deny-toml`) names `adbc_core` lifting the `arrow-schema <59` cap as a re-evaluation trigger, and adbc_core 0.24 lifts that cap. The re-evaluation finds that the advisory still applies. `parquet` 58.x requires `thrift ^0.17`, the fix is in `parquet` 59.x, and `parquet` 59.x needs arrow 59. The `deny.toml` reason and the `code-quality/dependencies` scenario "GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift" already name this blocker and the move to arrow 59 as the trigger. Removing the suppression would fail the CI advisory gate, because `thrift` 0.17.0 stays in the dependency tree. The plan conforms to ADR-003.
- **Consequences:**
  - The trigger that ADR-003 names, `adbc_core` lifting the `arrow-schema <59` cap, has fired and no longer applies. The operative trigger is the move to arrow and parquet 59, which the reason in `deny.toml` and the `code-quality/dependencies` scenario "GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift" state.
- **Promotes to ADR:** no

### [7] A test at the C ABI proves the -1 row count

- **Decision:** `tests/driver_manager_tests.rs` gains a test that loads the release cdylib with `libloading`, fills an `adbc_ffi::FFI_AdbcDriver` through `AdbcDriverExasolInit`, and calls `StatementExecuteQuery` with an `arrow::ffi_stream::FFI_ArrowArrayStream` result stream and `rows_affected` set to 42 beforehand. `adbc_ffi` 0.24 and `libloading` 0.8 become dev-dependencies, and `arrow = { version = "58", features = ["ffi"] }` under `[dev-dependencies]` enables the Arrow C stream type for the test. `adbc_ffi` and `libloading` are already in `Cargo.lock` as dependencies of `adbc_driver_manager`, and `adbc_ffi` already builds `arrow-array` with `ffi`, so no new crate enters the dependency tree.
- **Alternatives:** Assert through `ManagedStatement::execute`: not possible, because the Rust driver manager passes a null `rows_affected` on the query path, as apache/arrow-adbc#4469 states. Call `FfiDriver` in process under `#[cfg(feature = "ffi")]`: rejected, because the other tests in the file exercise the release cdylib, and the test would not compile without the feature. Check only with a manual Python run: rejected, because a manual check does not guard against a later dependency change.
- **Rationale:** The upstream regression test of apache/arrow-adbc#4469, `test_statement_execute_query_sets_rows_affected` in `rust/driver/dummy/tests/driver_exporter_dummy.rs`, drives the same C ABI calls and serves as the pattern.
- **Promotes to ADR:** no

### [8] Over the native protocol, a batch update larger than one data message runs as consecutive executions

- **Decision:** The native transport keeps the data message of each prepared-statement execution within the maximum data message size that Exasol reports at login. A data message is the protocol message that carries the parameter values of one execution. When the parameter set of a statement that returns an affected-row count does not fit in one data message, the native transport splits the rows into consecutive ranges in input order. It runs one execution per range, stops at the first failing execution, and returns the sum of the row counts. A range that holds a single row is sent even when it exceeds the size, as the per-row loop sends it today. A statement that returns a result set, a parameter set that fits, and the WebSocket transport keep one execution.
- **Alternatives:**
  - Send every batch in one execution whatever its size: rejected, because over the native protocol Exasol drops the connection when a data message is between about 108 MB and 135 MB or larger, and the session and its open transaction are lost.
  - Split the batch in `src/adbc_ffi.rs`: rejected by the requester (Interview) and here, because only the native transport knows the encoded message size. An estimate from the Arrow batch or from the JSON parameter values has no fixed bound, for example a one-digit integer takes 1 byte as JSON and 9 bytes on the native wire. The FFI layer would also repeat wire-format knowledge.
  - Reject an oversize batch before sending: rejected, because a large `executemany` that the per-row loop completes today would fail.
  - Send one execution as several data messages through the separate `total_rows` and `rows_in_msg` header fields: rejected, because neither the Exasol WebSocket API documentation nor this codebase describes how a client sends the remaining rows of one execution, so the plan cannot specify or verify it.
  - Split at a fixed row count: rejected, because rows differ in width, so no row count bounds the message size.
- **Rationale:** Exasol documents `maxDataMessageSize` in its login response as the "maximum size of a data message in bytes". The local `exasol/docker-db` 2026.1.0 container reports 67,108,864 bytes (64 MiB), which equals the crate's `MAX_DATA_MESSAGE_SIZE` fallback. `NativeTcpTransport::build_execute_prepared_payload` writes the whole parameter set as one message. A reviewer probe on the native transport called `Connection::execute_batch_update` with rows of about 54 encoded bytes. 2,000,000 rows (about 108 MB) succeeded. 2,500,000 rows (about 135 MB) failed with `Failed to send message: Broken pipe (os error 32)`, and the next query on the same connection failed the same way. Earlier probes over the WebSocket transport sent 100,000, 1,000,000, and 3,000,000 two-column rows (2.5 MB, 26 MB, and 80 MB of JSON) in one execution each, and all three succeeded. The native transport writes the parameter values row by row after the column metadata (`write_parameter_rows`), so a range of consecutive rows is a contiguous span of the payload, and the transport can measure each range exactly. Splitting at the reported size keeps every message within the documented limit unless the message holds a single row, and each message of the per-row loop holds a single row today. No batch that the per-row loop completes today breaks the connection. The requester kept the split in `src/transport/native/mod.rs` (entry [11]).
- **Consequences:**
  - A bound batch whose parameter values fit in one data message runs as one execution and takes effect for all of its rows or for none (entry [4]). 64 MiB holds about 1,200,000 rows of 54 encoded bytes, or about 33,000 rows that each carry one 2,000-character string.
  - A larger batch runs as consecutive executions. With autocommit on, a failing execution leaves the rows of the earlier executions committed. With autocommit off, the earlier executions belong to the open transaction, and the probe of entry [4] showed that an Exasol error rolls that transaction back.
  - The connection stays usable after an oversize batch.
  - When a range answers with a result set instead of a row count, the native transport sends no later range and returns that result unchanged. `Connection::execute_batch_update` then rejects it with `QueryError::UnexpectedResultSet`, as the recorded scenario "Batch update rejects a result-set response" requires. The split runs only for a statement whose prepare reported no result columns, so no test against Exasol reaches this path.
  - `Connection::execute_batch_update` in the Rust API gets the same behavior, because the split lives in the native transport.
  - The split runs below the lifecycle guard of `execute_prepared_statement` and changes no lifecycle state. A terminated transport returns the terminated-transport error before the first range.
  - The WebSocket transport keeps one message per execution. It is opt-in, and the WebSocket probes sent up to 80 MB of JSON in one execution without an error.
  - A bound execution holds three converted copies of the whole batch in driver memory at once: the row-major `Parameter` rows that `src/adbc_ffi.rs` builds, the column-major `serde_json::Value` parameter data that `Connection::execute_batch_update` and `Connection::execute_batch` build with `PreparedStatement::build_batch_parameters_data`, and the rows that the native transport encodes. A split batch also holds the data message of the range that the native transport sends next, up to the maximum data message size. The native transport builds each range message from the encoded rows just before it sends it, so it holds one range message at a time and never the messages of all ranges. Each `Parameter` and each `serde_json::Value` is an enum value, so a numeric value takes more memory than its 4 or 8 bytes in Arrow. A bound batch therefore needs several times its Arrow size in driver memory, and a large batch that the per-row loop completes today can exceed the available memory. Bulk ingestion through `adbc.ingest.target_table` stays the documented path for large loads.
  - The plan changes `src/transport/native/mod.rs` as it is on `main` and the § Constraints section of `specs/architecture.md`. The requester approved the change to `src/transport/native/mod.rs` (entry [11]).
- **Promotes to ADR:** no

### [9] FFI bound execution moves to the new feature adbc-driver/ffi-statement-execution

- **Decision:** The new feature holds ten scenarios: bound execution through `execute` and `execute_update`, the failure cases including a wrong column count, the zero-row case, a bound batch larger than one data message, and the -1 row count. One scenario states the once-per-row rule for a result-set statement with one or more bound rows. `prepared-statements/binding-and-execution` drops its three FFI scenarios, "Single execution with parameters", "FFI bind and execute_update with parameters", and "FFI bind and execute_query with parameters", and its description points to the new feature.
- **Alternatives:** Add the scenarios to `prepared-statements/binding-and-execution`: rejected, because the feature would hold 13 scenarios, above the threshold of 10, and FFI behavior would stay split across two features. Add the -1 scenario to `adbc-driver/driver-interface` (13 scenarios) or to `adbc-driver/statement-and-results` (10 scenarios): rejected for the same threshold. Add the scenarios to `prepared-statements/batch-execution`: rejected, because that feature specifies the `Connection` batch API, not the ADBC Statement.
- **Rationale:** One feature then owns what the ADBC Statement does with a bound batch. The new feature restates the three removed scenarios with the single-execution rule, so no requirement is lost.
- **Promotes to ADR:** no

### [10] A zero-row bound batch runs no execution

- **Decision:** With a zero-row bound batch, `execute_update` returns 0 and `execute` returns a reader with no batches and an empty schema, and neither sends an execution request. The statement is still prepared first, as today, so an invalid SQL text still fails.
- **Alternatives:** Pass the empty batch to `Connection::execute_batch_update`: rejected, because `build_batch_parameters_data` turns zero rows into no parameter data, and the answer of Exasol to a parameterized statement without parameter data is not verified. Return the result-set schema of a SELECT from the prepare metadata: rejected as out of scope; the reader keeps the empty schema it has today.
- **Rationale:** ADBC runs a bound statement once per bound row, so zero rows means zero executions. The per-row loop already behaves this way. ADR-004 and the `adbc-driver/statement-and-results` scenario "Empty result sets" cover a query that runs and returns no rows. With a zero-row bound batch no query runs, so neither applies.
- **Promotes to ADR:** no

### [11] The split builds on the native transport of PR #89 on main and changes one architecture constraint

- **Decision:** This PR targets `main`, which holds the merged PR #89 (plan `fix-transport-deadlines`). The split of entry [8] stays in `src/transport/native/mod.rs` and builds on the code of PR #89 as it is on `main`. The architecture delta changes § Constraints by one bullet: the native transport runs an oversize parameter set of a row-count statement as consecutive executions within the server's maximum data message size. The plan changes `src/adbc_ffi.rs`, `src/transport/native/mod.rs`, `tests/driver_manager_tests.rs`, `tests/integration_tests.rs` (two doc comments), `Cargo.toml`, `Cargo.lock`, `docs/driver-manager.md`, `docs/prepared-statements.md`, `specs/mission.md`, and `CHANGELOG.md`. It changes nothing in `src/adbc/connection.rs`, `src/query/`, `src/transport/lifecycle.rs`, `src/transport/protocol.rs`, or `src/transport/websocket.rs`.
- **Alternatives:**
  - Keep `src/transport/native/mod.rs` unchanged and split the batch in `src/adbc_ffi.rs` with a conservative per-value size bound: rejected by the requester, and in entry [8].
  - Move the range splitting of task 3.1 and its unit tests into a new child module of `src/transport/native/`: rejected. Review round 2 proposed it to shrink the text overlap with PR #89, and PR #89 is merged, so no overlap remains. The range splitting writes the same payload prefix and column metadata as `build_execute_prepared_payload` and uses the private wire-type helpers next to it, so a child module would put the `CMD_EXECUTE_PREPARED` payload layout in two files.
- **Rationale:** The requester answered the open question of review round 2 with "Base on #89 as this will be merged soon as stacked PR" (Interview), and PR #89 is merged into `main`. `main` holds the implementation and the recording of PR #89: `src/transport/lifecycle.rs`, `src/transport/deadline.rs`, the feature `connection-management/connection-timeout`, ADR `transport-owns-setup-deadline`, and the changes to `specs/architecture.md` and `src/transport/native/mod.rs`. On `main`, `execute_prepared_statement` starts with `self.lifecycle.require(ConnectionState::Authenticated, ...)`, and the split runs below that guard. The plan conforms to ADR `transport-owns-setup-deadline`: the split runs after the login, adds no client-side timer, and each execution keeps its server-enforced query timeout. ADR `client-give-up-terminates-connection` does not apply, because the split stops on a server error and never gives up on a response. § Constraints already states the fetch-side rule for the same server size, so the send-side rule belongs next to it. No component, boundary, interface, data flow, or external dependency changes: `adbc_ffi` still maps statement binding onto the adbc Connection, and the adbc crates are libraries that External Dependencies does not list.
- **Consequences:**
  - The architecture delta copies § Constraints from `specs/architecture.md` on `main`, and its BASE is the hash of that file on `main`, `35d6222b02ed3458266a38f69dc44bc6d199cf6a`.
  - If `main` changes `src/transport/native/mod.rs`, `specs/architecture.md`, or `CHANGELOG.md` before this PR merges, the branch merges `main` first, and the architecture delta takes the new BASE hash when `specs/architecture.md` changed.
  - `main` released the entries of PR #89 as 0.19.0, and `CHANGELOG.md` has no `## [Unreleased]` heading. The new entries go under a new `## [Unreleased]` heading above `## 0.19.0`, and `Cargo.toml` stays at 0.19.0 (Interview).
  - The diff checks of this PR compare against `main`.
  - The existing tests `test_execute_batch_update` and `test_execute_batch_select_single_row` cover the changed `prepared-statements/batch-execution` scenarios. Task 4.15 adds `/// Scenario: Batch update execution with affected row count` to the doc comment of `test_execute_batch_update` and `/// Scenario: Batch query execution returning a result set` to the doc comment of `test_execute_batch_select_single_row`, as `AGENTS.md` requires for a test that implements a spec scenario. The edit changes no test code in `tests/integration_tests.rs`.
- **Promotes to ADR:** no

### [12] A bound batch with the wrong column count fails with status InvalidArguments

- **Decision:** When `Connection::execute_batch_update` or `Connection::execute_batch` returns `QueryError::ParameterBindingError` for a bound batch, the FFI layer reports it with status `InvalidArguments`. The arity check stays in `PreparedStatement::build_batch_parameters_data`, which runs before any execution request (ADR-005). The FFI layer maps every other error of these calls with `to_adbc_error`, as today.
- **Alternatives:**
  - Keep status `Internal`, which `to_adbc_error` gives every error today: rejected, because a column count that differs from the parameter count is a caller error, and Python `adbc_driver_manager` raises `InternalError` for status `Internal`.
  - Check the column count against `parameter_count()` in `src/adbc_ffi.rs` before the call: rejected, because it repeats the arity check of ADR-005 in a second module.
- **Rationale:** The Python `adbc_driver_manager` maps status `InvalidArguments` to `ProgrammingError` (`convert_error` in `_lib.pyx`), which PEP 249 names for a wrong number of parameters. The feature already reports a value that cannot be converted with status `InvalidArguments`, so both caller errors of a bound batch get the same status. The 0.19.0 changelog already moves a driver manager URI error from `Internal` to `InvalidArguments`.
- **Consequences:**
  - The `adbc-driver/ffi-statement-execution` scenario "A bound batch with the wrong column count fails before execution" states the status and that no execution request reaches Exasol.
  - The FFI layer converts every bound value before `build_batch_parameters_data` checks the column count. A conversion error, reported first, therefore takes precedence over a wrong column count. A batch with both faults fails with the conversion error and its status, such as `NotImplemented` for an extra column of an unsupported Arrow type. The scenario covers only a batch whose values all convert.
  - The changelog marks the status change `Changed:`.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] A native-protocol batch above the data message size breaks the connection

- **Finding:** Entry [8] sent every bound row in one execution with no upper bound, based on WebSocket probes only. A reviewer probe on the native transport, the default for the FFI cdylib, showed that a batch of about 135 MB breaks the connection and loses the session, while a batch of about 108 MB succeeds. The reviewer tagged the choice of behavior for a human.
- **Direction change:** Entry [8] states the native probe and resolves the choice with the size that Exasol documents and reports at login: over the native protocol, the transport runs a row-count batch whose data message would exceed that size as consecutive executions. No batch that the per-row loop completes today breaks the connection, so the plan makes this choice without a human answer. `prepared-statements/batch-execution` gains the scenario "Batch update larger than one data message over the native protocol". `adbc-driver/ffi-statement-execution` gains the scenarios "A bound batch larger than one data message is stored in full over the native protocol" and "A failed execution of a split bound batch keeps the rows of the earlier executions", which state the stored rows, the error, and the usable connection. `plan.md` gains tasks 3.1 to 3.4, the integration tests of tasks 4.11 and 4.12, their § Scenario Coverage rows, and the limit in § Context, § Impact, and tasks 5.1 and 5.2. The § Context bullet names the WebSocket transport for the 3,000,000-row probe. The architecture delta changes § Constraints, and entry [11] records the overlap with PR #89 in `src/transport/native/mod.rs`.
- **Promotes to ADR:** no

### [2] [plan-review] The C ABI test needs the Arrow ffi feature

- **Finding:** Task 3.2 of the reviewed plan passed an Arrow C stream to `StatementExecuteQuery`, but the stream type exists only under the `ffi` feature of `arrow`, and no task enabled it.
- **Direction change:** Task 1.1 adds `arrow = { version = "58", features = ["ffi"] }` under `[dev-dependencies]`. Task 4.2 names `arrow::ffi_stream::FFI_ArrowArrayStream`. § Dependencies and entry [7] name the feature. A scratch copy with the task 1.1 manifest compiled a test that uses `arrow::ffi_stream::FFI_ArrowArrayStream`, `adbc_ffi::FFI_AdbcDriver`, `adbc_core::constants::ADBC_VERSION_1_1_0`, and `libloading::Library` with `cargo check --features ffi`.
- **Promotes to ADR:** no

### [3] [plan-review] The lockfile claim omitted libloading in the exarrow-rs entry

- **Finding:** Four places stated that `Cargo.lock` changes only the three adbc entries, but `libloading` as a direct dev-dependency also joins the dependency list of the `exarrow-rs` entry.
- **Direction change:** Task 1.2, task 6.3, the Checklist "Lockfile" row, and the Decision line of entry [5] state the exact change. A scratch `cargo update -p adbc_core -p adbc_ffi -p adbc_driver_manager` with the task 1.1 manifest, including the `arrow` `ffi` dev-dependency, changed the version and checksum of the three adbc packages, added `libloading` to the `exarrow-rs` entry, and changed nothing else.
- **Promotes to ADR:** no

### [4] [plan-review] The recorded batch-execution scenario promised a multi-row result-set batch

- **Finding:** The recorded scenario "Batch query execution returning a result set" requires one execution for multiple rows of a statement that returns a result set. Exasol rejects that with `0A000`, and the new FFI Background states the rejection, so the library would hold two contradictory statements.
- **Direction change:** The new delta `prepared-statements/batch-execution/spec.md` changes the scenario's GIVEN to one bound row and changes the Background to state the rejection. `plan.md` § Features lists the feature, and § Scenario Coverage maps the scenario to `test_execute_batch_select_single_row`. Entry [2] states that `Connection::execute_batch` passes the `0A000` error through for two or more rows.
- **Promotes to ADR:** no

### [5] [plan-review] Two Background sentences named values that no scenario depends on

- **Finding:** The `adbc-driver/ffi-statement-execution` Background named the size of 67,108,864 bytes (64 MiB) that one `exasol/docker-db` image reports, and the `prepared-statements/batch-execution` Background named SQLSTATE `0A000` for the rejected multi-row parameter set. No scenario step depends on either value, and a server that reports another size would make the recorded Background false.
- **Direction change:** The `adbc-driver/ffi-statement-execution` Background states only that Exasol reports a maximum data message size at login. The `prepared-statements/batch-execution` Background states the rejection without its SQLSTATE. The 64 MiB value stays in `plan.md` § Context and entry [8], and SQLSTATE `0A000` stays in entry [2].
- **Promotes to ADR:** no

### [6] [plan-review] The native-transport split changed a file that the requester had reserved for PR #89

- **Finding:** Review round 2 (`[INTENT_DRIFT]`, BLOCKER) found that tasks 3.1 to 3.3 changed `src/transport/native/mod.rs`, which the requester had asked the plan to stay out of while PR #89 changed it in parallel. Entry [11] justified the overlap with an allowance that the brief did not contain.
- **Direction change:** The requester stacked this PR on PR #89, set PR #89 to merge first, and kept the split in the native transport. Entries [8] and [11] and the Interview record the answer and the merge order. `plan.md` § Context, § Parallelization, and tasks 3.1 and 3.2 describe the code on the new base: the lifecycle guard of `execute_prepared_statement`, `SessionInfo::max_data_message_size` in `src/transport/messages.rs`, and `fetch_results` as the pattern for the size limit. The architecture delta copies § Constraints as PR #89 recorded it and has BASE `35d6222b02ed3458266a38f69dc44bc6d199cf6a`. The Checklist compares against `origin/fix/transport-deadlines` until PR #89 merges. The plan keeps the range splitting in `src/transport/native/mod.rs` instead of a child module (entry [11]). `open-questions.md` and the blocked status line of `plan.md` are deleted.
- **Promotes to ADR:** no

### [7] [plan-review] The headless choice of entry [8] did not account for driver memory

- **Finding:** Review round 2 (`[UNSTATED_ASSUMPTION]`, ADVISORY) found that the claim "No call that succeeds today fails" holds for the connection but not for memory, because the new path holds three converted copies of the whole batch.
- **Direction change:** Entry [8] Rationale states "No batch that the per-row loop completes today breaks the connection". Entry [8] Consequences and `plan.md` § Impact name the three full-batch copies and the data message of one range. Task 5.1 states that a bound batch needs several times its Arrow size in driver memory.
- **Promotes to ADR:** no

### [8] [plan-review] A split range that answered with a result set returned a protocol error

- **Finding:** Review round 2 (`[REQUIREMENT_CONFLICT]`, ADVISORY) found that task 3.2 returned `TransportError::ProtocolError` for a range that answered with a result set. `Connection::execute_batch_update` maps that error to `QueryError::ExecutionFailed`, while the recorded scenario "Batch update rejects a result-set response" requires an unexpected-result-set error.
- **Direction change:** As the requester chose, task 3.2 sends no later range and returns the result-set `QueryResult` unchanged, so `Connection::execute_batch_update` rejects it with `QueryError::UnexpectedResultSet`. Entry [8] Consequences states the behavior and that no test against Exasol reaches the path.
- **Promotes to ADR:** no

### [9] [plan-review] The FFI feature specified no outcome for a bound batch with the wrong column count

- **Finding:** Review rounds 1 and 2 (`[COMPLETENESS_GAP]`, ADVISORY) found that entry [1] describes the arity error of a bound batch, but no scenario of `adbc-driver/ffi-statement-execution` states its status or that no execution request is sent.
- **Direction change:** The feature gains the scenario "A bound batch with the wrong column count fails before execution", with status `InvalidArguments` (entry [12]). Tasks 2.2 and 2.3 map `QueryError::ParameterBindingError` to that status. Task 2.5 adds `bound_batch_with_wrong_column_count_sends_no_execution_request`, with `execute_prepared_statement` expected `.never()`, and task 4.13 adds `test_bind_wrong_column_count_stores_no_row`. § Scenario Coverage, § Impact, and task 5.2 carry the scenario and the status change. To keep the feature at ten scenarios, the two result-set scenarios merge into "execute runs a result-set statement once per bound row", which covers one and several bound rows (entry [9]). The optional scenario for `execute_update` on a result-set statement is not added, and entry [2] keeps that error unspecified.
- **Promotes to ADR:** no

### [10] [plan-review] Entry [2] did not say why the per-row routing stays out of Connection::execute_batch

- **Finding:** Review rounds 1 and 2 (`[INFORMATION_LEAKAGE]`, ADVISORY) found that the Exasol rule for multi-row result-set batches lives in the FFI layer while `Connection::execute_batch` still accepts such a batch and fails, and that entry [2] had no Alternatives line for routing inside `Connection::execute_batch`.
- **Direction change:** As the requester chose, the routing stays in the FFI layer. Entry [2] Alternatives records routing inside `Connection::execute_batch` and the reason it lost. Entry [2] Consequences states that the Rust API and the FFI give different results for a multi-row result-set batch, and that the FFI layer and the native transport each read `PreparedStatementHandle::result_columns`.
- **Promotes to ADR:** no

### [11] [plan-review] Entry [6] left the fired trigger of ADR-003 unstated

- **Finding:** Review round 1 (`[TACTICAL_SHORTCUT]`, ADVISORY) found that the trigger named in ADR-003 fires with this plan while the suppression stays, and that only `deny.toml` and `code-quality/dependencies` hold the operative trigger.
- **Direction change:** Entry [6] still does not promote to an ADR and gains a Consequences bullet: the trigger that ADR-003 names has fired and no longer applies, and the move to arrow and parquet 59 in `deny.toml` and `code-quality/dependencies` is the operative trigger.
- **Promotes to ADR:** no

### [12] [plan-review] Task and test wording claimed no transport call and repeated statements

- **Finding:** Review rounds 1 and 2 (`[PROSE_UNCLEAR]`, ADVISORY) found that tasks 2.2 and 2.3 claimed "no transport call" although they prepare first, that a Manual Testing row expected re-execution that the example does not run, that task 4.12 lacked a period, and that one § Context bullet joined four probe results without naming the transport of the large-batch probe.
- **Direction change:** Tasks 2.2 and 2.3 say "no execution request". The Manual Testing row expects "Runs a single-row insert, a batch insert, and a one-row batch select, and exits 0". Task 4.12 has the period. The § Context bullet states one probe result per sentence and names the WebSocket transport for the 3,000,000-row probe.
- **Promotes to ADR:** no

### [13] [plan-review] The plan described a PR stacked on an open PR #89

- **Finding:** PR #89 is merged into `main` and released as 0.19.0, and the base of PR #90 is `main`. `plan.md` and entries [5], [8], [11], and [12] still described a PR stacked on an open PR #89. They compared the Checklist diffs against `origin/fix/transport-deadlines`, kept version 0.18.0, and put the changelog entries under an `## [Unreleased]` heading that `main` no longer has.
- **Direction change:** `plan.md` § Context, § Impact, § Parallelization, tasks 1.1, 5.2, and 6.3, and the Checklist rows "Lockfile" and "Changelog" describe the plan on `main`. The native-transport split changes the code as it is on `main`, and the Checklist diffs compare against `main`. `Cargo.toml` stays at 0.19.0, and task 5.2 adds a new `## [Unreleased]` heading above `## 0.19.0` for the entries of this plan. Entries [5], [8], [11], and [12] and the Interview state the same, and this entry replaces the Checklist note of review finding [6]. Every design decision is unchanged. The source files, symbols, and test helpers that the tasks name exist unchanged on `main`. The architecture delta BASE `35d6222b02ed3458266a38f69dc44bc6d199cf6a` equals the hash of `specs/architecture.md` on `main`.
- **Promotes to ADR:** no

### [14] [plan-review] A split batch held the payloads of all ranges at once

- **Finding:** Review round 3 (`[NFR_IGNORED]`, ADVISORY) found that task 3.1 returned one payload per range, so a split batch held every range payload next to the encoded rows, while § Impact and entry [8] counted the data message of one range only.
- **Direction change:** Task 3.1 has a split function that returns the encoded rows and the row ranges and builds no payload, and a range-payload function that builds the payload of one range. Task 3.2 builds each range payload just before it sends it and drops it before the next range. The task 3.3 tests assert on the per-range payloads. § Impact and entry [8] Consequences state that the native transport holds one range message at a time.
- **Promotes to ADR:** no

### [15] [plan-review] The manual check of the failed batch ran with autocommit off

- **Finding:** Review round 3 (`[UNSTATED_ASSUMPTION]`, ADVISORY) found that the two § Manual Testing rows of the failed batch used `adbc_driver_manager.dbapi.connect`, which turns autocommit off. The Exasol error then rolls back the open transaction, so the per-row loop also leaves 0 rows, and the count did not prove the all-or-nothing rule.
- **Direction change:** § Manual Testing defines `CONNECT_AC`, which passes `autocommit=True` to `d.connect`, and the two rows of the failed batch use it without `c.commit()`. The expected output states that the per-row loop committed row 1 and printed `(1,)`.
- **Promotes to ADR:** no

### [16] [plan-review] The wrong-column-count scenario covered batches whose values fail conversion

- **Finding:** Review round 3 (`[REQUIREMENT_CONFLICT]`, ADVISORY) found that the scenario "A bound batch with the wrong column count fails before execution" covered any bound batch whose column count differs from N. Task 2.1 converts every value before the arity check, so an extra column with an unconvertible value fails with the conversion error, which can have status `NotImplemented` and does not state N.
- **Direction change:** The scenario GIVEN of `adbc-driver/ffi-statement-execution` is limited to a bound batch whose values all convert to Exasol parameters. Entry [12] Consequences states that a conversion error, reported first, takes precedence over a wrong column count. Task 4.13 names the three bound columns as `Int32`, as task 2.5 does.
- **Promotes to ADR:** no

### [17] [plan-review] The conversion error did not name the column and row

- **Finding:** Review round 3 (`[REQUIREMENT_CONFLICT]`, ADVISORY) found that the recorded `adbc-driver/driver-interface` scenario "Type conversion errors" requires the error to name the column and the row, and task 2.1 returned the error of `arrow_value_to_parameter` unchanged, whose message names neither.
- **Direction change:** Task 2.1 prefixes the message of the first conversion error with `column <c>, row <r>: `, using the zero-based column index and row index, and keeps the status. The task 2.5 test `unconvertible_bound_value_sends_no_execution_request` asserts that the message contains `column 0, row 1`. Entry [4] Consequences states the rule.
- **Promotes to ADR:** no

### [18] [plan-review] No scenario stated the WebSocket behavior for a large batch update

- **Finding:** Review round 3 (`[COMPLETENESS_GAP]`, ADVISORY) found that the changed `prepared-statements/batch-execution` scenarios cover a batch that fits one data message and a larger batch over the native protocol only, so the recorded library would say nothing about a large batch over the WebSocket protocol. Task 5.1, § Impact, and the `Changed:` entry of task 5.2 also omitted the transport.
- **Direction change:** The `prepared-statements/batch-execution` Background states that over the WebSocket protocol a batch update runs as one execution whatever its size. Task 5.1, § Impact, and the `Changed:` entry of task 5.2 state that a larger batch runs as consecutive executions over the native protocol.
- **Promotes to ADR:** no

### [19] [plan-review] Two batch tests lacked their Scenario lines

- **Finding:** Review round 3 (`[TRACEABILITY_GAP]`, ADVISORY) found that `test_execute_batch_update` and `test_execute_batch_select_single_row` implement two changed `prepared-statements/batch-execution` scenarios without the `/// Scenario:` line that `AGENTS.md` requires, and entry [11] kept `tests/integration_tests.rs` unchanged.
- **Direction change:** Task 4.15 in group A adds the two `/// Scenario:` lines to the doc comments of these tests. Section 4 of `plan.md`, the group A Knowledge, and the § Scenario Coverage note name the edit. Entry [11] lists `tests/integration_tests.rs` among the changed files, limited to two doc comments, and its last Consequences bullet states the edit. No task number changes.
- **Promotes to ADR:** no

### [20] [plan-review] Tests that call execute after execute_update did not bind again

- **Finding:** Review round 3 (`[PROSE_UNCLEAR]`, ADVISORY) found that `execute_update` and `execute()` each consume the binding, also when they fail, and that the three task 2.5 tests and tasks 4.9, 4.10, and 4.13 called `execute()` after `execute_update` without stating a new `bind()`.
- **Direction change:** The task 2.5 preamble states that both calls consume the binding. The tests `zero_row_bound_batch_sends_no_execution_request`, `unconvertible_bound_value_sends_no_execution_request`, and `bound_batch_with_wrong_column_count_sends_no_execution_request`, and tasks 4.9, 4.10, and 4.13 state that the test binds the batch again before it calls `execute()`, as task 4.8 does.
- **Promotes to ADR:** no
