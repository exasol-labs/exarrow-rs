# Decision Log: remove-query-timeout

## Interview

Headless mode — no live interview. The original bug report drove decisions [1]-[6]: exarrow-rs enforced a client-side query timeout that killed a long-running query even though the user's server-side session `QUERY_TIMEOUT` was 0 (unlimited). The root cause is the `tokio::time::timeout` layer in `Connection::execute_statement`, confirmed by the literal error text `Query timeout after {}ms`.

**2026-07-15 human correction.** The caller corrected the underlying design after the plan first validated: "Plan for: no timeout on default, opt-in timeout." This supersedes the original full-removal direction. The corrected requirement:

- Default (no config given): NO client-side query timeout. A query runs indefinitely, bounded only by the server. This is the fix for the original bug and is unchanged from the removal plan's user-visible default.
- The timeout capability remains as an explicit opt-in. When a caller configures a timeout — `ConnectionParams::query_timeout()`, the `query_timeout=` connection-string parameter, or `Statement::set_timeout()` — the driver MUST still enforce it (cancel the query when it elapses) exactly as today.

Decisions [7]-[11] below record the corrected design and name the superseded decisions.

## Design Decisions

### [1] Fully remove the query-timeout config surface, not just the enforcement

**Status: SUPERSEDED 2026-07-15 by [7].** Retained for history.

- **Decision:** Remove client-side enforcement in `execute_statement` AND the config surface that fed it: `ConnectionParams::query_timeout` (field + builder method), the `query_timeout` connection-string arm, `Statement::timeout_ms`/`set_timeout()`, `SessionConfig::query_timeout`, and `QueryError::Timeout`.
- **Alternatives:** (b) keep the config surface as a documented no-op; (c) keep the `ConnectionParams` field but stop reading it.
- **Rationale:** An unenforced timeout knob is a footgun. The crate is pre-1.0 and has a `Breaking:` CHANGELOG convention, so removing public API is cheap.
- **Why superseded:** The human correction keeps the timeout as an enforced opt-in, so the config surface stays. Only the default changes to unbounded. See [7].
- **Promotes to ADR:** no

### [2] Connection-string `query_timeout=` is tolerated and ignored, not rejected

**Status: SUPERSEDED 2026-07-15 by [9].** Retained for history.

- **Decision:** Remove the dedicated `"query_timeout"` parse arm; let the parameter fall through to the unknown-parameter path as an ignored custom attribute.
- **Rationale:** Under full removal the value had no meaning, so silent tolerance was a smaller break than rejection.
- **Why superseded:** Under the corrected design the parameter is functional again — it must be parsed and enforced, and invalid values must still be rejected. See [9].
- **Promotes to ADR:** no

### [3] Reversal of PR #31 ("Statement inherits query_timeout from connection params")

**Status: SUPERSEDED 2026-07-15 by [10].** Retained for history.

- **Decision:** Remove the `adbc-driver/statement-and-results` scenario "Statement inherits connection query timeout" and the mechanism it specified.
- **Rationale:** With enforcement gone, the inherited field was dead.
- **Why superseded:** Enforcement stays, so inheritance stays and is extended to also carry "no timeout". PR #31's inheritance wiring is kept, not reversed. See [10].
- **Promotes to ADR:** no

### [4] Scope boundary: connection, idle, and cancel-acknowledgment timeouts are untouched

**Status: unchanged and still in force.**

- **Decision:** Leave `connection_timeout` (transport connect), `idle_timeout`, and the cancellation-acknowledgment timeout scenarios in `query-execution/execution` unchanged.
- **Alternatives:** Sweep all timeout config in one pass.
- **Rationale:** Those govern distinct concerns — establishing a socket, closing idle connections, and how long to wait for the server to confirm a cancel. None limits how long a running query may take. This boundary holds under the corrected design.
- **Promotes to ADR:** no

### [5] Verification of "no client-side timeout" via a bounded heavy query

**Status: SUPERSEDED 2026-07-15 by [11].** Retained for history.

- **Decision:** Prove new behavior with a multi-second server-side query that completes without a timeout error.
- **Why superseded:** The corrected design needs two proofs, not one: (a) default-unbounded lets a long query complete, and (b) an explicitly configured short timeout still cancels. See [11].
- **Promotes to ADR:** no

### [6] Headless assumption: breaking public API is permitted without a major bump

**Status: AMENDED 2026-07-15 by [12].** Retained for history; the semver posture still holds, but the specific breaking surface changed.

- **Decision:** Treat public-API removal as acceptable under the crate's pre-1.0 semver policy, marked `Breaking:` in the CHANGELOG.
- **Rationale:** The crate is 0.13.0 (pre-1.0); CHANGELOG shows repeated `Breaking:` entries.
- **Why amended:** No public method is removed now. The breaking surface is a type change on the public `ConnectionParams::query_timeout` field (`Duration` -> `Option<Duration>`) plus the changed default behavior. See [12].
- **Promotes to ADR:** no

## Design Decisions — 2026-07-15 revision (design reversal)

### [7] Keep the query timeout as an enforced opt-in; change only the default to unbounded

- **Decision:** Retain client-side enforcement in `execute_statement`, the `ConnectionParams::query_timeout()` builder, the `query_timeout=` connection-string parameter, `Statement::set_timeout()`, and `QueryError::Timeout`. Change only the default: absent explicit configuration, no client-side timeout applies and the query runs indefinitely.
- **Supersedes:** [1] "Fully remove the query-timeout config surface, not just the enforcement".
- **Alternatives:** Full removal of the capability (the superseded [1]).
- **Rationale:** The human correction requires the capability to survive as opt-in. Full removal would delete a feature callers rely on. The original bug is fixed by the default change alone — the driver no longer imposes a timeout the caller never asked for.
- **Promotes to ADR:** yes

### [8] Represent the configurable timeout as an `Option` where `None` means unbounded

- **Decision:** Change `ConnectionParams::query_timeout` from `Duration` to `Option<Duration>` and `Statement::timeout_ms` from `u64` to `Option<u64>`. `None` means "no client-side timeout"; `Some(d)` is enforced. `create_statement()` propagates the connection's `Option` to the Statement. `execute_statement()` wraps execution in `tokio::time::timeout` only for `Some`, and awaits the query directly for `None` — no timer future is constructed in the `None` path.
- **Alternatives:** A `Duration` sentinel (`Duration::MAX` or `Duration::ZERO`) for "unbounded".
- **Rationale:** `Option<Duration>` is the idiomatic Rust representation of "a value that may be absent". A sentinel is easy to misread as a bug and forces every reader to know the magic value. `None` makes the unbounded path explicit and lets the compiler enforce the two-arm split in `execute_statement`.
- **Promotes to ADR:** yes

### [9] Connection-string `query_timeout=` stays functional and still rejects invalid values

- **Decision:** Keep the dedicated `"query_timeout"` parse arm in `apply_query_params`. It parses `<seconds>` into `Some(Duration::from_secs(...))` and continues to reject a non-numeric value (for example `query_timeout=not_a_number`) with `ConnectionError::InvalidParameter`. Only the unset case changes: a DSN with no `query_timeout=` yields `None` (unbounded) rather than the former 300 s default.
- **Supersedes:** [2] "Connection-string `query_timeout=` is tolerated and ignored, not rejected".
- **Alternatives:** Drop the parse arm (the superseded [2]).
- **Rationale:** The parameter is functional again, so it must round-trip and validate. Rejecting garbage values preserves the existing contract and catches typos; silently ignoring a malformed timeout would mask configuration errors.
- **Promotes to ADR:** no

### [10] Statement still inherits the connection timeout, now including "no timeout"

- **Decision:** Keep PR #31's inheritance: `create_statement()` copies the connection's query timeout onto the new Statement. Extend it so the inherited value is the connection's `Option`: `Some(d)` when configured, `None` when not. A Statement MUST NOT fall back to any hardcoded non-zero default independent of the connection.
- **Supersedes:** [3] "Reversal of PR #31 (Statement inherits query_timeout from connection params)".
- **Alternatives:** Reverse the inheritance (the superseded [3]).
- **Rationale:** Inheritance is the mechanism that makes opt-in work end to end. The only defect in the old behavior was the hardcoded `120_000 ms` Statement default and the `300 s` connection default, both replaced by `None`.
- **Promotes to ADR:** no

### [11] Verify both the default-unbounded path and the explicit-timeout path

- **Decision:** Add two integration tests. (a) `test_no_query_timeout_by_default_allows_long_query`: a multi-second server-side query on a default connection returns a result set, never a timeout error. (b) `test_explicit_query_timeout_is_enforced`: with a short explicit timeout configured, a slower query returns `QueryError::Timeout`.
- **Supersedes:** [5] "Verification of no client-side timeout via a bounded heavy query".
- **Alternatives:** A single default-unbounded test (the superseded [5]).
- **Rationale:** The corrected design has two behaviors, so it needs two proofs. Test (b) guards against a regression that drops enforcement entirely; test (a) guards against re-introducing a default timeout. Both use bounded server-side work to stay inside the CI time budget.
- **Promotes to ADR:** no

### [12] Semver: the change is breaking via a public field type change, marked `Breaking:`

- **Decision:** Mark the change `Breaking:` in the CHANGELOG under the next version bump. The breaking surface is the `ConnectionParams::query_timeout` field type (`Duration` -> `Option<Duration>`) and the changed default (queries no longer time out at 300 s / 120 s unless configured). No public method is removed.
- **Amends:** [6] "Headless assumption: breaking public API is permitted without a major bump".
- **Rationale:** The crate is pre-1.0 (0.13.0) with an established `Breaking:` convention, so a breaking change in a minor release is precedented and reducible — no escalation needed. The exact next version is set at implementation time per the repo's per-change bump convention.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Missing DELTA:REMOVED marker on statement-and-results scenario

- **Finding:** Round 1 flagged a missing `DELTA:REMOVED` marker on the "Statement inherits connection query timeout" scenario.
- **Direction change:** Moot under the corrected design. The scenario is no longer removed; it is `DELTA:CHANGED` to cover both the configured and no-timeout inheritance cases (decision [10]). The delta now carries a matched `DELTA:CHANGED`/`/DELTA:CHANGED` pair.
- **Promotes to ADR:** no

### [plan-review] Task granularity — two tasks editing the same file in one parallel group

- **Finding:** Round 1 advised against placing two tasks that both edit `src/adbc/connection.rs` in the same parallel group.
- **Direction change:** All `src/adbc/connection.rs` edits (the `create_statement` propagation, the conditional `execute_statement` wrap, and the `SessionConfig` literal update) are consolidated into a single task (Task 2), which is the only task touching that file. No parallel group contains two tasks that edit the same file.
- **Promotes to ADR:** no

### [plan-review] Connection-string parsing must still reject garbage values

- **Finding:** Round 1 flagged that invalid `query_timeout=` values (for example `query_timeout=not_a_number`) must have defined handling.
- **Direction change:** The parse arm is retained and still rejects non-numeric values with `ConnectionError::InvalidParameter` (decision [9]). The `test_parse_invalid_query_timeout_value` unit test is kept, not deleted.
- **Promotes to ADR:** no
