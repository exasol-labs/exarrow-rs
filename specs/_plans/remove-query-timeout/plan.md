# Plan: remove-query-timeout

## Summary

Change the driver's client-side query timeout from an always-on default to an opt-in feature. Absent explicit configuration, a query runs with no client-side timeout until the server returns or the server-side session `QUERY_TIMEOUT` elapses. A caller who configures a timeout still gets enforcement exactly as today.

## Design

### Context

`Connection::execute_statement` wraps query execution in `tokio::time::timeout` and returns `QueryError::Timeout` when the timer elapses. The timeout came from an always-present default (300 s on `ConnectionParams`, 120 s on `Statement`). A user set the Exasol session `QUERY_TIMEOUT` to 0 (unlimited), yet the driver still killed the query after its own default. The driver must not impose a timeout the caller never asked for — but callers who do ask for one must still get it.

- **Goals** — Default to no client-side query timeout so a query runs indefinitely unless the caller opts in. Preserve the opt-in timeout capability (`ConnectionParams::query_timeout()`, the `query_timeout=` connection-string parameter, `Statement::set_timeout()`) and its enforcement (`tokio::time::timeout` + `QueryError::Timeout`).
- **Non-Goals** — Change connection-establishment timeout (`connection_timeout`) or idle timeout (`idle_timeout`); change the cancellation-acknowledgment timeout in `query-execution/execution`; add a server-side `QUERY_TIMEOUT` control API; remove the `query_timeout` config surface.

### Decision

Change only the default, from a fixed `Duration` to "unbounded", by making the timeout an `Option`: `None` means no client-side timeout, `Some(d)` is enforced. Rationale in `decision-log.md` [7], [8]. This supersedes the original full-removal direction (decision [1]).

#### Architecture

```
create_statement ──copies Option──▶ Statement.timeout_ms: Option<u64>
                                              │
                       execute_statement reads it:
                         Some(ms) ─▶ tokio::time::timeout(ms, execute) ─elapse─▶ QueryError::Timeout
                         None      ─▶ execute directly (no timer future at all)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| `Option` for "value may be absent" | `ConnectionParams::query_timeout`, `Statement::timeout_ms` | `None` = unbounded is idiomatic and compiler-checked; avoids a `Duration::MAX`/`0` sentinel that reads as a bug |
| Conditional enforcement | `execute_statement` | Only construct a timer when a timeout is configured; the `None` path awaits the query with no wrapping future |
| Inherit-from-connection | `create_statement` | Statement inherits the connection's `Option`, including `None`; no hardcoded Statement default |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Keep the timeout as an enforced opt-in; change only the default to unbounded | Full removal of the capability (superseded [1]) | The human correction requires the capability to survive; the bug is fixed by the default change alone |
| Represent the timeout as `Option<Duration>` / `Option<u64>` (`None` = unbounded) | A `Duration` sentinel (`MAX` or `0`) | `Option` is idiomatic and self-documenting; a sentinel is easy to misread later |
| Connection-string `query_timeout=` stays functional and still rejects invalid values | Drop the parse arm (superseded [2]) | The parameter is enforced again, so it must round-trip and validate; rejecting garbage catches typos |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| connection-management/session-and-lifecycle | CHANGED | `connection-management/session-and-lifecycle/spec.md` |
| adbc-driver/statement-and-results | CHANGED | `adbc-driver/statement-and-results/spec.md` |

## Migration

| Current | New |
|---------|-----|
| `ConnectionParams::query_timeout: Duration` | Type is now `Option<Duration>`. `None` = no client-side timeout (the new default). |
| No `query_timeout` set | Was a silent 300 s / 120 s client-side timeout; now unbounded (no client-side limit). |
| `ConnectionBuilder::query_timeout(Duration)` | Unchanged. Still configures and enforces a client-side timeout. |
| `exasol://...?query_timeout=600` | Unchanged. Parses to `Some(600 s)` and is enforced. |
| `exasol://...?query_timeout=not_a_number` | Unchanged. Still rejected with `InvalidParameter`. |
| `Statement::set_timeout` / `Statement::timeout_ms` | Preserved. `timeout_ms()` now returns `Option<u64>`; default is `None`. |
| `QueryError::Timeout { timeout_ms }` | Preserved. Surfaces only when an explicit timeout elapses. |

## Implementation Tasks

1. In `src/connection/params.rs`, change the `ConnectionParams::query_timeout` field type from `Duration` to `Option<Duration>` (line 34) and update its `Debug` entry (line 173). In `build()`, replace `let query_timeout = self.query_timeout.unwrap_or(Duration::from_secs(300));` (line 374) with `let query_timeout = self.query_timeout;` and keep the struct-literal field (line 391). Keep the builder field (line 208, already `Option<Duration>`), the `query_timeout()` builder method (line 278), and the `"query_timeout"` parse arm (lines 522-529) intact — the parse arm still parses `<seconds>` into `Some(Duration)` and still returns `InvalidParameter` for a non-numeric value.
2. In `src/adbc/connection.rs`: (a) in `create_statement` (line 268-272), propagate the connection's timeout conditionally — `if let Some(d) = self.params.query_timeout { stmt.set_timeout(d.as_millis() as u64); }` — leaving the Statement at its `None` default when the connection has none; (b) in `execute_statement` (lines 310-322), branch on `stmt.timeout_ms()`: for `Some(ms)` keep the current `tokio::time::timeout(Duration::from_millis(ms), ...)` wrap that maps elapse to `QueryError::Timeout { timeout_ms: ms }`, and for `None` execute the query directly (`transport_guard.execute_query(&final_sql).await`) with no timer future, mapping errors to `QueryError::ExecutionFailed`; keep the `use std::time::Duration;` (line 25) and `use tokio::time::timeout;` (line 28) imports (still used in the `Some` arm); (c) remove the `query_timeout: params.query_timeout,` field from the `SessionConfig { .. }` literal (line 169), since `SessionConfig::query_timeout` is removed as dead code in Task 4. [expert]
3. In `src/query/statement.rs`, change the `timeout_ms` field from `u64` to `Option<u64>` (line 278), change its default from `120_000` to `None` (line 292), update the getter `timeout_ms()` to return `Option<u64>` (lines 308-309), keep `set_timeout(&mut self, timeout_ms: u64)` but store `Some(timeout_ms)` (lines 313-314), and update the `Debug` entry (line 391).
4. In `src/connection/session.rs`, remove the dead `SessionConfig::query_timeout` field (lines 34-35) and its default initializer (line 47). Grep confirms the field is set at `connection.rs:169` but read nowhere; it is dead independent of this change.
5. Update unit tests. In `src/query/statement.rs`: change `test_statement_creation` (line 510) to assert `stmt.timeout_ms() == None`; change `test_statement_set_timeout` (lines 538-541) to assert `set_timeout(30_000)` yields `Some(30_000)`. In `src/connection/params.rs`: change `test_builder_default_values` (line 893) to assert `params.query_timeout == None`; change `test_builder_full` (lines 623/636) to assert `params.query_timeout == Some(Duration::from_secs(60))`; change `test_parse_query_timeout` (lines 926-930) to assert `Some(Duration::from_secs(60))`; keep `test_parse_invalid_query_timeout_value` (lines 962-969) unchanged — invalid values are still rejected. Leave `test_builder_validation_timeout` untouched (it exercises `connection_timeout`).
6. Add two integration tests in `tests/integration_tests.rs`: `test_no_query_timeout_by_default_allows_long_query` executes a multi-second server-side query on a default connection and asserts it returns a result set (never a timeout error); `test_explicit_query_timeout_is_enforced` configures a short explicit `query_timeout`, runs a slower query, and asserts it returns `QueryError::Timeout`.
7. Update `docs/setup-and-connect.md`: change the `query_timeout` parameter table row to state the default is now no client-side timeout (opt-in) rather than 300 s, and keep the `query_timeout=600` example as a valid opt-in.
8. Add a `Breaking:` entry to `CHANGELOG.md` under a new version header: the `ConnectionParams::query_timeout` field type changes to `Option<Duration>` and the default becomes no client-side timeout; the opt-in timeout and its enforcement are preserved.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1, Task 3, Task 4, Task 7, Task 8 |
| Group B | Task 2 |
| Group C | Task 5, Task 6 |

Sequential dependencies:
- Group A edits five distinct files (`params.rs`, `statement.rs`, `session.rs`, `docs`, `CHANGELOG`) with no overlap.
- Task 2 (`src/adbc/connection.rs`) depends on Group A: it reads the new `Option<Duration>` field (Task 1), the new `Option<u64>` Statement type (Task 3), and the removed `SessionConfig` field (Task 4). It is the only task editing `connection.rs`.
- Group C runs after A and B. Task 5 edits the same files as Tasks 1 and 3 (`params.rs`, `statement.rs` test modules), so it must follow them. Task 6 exercises the runtime behavior Task 2 implements.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Field + default | `src/connection/session.rs` `SessionConfig::query_timeout` | Set at `connection.rs:169`, read nowhere; dead before and after this change |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| session-and-lifecycle: Query timeout (CHANGED — opt-in enforcement) | Integration | `tests/integration_tests.rs` | `test_explicit_query_timeout_is_enforced` |
| session-and-lifecycle: No query timeout by default (NEW) | Integration | `tests/integration_tests.rs` | `test_no_query_timeout_by_default_allows_long_query` |
| adbc-driver/statement-and-results: Statement inherits connection query timeout (CHANGED — both Some and None) | Unit + Integration | `src/query/statement.rs`; `tests/integration_tests.rs` | `test_statement_creation` (asserts no hardcoded default: `timeout_ms() == None`); `test_explicit_query_timeout_is_enforced` (Some inherited); `test_no_query_timeout_by_default_allows_long_query` (None inherited) |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| session-and-lifecycle (default) | `cargo test --test integration_tests test_no_query_timeout_by_default_allows_long_query -- --nocapture` | A multi-second query returns rows; no `Query timeout` error |
| session-and-lifecycle (opt-in) | `cargo test --test integration_tests test_explicit_query_timeout_is_enforced -- --nocapture` | The slower query returns `QueryError::Timeout` |
| session-and-lifecycle (default) | `exapump sql 'SELECT COUNT(*) FROM (SELECT 1 FROM (VALUES BETWEEN 1 AND 100000) a CROSS JOIN (VALUES BETWEEN 1 AND 100000) b)'` | Query completes and returns a count (no client-side abort) |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Unit test | `cargo test --lib` | 0 failures |
| Integration test | `cargo test --test integration_tests` | 0 failures (Exasol running) |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
