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

## Design Decisions

### [1] The FFI path sends a bound batch through the existing Connection batch methods

- **Decision:** `execute_update` with a bound batch calls `Connection::execute_batch_update` once with every bound row. `execute` with a bound batch calls `Connection::execute_batch`, either once for all rows or once per row, as entry [2] states. The FFI layer converts the RecordBatch into row-major `Parameter` rows and passes them to these methods unchanged. The new feature `adbc-driver/ffi-statement-execution` requires one execution per bound batch that fits in one data message, which settles the spec question of #67.
- **Alternatives:** Document the per-row loop as intentional: rejected, because N rows cost N round trips and a failure leaves the earlier rows written. Build column-major wire data from the Arrow arrays inside `src/adbc_ffi.rs`: rejected, because it copies the wire-format knowledge of `PreparedStatement::build_batch_parameters_data` into a second module and bypasses the arity check of ADR-005. Split the batch into chunks in the FFI layer: rejected in entry [8].
- **Rationale:** The `Connection` batch methods already own column-major assembly, the arity check, and the session-state updates, and integration tests cover them. Reusing them keeps one execution path for the Rust API and the FFI. The FFI layer keeps its one job, translating Arrow values into `Parameter` values. The row-major detour costs one in-memory transpose, which is small compared with one network round trip per row.
- **Consequences:** The FFI path conforms to ADR-005 (`fail-fast-per-row-arity-check-in-batch-execution`): a bound batch whose column count differs from the parameter count fails with a parameter binding error before any transport call. `bind_row_as_parameters` in `src/adbc_ffi.rs` becomes dead code and is deleted. `src/adbc/connection.rs` and `src/query/prepared.rs` stay unchanged.
- **Promotes to ADR:** no

### [2] `execute` runs a multi-row bound batch once per row only for a statement that returns a result set

- **Decision:** For `execute` with a bound batch, a statement that returns a result set and has two or more bound rows runs once per bound row. Each run calls `Connection::execute_batch` with one row, and the reader returns the batches of all runs in bound-row order. Every other bound batch runs as one `Connection::execute_batch` call with all rows. The FFI layer reads whether a statement returns a result set from the result-set column metadata of the prepared statement: `PreparedStatementHandle::result_columns` is empty for a statement that returns an affected-row count.
- **Alternatives:** Send every batch as one execution and fall back to per-row execution on SQLSTATE `0A000`: rejected, because it depends on error matching and spends one failing round trip on every multi-row query. Reject a multi-row bound batch for a statement that returns a result set: rejected, because it removes behavior that works today. Run every `execute` once per row: rejected, because the Python `cursor.execute(sql, batch)` path of #78 is a DML path and would keep N round trips.
- **Rationale:** A probe against the local `exasol/docker-db:latest` container sent a three-row parameter set for `SELECT a, b FROM t WHERE a = ?`. Exasol rejected it with `Feature not supported: Prepared statement with multiple result tables` (SQLSTATE `0A000`). The same probe sent a three-row parameter set for an INSERT, and Exasol answered with `rowCount` 3. The prepare response already carries the result-set columns on both transports, so the routing costs no extra round trip.
- **Consequences:** A multi-row bound SELECT keeps one round trip per row, because Exasol requires it. A one-row bound SELECT and every bound DML statement take one round trip. `execute_update` does not route: it always sends one `Connection::execute_batch_update` call. For a statement that returns a result set, that call still fails, as it does today: one bound row gets `QueryError::UnexpectedResultSet` from `Connection::execute_batch_update`, and two or more rows get the Exasol `0A000` error. The plan specifies neither error text.
  - `Connection::execute_batch` passes the Exasol `0A000` error through unchanged for two or more rows of a statement that returns a result set. The `prepared-statements/batch-execution` scenario "Batch query execution returning a result set" therefore specifies one bound row, and `test_execute_batch_select_single_row` covers it.
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
- **Promotes to ADR:** no

### [5] The plan bumps adbc_core, adbc_ffi, and adbc_driver_manager to 0.24

- **Decision:** `Cargo.toml` requires `adbc_core` and `adbc_ffi` 0.24.0 as optional dependencies and `adbc_core` and `adbc_driver_manager` 0.24 as dev-dependencies. `Cargo.lock` moves `adbc_core`, `adbc_ffi`, and `adbc_driver_manager` from 0.23.0 to 0.24.0, adds `libloading` to the dependency list of the `exarrow-rs` entry, and adds, removes, or changes no other package. The crate version stays 0.18.0.
- **Alternatives:** Leave the bump to a separate PR: rejected, because #78 lists the -1 row count as part of its fix and the brief bundles it.
- **Rationale:** adbc_ffi 0.24.0 writes -1 to `rows_affected` on the query path of `AdbcStatementExecuteQuery` (apache/arrow-adbc#4469). A diff of the 0.23.0 and 0.24.0 sources in the local cargo registry found no change to the `Driver`, `Database`, `Connection`, `Statement`, or `Optionable` traits. `InfoCode` gains an `Other(u32)` variant, but the enum was already `#[non_exhaustive]`, and `src/adbc_ffi.rs` names no `InfoCode` variant. The public functions of `adbc_driver_manager` are unchanged. All three 0.24.0 crates require `arrow-array` and `arrow-schema` `>=58, <60`, which admits the pinned arrow 58, and Rust 1.85, below the CI toolchain 1.92.0. The PR #89 plan changes neither `Cargo.toml` nor `Cargo.lock`.
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
  - Split the batch in `src/adbc_ffi.rs`: rejected, because only the native transport knows the encoded message size. An estimate from the Arrow batch or from the JSON parameter values has no fixed bound, for example a one-digit integer takes 1 byte as JSON and 9 bytes on the native wire. The FFI layer would also repeat wire-format knowledge.
  - Reject an oversize batch before sending: rejected, because a large `executemany` that the per-row loop completes today would fail.
  - Send one execution as several data messages through the separate `total_rows` and `rows_in_msg` header fields: rejected, because neither the Exasol WebSocket API documentation nor this codebase describes how a client sends the remaining rows of one execution, so the plan cannot specify or verify it.
  - Split at a fixed row count: rejected, because rows differ in width, so no row count bounds the message size.
- **Rationale:** Exasol documents `maxDataMessageSize` in its login response as the "maximum size of a data message in bytes". The local `exasol/docker-db` 2026.1.0 container reports 67,108,864 bytes (64 MiB), which equals the crate's `MAX_DATA_MESSAGE_SIZE` fallback. `NativeTcpTransport::build_execute_prepared_payload` writes the whole parameter set as one message. A reviewer probe on the native transport called `Connection::execute_batch_update` with rows of about 54 encoded bytes. 2,000,000 rows (about 108 MB) succeeded. 2,500,000 rows (about 135 MB) failed with `Failed to send message: Broken pipe (os error 32)`, and the next query on the same connection failed the same way. Earlier probes over the WebSocket transport sent 100,000, 1,000,000, and 3,000,000 two-column rows (2.5 MB, 26 MB, and 80 MB of JSON) in one execution each, and all three succeeded. The native transport writes the parameter values row by row after the column metadata (`write_parameter_rows`), so a range of consecutive rows is a contiguous span of the payload, and the transport can measure each range exactly. Splitting at the reported size keeps every message within the documented limit unless the message holds a single row, and each message of the per-row loop holds a single row today. No batch that the per-row loop completes today breaks the connection, so the plan makes this choice in headless mode without a human answer. Rejecting an oversize batch remains the alternative if the requester prefers it.
- **Consequences:**
  - A bound batch whose parameter values fit in one data message runs as one execution and takes effect for all of its rows or for none (entry [4]). 64 MiB holds about 1,200,000 rows of 54 encoded bytes, or about 33,000 rows that each carry one 2,000-character string.
  - A larger batch runs as consecutive executions. With autocommit on, a failing execution leaves the rows of the earlier executions committed. With autocommit off, the earlier executions belong to the open transaction, and the probe of entry [4] showed that an Exasol error rolls that transaction back.
  - The connection stays usable after an oversize batch.
  - `Connection::execute_batch_update` in the Rust API gets the same behavior, because the split lives in the native transport.
  - The WebSocket transport keeps one message per execution and was not probed above 80 MB of JSON. It is opt-in, and PR #89 changes `src/transport/websocket.rs`.
  - A bound execution holds three converted copies of the whole batch in driver memory at once: the row-major `Parameter` rows that `src/adbc_ffi.rs` builds, the column-major `serde_json::Value` parameter data that `Connection::execute_batch_update` and `Connection::execute_batch` build with `PreparedStatement::build_batch_parameters_data`, and the rows that the native transport encodes. A split batch also holds the data message of one range, up to the maximum data message size. Each `Parameter` and each `serde_json::Value` is an enum value, so a numeric value takes more memory than its 4 or 8 bytes in Arrow. A bound batch therefore needs several times its Arrow size in driver memory, and a large batch that the per-row loop completes today can exceed the available memory. Bulk ingestion through `adbc.ingest.target_table` stays the documented path for large loads.
  - The plan changes `src/transport/native/mod.rs` and the § Constraints section of `specs/architecture.md` (entry [11]).
- **Promotes to ADR:** no

### [9] FFI bound execution moves to the new feature adbc-driver/ffi-statement-execution

- **Decision:** The new feature holds ten scenarios: bound execution through `execute` and `execute_update`, the failure cases, the zero-row case, a bound batch larger than one data message, and the -1 row count. `prepared-statements/binding-and-execution` drops its three FFI scenarios, "Single execution with parameters", "FFI bind and execute_update with parameters", and "FFI bind and execute_query with parameters", and its description points to the new feature.
- **Alternatives:** Add the scenarios to `prepared-statements/binding-and-execution`: rejected, because the feature would hold 13 scenarios, above the threshold of 10, and FFI behavior would stay split across two features. Add the -1 scenario to `adbc-driver/driver-interface` (13 scenarios) or to `adbc-driver/statement-and-results` (10 scenarios): rejected for the same threshold. Add the scenarios to `prepared-statements/batch-execution`: rejected, because that feature specifies the `Connection` batch API, not the ADBC Statement.
- **Rationale:** One feature then owns what the ADBC Statement does with a bound batch. The new feature restates the three removed scenarios with the single-execution rule, so no requirement is lost.
- **Promotes to ADR:** no

### [10] A zero-row bound batch runs no execution

- **Decision:** With a zero-row bound batch, `execute_update` returns 0 and `execute` returns a reader with no batches and an empty schema, and neither sends an execution request. The statement is still prepared first, as today, so an invalid SQL text still fails.
- **Alternatives:** Pass the empty batch to `Connection::execute_batch_update`: rejected, because `build_batch_parameters_data` turns zero rows into no parameter data, and the answer of Exasol to a parameterized statement without parameter data is not verified. Return the result-set schema of a SELECT from the prepare metadata: rejected as out of scope; the reader keeps the empty schema it has today.
- **Rationale:** ADBC runs a bound statement once per bound row, so zero rows means zero executions. The per-row loop already behaves this way. ADR-004 and the `adbc-driver/statement-and-results` scenario "Empty result sets" cover a query that runs and returns no rows. With a zero-row bound batch no query runs, so neither applies.
- **Promotes to ADR:** no

### [11] The plan changes one architecture constraint and overlaps PR #89 in three places

- **Decision:** The architecture delta changes § Constraints by one bullet: the native transport runs an oversize parameter set of a row-count statement as consecutive executions within the server's maximum data message size (entry [8]). The plan changes `src/adbc_ffi.rs`, `src/transport/native/mod.rs`, `tests/driver_manager_tests.rs`, `Cargo.toml`, `Cargo.lock`, `docs/driver-manager.md`, `docs/prepared-statements.md`, `specs/mission.md`, and `CHANGELOG.md`. It changes nothing in `src/adbc/connection.rs`, `src/query/`, `src/transport/protocol.rs`, `src/transport/websocket.rs`, or `tests/integration_tests.rs`.
- **Alternatives:** Keep the plan out of `src/transport/native/mod.rs`: rejected in entry [8], because only the native transport knows the encoded message size.
- **Rationale:** § Constraints already states the fetch-side rule for the same server size, so the send-side rule belongs next to it. No component, boundary, interface, data flow, or external dependency changes: `adbc_ffi` still maps statement binding onto the adbc Connection, and the adbc crates are libraries that External Dependencies does not list. The PR #89 plan changes `src/transport/native/mod.rs` (`connect`, the lifecycle steps, the state guards, and the native test module), the HTTP transport, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `docs/setup-and-connect.md`, `docs/import-export.md`, `CHANGELOG.md`, and § Components, § Data Flow, and § Constraints of `specs/architecture.md`. The PR #89 plan leaves `src/adbc/connection.rs` and `src/adbc_ffi.rs` unchanged and lists its changes to `src/adbc/connection.rs` as follow-ups. The source overlap with PR #89 is `src/transport/native/mod.rs`: task 4.2 of PR #89 replaces the guard at the top of `execute_prepared_statement` and changes the native test module, and tasks 3.2 and 3.3 of this plan change the same function and test module.
- **Consequences:**
  - In `src/transport/native/mod.rs`, this plan adds one private function next to `build_execute_prepared_payload`, changes the body of `execute_prepared_statement` below its state guard, and adds unit tests to the test module. The PR that merges second resolves any text conflict next to the guard and in the test module.
  - Both architecture deltas change § Constraints. The plan that is recorded second re-bases its block and its BASE hash on the recorded section.
  - Both plans add entries under `## [Unreleased]` in `CHANGELOG.md`. The PR that merges second resolves the text conflict there.
  - The existing tests `test_execute_batch_update` and `test_execute_batch_select_single_row` cover the changed `prepared-statements/batch-execution` scenarios and keep their doc comments without a `/// Scenario:` line, so this plan does not edit `tests/integration_tests.rs`.
  - This plan calls the `Connection` batch methods without changing them, so the PR #89 changes to the connect path do not interact with it.
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
