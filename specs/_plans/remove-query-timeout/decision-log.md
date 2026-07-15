# Decision Log: remove-query-timeout

## Interview

Headless mode — no live interview. The original bug report drove decisions [1]-[6]: exarrow-rs enforced a client-side query timeout that killed a long-running query even though the user's server-side session `QUERY_TIMEOUT` was 0 (unlimited). The root cause is the `tokio::time::timeout` layer in `Connection::execute_statement`, confirmed by the literal error text `Query timeout after {}ms`.

**2026-07-15 human correction.** The caller corrected the underlying design after the plan first validated: "Plan for: no timeout on default, opt-in timeout." This supersedes the original full-removal direction. The corrected requirement:

- Default (no config given): NO client-side query timeout. A query runs indefinitely, bounded only by the server. This is the fix for the original bug and is unchanged from the removal plan's user-visible default.
- The timeout capability remains as an explicit opt-in. When a caller configures a timeout — `ConnectionParams::query_timeout()`, the `query_timeout=` connection-string parameter, or `Statement::set_timeout()` — the driver MUST still enforce it (cancel the query when it elapses) exactly as today.

Decisions [7]-[12] recorded that corrected design (client-side enforcement kept as opt-in).

**2026-07-15 second correction (design reversal).** The caller reviewed the opt-in-client-timer design and rejected the enforcement mechanism itself, in two messages:

1. "killt das immer noch den client, weil der query timeout ist ein session/connection option und sollte von der DB getriggert werden" — the client-side `tokio::time::timeout` wrap still kills the client. `query_timeout` is a session/connection option and should be triggered by the DB.
2. "also auch client seitig das ist nicht gut, das lässt im zweifel sessions offen, bis die DB nach einer gewissen zeit merkt der client ist nicht mehr da ... wenn dann sollte der client die session direkt killen, wenn er selber ein timeout machen will" — client-side enforcement leaves the server session open until Exasol's idle reaper notices; if the client does enforce its own cutoff, it must directly kill the session.

The corrected requirement: forward a configured timeout to Exasol as the `queryTimeout` session attribute so the server enforces and reports it; remove the client-side timer entirely; and require that any remaining client-side give-up on a running query terminate the connection rather than abandon the in-flight request. Decisions [13]-[18] below record this second reversal and name the superseded decisions.

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

**Status: AMENDED 2026-07-15 by [17].** The `connection_timeout` and `idle_timeout` boundaries hold. The cancellation-acknowledgment boundary is narrowed: the second human correction explicitly connects the query-timeout give-up behavior to the `query-execution/execution` "Cancel with timeout" fallback, so that scenario is now revised (spec-only) rather than left untouched. See [17].

- **Decision:** Leave `connection_timeout` (transport connect), `idle_timeout`, and the cancellation-acknowledgment timeout scenarios in `query-execution/execution` unchanged.
- **Alternatives:** Sweep all timeout config in one pass.
- **Rationale:** Those govern distinct concerns — establishing a socket, closing idle connections, and how long to wait for the server to confirm a cancel. None limits how long a running query may take.
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

**Status: SUPERSEDED 2026-07-15 by [13].** Retained for history. This decision kept the client-side `tokio::time::timeout` enforcement mechanism; the second human correction rejects that mechanism.

- **Decision:** Retain client-side enforcement in `execute_statement`, the `ConnectionParams::query_timeout()` builder, the `query_timeout=` connection-string parameter, `Statement::set_timeout()`, and `QueryError::Timeout`. Change only the default: absent explicit configuration, no client-side timeout applies and the query runs indefinitely.
- **Supersedes:** [1] "Fully remove the query-timeout config surface, not just the enforcement".
- **Alternatives:** Full removal of the capability (the superseded [1]).
- **Rationale:** The human correction requires the capability to survive as opt-in. Full removal would delete a feature callers rely on. The original bug is fixed by the default change alone — the driver no longer imposes a timeout the caller never asked for.
- **Promotes to ADR:** yes

### [8] Represent the configurable timeout as an `Option` where `None` means unbounded

**Status: SUPERSEDED 2026-07-15 by [14].** Retained for history. The `Option` representation survives, but its `Some` arm no longer constructs a client-side timer; it forwards the value to the server.

- **Decision:** Change `ConnectionParams::query_timeout` from `Duration` to `Option<Duration>` and `Statement::timeout_ms` from `u64` to `Option<u64>`. `None` means "no client-side timeout"; `Some(d)` is enforced. `create_statement()` propagates the connection's `Option` to the Statement. `execute_statement()` wraps execution in `tokio::time::timeout` only for `Some`, and awaits the query directly for `None` — no timer future is constructed in the `None` path.
- **Alternatives:** A `Duration` sentinel (`Duration::MAX` or `Duration::ZERO`) for "unbounded".
- **Rationale:** `Option<Duration>` is the idiomatic Rust representation of "a value that may be absent". A sentinel is easy to misread as a bug and forces every reader to know the magic value. `None` makes the unbounded path explicit and lets the compiler enforce the two-arm split in `execute_statement`.
- **Promotes to ADR:** yes

### [9] Connection-string `query_timeout=` stays functional and still rejects invalid values

- **Decision:** Keep the dedicated `"query_timeout"` parse arm in `apply_query_params`. It parses `<seconds>` into `Some(Duration::from_secs(...))` and continues to reject a non-numeric value (for example `query_timeout=not_a_number`) with `ConnectionError::InvalidParameter`. Only the unset case changes: a DSN with no `query_timeout=` yields `None` (unbounded) rather than the former 300 s default.
- **Supersedes:** [2] "Connection-string `query_timeout=` is tolerated and ignored, not rejected".
- **Alternatives:** Drop the parse arm (the superseded [2]).
- **Rationale:** The parameter is functional again, so it must round-trip and validate. Rejecting garbage values preserves the existing contract and catches typos; silently ignoring a malformed timeout would mask configuration errors.
- **2026-07-15 amendment (by [13]):** Still in force. The parsed `Some(Duration)` is now forwarded to Exasol as the `queryTimeout` session attribute instead of feeding a client-side timer. Parsing and validation are unchanged.
- **Promotes to ADR:** no

### [10] Statement still inherits the connection timeout, now including "no timeout"

- **Decision:** Keep PR #31's inheritance: `create_statement()` copies the connection's query timeout onto the new Statement. Extend it so the inherited value is the connection's `Option`: `Some(d)` when configured, `None` when not. A Statement MUST NOT fall back to any hardcoded non-zero default independent of the connection.
- **Supersedes:** [3] "Reversal of PR #31 (Statement inherits query_timeout from connection params)".
- **Alternatives:** Reverse the inheritance (the superseded [3]).
- **Rationale:** Inheritance is the mechanism that makes opt-in work end to end. The only defect in the old behavior was the hardcoded `120_000 ms` Statement default and the `300 s` connection default, both replaced by `None`.
- **2026-07-15 amendment (by [13], [15]):** Still in force. The inherited value now drives the server-side `queryTimeout` attribute: `execute_statement` reconciles a per-statement override against the session's applied timeout before executing, rather than arming a client-side timer.
- **Promotes to ADR:** no

### [11] Verify both the default-unbounded path and the explicit-timeout path

**Status: SUPERSEDED 2026-07-15 by [16].** Retained for history. Two tests are still added, but the explicit-timeout test now asserts a server-originated abort and connection reuse, not a client-timer elapse.

- **Decision:** Add two integration tests. (a) `test_no_query_timeout_by_default_allows_long_query`: a multi-second server-side query on a default connection returns a result set, never a timeout error. (b) `test_explicit_query_timeout_is_enforced`: with a short explicit timeout configured, a slower query returns `QueryError::Timeout`.
- **Supersedes:** [5] "Verification of no client-side timeout via a bounded heavy query".
- **Alternatives:** A single default-unbounded test (the superseded [5]).
- **Rationale:** The corrected design has two behaviors, so it needs two proofs. Test (b) guards against a regression that drops enforcement entirely; test (a) guards against re-introducing a default timeout. Both use bounded server-side work to stay inside the CI time budget.
- **Promotes to ADR:** no

### [12] Semver: the change is breaking via a public field type change, marked `Breaking:`

- **Decision:** Mark the change `Breaking:` in the CHANGELOG under the next version bump. The breaking surface is the `ConnectionParams::query_timeout` field type (`Duration` -> `Option<Duration>`) and the changed default (queries no longer time out at 300 s / 120 s unless configured). No public method is removed.
- **Amends:** [6] "Headless assumption: breaking public API is permitted without a major bump".
- **Rationale:** The crate is pre-1.0 (0.13.0) with an established `Breaking:` convention, so a breaking change in a minor release is precedented and reducible — no escalation needed. The exact next version is set at implementation time per the repo's per-change bump convention.
- **2026-07-15 amendment (by [13]):** Still in force and still breaking. The breaking surface is unchanged in shape (the `ConnectionParams::query_timeout` field type plus a behavior change), but the behavior change is now "the timeout is server-enforced and the client-side timer is removed" rather than "the default becomes unbounded".
- **Promotes to ADR:** no

## Design Decisions — 2026-07-15 second revision (server-side enforcement)

### [13] Forward the configured timeout to Exasol; remove client-side enforcement

- **Decision:** Enforce a configured query timeout server-side. When a caller configures a timeout, the driver forwards it to Exasol as the `queryTimeout` session attribute; the server aborts an over-running query and reports the abort through the normal response cycle. Remove the `tokio::time::timeout` wrap in `execute_statement` entirely — no client-side timer is constructed in any path.
- **Supersedes:** [7] "Keep the query timeout as an enforced opt-in; change only the default to unbounded".
- **Alternatives:** Keep the client-side `tokio::time::timeout` wrap as the enforcement mechanism (the superseded [7]/[8]).
- **Rationale:** The human correction states the timeout is a session/connection option the DB should trigger. This driver's `Connection` owns a single WebSocket exclusively; when the client-side timer elapses and the wrapped future is dropped, the outstanding request/response cycle is abandoned mid-flight, desyncing the transport and leaving the `Connection` unusable and the server session dangling until idle-reap. Server enforcement returns the timeout as a normal error, so the transport stays in sync and the connection remains usable.
- **Promotes to ADR:** yes

### [14] The `Option` value selects whether the `queryTimeout` attribute is set, not a client timer

- **Decision:** Keep `ConnectionParams::query_timeout: Option<Duration>` and `Statement::timeout_ms: Option<u64>`. Redefine the semantics: `Some(d)` means "forward `d.as_secs()` to Exasol as the `queryTimeout` session attribute"; `None` means "set no attribute; the server's own `QUERY_TIMEOUT` governs". No arm constructs a client-side timer.
- **Supersedes:** [8] "Represent the configurable timeout as an `Option` where `None` means unbounded".
- **Alternatives:** A `Duration` sentinel for "no attribute".
- **Rationale:** `Option` remains the idiomatic representation of an absent value. Only its meaning shifts from "arm/skip a client timer" to "set/skip a server attribute", so the public type stays stable while the mechanism changes underneath.
- **Promotes to ADR:** yes

### [15] Set the attribute via `setAttributes`, mirroring `set_autocommit`

- **Decision:** Add `TransportProtocol::set_query_timeout(timeout_secs: u64)`, implemented on both transports by sending the `queryTimeout` attribute — the WebSocket transport via a `setAttributes` command (`SetAttributesRequest`), the native transport via `CMD_SET_ATTRIBUTES` — exactly as `set_autocommit` already sets the `autocommit` attribute. `connect()` calls it once after authentication when the connection configures a timeout. `execute_statement()` reconciles a per-statement override against the session's applied value before executing.
- **Alternatives:** Issue `ALTER SESSION SET QUERY_TIMEOUT = n` as SQL; set the attribute in the login command's `attributes` object.
- **Rationale:** `set_autocommit` is a proven precedent for setting a session attribute post-authenticate over the same request/response path, so `queryTimeout` reuses it. `queryTimeout` is confirmed as a client-settable number-of-seconds attribute in the Exasol WebSocket API (and already documented in `docs/setup-and-connect.md`), so the wire format is known, not guessed. A typed attribute avoids composing session-management SQL. Post-authenticate `setAttributes` is chosen over login attributes because it also serves the per-statement reconcile path with one mechanism.
- **Promotes to ADR:** yes

### [16] Verify server-side enforcement and connection reuse after a timeout

- **Decision:** Add two integration tests. (a) `test_no_query_timeout_by_default_allows_long_query`: a multi-second server-side query on a default connection returns a result set, never a timeout error. (b) `test_explicit_query_timeout_is_enforced`: with a short explicit timeout configured, a slower server-side query returns a server-originated error, and a follow-up `SELECT 1` on the same `Connection` succeeds — proving the transport did not desync.
- **Supersedes:** [11] "Verify both the default-unbounded path and the explicit-timeout path".
- **Alternatives:** Assert only that an error is returned, without checking connection reuse (the superseded [11]).
- **Rationale:** The defect the human flagged is transport desync after a timeout, so the test must prove the connection survives a timeout, not merely that an error is returned.
- **Promotes to ADR:** no

### [17] Client-side give-up MUST terminate the connection; narrow the cancel-timeout scope

- **Decision:** Establish the principle that any client-side decision to give up on a running query MUST terminate the connection, rather than abandon the in-flight request and leave the session open server-side. Apply it by revising the `query-execution/execution` "Cancel with timeout" scenario (CHANGED): the acknowledgment-timeout fallback MUST close the connection and MUST NOT merely abort local execution. This is a spec-only change — query cancellation is unimplemented (`cancel` returns "not implemented"), so the plan adds no cancel implementation task; the scenario constrains a future implementation.
- **Amends:** [4] "Scope boundary: connection, idle, and cancel-acknowledgment timeouts are untouched" (narrows the cancel-acknowledgment part).
- **Alternatives:** Leave "Cancel with timeout" untouched and treat cancel as strictly out of scope; escalate the scope question to a human.
- **Rationale:** The second human correction explicitly connects the query-timeout give-up to the "Cancel with timeout" fallback text, so the boundary in [4] no longer holds for that scenario. The change is a reversible spec edit that does not alter current user-facing behavior (cancel is unimplemented), so it clears the headless bar for making the call rather than escalating. The redesign in [13] already removes the only implemented client-side query-give-up path, so the driver ships no soft-abandonment; this decision codifies the principle for the remaining unimplemented path.
- **Promotes to ADR:** yes

### [18] Statement timeout is reconciled to the session in both directions, with write-back

- **Decision:** Keep `Statement::set_timeout()` / `Statement::timeout_ms` as the per-statement surface. Because `queryTimeout` is session-level, `execute_statement` reconciles it in **both directions** on every execute: it computes the statement's target seconds (`Some(ms)` → rounded up to ≥1 s; `None` → `0`, since Exasol treats `queryTimeout=0` as unlimited), compares them to the session's applied seconds stored in the repurposed `SessionConfig::query_timeout`, and when they differ calls `set_query_timeout(target_secs)` — **including a reset call `set_query_timeout(0)` for the `None` target** — then **writes the applied value back** into `SessionConfig::query_timeout`. `create_statement` continues to inherit the connection's `Option`.
- **Alternatives:** Reconcile only in the `Some` direction without write-back (the round-1 design; rejected — see the `[plan-review]` finding below); drop the per-statement surface and support only connection-level timeouts; send `setAttributes` before every statement unconditionally.
- **Rationale:** Exasol has no per-statement timeout in the protocol, but the public `Statement::set_timeout` API must keep working. A one-directional (`Some`-only) reconcile without write-back leaves a stale server `queryTimeout` in place: a statement that set `Some(5 s)` followed by an inherited-`None` statement on the same `&mut` connection would run the second (deliberately unbounded) query under the previous 5 s limit — the same class of defect as the original bug, reachable through the preserved `set_timeout` surface. Reconciling both directions and resetting to `0` for `None`, then writing the applied value back so the next statement sees the correct baseline, closes that hole while keeping the fewest extra round-trips (none when the statement matches the applied session value). Repurposing the formerly dead `SessionConfig::query_timeout` field as the applied-value store turns dead code live instead of deleting it.
- **Promotes to ADR:** yes

### [19] Map the server query-timeout abort to `QueryError::Timeout` by SQL state `R0001`

- **Decision:** A server-reported query-timeout abort is identified by Exasol SQL state **`R0001`** with message text `Query terminated because timeout has been reached.` `execute_statement` maps a failure carrying that SQL code to `QueryError::Timeout { timeout_ms }`; any other execution failure maps to `QueryError::ExecutionFailed`. Match on the SQL code (`R0001`) as the primary signal, with the message text as a secondary fallback.
- **Alternatives:** Match on message substring only; defer the mapping to implementation-time inspection and leave the Migration table promising `QueryError::Timeout` unconditionally; soften the Migration promise to "an error" without naming the variant.
- **Rationale:** Confirmed empirically against the running Exasol Docker container (`exasol/docker-db:latest`): `ALTER SESSION SET QUERY_TIMEOUT=1` followed by a heavy `CROSS JOIN` returns `SQL state: R0001 — Query terminated because timeout has been reached.` With the client-side timer removed, a true timeout abort and a generic execution failure are otherwise indistinguishable to both the driver and the test, so the mapping must key on a specific, verified signal rather than a guess. `test_explicit_query_timeout_is_enforced` asserts the `QueryError::Timeout` variant specifically (not merely that an error occurred), which is only sound because the mapping is pinned here.
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

### [plan-review] One-directional reconcile leaks a stale server timeout into a later no-timeout statement

- **Finding:** Round 1 (round-2 review of this revision) flagged a `[REQUIREMENT_CONFLICT]` BLOCKER: Task 5(c) reconciled only in the `Some` direction and never wrote the applied value back to `SessionConfig::query_timeout`. On a `query_timeout = None` connection, a statement calling `set_timeout(5000)` would set the server `queryTimeout=5`, and a subsequent inherited-`None` statement — whose reconcile guard `Some(ms) differs` is false for `None` — would fire no reset, so the server kept `queryTimeout=5` and aborted the deliberately unbounded query at 5 s. Because `execute_statement` takes `&mut self`, this is deterministic sequential staleness, and it violates the NEW scenario "No query timeout by default" and reproduces the original bug's defect class through the preserved `Statement::set_timeout` surface.
- **Direction change:** Task 5(c) now reconciles in **both directions** and records what it applied: it computes the statement's target (`Some(ms)` → seconds rounded up to ≥1; `None` → reset, seconds `0`), compares against the applied session seconds in `SessionConfig::query_timeout`, and when they differ calls `set_query_timeout(target_secs)` — **including `set_query_timeout(0)` for the `None` target** (Exasol `queryTimeout=0` = unlimited) — then **writes the applied value back** into `SessionConfig::query_timeout` so the next statement reconciles against the correct baseline. Decision [18] and the plan's architecture diagram now state the write-back and the `None`→reset-to-`0` path explicitly; Task 4 documents the write-back; the NEW spec scenario gains an explicit reset clause. A new integration test, `test_reconcile_clears_stale_timeout_for_default_statement`, asserts that a default statement following a `set_timeout` statement on the same connection runs unbounded (Task 7, Scenario Coverage, Manual Testing).
- **Also incorporated (advisories):** (1) Sub-second timeouts now round **up** to `1 s` (`div_ceil` over milliseconds) rather than truncating to `0 s` via `as_secs()` — necessary because the BLOCKER fix reserves `0` exclusively for the `None`/unlimited reset, so a truncated sub-second value would otherwise silently invert a tight timeout into "unlimited" (Task 5(c), Migration table). (2) The server query-timeout error mapping was confirmed against the running container (SQL state `R0001`, message "Query terminated because timeout has been reached.") and pinned in decision [19]; `test_explicit_query_timeout_is_enforced` now asserts the specific `QueryError::Timeout` variant, and the Migration promise references the pinned code instead of an unverified claim.
- **Promotes to ADR:** yes
