# Decisions: remove-query-timeout

## ADR: Forward the configured query timeout to Exasol; remove client-side enforcement

**ID:** server-enforced-query-timeout
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

A `Connection` owns one WebSocket transport with no request pooling. Wrapping `execute_statement` in `tokio::time::timeout` dropped the future mid-request on elapse. That desynced the transport, left the `Connection` unusable, and kept the server session open until Exasol's idle reaper removed it.

### Decision

Exasol enforces the query timeout. The driver forwards a configured timeout as the `queryTimeout` session attribute, and the server reports an abort through the normal response. No client-side timer exists in `execute_statement`. `TransportProtocol::set_query_timeout` sets the attribute through `setAttributes`, as `set_autocommit` does. One mechanism serves the connection-level set and the per-statement reconcile.

### Options Considered

| Option | Verdict |
|--------|---------|
| Server-enforced `queryTimeout` attribute | ✓ Chosen. The abort is a normal error and the transport stays in sync |
| Client-side `tokio::time::timeout` wrap | ✗ Rejected. Dropping the future desyncs the transport and leaks the server session |

### Consequences

The connection survives a query timeout. `ConnectionParams::query_timeout` is `Option<Duration>` and `Statement::timeout_ms` is `Option<u64>`. `Some` sets the attribute and `None` sets none. The type change from `Duration` is breaking and is noted in the CHANGELOG. `execute_statement` reconciles the statement timeout against the applied value in `SessionConfig::query_timeout` in both directions, rounds sub-second values up to 1s, and resets with `set_query_timeout(0)` for `None`, so a prior timeout never leaks onto a later untimed statement.

## ADR: Client-side give-up on a running query MUST terminate the connection

**ID:** client-give-up-terminates-connection
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

Abandoning an in-flight request leaves the server session open until the idle reaper acts. A soft local abort on give-up would leave the session open.

### Decision

Any client-side decision to give up on a running query MUST terminate the connection and MUST NOT abandon the in-flight request. A cancellation-acknowledgment timeout fallback MUST close the connection. Cancellation is unimplemented and unspecified, so the rule constrains a future implementation.

### Options Considered

| Option | Verdict |
|--------|---------|
| Close the connection on any client-side give-up | ✓ Chosen. The server session is released immediately |
| Soft local abort on give-up | ✗ Rejected. It leaves the session open server-side |

### Consequences

A future cancel implementation cannot reintroduce the session leak.
