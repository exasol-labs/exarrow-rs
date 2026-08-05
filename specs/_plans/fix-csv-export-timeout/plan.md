# Plan: fix-csv-export-timeout

> **Status:** blocked — see open-questions.md

## Summary

`CsvExportOptions::timeout_ms` becomes `Option<u64>` defaulting to `None`, so a long export runs until the server finishes it. A caller who wants a client-side bound sets it explicitly, and an elapsed bound no longer leaves the transport desynced.

## Design

### Context

`export_to_callback` wraps SQL execution, HTTP tunnel transfer, and the caller's callback in `tokio::time::timeout(Duration::from_millis(options.timeout_ms), ...)` with a default of 300,000 ms (`src/export/csv.rs:103`, `:125`, `:486`). The value is a plain `u64`, so no caller can remove the bound. An export that legitimately runs longer than five minutes fails with `Export timed out after 300000ms` even when the server has no query timeout set. GitHub issue #52 reports this.

Version 0.14.0 fixed the same defect class for query execution and recorded the reasoning in `specs/_decision/008-remove-query-timeout.md`. Two of its accepted ADRs constrain this plan:

- `query-timeout-option-semantics` settles one choice this plan inherits: represent "no timeout configured" as an `Option`, not a sentinel value, with `Statement::timeout_ms: Option<u64>` as the concrete shape. That ADR's closing clause, "No arm constructs a client-side timer", is scoped to `ConnectionParams::query_timeout` and `Statement::timeout_ms`, both of which map onto a server-enforced session attribute. Export has no such attribute of its own, so this plan keeps a client-side arm and decision [2] of the decision log records why.
- `client-give-up-terminates-connection` established that any client-side decision to give up on a running request MUST terminate the connection rather than abandon the in-flight request. The `connection-management/session-and-lifecycle` Background carries this rule.

The current export timer violates the second rule. On elapse it drops the `execute_query` future on a transport the `Connection` owns exclusively, leaving the EXPORT response unread. The next statement on that connection reads the stale response. It also drops the `JoinHandle` of the spawned HTTP task, which then runs detached while holding the tunnel socket.

Removing the default bound exposes a second defect. `export_to_callback` awaits `sql_task`, then awaits `tokio::join!(http_task, callback_task)`, and only then propagates the SQL error (`src/export/csv.rs:490-496`). The HTTP task blocks in `handle_export_request()`, and `src/transport/http_transport.rs` contains no read timeout: `grep -c timeout src/transport/http_transport.rs` returns 0. Today the 300-second wrap is the only guarantee that the call returns. Task 3.1 therefore short-circuits on a failed `sql_result` instead of relying on Exasol to close the tunnel socket.

- **Goals** - remove the fixed default bound; keep an opt-in bound that is safe to trigger; guarantee the default path returns when the EXPORT statement fails; give the "terminate on give-up" rule a real implementation.
- **Non-Goals** - the matching exapump CLI option (exasol-labs/exapump#38, separate repo); a timeout knob on `ArrowExportOptions` or `ParquetExportOptions`; a client-side bound for CSV import, which has none today; removing the `src/export/csv.rs` per-file coverage-floor exemption.

### Decision

`timeout_ms` becomes `Option<u64>` with default `None`. `None` arms no timer. `Some(ms)` arms the existing client-side timer. On elapse the export aborts the HTTP task and returns `ExportError::Timeout`, and it terminates the transport only when the EXPORT response was still unread.

That condition matters because the timed region spans SQL execution, tunnel transfer, and the callback. An elapse after `sql_task.await` returned leaves the transport in sync, so terminating would destroy a healthy connection on the knob's primary use case, a slow callback. The rework tracks whether the SQL await completed in a flag owned outside the timed block, because the timed future is dropped on elapse and cannot report its own progress.

Termination needs a path that performs no protocol round-trip. `TransportProtocol::close()` sends a disconnect command and awaits its response, so calling it after abandoning an in-flight EXPORT would block until that EXPORT finishes server-side. A new required trait method `terminate(&mut self)` drops the socket and marks the transport disconnected without any I/O.

#### Architecture

```
CsvExportOptions.timeout_ms: Option<u64>
        │
        ├─ None ────────▶ SQL + HTTP + callback, no timer
        │                 server-enforced queryTimeout / QUERY_TIMEOUT governs
        │
        └─ Some(ms) ────▶ tokio::time::timeout(ms, work)

  work (both arms):
        sql_task.await
            │
            ├─ Err ──▶ http_task.abort(); drop callback; Err(SqlExecutionError)
            │
            └─ Ok ───▶ sql_done = true
                       join!(&mut http_task, callback_task) ──▶ Ok(result)

  on elapse (Some arm only):
        http_task.abort()
        if !sql_done { transport.terminate() }
        Err(ExportError::Timeout)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| `Option` for "not configured" | `CsvExportOptions::timeout_ms` | Matches `Statement::timeout_ms: Option<u64>` per ADR `query-timeout-option-semantics`; the compiler checks absence, no sentinel to memorize |
| Builder keeps the non-optional argument | `CsvExportOptions::timeout_ms(u64)` | Mirrors `ConnectionParams::query_timeout(Duration)`, which wraps in `Some`; existing call sites keep compiling |
| Terminate without a round-trip | `TransportProtocol::terminate()` | An abandoned in-flight request makes the next response unmatchable, so a graceful close cannot be trusted to return |
| `JoinHandle` held outside the timed block, borrowed into it | `export_to_callback` | The handle must survive the timer so the elapse path can abort the HTTP task. Every exit path awaits or aborts it, because a dropped handle detaches the task |
| Fail-fast on the SQL error | `export_to_callback` | The tunnel read has no timeout, so awaiting the HTTP task before propagating a SQL error can hang forever once the default bound is gone |
| Progress flag owned outside the timed block | `export_to_callback` | The timed future is dropped on elapse, so the conditional terminate needs the flag to survive it |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| `Option<u64>` in milliseconds | `Option<Duration>`, matching `ConnectionParams::query_timeout` | `Option<Duration>` would make the field name `timeout_ms` a lie and force a second rename. `Statement::timeout_ms: Option<u64>` is the closer in-repo precedent, and issue #52 names the millisecond field directly |
| Keep an opt-in client-side timer | Delete the field and rely only on server-enforced `query_timeout` | The knob bounds the caller's callback and a tunnel that stalls while the statement still runs server-side, neither of which a server attribute reaches |
| Terminate only when the EXPORT response is unread | Terminate on every elapse | An elapse during the callback leaves the transport in sync, so terminating would break a healthy connection on the knob's primary use case |
| New `TransportProtocol::terminate()` | Bound the existing `close()` disconnect round-trip with a short timer | Bounding `close()` changes every connection-close path and invents a magic wait. `terminate()` is additive and names a distinct concept |
| Have `Connection::is_closed()` read the transport too | Leave it reading session state alone; or mark the session closed from all six `Connection` export entry points | One edit in one method beats both. Leaving it alone would spec a permanent disagreement between `is_closed()` and the socket; mutating session state from six entry points would spread one decision across six call sites |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| import-export/csv-export | CHANGED | `specs/_plans/fix-csv-export-timeout/import-export/csv-export/spec.md` |
| connection-management/session-and-lifecycle | CHANGED | `specs/_plans/fix-csv-export-timeout/connection-management/session-and-lifecycle/spec.md` |

The session-and-lifecycle delta changes `## Background` prose, not only scenarios. `/speq:record` merges marked `## Scenarios` by default, so the Background amendment is wrapped in `<!-- DELTA:CHANGED -->` markers and MUST be merged with it. Without that amendment the recorded Background keeps an unscoped "SHALL NOT wrap query execution in a client-side timer", which contradicts the export timer this plan keeps.

## Impact

This release breaks the public API in six ways.

**`CsvExportOptions::timeout_ms` changes type from `u64` to `Option<u64>`.** Code that reads or assigns the field directly no longer compiles. The builder method keeps its `u64` argument, so `.timeout_ms(60_000)` call sites are unaffected (`src/export/csv.rs:188`).

**CSV export arms no client-side timer by default.** An export that previously failed at 300 seconds now runs until the server finishes the EXPORT statement. Callers who relied on the implicit bound as a safety net must now set `timeout_ms` explicitly, or set the server-enforced `query_timeout=` connection parameter. The Arrow and Parquet export paths build their CSV options from `CsvExportOptions::default()` (`src/export/arrow.rs:740`, `src/export/parquet.rs:759`), so they lose the same implicit bound. A failed EXPORT statement now returns without waiting for the HTTP transport task. A tunnel that stalls while the statement still runs server-side has no client-side bound unless the caller sets `timeout_ms`, because `src/transport/http_transport.rs` has no read timeout. That risk is worse in `Connection::blocking_export_csv_to_file` (`src/adbc/connection.rs:2065`) and `Connection::blocking_export_to_parquet` (`:2095`): a stalled tunnel now blocks the calling OS thread indefinitely, which the caller cannot cancel.

**An elapsed explicit export timeout can terminate the transport.** The driver terminates it when the timeout elapses before the EXPORT response was read. It leaves the transport usable when the elapse happens later, for example during a slow callback. Before this change the connection stayed open with an unread EXPORT response in either case, and the next statement read that stale response. Callers that caught `ExportError::Timeout` and reused the connection got silent corruption. After a terminating elapse they now get a failure on the next operation and must reconnect. Output already written to the caller's sink may be a partial CSV document. Discard it after a timeout on either branch.

**`Connection::is_closed()` now reports a terminated transport as closed.** It previously read session state alone, so it returned `false` for a connection whose socket was gone. Code that treated `is_closed() == false` as proof of a live session sees a changed answer after a terminating export timeout.

**`ExportError::Timeout` gains a `transport_terminated: bool` field.** Code that constructs or exhaustively destructures the variant no longer compiles. The field reports which branch fired, so a caller decides whether to reconnect without probing the connection.

**`TransportProtocol` gains a required method, `terminate()`.** The trait is public through `exarrow_rs::transport`, so any external implementor must add the method. No in-repo implementor outside the two transports and the shared test mock exists.

Release: bump to 0.16.0, keeping breaking changes in the minor slot as 0.13.0 and 0.14.0 did, and add the matching `CHANGELOG.md` entry. `/speq:implement-pr` performs the bump; this plan does not.

## Dependencies

None. The change uses `tokio::time::timeout` and `JoinHandle::abort`, both already in use.

## Migration

| Current | New |
|---------|-----|
| `options.timeout_ms == 300_000` by default | `options.timeout_ms == None` by default |
| `let ms: u64 = options.timeout_ms;` | `let ms: Option<u64> = options.timeout_ms;` |
| `options.timeout_ms = 60_000;` | `options.timeout_ms = Some(60_000);` |
| `.timeout_ms(60_000)` | `.timeout_ms(60_000)` (unchanged) |
| Export timeout leaves the connection open and desynced | Export timeout terminates the transport only when it elapses before the EXPORT response is read; reconnect before the next operation in that case |
| `ExportError::Timeout { timeout_ms }` | `ExportError::Timeout { timeout_ms, transport_terminated }`; match on `transport_terminated` to decide whether to reconnect |
| `is_closed()` returns `false` after an export timeout terminated the transport | `is_closed()` returns `true` once the transport is terminated |
| Output written before an export timeout was of unstated completeness | Discard any output written to the sink after `ExportError::Timeout`; it may be a partial CSV document |

## Implementation Tasks

0. Prerequisite check, run alone before any other task
   - [ ] 0.1 Confirm an HTTP-tunnel CSV export succeeds against the CI container image `exasol/docker-db:2025.2.0` (`.github/workflows/ci.yml:229-237`), not only against `exasol/docker-db:latest`. Run one existing export test from `tests/import_export_tests.rs` against a container started from the pinned image. Tasks 4.1 to 4.4 depend on this working, because they place export tests in the CI-run suite. This task runs against an unmodified tree, so it must not run concurrently with any task that edits `src/` or `tests/`: a concurrent `cargo test` either blocks on the target-directory lock or builds a tree mid-edit.
     - If the export succeeds, proceed to Group A and skip task 6.1.
     - If the export fails, stop and escalate to the human before starting task 4.1. Do not silently relocate the tests. The fallback is task 6.1, which the human must approve, because it changes CI.
1. Shared test helpers
   - [ ] 1.1 Move `long_running_count_query` (`tests/integration_tests.rs:2958`) and `disable_query_cache` (`:2968`) into `tests/common/mod.rs` as public items, then import them in `tests/integration_tests.rs` and delete the local definitions. Mark both with `#[allow(dead_code)]`, matching the existing convention at `tests/common/mod.rs:152`, `:170`, and `:211`: that module compiles into five test binaries, and CI lints with `-D warnings` (`.github/workflows/ci.yml:81`), so an item unused by the other four binaries fails the Lint job.
2. Transport termination
   - [ ] 2.1 Add `fn terminate(&mut self)` to `TransportProtocol` and implement it everywhere in one task, because a required trait method with no default body makes every incomplete impl a compile error. Splitting the trait change from the implementations would leave the crate uncompilable at the group boundary. [expert]
     - Declare `fn terminate(&mut self)` in `src/transport/protocol.rs`. The doc comment states the design intent: drop the socket without any protocol round-trip, for use when the driver gives up on an in-flight request and can no longer match a response to it. State in the same comment that the non-async signature is what forbids awaited I/O, so no test needs to assert it.
     - Add the method to the shared mock in `src/transport/test_support.rs`.
     - Implement it on the native transport in `src/transport/native/mod.rs`: drop the stream, clear the session, set the state to `Closed`. Put that local teardown in `terminate()` and have `close()` call it after its optional disconnect round-trip, because the three lines are already the tail of `close()` and the teardown needs one owner per transport. Add a unit test asserting `is_connected()` returns `false` afterwards.
     - Implement it on the WebSocket transport in `src/transport/websocket.rs` with the same observable outcome and the same `close()` delegation, plus a matching unit test.
   - [ ] 2.2 Add `test_terminate_marks_websocket_transport_disconnected` to `tests/websocket_integration_tests.rs`: connect over the WebSocket transport, call `terminate()`, assert `is_connected()` is `false` and that the next operation fails. `Cargo.toml` sets `default = ["native"]`, so tasks 4.1 to 4.4 exercise the native `terminate()` only, and task 2.1's WebSocket unit test is compile-checked but never executed in CI (`.github/workflows/ci.yml:255`). CI does run this suite (`:252`), so this is the only executed coverage of the WebSocket half of a cross-transport requirement.
   - [ ] 2.3 Change `Connection::is_closed()` (`src/adbc/connection.rs:1053-1055`) to `self.session.is_closed().await || !self.transport.lock().await.is_connected()`, so one fact about liveness has one answer. Update `is_closed_reflects_the_session_state` (`:3956`) and confirm the two existing assertions at `:3020` and `:3968` still hold, since a live transport reports `is_connected() == true`.
3. Optional export timeout
   - [ ] 3.1 Change `CsvExportOptions::timeout_ms` to `Option<u64>` with default `None` in `src/export/csv.rs`, and rework the timeout handling in `export_to_callback` (`src/export/csv.rs:486-506`) in the same task, because `:486` and `:505` are the field's only consumers and the crate does not compile between the two changes. Keep the builder signature `timeout_ms(u64)` and wrap the argument in `Some`. Update `test_csv_export_options_default` and `test_csv_export_options_builder`. The rework must satisfy five requirements: [expert]
     - Run the SQL, HTTP, and callback composition with no timer when `timeout_ms` is `None`, and inside `tokio::time::timeout` when it is `Some(ms)`.
     - Short-circuit a failed `sql_task`: abort `http_task` and drop the callback future, then return the SQL error, instead of awaiting `tokio::join!` first. Without this, the default path can hang forever, because `src/transport/http_transport.rs` has no read timeout.
     - Hold the `http_task` `JoinHandle` outside the timed block and borrow it in. Every exit path out of `export_to_callback` MUST either await or abort the handle, because dropping it detaches the task. The four paths are: the SQL error short-circuit, the HTTP join error (`src/export/csv.rs:497-499`), the callback error (`:501`), and the timer elapse.
     - Track whether `sql_task.await` completed in a flag owned outside the timed block, and on elapse call `terminate()` only when it did not. An elapse after the SQL response was consumed leaves the transport in sync, so terminating would break a healthy connection. Bind the timeout result to its own `let` statement, so every borrow the timed future took is released before the elapse handling runs, then read the flag and call `terminate()` after that statement. A `match tokio::time::timeout(dur, work).await { ... }` keeps the temporary alive across the arms, which fails borrow-check on both the flag read and the `&mut ws_transport` reborrow that `terminate()` needs, because `sql_task` holds `&mut *ws_transport` for the whole timed future (`src/export/csv.rs:472-479`). A `std::cell::Cell<bool>` borrowed immutably removes that conflict outright and is the preferred shape.
     - Add a `transport_terminated: bool` field to `ExportError::Timeout` (`src/export/csv.rs:67-68`), set from the same flag that gates the `terminate()` call, so a caller decides whether to reconnect without probing. Name the field for the transport, not the connection, matching the spec's own wording. `thiserror`'s `#[error("...")]` takes one format string, so branch on the bool with an inline conditional and emit exactly these two texts: `Export timed out after {timeout_ms}ms; the transport was terminated, reconnect before the next operation` when true, and `Export timed out after {timeout_ms}ms; the transport is still usable` when false. Update the display unit test at `src/export/csv.rs:849` to assert both.
   - [ ] 3.2 Remove the duplicated CSV-option construction in the Arrow and Parquet export paths. `src/export/arrow.rs:739-746` and `src/export/parquet.rs:758-765` hold a character-for-character identical builder chain, so add one `pub(crate)` function in `src/export/csv.rs` that both call. Give it a single `pub(crate)` struct parameter with named fields (`column_separator`, `column_delimiter`, `host`, `port`, `use_tls`), not five positional parameters: two of the five are adjacent `char`s, and a transposed pair would compile and silently produce wrong CSV. Add one unit test asserting the returned options carry no export timeout.
4. Scenario tests

   Every test in tasks 4.1 to 4.3 and 4.5 calls `disable_query_cache(&mut conn)` immediately after connecting, which the Query Timeout Tests Background makes mandatory (`tests/integration_tests.rs:2949-2954`): a cached repeat of the same cartesian-product query returns in about 0.1 seconds instead of about 4 seconds. Each test also uses a `side_rows` value used nowhere else in the file, so no test can consume another's cached result. The file already uses 60,000 (`:2985`) and 100,000 (`:3016`, `:3067`).

   - [ ] 4.1 Add `test_csv_export_default_arms_no_client_side_timer` to the Query Timeout Tests section of `tests/integration_tests.rs`: export `long_running_count_query(110_000)` with `CsvExportOptions::default().use_tls(false)` on a connection with no `query_timeout`, and assert the export completes.
   - [ ] 4.2 Add `test_csv_export_server_timeout_aborts_and_keeps_connection_usable` to the same section: connect with `&query_timeout=2`, export `long_running_count_query(120_000)` with default options, assert `ExportError::SqlExecutionError` rather than `ExportError::Timeout`, then assert a follow-up `conn.query("SELECT 1")` succeeds. This test is also the guard on the short-circuit requirement in task 3.1: without it the call can hang, and the CI job's `timeout-minutes: 30` turns that into a job failure rather than a test failure.
   - [ ] 4.3 Add `test_csv_export_explicit_timeout_terminates_connection` to the same section: export `long_running_count_query(130_000)` with `.timeout_ms(1_000)`, assert `ExportError::Timeout { timeout_ms: 1000, transport_terminated: true }`, then assert a follow-up `conn.query("SELECT 1")` fails. One second elapses long before the EXPORT response arrives, so this exercises the terminating branch of task 3.1. Two further requirements:
     - Do not call `conn.close()` at the end of this test. Every other test in the section does, but the transport is already terminated, so `close()` returns an error and the conventional `.expect` would panic and turn the test that proves the fix into a false red. Drop the connection instead and say so in a comment.
     - Open a second connection afterwards and assert the abandoned session is absent from `SYS.EXA_ALL_SESSIONS`. This is the only check on the assumption in decision [3] that Exasol releases a session when the client socket closes. If the session is still listed, record the observed reap behavior and escalate: `terminate()` would then satisfy the letter of ADR `client-give-up-terminates-connection` and not its stated purpose.
   - [ ] 4.4 Add `test_csv_export_timeout_during_callback_keeps_connection_usable` to the same section, covering the non-terminating branch of task 3.1. Export a small, fast query through `export_csv_to_stream` with `.timeout_ms(10_000)` and a test-local `AsyncWrite` whose `poll_write` holds a `tokio::time::Sleep` of 30 seconds, so the SQL response is consumed long before the timer elapses in the callback. Assert `ExportError::Timeout`, then assert a follow-up `conn.query("SELECT 1")` succeeds. The timer starts before `sql_task.await`, so the bound must exceed a cold-container EXPORT plus tunnel handshake or the elapse lands in the SQL leg and the test asserts the terminating branch by accident; the writer stall must then exceed the bound by a wide margin. Perform one small throwaway export before the measured one so the tunnel path is warm. This test needs no cartesian-product query, and needs no cache reset, because it is the writer that is slow.
   - [ ] 4.5 Add `test_csv_export_runs_past_the_former_five_minute_limit` to `tests/import_export_tests.rs`, marked `#[ignore]` and gated on the `EXARROW_LONG_EXPORT_CHECK` environment variable. It exports `long_running_count_query(side_rows)` with default options, where `side_rows` defaults to 650,000 and is overridable through `EXARROW_LONG_EXPORT_SIDE_ROWS`. The default comes from the documented 60,000-per-side run at about 4 seconds: work scales with the square of `side_rows`, so 650,000 is roughly 120 times the work, or about 8 minutes. Assert that the export completes without `ExportError::Timeout`, and print the elapsed time rather than asserting on it, because machine speed moves the wall clock in both directions. On pre-fix code the same test fails at about 300 seconds with `Export timed out after 300000ms`. Without the gate variable the test prints that it is an opt-in long check and returns.
5. Documentation
   - [ ] 5.1 Document the export timeout in `docs/import-export.md`: CSV export arms no client-side timer by default, `CsvExportOptions::timeout_ms` opts one in, an elapsed timeout terminates the transport only when it fires before the EXPORT response arrives, output written before a timeout may be a partial CSV document, and `query_timeout=` is the server-enforced alternative that leaves the connection usable in every case.
6. Conditional fallback, executed only if task 0.1 failed and the human approved it
   - [ ] 6.1 Add a CI step to `.github/workflows/ci.yml` running exactly the five new test names from `tests/import_export_tests.rs`, and move tasks 4.1 to 4.4 into that file. Do not run the whole suite: its runtime against the CI container has never been measured, and several of its tests insert 20,000 rows. Skip this task entirely when task 0.1 succeeded.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Prerequisite | 0.1 (alone, against an unmodified tree) |
| Group A | 1.1, 2.1 |
| Group B | 2.2, 2.3, 3.1 |
| Group C | 3.2, 5.1 |
| Group D | 4.1, 4.2, 4.3, 4.4 (sequential, in that order) |
| Group E | 4.5 |
| Conditional | 6.1 (only if 0.1 failed and the human approved it) |

Sequential dependencies:
- Prerequisite -> Group A (0.1 needs a tree nobody is editing)
- Group A -> Group B (3.1 calls `terminate()`, and 2.2 and 2.3 need it declared and implemented)
- Group B -> Group C (3.1 owns `src/export/csv.rs` alone, and 3.2 asserts the new default)
- Group C -> Group D (the tests assert the reworked behavior)
- Group D -> Group E (both compete for the single Exasol container, and tasks 4.3 and 4.4 assert on wall-clock margins)

Group D is a sequential list, not a parallel group. All four tasks append a test to the same Query Timeout Tests section of `tests/integration_tests.rs`. Concurrent agents would lose each other's edits silently, because whichever write survives still compiles.

Every parallel group is file-disjoint. Group B splits `tests/websocket_integration_tests.rs`, `src/adbc/connection.rs`, and `src/export/csv.rs`; Group C splits `src/export/csv.rs` from `docs/import-export.md`.

Groups A and B each hold some work together on purpose. Task 2.1 adds a required trait method together with both its implementations. Task 3.1 changes the `timeout_ms` type together with its only consumer and the error field that consumer sets. The crate therefore compiles at every group boundary, and every unit test can be verified green where it lands.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None | - | The `tokio::time::timeout` wrap is reworked, not deleted. `ExportError::Timeout` stays constructed on the opt-in path, so it does not become a never-constructed variant of the kind 0.13.0 removed |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| csv-export: No client-side export timeout by default | Integration | `tests/integration_tests.rs` | `test_csv_export_default_arms_no_client_side_timer` |
| csv-export: No client-side export timeout by default | Integration | `tests/import_export_tests.rs` | `test_csv_export_runs_past_the_former_five_minute_limit` |
| csv-export: No client-side export timeout by default | Unit | `src/export/csv.rs` | `test_csv_export_options_default` |
| csv-export: Arrow and Parquet exports inherit the CSV export defaults | Unit | `src/export/csv.rs` | `test_shared_csv_options_configure_no_export_timeout` |
| csv-export: Server-enforced timeout governs an export | Integration | `tests/integration_tests.rs` | `test_csv_export_server_timeout_aborts_and_keeps_connection_usable` |
| csv-export: Explicit export timeout stops the export (terminating branch) | Integration | `tests/integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection` |
| csv-export: Explicit export timeout stops the export (non-terminating branch) | Integration | `tests/integration_tests.rs` | `test_csv_export_timeout_during_callback_keeps_connection_usable` |
| csv-export: Explicit export timeout stops the export (error reports the branch) | Integration | `tests/integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection` asserts `transport_terminated: true`; `test_csv_export_timeout_during_callback_keeps_connection_usable` asserts `false` |
| csv-export: Explicit export timeout stops the export (`Display` on both branches) | Unit | `src/export/csv.rs` | `test_export_error_display` (extended at `:849`) |
| csv-export: Explicit export timeout stops the export | Unit | `src/export/csv.rs` | `test_csv_export_options_builder` |
| session-and-lifecycle: Terminate a connection whose in-flight response is no longer trusted | Integration | `tests/integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection` |
| session-and-lifecycle: Terminate a connection whose in-flight response is no longer trusted (native) | Unit | `src/transport/native/mod.rs` | `test_terminate_marks_transport_disconnected` |
| session-and-lifecycle: Terminate a connection whose in-flight response is no longer trusted (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_terminate_marks_websocket_transport_disconnected` |
| session-and-lifecycle: Terminate a connection whose in-flight response is no longer trusted (`is_closed()`) | Unit | `src/adbc/connection.rs` | `is_closed_reflects_the_session_state` (extended at `:3956`) |

Coverage limits:

- No test that runs automatically crosses the former 300-second bound. Task 4.1 exports about 13 seconds of work by the scaling model in task 4.5. Only `EXARROW_LONG_EXPORT_CHECK=1 cargo test --test import_export_tests test_csv_export_runs_past_the_former_five_minute_limit -- --ignored` proves the bound is gone, and the Checklist row below runs exactly that. Without it, `options.timeout_ms.unwrap_or(300_000)` would survive every other check, because the default-value unit test still passes on `None`.
- Task 4.5 stays out of the default suites on purpose. An eight-minute test in the CI integration job would consume a quarter of its 30-minute budget for one assertion. Tasks 4.1 to 4.4 pin the mechanism cheaply instead: which side enforces the bound, and what happens on each elapse branch.
- CI runs `integration_tests`, `websocket_integration_tests`, and `driver_manager_tests`, and never runs `import_export_tests` (`.github/workflows/ci.yml:246-263`). Tasks 4.1 to 4.4 therefore go into `integration_tests.rs`, next to the existing Query Timeout Tests section, so they actually run. That section already owns `long_running_count_query` and `disable_query_cache`, which task 1.1 promotes to `tests/common`.
- HTTP-tunnel export has never run in the CI integration job. Task 0.1 confirms it works against the pinned CI image first. A failing 0.1 stops the run and escalates to the human. Task 6.1 is the approved fallback: it adds a CI step scoped to the five new test names. Without such a step the change would ship with no CI-visible proof of either new behavior.
- Task 2.1's WebSocket unit test does not run in CI, which only compiles the `websocket` feature (`cargo test --no-default-features --features websocket --tests --no-run`, `.github/workflows/ci.yml:255`). AGENTS.md documents this. Task 2.2 therefore adds an executed WebSocket test to `tests/websocket_integration_tests.rs`, which CI does run (`:252`). `Cargo.toml` sets `default = ["native"]`, so tasks 4.1 to 4.4 reach the native `terminate()` only.
- The `src/export/csv.rs` per-file coverage-floor exemption (48.4%) stays. The new unit tests touch option construction, not the uncovered write paths that the exemption names.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| import-export/csv-export | `EXARROW_LONG_EXPORT_CHECK=1 cargo test --test import_export_tests test_csv_export_runs_past_the_former_five_minute_limit -- --ignored --nocapture` | Test passes: the export completes and returns no `ExportError::Timeout`. The log reports the elapsed time, expected in the range of several minutes. The same command on the pre-fix code fails at about 300 seconds with `Export timed out after 300000ms`. Raise or lower `EXARROW_LONG_EXPORT_SIDE_ROWS` if the run is far shorter or longer than 8 minutes on this machine |
| import-export/csv-export | `cargo run --example import_export` | Every import and export section completes. No `Export timed out` line appears |
| connection-management/session-and-lifecycle | `REQUIRE_EXASOL=1 cargo test --test integration_tests test_csv_export_explicit_timeout_terminates_connection -- --nocapture` | Test passes. The log shows `ExportError::Timeout` at about 1 second, then a failure on the follow-up `SELECT 1` rather than a stale row |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --test integration_tests -- --test-threads=1` | 0 failures |
| Export tests | `EXARROW_LONG_EXPORT_CHECK=1 cargo test --test import_export_tests -- --ignored` | 0 failures. The variable is required: without it task 4.5 skips, and no run crosses the former 300-second bound |
| WebSocket build check | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage floors | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Exit 0 |
| Spec validation | `speq plan validate fix-csv-export-timeout` | pass |
