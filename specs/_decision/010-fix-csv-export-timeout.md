# Decisions: fix-csv-export-timeout

## ADR: Keep an opt-in client-side export timer instead of deleting the knob

**ID:** export-timer-stays-opt-in
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

This decision scopes, and does not reverse or supersede, the query-timeout decision in `specs/_decision/008-remove-query-timeout.md`, which holds that no arm constructs a client-side timer, scoped to `ConnectionParams::query_timeout` and `Statement::timeout_ms`, both of which map onto the server-enforced `queryTimeout` session attribute. CSV export has no equivalent server attribute reaching the caller's callback or a stalled tunnel: `src/transport/http_transport.rs` has no read timeout of its own, confirmed by `grep -c timeout` returning 0.

### Decision

`Some(ms)` still arms a client-side `tokio::time::timeout` around SQL execution, tunnel transfer, and the callback. Only the default changes to `None`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep an opt-in client-side timer | ✓ Chosen — bounds work a server attribute cannot reach: the caller's callback and a tunnel stalled while the statement still runs server-side |
| Delete `timeout_ms` and `ExportError::Timeout`, rely only on server-enforced `queryTimeout` | ✗ Rejected — leaves the callback and a stalled tunnel with no bound at all |
| Reinterpret `Some(ms)` as a server attribute set before the EXPORT statement, mirroring 0.14.0 | ✗ Rejected — `export_to_callback` receives only a `&mut T` bounded by `TransportProtocol` and cannot reconcile the session's already-applied `queryTimeout`, so setting the attribute behind `execute_statement`'s back would leak a stale timeout onto the next statement |

### Consequences

CSV export keeps a client-side timer as a distinct, export-scoped setting alongside the server-enforced `query_timeout` connection parameter. The `session-and-lifecycle` Background states that scope explicitly, so the two mechanisms coexist without contradiction in the permanent library. `CsvExportOptions::timeout_ms` is `Option<u64>` milliseconds with default `None`, and the builder method keeps its `u64` argument. The Background's prohibition is scoped to query execution through `Connection::execute_statement()` and points to the export-scoped setting in `import-export/csv-export`.

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
