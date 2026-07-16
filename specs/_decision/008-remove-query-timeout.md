# Decisions: remove-query-timeout

## ADR: Forward the configured query timeout to Exasol; remove client-side enforcement

**ID:** server-enforced-query-timeout
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

`Connection::execute_statement` wrapped query execution in `tokio::time::timeout`. This driver's `Connection` owns its transport exclusively — a single WebSocket, no request pooling. Dropping the wrapped future on timer elapse abandoned an outstanding request/response cycle mid-flight, desyncing the transport and leaving the `Connection` unusable, while the server session stayed open until Exasol's own idle reaper noticed the client was gone.

### Decision

Enforce a configured query timeout server-side. When a caller configures a timeout, the driver forwards it to Exasol as the `queryTimeout` session attribute; the server aborts an over-running query and reports the abort through the normal response cycle. The `tokio::time::timeout` wrap in `execute_statement` is removed entirely — no client-side timer is constructed in any path.

### Options Considered

| Option | Verdict |
|--------|---------|
| Server-enforced `queryTimeout` session attribute | ✓ Chosen — the server reports the abort as a normal error; the transport never desyncs |
| Client-side `tokio::time::timeout` wrap (prior behavior) | ✗ Rejected — dropping the future mid-request desyncs the single owned transport and leaks the server session |

### Consequences

The connection survives a query timeout and remains usable for subsequent statements. The public `ConnectionParams::query_timeout` field type changes from `Duration` to `Option<Duration>`, a breaking change marked in the CHANGELOG.

## ADR: The `Option` timeout value selects whether the `queryTimeout` attribute is set, not a client timer

**ID:** query-timeout-option-semantics
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

`ConnectionParams::query_timeout` and `Statement::timeout_ms` needed a representation for "a timeout may or may not be configured" once enforcement moved server-side.

### Decision

Keep `ConnectionParams::query_timeout: Option<Duration>` and `Statement::timeout_ms: Option<u64>`. `Some(d)` means "forward `d.as_secs()` to Exasol as the `queryTimeout` session attribute"; `None` means "set no attribute; the server's own `QUERY_TIMEOUT` governs". No arm constructs a client-side timer.

### Options Considered

| Option | Verdict |
|--------|---------|
| `Option<Duration>` / `Option<u64>`, `None` = no attribute | ✓ Chosen — idiomatic, compiler-checked absence, public type stays stable across the mechanism change |
| `Duration` sentinel (`MAX` or `ZERO`) for "no attribute" | ✗ Rejected — a magic value is easy to misread as a bug and forces every reader to know it |

### Consequences

The public field type is unchanged in shape from the prior `Option`-based design; only its meaning shifts from "arm/skip a client timer" to "set/skip a server attribute".

## ADR: Set the `queryTimeout` attribute via `setAttributes`, mirroring `set_autocommit`

**ID:** query-timeout-via-set-attributes
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

Exasol exposes `queryTimeout` as a session attribute, settable over the WebSocket `setAttributes` command and equivalent to `ALTER SESSION SET QUERY_TIMEOUT = n`. The driver already sets the `autocommit` attribute this way post-authenticate.

### Decision

Add `TransportProtocol::set_query_timeout(timeout_secs: u64)`, implemented on both transports by sending the `queryTimeout` attribute — the WebSocket transport via a `setAttributes` command, the native transport via `CMD_SET_ATTRIBUTES` — exactly as `set_autocommit` already sets `autocommit`. `connect()` calls it once after authentication when the connection configures a timeout; `execute_statement()` reconciles a per-statement override against the session's applied value before executing.

### Options Considered

| Option | Verdict |
|--------|---------|
| `setAttributes` command, mirroring `set_autocommit` | ✓ Chosen — a proven precedent for a post-authenticate session attribute over the same request/response path; the wire format is confirmed against the Exasol WebSocket API, not guessed |
| `ALTER SESSION SET QUERY_TIMEOUT = n` SQL | ✗ Rejected — composes session-management SQL instead of using a typed attribute |
| Set the attribute in the login command's `attributes` object | ✗ Rejected — does not serve the per-statement reconcile path, which needs a post-authenticate call |

### Consequences

One mechanism (`set_query_timeout`) serves both the connection-level initial set and the per-statement reconcile.

## ADR: Client-side give-up on a running query MUST terminate the connection

**ID:** client-give-up-terminates-connection
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

An abandoned in-flight request leaves the server session dangling until Exasol's idle reaper eventually notices the client is gone. The `query-execution/execution` "Cancel with timeout" scenario's prior text ("forcefully abort the local query execution ... close the connection if necessary") permitted a soft local abort that left the session open.

### Decision

Establish the principle that any client-side decision to give up on a running query MUST terminate the connection rather than abandon the in-flight request. Apply it to the "Cancel with timeout" scenario: the acknowledgment-timeout fallback MUST close the connection and MUST NOT merely abort local execution. This is a spec-only change — query cancellation is unimplemented (`cancel` returns "not implemented") — so it constrains a future implementation rather than changing current behavior.

### Options Considered

| Option | Verdict |
|--------|---------|
| Require the connection to close on any client-side give-up | ✓ Chosen — releases the server session immediately instead of leaking it until idle-reap |
| Leave "Cancel with timeout" as a soft local abort | ✗ Rejected — the flagged anti-pattern; leaves the session open server-side |

### Consequences

The redesign that removes the client-side query timer already eliminates the only implemented give-up path; this decision codifies the principle for the remaining unimplemented cancel path so a future cancel implementation cannot reintroduce the same class of session leak.

## ADR: Reconcile the per-statement query timeout against the session in both directions, with write-back

**ID:** query-timeout-bidirectional-reconcile
**Plan:** remove-query-timeout
**Status:** Accepted

### Context

`queryTimeout` is session-level in the Exasol protocol, but the driver's public `Statement::set_timeout()` / `Statement::timeout_ms` surface is per-statement, and `execute_statement` takes `&mut self` so statements execute sequentially on one connection. An initial one-directional reconcile — only forwarding a `Some(ms)` statement target and never writing the applied value back — leaves a stale server `queryTimeout` in place: a statement that sets `Some(5s)` followed by an inherited-`None` statement on the same connection would run the second, deliberately unbounded, query under the previous 5s limit. That reproduces the original bug's defect class through the preserved `set_timeout` surface, and was flagged as a `[REQUIREMENT_CONFLICT]` blocker in plan review because it violates the "No query timeout by default" scenario for any statement following a timed one.

### Decision

`execute_statement` reconciles the session `queryTimeout` in both directions on every execute. It computes the statement's target seconds (`Some(ms)` rounds up to at least 1s via `div_ceil` over milliseconds; `None` targets `0`, since Exasol treats `queryTimeout=0` as unlimited), compares them to the session's applied seconds stored in the repurposed `SessionConfig::query_timeout`, and when they differ calls `set_query_timeout(target_secs)` — including a reset call `set_query_timeout(0)` for the `None` target — then writes the applied value back into `SessionConfig::query_timeout` so the next statement reconciles against the correct baseline.

### Options Considered

| Option | Verdict |
|--------|---------|
| Bidirectional reconcile with write-back, including the `None`→`0` reset | ✓ Chosen — closes the stale-timeout hole with the fewest extra round-trips (none when the statement matches the applied session value) |
| One-directional reconcile (`Some`-only), no write-back | ✗ Rejected — leaks a prior statement's timeout onto a later no-timeout statement on the same connection |
| Drop the per-statement surface; support only connection-level timeouts | ✗ Rejected — breaks the existing public `Statement::set_timeout` API |
| Send `setAttributes` unconditionally before every statement | ✗ Rejected — an unconditional round-trip on every execute when the value has not changed |

### Consequences

The formerly dead `SessionConfig::query_timeout` field becomes live as the applied-value store, turning dead code live rather than deleting it. Sub-second timeouts round up to 1s instead of truncating to 0, since 0 is now reserved exclusively for the unlimited reset path.
