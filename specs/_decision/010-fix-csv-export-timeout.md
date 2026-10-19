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
