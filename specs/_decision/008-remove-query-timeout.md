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

The connection survives a query timeout. `ConnectionParams::query_timeout` is `Option<Duration>` and `Statement::timeout_ms` is `Option<u64>`. `Some` sets the attribute and `None` sets none. The type change from `Duration` is breaking and is noted in the CHANGELOG.

## ADR: Client-side give-up on a running query MUST terminate the connection

**ID:** client-give-up-terminates-connection
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

Abandoning an in-flight request leaves the server session open until the idle reaper acts. The "Cancel with timeout" scenario in `query-execution/execution` allowed a soft local abort that left the session open.

### Decision

Any client-side decision to give up on a running query MUST terminate the connection and MUST NOT abandon the in-flight request. In "Cancel with timeout", the acknowledgment-timeout fallback MUST close the connection. This changes the spec only. Cancellation is unimplemented, so the rule constrains a future implementation.

### Options Considered

| Option | Verdict |
|--------|---------|
| Close the connection on any client-side give-up | ✓ Chosen. The server session is released immediately |
| Soft local abort in "Cancel with timeout" | ✗ Rejected. It leaves the session open server-side |

### Consequences

A future cancel implementation cannot reintroduce the session leak.

## ADR: Reconcile the per-statement query timeout against the session in both directions, with write-back

**ID:** query-timeout-bidirectional-reconcile
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

`queryTimeout` is a session attribute, but `Statement::set_timeout()` is per statement. Statements run sequentially on one connection through `execute_statement(&mut self)`. Forwarding only `Some` values leaves a stale server timeout. A statement with `Some(5s)` followed by a `None` statement would run the second under the 5s limit, which violates the "No query timeout by default" scenario.

### Decision

On every execute, `execute_statement` compares the statement's target seconds with the applied seconds stored in `SessionConfig::query_timeout`. The target for `Some(ms)` is `ms` rounded up with `div_ceil`, at least 1s. The target for `None` is 0, because Exasol treats `queryTimeout=0` as unlimited. When they differ, it calls `set_query_timeout(target_secs)`, including `set_query_timeout(0)` for `None`. It then writes the applied value back to `SessionConfig::query_timeout`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Bidirectional reconcile with write-back | ✓ Chosen. Closes the stale-timeout hole and sends no round-trip when values match |
| One-directional reconcile without write-back | ✗ Rejected. A prior statement's timeout leaks onto a later untimed statement |
| Connection-level timeout only | ✗ Rejected. It breaks the public `Statement::set_timeout` API |
| Send `setAttributes` before every statement | ✗ Rejected. It adds a round-trip when nothing changed |

### Consequences

`SessionConfig::query_timeout` stores the applied value. Sub-second timeouts round up to 1s, because 0 is reserved for the unlimited reset.
