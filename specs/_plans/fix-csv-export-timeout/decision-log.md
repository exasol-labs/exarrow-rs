# Decision Log: fix-csv-export-timeout

## Interview

No live interview took place. `/speq:plan-pr` ran this plan in headless mode, so the orchestrator passed GitHub issue exasol-labs/exarrow-rs#52 as the full requirement and the planner assumed the conventional defaults recorded below.

**Q:** What behavior does issue #52 require?
**A:** `CsvExportOptions.timeout_ms` is a fixed `u64` defaulting to 300,000, and `export_to_callback` wraps the whole export in `tokio::time::timeout` with it. A long export fails with `Export timed out after 300000ms` even when the server has no query timeout. The issue asks for the field to become optional, defaulting to no timeout, so the server enforces its own query timeout instead.

**Q:** Which precedent in this codebase governs the change?
**A:** Version 0.14.0 applied the same fix to query execution. `ConnectionParams::query_timeout` is `Option<Duration>` defaulting to `None`, `Statement::timeout_ms` is `Option<u64>` defaulting to `None`, and `specs/_decision/008-remove-query-timeout.md` records five accepted ADRs. The issue names this precedent and asks the change to follow the same pattern.

**Q:** Is the exapump CLI option in scope?
**A:** No. The issue states that exasol-labs/exapump#38 tracks it in a separate repo and is out of scope.

**Q:** Which decision did the orchestrator flag as the planner's call rather than an escalation?
**A:** Whether `timeout_ms` becomes `Option<u64>` or `Option<Duration>`. See decision 1.

## Design Decisions

### [1] Represent the export timeout as `Option<u64>` milliseconds, not `Option<Duration>`

- **Decision:** `CsvExportOptions::timeout_ms` becomes `Option<u64>` with default `None`, keeping the millisecond unit and the field name.
- **Alternatives:** `Option<Duration>`, matching `ConnectionParams::query_timeout`. Rejected because it would make the field name `timeout_ms` inaccurate, forcing a rename to `timeout` as a second breaking change beyond what issue #52 asks for.
- **Rationale:** `Statement::timeout_ms: Option<u64>`, introduced by the same 0.14.0 change, is the closer in-repo precedent for a millisecond-suffixed field. Issue #52 names the millisecond field directly. `ExportError::Timeout { timeout_ms: u64 }` already reports milliseconds, so the error type needs no change.
- **Promotes to ADR:** yes

### [2] Keep an opt-in client-side export timer instead of deleting the knob

- **Decision:** `Some(ms)` still arms a client-side `tokio::time::timeout` around SQL execution, tunnel transfer, and the callback. Only the default changes.
- **Alternatives:** Delete `timeout_ms` and `ExportError::Timeout` entirely and rely on the server-enforced `queryTimeout` session attribute, which is what ADR `server-enforced-query-timeout` did for query execution. Also considered: reinterpret `Some(ms)` as a server attribute set before the EXPORT statement, mirroring 0.14.0's mechanism exactly.
- **Rationale:** The client-side timer bounds work a server attribute cannot reach: the caller's callback, and a tunnel that stalls while the EXPORT statement still runs server-side. `src/transport/http_transport.rs` has no read timeout, so nothing else bounds either case. That reason is verifiable in this repo and carries the decision on its own. The server-attribute variant was rejected for a second reason: `export_to_callback` receives only a `&mut T` bounded by `TransportProtocol` and cannot reconcile the session's applied `queryTimeout` value, so setting the attribute behind `execute_statement`'s back would leak a stale timeout onto the next statement. ADR `query-timeout-bidirectional-reconcile` records that exact defect as a plan-review blocker.
- **Assumption:** assumes exasol-labs/exapump#38 binds a CLI flag to `CsvExportOptions::timeout_ms` rather than to the `query_timeout=` connection parameter. Unverified from this repo: issue #52 says only that exapump#38 "tracks the matching CLI option". This assumption is supporting, not load-bearing.
- **Scope of the prior ADR:** ADR `query-timeout-option-semantics` closes with "No arm constructs a client-side timer". That clause is scoped to `ConnectionParams::query_timeout` and `Statement::timeout_ms`, which both map onto the server-enforced `queryTimeout` session attribute. CSV export has no equivalent attribute of its own, so this decision keeps a client-side arm and does not reverse that ADR.
- **Promotes to ADR:** yes

### [3] Add `TransportProtocol::terminate()` for a give-up that cannot trust the response stream

- **Decision:** Add a required trait method `fn terminate(&mut self)` that drops the socket and marks the transport disconnected without any protocol round-trip. `export_to_callback` calls it when the export timer elapses, before returning `ExportError::Timeout`.
- **Alternatives:** Call the existing `close()`. Rejected because `close()` sends a disconnect command and awaits its response, so after an abandoned EXPORT it would wait for that EXPORT's response and block for as long as the statement keeps running. Also considered: bound the disconnect round-trip inside both `close()` implementations with a short timer. Rejected because it changes every connection-close path and invents a magic wait value, and because it would leave the export path depending on an undocumented property of `close()`.
- **Rationale:** ADR `client-give-up-terminates-connection` requires that any client-side give-up terminate the connection rather than abandon the in-flight request. Until now no code path implemented that rule, because 0.14.0 removed the only give-up path that existed. `terminate()` gives the rule an implementation that the unimplemented `cancel` path can reuse. It is additive, so no existing behavior changes.
- **Assumption:** assumes Exasol releases a session when the client socket closes. This is load-bearing for the ADR's stated benefit, "release the server session immediately instead of leaking it until idle-reap", because `terminate()` deliberately sends no disconnect command and tells the server nothing. Unverified when this plan was written. Task 4.3 checks it against `SYS.EXA_ALL_SESSIONS` and escalates if the abandoned session survives, in which case `terminate()` satisfies the ADR's letter and not its purpose.
- **Promotes to ADR:** yes

### [4] Put CSV-export scenario tests in `tests/integration_tests.rs`, not `tests/import_export_tests.rs`

- **Decision:** The four CI-relevant scenario tests go into the existing Query Timeout Tests section of `tests/integration_tests.rs`. Only the eight-minute regression proof goes into `tests/import_export_tests.rs`. Task 0.1 confirms HTTP-tunnel export works against the pinned CI image first, because that path has never run in the CI integration job.
- **Alternatives:** Put all five in `tests/import_export_tests.rs` with the other export tests, and add a CI step that runs the whole suite. Rejected as scope creep with unknown cost: the suite is 91 KB of tests, several of which insert 20,000 rows, and its runtime against the CI container has never been measured. If task 0.1 shows the tunnel unusable in CI, the fallback is a CI step scoped to the new test names only, not the whole suite.
- **Rationale:** `.github/workflows/ci.yml` runs `integration_tests`, `websocket_integration_tests`, and `driver_manager_tests`. It never runs `import_export_tests`, so a scenario test placed there proves nothing on a pull request. The Query Timeout Tests section already owns `long_running_count_query` and `disable_query_cache`, which this plan promotes to `tests/common/mod.rs` so both suites can use them.
- **Promotes to ADR:** yes

### [5] Keep the builder argument non-optional

- **Decision:** `CsvExportOptions::timeout_ms(timeout: u64)` keeps its signature and wraps the argument in `Some`.
- **Alternatives:** Accept `Option<u64>`, or add a separate `no_timeout()` method. Rejected: the first breaks every existing call site for no gain, and the second is redundant because the default is already `None` and the field is public.
- **Rationale:** Mirrors `ConnectionParams::query_timeout(Duration)`, which also takes a non-optional argument and wraps it. It also confines the breaking change to direct field access.
- **Promotes to ADR:** no

### [6] Prove the removed 300-second bound with an opt-in long test

- **Decision:** `test_csv_export_runs_past_the_former_five_minute_limit` is `#[ignore]`d and additionally gated on `EXARROW_LONG_EXPORT_CHECK`, so neither the default suite nor `cargo test --test import_export_tests -- --ignored` pays its cost. Its size comes from a named `side_rows` default of 650,000, overridable through `EXARROW_LONG_EXPORT_SIDE_ROWS`, and it asserts that the export completes without `ExportError::Timeout` rather than asserting a wall-clock figure, which machine speed moves in both directions.
- **Alternatives:** No long test at all, relying on the unit assertion that the default is `None`. Rejected because no other test in the plan fails on the pre-fix code, and a plan whose central claim has no regression proof cannot be verified. Also considered: run it unconditionally in the manual suite. Rejected because it would add roughly eight minutes to a documented developer command.
- **Rationale:** It is the only test that distinguishes the old behavior from the new one end to end. Gating it keeps that proof available as a named manual command without taxing every run.
- **Promotes to ADR:** no

### [7] Make `Connection::is_closed()` read the transport as well as the session

- **Decision:** `Connection::is_closed()` becomes `self.session.is_closed().await || !self.transport.lock().await.is_connected()`, so a terminated transport reports the connection as closed. The caller also learns the outcome directly from `ExportError::Timeout::transport_terminated`, so no caller needs to probe.
- **Alternatives:** Leave `is_closed()` reading session state alone and specify the disagreement, which earlier revisions of this plan did. Rejected: it would freeze a `SHALL NOT` into the permanent library making one fact, this connection is dead, have two owners that must disagree by spec, and a library `SHALL NOT` is far more expensive to undo than one method body. Also considered: mark the session closed from all six `Connection` export entry points (`export_csv_to_file`, `export_csv_to_stream`, `export_csv_to_list`, `export_to_parquet`, `export_to_record_batches`, `export_to_arrow_ipc`). Rejected because it spreads one decision across six call sites.
- **Rationale:** The cheap option was missed in earlier revisions. `is_closed()` is a two-line `async fn` (`src/adbc/connection.rs:1053-1055`) and `self.transport` is an `Arc<Mutex<dyn TransportProtocol>>` reachable from `&self`, so the fix is one edit in one method plus one test update. No internal caller holds the transport lock while calling `is_closed()`, so the added `lock().await` introduces no deadlock: `is_closed()` takes `&self` while every operation takes `&mut self`, which Rust already serializes.
- **Promotes to ADR:** no

### [8] Add no timeout knob to `ArrowExportOptions` or `ParquetExportOptions`

- **Decision:** Neither struct gains a timeout field. Both keep building their CSV options from `CsvExportOptions::default()`, so both inherit the new `None` default.
- **Alternatives:** Add a matching field to each. Rejected as speculative: no issue asks for it, and both structs already omit every other CSV knob they do not need.
- **Rationale:** Both paths silently inherited the 300-second bound, so the default change fixes them at no cost. Adding a knob would be a configuration parameter standing in for a decision the module can already make.
- **Promotes to ADR:** no

### [9] Release the change as 0.16.0

- **Decision:** Recommend a minor bump to 0.16.0 with a `CHANGELOG.md` entry naming all four breaking changes. `/speq:implement-pr` performs the bump.
- **Alternatives:** A major bump to 1.0.0. Rejected because the crate is pre-1.0 and both 0.13.0 and 0.14.0 shipped breaking changes in the minor slot.
- **Rationale:** Consistency with the crate's own release history, which AGENTS.md ties to a mandatory CHANGELOG entry per version bump.
- **Promotes to ADR:** no

### [10] Specify `terminate()` in `connection-management/session-and-lifecycle`

- **Decision:** The termination requirement gets a scenario in `connection-management/session-and-lifecycle`, and the export-visible consequence stays in `import-export/csv-export`.
- **Alternatives:** Specify it only in `import-export/csv-export`, or add parallel scenarios to `native-client/protocol` and `websocket-client/protocol`. Rejected: the first leaves a cross-transport requirement with no home, and the second duplicates one requirement across two transport features.
- **Rationale:** The `session-and-lifecycle` Background already carries the rule that a client-side give-up MUST terminate the connection. The scenario that implements that rule belongs in the same feature, stated once for both transports.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] The default path has no bound when the EXPORT statement fails

- **Finding:** `plan-reviewer` traced `export_to_callback` and found it awaits `tokio::join!(http_task, callback_task)` before propagating the SQL error (`src/export/csv.rs:490-496`). The HTTP task blocks in `handle_export_request()`, and `src/transport/http_transport.rs` has no read timeout at all, confirmed by `grep -c timeout` returning 0. Today the 300-second wrap is the only guarantee that the call returns. Removing the default bound would leave the return conditional on Exasol closing the tunnel socket, an unverified assumption about server behavior. Task 4.2 would have hit it in the CI job, where `timeout-minutes: 30` turns a hang into a job failure.
- **Direction change:** Task 3.1 now requires a short-circuit: a failed `sql_task` aborts `http_task`, drops the callback future, and returns the SQL error without awaiting the join. The "Server-enforced timeout governs an export" scenario gained a matching normative bullet, the Design Context and Architecture diagram record the mechanism, and Impact paragraph 2 states that a tunnel stalling while the statement still runs has no client-side bound unless the caller sets one.
- **Promotes to ADR:** yes

### [2] [plan-review] The `JoinHandle` restructure would leak the HTTP task on three exit paths

- **Finding:** Task 3.1 defined the handle's disposition only on the elapse path. The handle is currently moved into `tokio::join!`, so it is awaited on every path. Once it is borrowed instead, the SQL error return, the HTTP join error, and a callback error would each drop it, and dropping a `JoinHandle` detaches the task. The restructure would have introduced the detached-task leak that the plan cites as a current defect, on paths that do not leak today.
- **Direction change:** Task 3.1 now states the invariant for every exit: each return path out of `export_to_callback` MUST either await or abort `http_task`, and it names all four paths. The Patterns table records the invariant alongside the borrow.
- **Promotes to ADR:** no

### [3] [plan-review] Unconditional termination would destroy a healthy connection

- **Finding:** The timed region spans SQL execution, tunnel transfer, and the callback. When the timer elapses after `sql_task.await` returned, the EXPORT response is already consumed and the transport is in sync, so nothing is unmatchable. Terminating there breaks a healthy connection, and decision [2] names a slow callback as the knob's primary use case, so that branch is the expected one rather than an edge case.
- **Direction change:** The "Explicit export timeout" scenario now splits by state: terminate when the timeout elapses before the EXPORT response is consumed, leave the transport usable when it elapses after. The scenario title dropped "terminates the connection", which asserted the unconditional behavior. Task 3.1 tracks the SQL await's completion in a flag owned outside the timed block, because the timed future is dropped on elapse. Impact paragraph 3 and the Consequences table now state the condition, and new task 4.4 covers the non-terminating branch with a deliberately slow writer.
- **Promotes to ADR:** yes

### [4] [plan-review] Three new tests would pass vacuously on a cached query result

- **Finding:** Tasks 4.1 to 4.3 land in a test section whose Background makes `disable_query_cache` mandatory, with a documented cache effect of about 4 seconds cold against about 0.1 seconds cached (`tests/integration_tests.rs:2949-2954`). None of the three called it, and tasks 4.2 and 4.3 both reused `long_running_count_query(100_000)`, already used by two existing tests, one of which runs it to completion. A cache hit would return in about 0.1 seconds, so task 4.2 would pass without a server abort and task 4.3 would fail without ever arming an elapse.
- **Direction change:** All four cartesian-product tests now call `disable_query_cache(&mut conn)` immediately after connecting, and each uses a `side_rows` value used nowhere else in the file: 110,000, 120,000, 130,000, and 650,000 for the long check. The task list records which values the file already uses.
- **Promotes to ADR:** no

### [5] [plan-review] Splitting the type change from its only consumer left the crate uncompilable

- **Finding:** Task 3.1 changed `timeout_ms` to `Option<u64>` in Group A, while its only consumers at `src/export/csv.rs:486` and `:505` were fixed two sequential groups later. The crate would not compile between the two, so neither Group B task could verify its own unit test green.
- **Direction change:** The type change and the `export_to_callback` rework merged into one `[expert]` task. Round 2 found the same defect relocated one seam earlier and merged the transport tasks too; see finding [6].
- **Promotes to ADR:** no

### [6] [plan-review] Round 2: the same compile break relocated to the Group A seam

- **Finding:** `plan-reviewer` found that round 1's fix moved the defect rather than removing it. Task 2.1 added `terminate()` as a required trait method with no default body and updated only the mock, leaving both real implementors incomplete: `NativeTcpTransport` (`src/transport/native/mod.rs:760`) in task 2.2 and `WebSocketTransport` (`src/transport/websocket.rs:403`) in task 2.3. `cargo build` would fail at the Group A boundary, and the Parallelization section asserted the opposite. Both Group B tasks specify a unit test, so each agent would have had to write a failing test in a tree that does not compile and could not attribute its own red. The reviewer's premortem names the likely unblock: give `terminate()` a default no-op body and ship a WebSocket transport that never terminates anything, undetected because its unit tests are compile-checked only in CI.
- **Direction change:** Tasks 2.1, 2.2, and 2.3 merged into one `[expert]` task 2.1 that declares the method, adds it to the mock, and implements it on both transports with one unit test each. Group B is deleted, the merged task joins Group A, the remaining groups are renumbered A to D, and the compile claim now covers both merged tasks. The human approved this merge on PR #54.
- **Promotes to ADR:** no

### [7] [plan-review] Round 2: `ExportError::Timeout` discarded the branch the driver knew

- **Finding:** A caller catching `ExportError::Timeout` could not tell which branch fired. The variant carries no discriminator, decision [7] leaves `Connection::is_closed()` returning `false` after termination, and task 3.2 resolved the gap in prose with "the connection may have been terminated". The driver knows the answer when it constructs the error, so the hedge discarded information the code held, forcing every caller to either reconnect defensively or issue a probe statement whose failure is indistinguishable from any other failure.
- **Direction change:** The field was added to `ExportError::Timeout`, set from the same flag that gates the `terminate()` call, with `Display` stating the outcome. The "Explicit export timeout stops the export" scenario requires the error to report it, Impact carries it as a breaking change, the Migration table gains a row, and both branch tests assert the field. Round 3 renamed it `transport_terminated` and folded the work into task 3.1; see finding [17].
- **Promotes to ADR:** no

### [8] [plan-review] Round 2: task 4.4 had no timing margin for the SQL leg

- **Finding:** The timer starts before `sql_task.await`, so task 4.4's 2-second bound had to cover the tunnel connect, the EXPORT statement, the full tunnel read, and the HTTP 200 before any budget was left for the callback. On a cold container the SQL leg alone can exceed 2 seconds, leaving `sql_done` false at elapse. The driver would terminate, and the test would fail asserting a usable connection. Task 4.4 is the only coverage of the non-terminating branch, so a flake there would quarantine the exact behavior round-1 finding [3] was raised to protect.
- **Direction change:** The bound rises to 10 seconds and the writer stall to 30 seconds, the task states why each margin exists, and the test performs one small throwaway export first to warm the tunnel path.
- **Promotes to ADR:** no

### [9] [plan-review] Round 2: the borrow region for the progress flag was unstated

- **Finding:** The plan said the flag is "owned outside the timed block" but not which borrow region it must survive. The elapse path needs the flag and `&mut ws_transport` back, and `sql_task` holds `&mut *ws_transport` for the whole timed future (`src/export/csv.rs:472-479`). Writing `match tokio::time::timeout(dur, work).await { ... }` keeps the temporary alive across the arms, so both the flag read and the `terminate()` reborrow fail borrow-check, and an implementer would be tempted to move `terminate()` into the `map_err` closure where the same borrow is still held.
- **Direction change:** Task 3.1 bullet 4 now requires binding the timeout result to its own `let` statement so every borrow is released before the elapse handling runs, and names `std::cell::Cell<bool>` borrowed immutably as the preferred shape, which removes the conflict outright.
- **Promotes to ADR:** no

### [10] [plan-review] Round 2: the recorded "Query timeout" scenario would contradict this plan

- **Finding:** The recorded scenario ends "the driver MUST NOT wrap query execution in a client-side timer" under a GIVEN that includes the `query_timeout=` connection-string parameter. A caller who sets both `query_timeout=` and `timeout_ms` puts `ws_transport.execute_query(&export_sql)` inside `tokio::time::timeout` while that GIVEN holds. The scoping argument existed only in plan.md and the decision log, both of which `/speq:record` archives, so the permanent library would carry the contradiction.
- **Direction change:** The session-and-lifecycle delta now wraps the recorded "Query timeout" scenario in `<!-- DELTA:CHANGED -->` and narrows the prohibition to query execution through `Connection::execute_statement()`, stating in the same bullet that an explicit `CsvExportOptions::timeout_ms` bounds an export's combined SQL, transfer, and callback and is outside the prohibition. The scoping now lands in the permanent library rather than in an archived artifact.
- **Promotes to ADR:** yes

### [11] [plan-review] Round 2: the Migration table still stated unconditional termination

- **Finding:** Round-1 finding [3]'s fix reached Impact and Consequences but not Migration, whose last row still read "Export timeout terminates the transport; reconnect before the next operation". Migration is the row a CHANGELOG author and an upgrading caller read first, so the one artifact stating the superseded behavior was the one aimed at the audience that acts on it.
- **Direction change:** The row now states that termination happens only when the timeout elapses before the EXPORT response is read.
- **Promotes to ADR:** no

### [12] [plan-review] Round 2: task 3.3 would have given one decision two owners

- **Finding:** Task 3.3 planned one extracted function per module, but `src/export/arrow.rs:739-746` and `src/export/parquet.rs:758-765` hold a character-for-character identical builder chain over the same five inputs. Extracting twice would promote an accidental duplication into a deliberate one, with nothing keeping the two owners in agreement, and would pay for it again with two unit tests.
- **Direction change:** The task now adds one `pub(crate)` function in `src/export/csv.rs` that both modules call, with one unit test. Round 3 restated its rationale as duplication removal rather than service to a spec bullet the plan itself introduced, replaced the five positional parameters with one named-field struct, and renumbered it 3.2; see finding [18].
- **Promotes to ADR:** no

### [13] [plan-review] Round 2: Group E scheduled four concurrent writes to one file

- **Finding:** Group E ran tasks 4.1 to 4.4 in parallel, and all four append a test to the same Query Timeout Tests section of `tests/integration_tests.rs`.
- **Direction change:** Those four now run sequentially in that order. The accompanying claim that "every other group in the plan is file-disjoint by construction" was wrong: round 3 found the same defect still live in Group C, where tasks 3.2 and 3.3 both wrote `src/export/csv.rs`. See finding [19].
- **Promotes to ADR:** no

### [14] [plan-review] Round 2: three Impact sentences exceeded the 25-word cap

- **Finding:** Governed Impact prose carried a 42-word sentence holding two ideas, plus a 26-word and a 27-word sentence.
- **Direction change:** The 42-word sentence split into two, ", which 0.14.0 introduced" was cut, and the 27-word sentence split at its semicolon.
- **Promotes to ADR:** no

### [15] [plan-review] Round 3: the unscoped prohibition survived in `## Background`

- **Finding:** Round 2's fix narrowed the "Query timeout" scenario but left the session-and-lifecycle `## Background` byte-identical to the recorded one, still reading "the driver SHALL NOT wrap query execution in a client-side timer". Background prose has no `WHEN` to scope it and governs every scenario in the feature unconditionally, so the conflict was stronger there than in the scenario that was fixed. Finding [10]'s claim that "the scoping now lands in the permanent library" was half true, and the untouched half was the normative prose `/speq:record` carries forward. The reviewer's premortem is concrete: a future planner reads the recorded Background, deletes the export timer as a spec violation, and reintroduces the inverse of issue #52.
- **Direction change:** `## Background` is now wrapped in `<!-- DELTA:CHANGED -->` markers, the clause is narrowed to "through `Connection::execute_statement()`", and one sentence points to the export-scoped setting in `import-export/csv-export`. plan.md § Features states that this delta changes Background prose and that the amendment MUST be merged, since `/speq:record` merges marked `## Scenarios` by default and this repo's CLAUDE.md flags unmarked Background edits as a known hazard.
- **Promotes to ADR:** yes

### [16] [plan-review] Round 3: Group C still had two concurrent writers on one file

- **Finding:** Tasks 3.2 and 3.3 both edited `src/export/csv.rs`, the identical defect finding [13] fixed for the old Group E. The loss would have been silent, because whichever write survived still compiles, and it would have surfaced two groups later as four tests failing on a missing error field.
- **Direction change:** Task 3.2's error-field work merged into task 3.1, which already reworks `export_to_callback` and already sets the flag the field reports. The former task 3.3 is renumbered 3.2 and holds Group C alone with the documentation task. Finding [13]'s over-broad file-disjointness claim is corrected, and the Parallelization section now names the disjoint files per group instead of asserting disjointness.
- **Promotes to ADR:** no

### [17] [plan-review] Round 3: the plan's central claim had no test that runs anywhere

- **Finding:** The "No client-side export timeout by default" scenario required an export running longer than 300 seconds, but its mapped test exports about 13 seconds of work by the plan's own scaling model, missing the condition by a factor of 20. The only test that could satisfy it was `#[ignore]`d and gated on `EXARROW_LONG_EXPORT_CHECK`, which no Checklist command set. The mutation `options.timeout_ms.unwrap_or(300_000)` would have survived every automated check the plan defined.
- **Direction change:** The scenario's `WHEN` is rewritten to "however long its combined SQL execution and data transfer run", so the requirement is the timer's absence rather than a duration threshold. The Checklist "Export tests" row now sets `EXARROW_LONG_EXPORT_CHECK=1`, so the documented verification executes task 4.5. The first Coverage-limits bullet states plainly that no automatically-run test crosses the former bound and names the one command that does. The Arrow and Parquet construction fact moved out of that scenario into its own `DELTA:NEW` scenario, since it is static and runs no export.
- **Promotes to ADR:** no

### [18] [plan-review] Round 3: eight further advisories

- **Finding:** The reviewer raised eight non-blocking findings, all verified against the tree before acting: `ExportError::Timeout`'s field name contradicted the plan's own spec wording and both `Display` strings were unspecified; task 4.3's assertion pattern was stale and its conventional `conn.close()` cleanup would panic on a terminated transport; the WebSocket half of `terminate()` had zero executed CI coverage, since `Cargo.toml` sets `default = ["native"]` and `tests/websocket_integration_tests.rs` contains no export test; task 0.1 raced tasks 1.1 and 2.1 for the target-directory lock and its failure branch had no owner; the blocking export wrappers were absent from Impact; the prescribed `terminate()` body duplicated the tail of `close()` and its "performs no I/O" test was not writable; the shared-options helper's five positional parameters included two transposable adjacent `char`s; and governed Testing and Parallelization prose still broke the 25-word cap.
- **Direction change:** Field renamed `transport_terminated` with both `Display` texts specified verbatim; task 4.3 gains the correct pattern, a no-`close()` instruction, and a `SYS.EXA_ALL_SESSIONS` check for decision [3]'s new assumption; task 2.2 adds an executed WebSocket integration test; task 0.1 runs alone ahead of Group A with an explicit escalate-on-failure branch and a conditional task 6.1 for the approved fallback; Impact names `blocking_export_csv_to_file` and `blocking_export_to_parquet`; task 2.1 requires `close()` to delegate to `terminate()` and states the non-async signature in the trait doc comment instead of asserting it in a test; the helper takes one named-field struct; and the three long sentences are split.
- **Promotes to ADR:** no

### [19] [plan-review] Round 3: the AND-step warning was not a reason to fold a requirement

- **Finding:** Round 2 folded the export carve-out into the "Query timeout" prohibition bullet to avoid a four-AND-step validator warning. The reviewer showed the reasoning was wrong: the threshold is a non-blocking `WARN`, and the library already carries it, confirmed by `speq feature validate adbc-driver/transactions` reporting exactly that warning today. The fold produced a 38-word bullet carrying a prohibition and a carve-out, with the carve-out stated in no RFC-2119 keyword and unreachable from the scenario's own `GIVEN`.
- **Direction change:** The carve-out is cut from the bullet, leaving one singular prohibition. It now lives only in the Background amendment from finding [15], which is where it is reachable. Avoiding an advisory warning is not a reason to write a non-singular requirement, and future rounds should not treat a clean validator run as evidence of requirement quality.
- **Promotes to ADR:** yes
