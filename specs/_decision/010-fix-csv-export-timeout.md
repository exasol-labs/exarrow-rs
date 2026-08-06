# Decisions: fix-csv-export-timeout

## ADR: Represent the export timeout as `Option<u64>` milliseconds, not `Option<Duration>`

**ID:** export-timeout-option-u64-milliseconds
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

Issue #52 reports that `CsvExportOptions::timeout_ms` is a fixed `u64` defaulting to 300,000, so no caller can remove the client-side bound. Version 0.14.0 fixed the same defect class for query execution: `Statement::timeout_ms` became `Option<u64>` and `ConnectionParams::query_timeout` became `Option<Duration>`, recorded in `specs/_decision/008-remove-query-timeout.md`. The export field needed the same "not configured" representation, and the existing repo carried two competing precedents for its shape.

### Decision

`CsvExportOptions::timeout_ms` becomes `Option<u64>` with default `None`, keeping the millisecond unit and the field name.

### Options Considered

| Option | Verdict |
|--------|---------|
| `Option<u64>`, keeping the field name and unit | ✓ Chosen — matches the closer in-repo precedent, `Statement::timeout_ms: Option<u64>`; issue #52 names the millisecond field directly; `ExportError::Timeout { timeout_ms: u64 }` already reports milliseconds |
| `Option<Duration>`, matching `ConnectionParams::query_timeout` | ✗ Rejected — would make the field name `timeout_ms` inaccurate, forcing a rename as a second breaking change beyond what issue #52 asks for |

### Consequences

The error type needs no unit change. The builder method keeps its `u64` argument and wraps it in `Some`, so existing `.timeout_ms(60_000)` call sites keep compiling.

## ADR: Keep an opt-in client-side export timer instead of deleting the knob

**ID:** export-timer-stays-opt-in
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

This decision scopes, and does not reverse or supersede, `query-timeout-option-semantics` (`specs/_decision/008-remove-query-timeout.md`), which closes with "No arm constructs a client-side timer," scoped to `ConnectionParams::query_timeout` and `Statement::timeout_ms`, both of which map onto the server-enforced `queryTimeout` session attribute. CSV export has no equivalent server attribute reaching the caller's callback or a stalled tunnel: `src/transport/http_transport.rs` has no read timeout of its own, confirmed by `grep -c timeout` returning 0.

### Decision

`Some(ms)` still arms a client-side `tokio::time::timeout` around SQL execution, tunnel transfer, and the callback. Only the default changes to `None`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep an opt-in client-side timer | ✓ Chosen — bounds work a server attribute cannot reach: the caller's callback and a tunnel stalled while the statement still runs server-side |
| Delete `timeout_ms` and `ExportError::Timeout`, rely only on server-enforced `queryTimeout` | ✗ Rejected — leaves the callback and a stalled tunnel with no bound at all |
| Reinterpret `Some(ms)` as a server attribute set before the EXPORT statement, mirroring 0.14.0 | ✗ Rejected — `export_to_callback` receives only a `&mut T` bounded by `TransportProtocol` and cannot reconcile the session's already-applied `queryTimeout`, so setting the attribute behind `execute_statement`'s back would leak a stale timeout onto the next statement |

### Consequences

CSV export keeps a client-side timer as a distinct, export-scoped setting alongside the server-enforced `query_timeout` connection parameter. The `session-and-lifecycle` Background now states that scope explicitly (see `session-and-lifecycle-background-scopes-client-timer-prohibition` below), so the two mechanisms coexist without contradiction in the permanent library.

## ADR: Add `TransportProtocol::terminate()` for a give-up that cannot trust the response stream

**ID:** transport-terminate-no-round-trip
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

`client-give-up-terminates-connection` (`specs/_decision/008-remove-query-timeout.md`) requires that any client-side give-up terminate the connection rather than abandon the in-flight request, but until this plan no code path implemented that rule — 0.14.0 removed the only give-up path that existed. `TransportProtocol::close()` sends a disconnect command and awaits its response, so calling it after abandoning an in-flight EXPORT would block until that EXPORT finishes server-side.

### Decision

Add a required trait method `fn terminate(&mut self)` that drops the socket and marks the transport disconnected without any protocol round-trip. `export_to_callback` calls it when the export timer elapses before the EXPORT response was read, ahead of returning `ExportError::Timeout`.

### Options Considered

| Option | Verdict |
|--------|---------|
| New `terminate()`, additive, no I/O | ✓ Chosen — gives the give-up rule a real implementation that a future `cancel` path can reuse; changes no existing behavior |
| Call the existing `close()` | ✗ Rejected — awaits the abandoned EXPORT's response and blocks for as long as the statement keeps running server-side |
| Bound the disconnect round-trip inside both `close()` implementations with a short timer | ✗ Rejected — changes every connection-close path, invents a magic wait value, and leaves the export path depending on an undocumented property of `close()` |

### Consequences

Both transports implement `terminate()` with the same observable outcome, and `close()` delegates its own teardown tail to it. `Connection::is_closed()` reads the transport as well as the session, so a terminated transport reports the connection as closed through one code path.

## ADR: Put CSV-export scenario tests in `tests/integration_tests.rs`, not `tests/import_export_tests.rs`

**ID:** csv-export-timeout-tests-in-ci-suite
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

`.github/workflows/ci.yml` runs `integration_tests`, `websocket_integration_tests`, and `driver_manager_tests`, but never runs `import_export_tests`. A scenario test placed in `import_export_tests.rs` proves nothing on a pull request. The Query Timeout Tests section of `tests/integration_tests.rs` already owns `long_running_count_query` and `disable_query_cache`. HTTP-tunnel export had never run in the CI integration job before this plan.

### Decision

The four CI-relevant scenario tests go into the existing Query Timeout Tests section of `tests/integration_tests.rs`. Only the eight-minute regression proof (`test_csv_export_runs_past_the_former_five_minute_limit`) goes into `tests/import_export_tests.rs`, gated on `EXARROW_LONG_EXPORT_CHECK`. A prerequisite task confirms HTTP-tunnel export succeeds against the pinned CI container image before either suite is touched.

### Options Considered

| Option | Verdict |
|--------|---------|
| Four scenario tests in `integration_tests.rs`; the long regression test in `import_export_tests.rs` | ✓ Chosen — the four run on every PR; the long test stays opt-in |
| All five in `tests/import_export_tests.rs`, with a new CI step running the whole suite | ✗ Rejected as scope creep with unknown cost — the suite is 91 KB, several tests insert 20,000 rows, and its CI runtime was unmeasured |

### Consequences

`long_running_count_query` and `disable_query_cache` were promoted from `tests/integration_tests.rs` to `tests/common/mod.rs` so both suites can use them. If the prerequisite check had failed, the approved fallback was a CI step scoped to the five new test names only, never the whole suite.

## ADR: The default export path must not hang when the EXPORT statement fails

**ID:** export-short-circuits-on-sql-error
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

`export_to_callback` awaited `tokio::join!(http_task, callback_task)` before propagating a SQL error. The HTTP task blocks in `handle_export_request()`, and `src/transport/http_transport.rs` has no read timeout at all. With the fixed 300-second wrap removed, awaiting the join before returning a SQL error would leave the call's return conditional on Exasol closing the tunnel socket, an unverified assumption about server behavior.

### Decision

A failed `sql_task` short-circuits: `export_to_callback` aborts `http_task`, drops the callback future, and returns the SQL error immediately, instead of awaiting `tokio::join!` first.

### Options Considered

| Option | Verdict |
|--------|---------|
| Short-circuit on a failed `sql_task` | ✓ Chosen — the return no longer depends on the tunnel socket closing on its own |
| Keep awaiting the join before propagating the SQL error | ✗ Rejected — can hang indefinitely once the default 300-second bound is removed, since the tunnel read has no timeout |

### Consequences

Every exit path out of `export_to_callback` — the SQL error short-circuit, the HTTP join error, the callback error, and the timer elapse — must either await or abort the `http_task` `JoinHandle`, since dropping it detaches the task. The "Server-enforced timeout governs an export" scenario states the short-circuit as a normative requirement.

## ADR: Terminate the transport only when the EXPORT response was still unread at elapse

**ID:** export-timeout-terminates-conditionally
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

The timed region in `export_to_callback` spans SQL execution, tunnel transfer, and the callback. When the timer elapses after `sql_task.await` returned, the EXPORT response is already consumed and the transport is in sync — nothing is unmatchable. Decision `export-timer-stays-opt-in` names a slow callback as the knob's primary use case, so that later-elapse branch is the expected case, not an edge case. Terminating unconditionally would destroy a healthy connection on exactly that primary use case.

### Decision

The driver terminates the transport when the export timeout elapses before the EXPORT response has been consumed. It leaves the transport usable when the elapse happens later, for example during a slow callback. A progress flag, set once `sql_task.await` completes and owned outside the timed block, gates which branch fires; `ExportError::Timeout` carries a `transport_terminated: bool` field reporting which branch fired so a caller decides whether to reconnect without probing the connection.

### Options Considered

| Option | Verdict |
|--------|---------|
| Terminate conditionally, gated on whether the EXPORT response was consumed | ✓ Chosen — matches the two states honestly: an unmatchable in-flight response versus a healthy, in-sync transport |
| Terminate on every elapse | ✗ Rejected — breaks a healthy connection on the knob's primary use case, a slow callback |

### Consequences

Before this change the connection stayed open with an unread EXPORT response in either case, and the next statement silently read that stale response — the defect issue #52 reports for query execution and this plan reports for export. After a terminating elapse the caller gets a failure on the next operation and must reconnect, rather than silent corruption. `Connection::is_closed()` now reports a terminated transport as closed.

## ADR: Scope the "no client-side timer" prohibition to `Connection::execute_statement()` in the session-and-lifecycle Background

**ID:** session-and-lifecycle-background-scopes-client-timer-prohibition
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

The recorded `connection-management/session-and-lifecycle` Background read "the driver SHALL NOT wrap query execution in a client-side timer" with no `WHEN` to scope it, so it governed every scenario in the feature unconditionally — including, on a literal reading, this plan's own opt-in export timer. Background prose carries forward through `/speq:record` regardless of which scenarios a later plan touches; a future planner reading the unscoped Background could read the export timer as a spec violation and delete it, reintroducing the inverse of issue #52.

### Decision

Narrow the Background's prohibition to query execution through `Connection::execute_statement()`, and add one sentence pointing to the export-scoped setting specified in `import-export/csv-export`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Scope the Background clause to `Connection::execute_statement()` and cross-reference `import-export/csv-export` | ✓ Chosen — the carve-out lands in the permanent library, in the one place it is reachable and governs every scenario in the feature |
| Scope only the "Query timeout" scenario, leave Background unscoped | ✗ Rejected — Background prose has no `WHEN` and governs unconditionally, so the conflict is stronger there than in any one scenario |
| Fold the carve-out into the "Query timeout" scenario's prohibition bullet as a second AND-clause | ✗ Rejected — produces a non-singular requirement carrying a prohibition and a carve-out in one bullet, unreachable from that scenario's own `GIVEN`; see `single-requirement-per-bullet-over-warning-avoidance` |

### Consequences

The `session-and-lifecycle` Background and the "Query timeout" scenario both now name `Connection::execute_statement()` specifically, so the CSV export timer this plan keeps does not contradict either.

## ADR: A non-blocking validator WARN is not a reason to write a non-singular requirement

**ID:** single-requirement-per-bullet-over-warning-avoidance
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

An earlier revision folded the export carve-out into the "Query timeout" scenario's prohibition bullet specifically to avoid a four-AND-step validator `WARN` ("recommended: 3 or fewer"). `speq feature validate adbc-driver/transactions` shows the library already carries that same non-blocking warning elsewhere. The fold produced a 38-word bullet stating both a prohibition and a carve-out, with the carve-out reachable from no RFC-2119 keyword and no scenario `GIVEN`.

### Decision

Cut the carve-out from the "Query timeout" scenario's prohibition bullet, leaving one singular requirement per bullet. The carve-out lives only in the Background amendment (`session-and-lifecycle-background-scopes-client-timer-prohibition`).

### Options Considered

| Option | Verdict |
|--------|---------|
| One singular requirement per bullet, accept the non-blocking `WARN` where it already applies | ✓ Chosen — the validator's AND-step threshold is advisory, and the library already tolerates it elsewhere |
| Fold the carve-out into the prohibition bullet to keep the scenario under the AND-step threshold | ✗ Rejected — trades a clean validator run for an unreachable, non-singular requirement |

### Consequences

Future plan and spec revisions should not treat a clean validator run as evidence of requirement quality on its own; a `WARN` is a prompt to consider splitting a scenario, not a mandate that overrides singularity.
