# Decisions: fix-csv-export-timeout

## ADR: Keep an opt-in client-side export timer instead of deleting the knob

**ID:** export-timer-stays-opt-in
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

This decision scopes `specs/_decision/008-remove-query-timeout.md` and does not supersede it. That ADR bans client-side timers for `ConnectionParams::query_timeout` and `Statement::timeout_ms`, which map to the server `queryTimeout` attribute. CSV export has no server attribute that bounds the caller's callback or a stalled tunnel. `src/transport/http_transport.rs` has no read timeout.

### Decision

`CsvExportOptions::timeout_ms` is `Option<u64>` milliseconds, default `None`, and the builder keeps its `u64` argument. `Some(ms)` arms a client-side `tokio::time::timeout` around SQL execution, tunnel transfer, and the callback. The `session-and-lifecycle` Background limits its timer prohibition to `Connection::execute_statement()` and points to the export setting in `import-export/csv-export`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Opt-in client-side timer | ✓ Chosen. It bounds the callback and a stalled tunnel, which no server attribute reaches |
| Delete `timeout_ms` and `ExportError::Timeout` | ✗ Rejected. The callback and a stalled tunnel get no bound |
| Set a server attribute before the EXPORT statement | ✗ Rejected. `export_to_callback` holds only a `&mut T: TransportProtocol` and cannot reconcile the session's applied `queryTimeout`, so a stale timeout would leak onto the next statement |

### Consequences

The export timer and the server-enforced `query_timeout` coexist as separate, scoped settings.

## ADR: Add `TransportProtocol::terminate()` for a give-up that cannot trust the response stream

**ID:** transport-terminate-no-round-trip
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

`client-give-up-terminates-connection` in `specs/_decision/008-remove-query-timeout.md` requires a give-up to terminate the connection. `TransportProtocol::close()` sends a disconnect and awaits the response. After an abandoned EXPORT, `close()` blocks until the EXPORT finishes server-side.

### Decision

`TransportProtocol` has a required method `fn terminate(&mut self)`. It drops the socket and marks the transport disconnected with no protocol round-trip. `export_to_callback` calls it before returning `ExportError::Timeout` when the timer elapses before the EXPORT response was read.

### Options Considered

| Option | Verdict |
|--------|---------|
| New additive `terminate()` with no I/O | ✓ Chosen. It implements the give-up rule and a future `cancel` can reuse it |
| Call `close()` | ✗ Rejected. It blocks while the abandoned EXPORT runs server-side |
| Bound the disconnect round-trip inside both `close()` implementations | ✗ Rejected. It changes every close path, adds a magic wait value, and relies on undocumented `close()` behavior |

### Consequences

Both transports implement `terminate()` with the same outcome, and `close()` delegates its teardown to it. `Connection::is_closed()` reads the transport as well as the session. `export_to_callback` terminates only when the EXPORT response is still unread at elapse, tracked by a progress flag, and `ExportError::Timeout` carries `transport_terminated: bool`. A timeout during a slow callback leaves the healthy transport open.
