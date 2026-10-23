# Decisions: fix-csv-export-timeout

## ADR: Keep an opt-in client-side export timer instead of deleting the knob

**ID:** export-timer-stays-opt-in
**Plan:** fix-csv-export-timeout
**Status:** Accepted

### Context

This decision scopes `server-enforced-query-timeout` and does not supersede it. That ADR bans client-side timers for query and statement timeouts. CSV export has no server setting that bounds the caller's callback or a stalled tunnel.

### Decision

CSV export keeps an optional client-side timeout, off by default. When set, it bounds SQL execution, tunnel transfer, and the callback together.

### Options Considered

| Option | Verdict |
|--------|---------|
| Opt-in client-side timer | ✓ Chosen. It bounds the callback and a stalled tunnel, which no server attribute reaches |
| Delete `timeout_ms` and `ExportError::Timeout` | ✗ Rejected. The callback and a stalled tunnel get no bound |
| Set a server attribute before the EXPORT statement | ✗ Rejected. `export_to_callback` holds only a `&mut T: TransportProtocol` and cannot reconcile the session's applied `queryTimeout`, so a stale timeout would leak onto the next statement |

### Consequences

The export timer and the server-enforced `query_timeout` coexist as separate, scoped settings.
