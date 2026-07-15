# Plan: remove-query-timeout

## Summary

Make the query timeout a server-enforced session setting instead of a client-side timer. When a caller configures a timeout, the driver forwards it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle; absent configuration, no attribute is set and the server's own `QUERY_TIMEOUT` governs.

## Design

### Context

`Connection::execute_statement` wraps query execution in `tokio::time::timeout` and returns `QueryError::Timeout` when the timer elapses, dropping the in-flight execution future. This driver's `Connection` owns its transport exclusively — a single WebSocket, no `Arc<Mutex<>>` request pooling. Dropping the future mid-request abandons an outstanding request/response cycle on that socket while the server is still processing it, leaving the transport desynced and the `Connection` unusable for later statements. The client-side timer therefore "kills the client" (the connection), not just the one query. It also leaves the session alive server-side until Exasol's own idle reaper eventually notices the client is gone, which can take a while.

`queryTimeout` is a genuine Exasol session attribute: a number in seconds, settable by the client via the WebSocket `setAttributes` command (confirmed against the [Exasol WebSocket API](https://github.com/exasol/websocket-api); the repository's own `docs/setup-and-connect.md` already lists it) and equivalent to `ALTER SESSION SET QUERY_TIMEOUT = n`. Forwarding the configured timeout to Exasol lets the server abort the query and return the timeout as a normal error response — no future drop, no abandoned request, no corrupted transport.

- **Goals** — Forward a configured query timeout to Exasol as the `queryTimeout` session attribute so the server enforces it. Remove the client-side `tokio::time::timeout` wrap in `execute_statement` entirely. Default to no timeout attribute: absent configuration, the server's own `QUERY_TIMEOUT` governs and the driver imposes no client-side limit. Keep the opt-in surface (`ConnectionParams::query_timeout()`, the `query_timeout=` connection-string parameter, `Statement::set_timeout()`). Establish the principle that any client-side decision to give up on a running query MUST terminate the connection rather than abandon the in-flight request.
- **Non-Goals** — Change the connection-establishment timeout (`connection_timeout`) or idle timeout (`idle_timeout`). Implement query cancellation (it remains unimplemented; this plan only tightens the `query-execution/execution` "Cancel with timeout" spec constraint, it does not build cancel). Add a runtime API to read or mutate `QUERY_TIMEOUT` outside the existing config surface.

### Decision

Forward the configured timeout to the server; do not enforce it client-side. A new transport method `set_query_timeout(seconds)` sends a `setAttributes` request carrying the `queryTimeout` attribute, mirroring the existing `set_autocommit` mechanism. `connect()` calls it once after authentication when the connection configures a timeout. `execute_statement` executes the query directly with no timer future; a server-reported timeout surfaces through the normal response as an error. The configurable value is an `Option`: `None` means "set no attribute", `Some(d)` means "forward `d.as_secs()`".

#### Architecture

```
ConnectionParams.query_timeout: Option<Duration>
        │
   connect(): Some(d) ─▶ transport.set_query_timeout(d.as_secs())  ──setAttributes{queryTimeout}──▶ Exasol
              None     ─▶ (no attribute set; server QUERY_TIMEOUT governs)
        │
   create_statement ──inherits Option──▶ Statement.timeout_ms: Option<u64>
        │
   execute_statement (reconcile in BOTH directions, then write back):
     target = stmt.timeout_ms()  Some(ms) ─▶ round up to ≥1 s | None ─▶ reset (0 = unlimited)
     target seconds ≠ SessionConfig.query_timeout seconds ─▶ set_query_timeout(target_secs)
        Some ─▶ set_query_timeout(secs)   None ─▶ set_query_timeout(0)  (reset to unlimited)
     write applied value back ─▶ SessionConfig.query_timeout = target   (so next stmt sees baseline)
     execute_query(sql) directly ── server aborts on timeout ─▶ error response ─▶ QueryError
     (no tokio::time::timeout, no future drop, transport stays in sync)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Forward-to-server session attribute | `transport.set_query_timeout`, `connect()` | The server aborts and reports the timeout through the normal request/response cycle; the transport never desyncs |
| Mirror existing `set_autocommit` | `TransportProtocol::set_query_timeout` (WebSocket + native) | A proven precedent already sets a session attribute post-authenticate; reuse the same shape |
| `Option` for "value may be absent" | `ConnectionParams::query_timeout`, `Statement::timeout_ms` | `None` = "set no attribute" is compiler-checked; avoids a `Duration::MAX`/`0` sentinel |
| Inherit-from-connection with reconcile | `create_statement`, `execute_statement` | A statement inherits the connection's `Option`; a per-statement override is pushed to the session before execution |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Server enforces the timeout via the `queryTimeout` session attribute | Client-side `tokio::time::timeout` wrap (superseded [7], [8]) | The client-side timer drops the in-flight future and desyncs the single owned transport, killing the connection and leaking the server session; server enforcement returns the timeout as a normal error |
| Send the attribute via `setAttributes` over the WebSocket | `ALTER SESSION SET QUERY_TIMEOUT=n` SQL | The driver already sets session attributes this way (`set_autocommit`); a typed attribute avoids injecting SQL and matches the confirmed wire format |
| `Option<Duration>` / `Option<u64>`, `None` = no attribute | `Duration` sentinel (`MAX`/`0`) | `Option` is idiomatic and self-documenting |
| Client give-up MUST close the connection | Soft local abort leaving the session open (the flagged anti-pattern) | An abandoned in-flight request leaves the server session dangling until idle-reap; terminating the connection releases it immediately |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| connection-management/session-and-lifecycle | CHANGED | `connection-management/session-and-lifecycle/spec.md` |
| adbc-driver/statement-and-results | CHANGED | `adbc-driver/statement-and-results/spec.md` |
| query-execution/execution | CHANGED | `query-execution/execution/spec.md` |

## Migration

| Current | New |
|---------|-----|
| `ConnectionParams::query_timeout: Duration` | Type is now `Option<Duration>`. `None` = no timeout attribute set (the new default). |
| No `query_timeout` set | Was a silent 300 s / 120 s client-side timer; now no attribute is set and the server's `QUERY_TIMEOUT` governs. |
| `ConnectionBuilder::query_timeout(Duration)` | Unchanged signature. Now forwards the value to Exasol as the `queryTimeout` session attribute instead of arming a client timer. |
| `exasol://...?query_timeout=600` | Parses to `Some(600 s)` and is forwarded to Exasol. |
| `exasol://...?query_timeout=not_a_number` | Unchanged. Still rejected with `InvalidParameter`. |
| Client-side `tokio::time::timeout` in `execute_statement` | Removed. The query executes directly; the server aborts and reports over-running queries. |
| `QueryError::Timeout { timeout_ms }` | Preserved. Now surfaces a server-reported timeout, not a client-timer elapse. The server-abort error is matched to this variant by its SQL code/message (pinned in decision [19]); an unrecognized failure maps to `QueryError::ExecutionFailed`. |
| `Statement::timeout_ms` / `set_timeout` | Preserved. `timeout_ms()` returns `Option<u64>`; a per-statement value is reconciled onto the session before execution, and a statement with no timeout resets the session `queryTimeout` to `0` (unlimited) so it never inherits a prior statement's server timeout. |
| Sub-second `query_timeout` / `set_timeout(<1000 ms)` | A positive sub-second timeout rounds **up** to `1 s` (`div_ceil` over milliseconds). It is never truncated to `0 s`, which Exasol treats as unlimited — the `0` value is reserved for the "no timeout" reset path only. |

## Implementation Tasks

1. In `src/connection/params.rs`, change the `ConnectionParams::query_timeout` field type from `Duration` to `Option<Duration>` (line 34) and update its `Debug` entry (line 173). In `build()`, replace `let query_timeout = self.query_timeout.unwrap_or(Duration::from_secs(300));` (line 374) with `let query_timeout = self.query_timeout;` and keep the struct-literal field (line 391). Keep the builder field (line 208, already `Option<Duration>`), the `query_timeout()` builder method (line 278), and the `"query_timeout"` parse arm (lines 522-529) intact — the parse arm still parses `<seconds>` into `Some(Duration)` and still returns `InvalidParameter` for a non-numeric value. The parsed value is now forwarded to the server rather than used for a client timer.
2. In `src/transport/messages.rs`, add a `SetAttributesRequest::query_timeout(seconds: u64)` constructor mirroring `autocommit` (line 787), inserting the key `queryTimeout` with a JSON number value. In `src/transport/native/attributes.rs`, add an `ATTR_QUERY_TIMEOUT` constant for the native `CMD_SET_ATTRIBUTES` key `queryTimeout`. In `src/transport/protocol.rs`, add `async fn set_query_timeout(&mut self, timeout_secs: u64) -> Result<(), TransportError>;` to the `TransportProtocol` trait (after `set_autocommit`, line 281). Implement it in `src/transport/websocket.rs` (mirror `set_autocommit`, lines 802-813: require `Authenticated` state, send `SetAttributesRequest::query_timeout`, check status) and in `src/transport/native/mod.rs` (mirror `set_autocommit`, lines 1200-1218: build an `AttributeSet` with `ATTR_QUERY_TIMEOUT`, send `CMD_SET_ATTRIBUTES`). Update the mockall trait mock in `src/query/results.rs` (line 826 region) to include `set_query_timeout`. [expert]
3. In `src/query/statement.rs`, change the `timeout_ms` field from `u64` to `Option<u64>` (line 278), change its default from `120_000` to `None` (line 292), update the getter `timeout_ms()` to return `Option<u64>` (lines 308-309), keep `set_timeout(&mut self, timeout_ms: u64)` but store `Some(timeout_ms)` (lines 313-314), and update the `Debug` entry if present.
4. In `src/connection/session.rs`, repurpose the previously dead `SessionConfig::query_timeout` field from `Duration` (lines 34-35) to `Option<Duration>` and change its default initializer from `Duration::from_secs(300)` to `None` (line 47). This field becomes live: it records the query timeout currently applied to the session and is the reconcile baseline. `connect()` seeds it (Task 5(a)) and `execute_statement` both reads it and writes the newly applied value back to it after every reconcile (Task 5(c)), so the stored value always reflects the server's current `queryTimeout`.
5. In `src/adbc/connection.rs`: (a) in `connect_with_transport`, after `authenticate` succeeds and before wrapping the transport in `Arc<Mutex<>>` (around lines 164-171), forward the connection timeout when set — `if let Some(d) = params.query_timeout { transport.set_query_timeout(secs_ceil(d)).await.map_err(...)?; }`, using the shared `secs_ceil` helper (see (c)) so `connect()` and the reconcile path resolve seconds identically — and set `SessionConfig.query_timeout = params.query_timeout` in the literal at line 169 to seed the applied-value baseline; (b) in `create_statement` (lines 268-272), inherit the connection's `Option` conditionally — `if let Some(d) = self.params.query_timeout { stmt.set_timeout(d.as_millis() as u64); }` — leaving the Statement at its `None` default otherwise; (c) rewrite `execute_statement` (lines 310-322): remove the `tokio::time::timeout` wrap entirely, then reconcile the session `queryTimeout` in **both directions** and record what was applied. Add a small helper `fn secs_ceil(d: Duration) -> u64` that rounds a duration up to whole seconds over milliseconds (`d.as_millis().div_ceil(1000) as u64`), so any positive sub-second timeout maps to at least `1 s` and never collapses to the `0`/unlimited sentinel (advisory 1). Compute the statement's target from `stmt.timeout_ms()`: `Some(ms)` → target seconds `secs_ceil(Duration::from_millis(ms))` (≥1); `None` → target "reset to unlimited" (seconds `0`, since Exasol treats `queryTimeout=0` as unlimited). Read the session's applied seconds from `SessionConfig.query_timeout` (`Some(d)` → `secs_ceil(d)`, `None` → `0`). If the target seconds differ from the applied seconds, call `set_query_timeout(target_secs)` on the locked transport — **including the `None`→`set_query_timeout(0)` reset call**, so a statement with no timeout following one that set a timeout clears the stale server value — then **write the applied value back**: `SessionConfig.query_timeout = stmt.timeout_ms().map(Duration::from_millis)` (or equivalently the target as `Option<Duration>`), so the next statement reconciles against the correct baseline. Only then `execute_query(&final_sql)` directly on the locked transport, mapping a server-reported query-timeout exception to `QueryError::Timeout { timeout_ms }` (matched by the server's SQL code/message, confirmed in decision [19]) and any other failure to `QueryError::ExecutionFailed`; (d) remove the now-unused `use tokio::time::timeout;` import (line 28); keep `use std::time::Duration;` (line 25) — it is now used by the reconcile write-back and `secs_ceil`. [expert]
6. Update unit tests. In `src/query/statement.rs`: change `test_statement_creation` (line 510) to assert `stmt.timeout_ms() == None`; change `test_statement_set_timeout` (lines 538-541) to assert `set_timeout(30_000)` yields `Some(30_000)`. In `src/connection/params.rs`: change `test_builder_default_values` (line 893) to assert `params.query_timeout == None`; change `test_builder_full` (lines 623/636) to assert `params.query_timeout == Some(Duration::from_secs(60))`; change `test_parse_query_timeout` (lines 926-930) to assert `Some(Duration::from_secs(60))`; keep `test_parse_invalid_query_timeout_value` (lines 962-969) unchanged. Leave `test_builder_validation_timeout` untouched.
7. Add integration tests in `tests/integration_tests.rs`: `test_no_query_timeout_by_default_allows_long_query` executes a multi-second server-side query on a default connection and asserts it returns a result set (never a timeout error); `test_explicit_query_timeout_is_enforced` configures a short explicit `query_timeout`, runs a slower server-side query, asserts the driver returns a server-originated `QueryError::Timeout` (assert the specific variant, not just any error — with the client timer removed a generic `ExecutionFailed` would otherwise be indistinguishable from a true timeout abort), and then asserts the same `Connection` executes a follow-up `SELECT 1` successfully — proving the transport did not desync; `test_reconcile_clears_stale_timeout_for_default_statement` opens a single connection, executes a first statement with `Statement::set_timeout(<short>)` (which sets the server `queryTimeout`), then on the **same connection** executes a second default statement (no `set_timeout`) that runs a multi-second server-side query, and asserts the second statement returns a result set and is **not** aborted — proving the `None`→reset-to-`0` write-back path clears the stale server timeout so an inherited-`None` statement runs unbounded.
8. Update `docs/setup-and-connect.md`: change the `query_timeout` parameter row (line 74) to state it is forwarded to Exasol as the server-enforced `queryTimeout` session attribute (seconds), default unset (server `QUERY_TIMEOUT` governs); cross-reference the `queryTimeout` session-attribute row (line 122) so the two are not described as independent knobs.
9. Add a `Breaking:` entry to `CHANGELOG.md` under a new version header: the `ConnectionParams::query_timeout` field type changes to `Option<Duration>`; a configured timeout is now enforced by the server via the `queryTimeout` session attribute instead of a client-side timer; the default is no timeout attribute.

The `query-execution/execution` "Cancel with timeout" spec delta carries no implementation task: query cancellation is unimplemented (`cancel` returns "not implemented"). The delta tightens the spec constraint for a future cancel implementation; see decision-log [17].

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1, Task 2, Task 3, Task 4, Task 8, Task 9 |
| Group B | Task 5 |
| Group C | Task 6, Task 7 |

Sequential dependencies:
- Group A edits disjoint files (`params.rs`; the transport layer `messages.rs`/`attributes.rs`/`protocol.rs`/`websocket.rs`/`native/mod.rs`/`results.rs`; `statement.rs`; `session.rs`; docs; CHANGELOG).
- Task 5 (`src/adbc/connection.rs`) depends on Group A: it calls the new `set_query_timeout` transport method (Task 2), reads the new `Option<Duration>` param (Task 1), the new `Option<u64>` Statement type (Task 3), and the repurposed `SessionConfig` field (Task 4). It is the only task editing `connection.rs`.
- Group C runs after A and B. Task 6 edits the same files as Tasks 1 and 3, so it follows them. Task 7 exercises the runtime behavior Task 5 implements.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| (none) | `src/connection/session.rs` `SessionConfig::query_timeout` | Formerly dead (read nowhere); this plan makes it live by using it as the applied-session-timeout source for reconciliation (Task 4, Task 5). No code is removed. |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| session-and-lifecycle: Query timeout (CHANGED — server-enforced) | Integration | `tests/integration_tests.rs` | `test_explicit_query_timeout_is_enforced` (asserts the `QueryError::Timeout` variant + connection reuse) |
| session-and-lifecycle: No query timeout by default (NEW) | Integration | `tests/integration_tests.rs` | `test_no_query_timeout_by_default_allows_long_query`; `test_reconcile_clears_stale_timeout_for_default_statement` (a default statement after a `set_timeout` statement on the same connection runs unbounded — the reset-to-`0` write-back path) |
| adbc-driver/statement-and-results: Statement inherits connection query timeout (CHANGED) | Unit + Integration | `src/query/statement.rs`; `tests/integration_tests.rs` | `test_statement_creation` (no hardcoded default: `timeout_ms() == None`); `test_statement_set_timeout`; `test_explicit_query_timeout_is_enforced` (Some inherited); `test_no_query_timeout_by_default_allows_long_query` (None inherited) |
| query-execution/execution: Cancel with timeout (CHANGED — must terminate the connection) | Deferred | — | No test or implementation task: cancel is unimplemented (`cancel` returns "not implemented"). The delta is a forward-looking spec constraint; see decision-log [17]. |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| session-and-lifecycle (default) | `cargo test --test integration_tests test_no_query_timeout_by_default_allows_long_query -- --nocapture` | A multi-second query returns rows; no timeout error |
| session-and-lifecycle (opt-in) | `cargo test --test integration_tests test_explicit_query_timeout_is_enforced -- --nocapture` | The slower query returns a server-originated `QueryError::Timeout`; a follow-up `SELECT 1` on the same connection succeeds |
| session-and-lifecycle (reconcile reset) | `cargo test --test integration_tests test_reconcile_clears_stale_timeout_for_default_statement -- --nocapture` | After a `set_timeout` statement, a default statement on the same connection runs a multi-second query to completion (no timeout abort) |
| session-and-lifecycle (server-enforced) | `exapump sql 'ALTER SESSION SET QUERY_TIMEOUT=2; SELECT COUNT(*) FROM (SELECT 1 FROM (VALUES BETWEEN 1 AND 100000) a CROSS JOIN (VALUES BETWEEN 1 AND 100000) b)'` | The server aborts the second statement with a query-timeout error |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Unit test | `cargo test --lib` | 0 failures |
| Integration test | `cargo test --test integration_tests` | 0 failures (Exasol running) |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
