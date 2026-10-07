# Plan Review Findings: fix-transport-deadlines (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 2, Advisory: 7)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now this plan failed. Three ways it got there:

1. The recorded `session-and-lifecycle` scenario says that every operation after termination fails and also that `close()` succeeds. A later change to `close()` follows the first step, breaks `test_csv_export_explicit_timeout_terminates_connection`, and no reader can tell which step is the contract. Routed to `[REQUIREMENT_CONFLICT]`.
2. The fix for #53 starts the tunnel TLS handshake together with the IMPORT statement, as #53 proposes. It collides with the `http-transport` scenario and the architecture constraint recorded here, which put the TLS handshake inside a 30-second setup deadline that ends before any statement is sent. Routed to `[HIDDEN_DEPENDENCY]`.
3. A refactor makes `authenticate()` start a fresh deadline. Every planned test still passes, because no transport test measures the budget shared across `connect()` and `authenticate()`, so a slow TLS handshake followed by a slow login again takes up to twice the timeout. In the same release, a caller passes `Duration::MAX` to the new public `connect_with_timeout` and the driver panics. Routed to `[TRACEABILITY_GAP]` and `[COMPLETENESS_GAP]`.

## Intent Fidelity
[no objection — axis checked: #76 (TLS handshake and login under one deadline, step named in the error, 30 s default and 300 s maximum kept, docs and changelog) maps to tasks 1.1 to 7.3. #57 (bound on TCP connect, EXA handshake, and TLS handshake, plus the doc fix in the `export_to_callback` doc, the field doc, the builder doc, and `docs/import-export.md`) maps to tasks 8.1 to 10.2. #56 (terminated state in both transports, an error that names the cause and says reconnect, the assertion in `test_csv_export_explicit_timeout_terminates_connection`) maps to tasks 2.1, 4.3 to 4.5, 5.3, 6.4, and 7.2. No task edits `src/adbc/connection.rs` or `src/adbc_ffi.rs`. `Cargo.toml` stays at 0.18.0, and the changelog entries go under `## [Unreleased]`. The #53 root cause stays out. `connection-management` goes from 6 to 7 features, `session-and-lifecycle` from 10 to 9 scenarios, `http-transport` from 7 to 9, and `import-export` gains no feature. The fixed 30-second tunnel bound follows the brief recorded in decision-log.md § Interview.]

## Feasibility

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: decision-log.md § [9]; import-export/http-transport/spec.md § "Tunnel setup fails with the step named when the peer stops answering"; architecture.md § Constraints (the HTTP tunnel setup line)
- Issue: The plan puts the tunnel TLS handshake inside the 30-second setup deadline that ends before the IMPORT or EXPORT statement is sent. Issue #53's draft diagnosis says Exasol answers the tunnel ClientHello only after it reads the statement, and it proposes running the TLS handshake and the statement at the same time. If that fix lands, the TLS step leaves the pre-statement setup phase. The new scenario step "one deadline SHALL bound the TCP connect, the EXA handshake, and the TLS handshake together" and the architecture constraint then have to change with it. Decision [9] mentions #53 only as "now fails after 30 seconds instead of hanging" and records no follow-up for this interaction.
- Fix: Add a Consequences bullet to decision-log.md entry [9]: the #53 fix decides whether the TLS handshake stays under the tunnel setup deadline, and it amends the `import-export/http-transport` scenario and the architecture § Constraints line to match. Add the same note to the follow-up list that plan.md § Impact sends to the PR description.

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md tasks 1.1 and 6.1; plan.md § Verification § Checklist
- Issue: The plan does not surface three facts about the build and test matrix. (a) `tests/common/mod.rs` is compiled into five test targets, and CI runs `cargo clippy --all-targets --all-features -- -D warnings` (`.github/workflows/ci.yml` line 81). The task 6.1 helper is used by `integration_tests` and `websocket_integration_tests` only, so `driver_manager_tests`, `import_export_tests`, and `native_protocol_tests` report it as dead code. That is why the neighbouring helpers carry `#[allow(dead_code)]`. (b) Only the `websocket`-gated transport constructs `SetupStep::WebSocketHandshake`. The default-feature lib build (`cargo build` in CI line 49, and `cargo clippy --all-targets` as AGENTS.md § Code quality states it) therefore reports a variant that is never constructed. This is inferred from the rustc `dead_code` lint, not compiled. (c) The Checklist omits `cargo test --features ffi --test native_transport_smoke_test -- --test-threads=1`, which CI runs (line 279) and which exercises the native connect path that task 4.1 rewrites.
- Fix: In task 6.1, mark the helper `#[allow(dead_code)]` like the other shared helpers. In task 1.1, gate `SetupStep::WebSocketHandshake` and its `Display` arm with `#[cfg(feature = "websocket")]`, or state another way to keep the default build free of warnings. Add the native transport smoke test row to § Checklist.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: connection-management/session-and-lifecycle/spec.md § "Terminate a connection whose in-flight response is no longer trusted"
- Issue: Two steps of the changed scenario contradict each other for `close()`. One step says "every subsequent operation on that transport MUST fail instead of reading the abandoned response". A later step says "a graceful close of the terminated transport SHALL succeed without a protocol round-trip". `close()` is a subsequent operation, so no test can satisfy both steps as written. The step "the error of every such operation, including connect and authenticate" inherits the same unclear scope. The plan's tests already pick one reading: `every_operation_after_terminate_reports_the_terminated_transport` leaves `close` out, and `close_after_terminate_succeeds_and_keeps_reporting_termination` expects `Ok`. The scenario also has seven steps, and `speq plan validate` warns about its six AND steps.
- Fix: In the DELTA:CHANGED block, reword the step to "every subsequent operation on that transport except `close` MUST fail instead of reading the abandoned response". Optionally move the error-text step, the close step, and the closed-versus-terminated step into a new DELTA:NEW scenario in the same file, for example "Operations after an export timeout name the termination". That brings `session-and-lifecycle` to 10 scenarios, which is at the limit. If you split it, update the `/// Scenario:` lines in tasks 4.5, 5.4, and 6.4 and the § Scenario Coverage rows to match.
- Escalation: MECHANICAL (both steps sit in one scenario of this plan, and the plan's own tests show the intended reading)

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md task 1.1 (`SetupDeadline::start`); plan.md task 8.1 (`connect_with_timeout`)
- Issue: Task 1.1 sets the deadline to "now plus the budget". `tokio::time::Instant + Duration` panics on overflow. Today's `tokio::time::timeout` uses `checked_add` and falls back to a far-future deadline instead. With the plan as written, the new public `HttpTransportClient::connect_with_timeout(host, port, use_tls, Duration::MAX)` panics instead of meaning "no limit". The public `transport::ConnectionParams::with_timeout(u64)` has no upper bound either, and only `connection::ConnectionParams` enforces 300 seconds. Whether `u64::MAX` milliseconds overflows depends on the platform's `Instant` range.
- Fix: In task 1.1, state that `start` uses `Instant::checked_add` and falls back to a far-future deadline on overflow. Add a unit test in task 1.2, for example `a_budget_that_overflows_the_clock_does_not_panic`, that starts a deadline with `Duration::MAX` and runs a step that finishes.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md tasks 4.2 and 5.2
- Issue: The tasks run the login "with the deadline taken from `setup_deadline`". They do not say whether "taken" means `Option::take`, or what `authenticate()` does when the field is `None`. A transport can reach `authenticate()` twice: a rejected password returns an error before the state changes, so the state stays `Connected`. If the first call took the deadline, the retry finds `None`, and an `expect` there panics in library code.
- Fix: In task 4.2, state that `authenticate()` copies the stored deadline (it is `Copy`) and leaves the field set, so a retry gets only the time that remains. State the `None` case explicitly, for example as `TransportError::ProtocolError("Must connect before authenticating")`. Apply the same text to task 5.2.

## Task Breakdown

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Scenario Coverage, the three rows for "Connection timeout default and maximum"; plan.md § Implementation Tasks
- Issue: The scenario "Connection timeout default and maximum" in the new feature `connection-management/connection-timeout` has no implementing task. The plan cites three existing tests in `src/connection/params.rs`. None of them carries a `/// Scenario:` line, which AGENTS.md § Code style requires, and no task adds one. `test_builder_validation_timeout` asserts only `result.is_err()`, so the THEN step "rejected with `ConnectionError::InvalidParameter`" is not verified. `test_parse_connection_timeout_param` parses `connection_timeout=15` and checks neither the default nor the maximum.
- Fix: Add a task to group A for `src/connection/params.rs`. Add `/// Scenario: Connection timeout default and maximum` to `test_builder_default_values` and `test_builder_validation_timeout`. Make `test_builder_validation_timeout` assert `ConnectionError::InvalidParameter` with `parameter == "connection_timeout"`. Either drop `test_parse_connection_timeout_param` from § Scenario Coverage, or add a connection-string case in which `?timeout=301` is rejected, with the same Scenario line. Add `src/connection/params.rs` to the group A Knowledge column.
- Escalation: MECHANICAL (settled by reading the cited tests and AGENTS.md)

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md tasks 4.5 and 5.4; plan.md § Scenario Coverage, the rows for "One deadline bounds every connection setup step"
- Issue: Only the `SetupDeadline` unit tests check that time spent in one step is not granted again to a later step. The transport tests assert a lower bound ("at least 300 ms passed") and no upper bound. A transport that starts a fresh deadline in `authenticate()` therefore passes every planned test, although the shared budget across `connect()` and `authenticate()` is what #76 asks for ("The time spent in one step is then not given again to the next step").
- Fix: Add a test to task 4.5, for example `authenticate_gets_only_the_time_left_after_connect`. Use `timeout_ms` 1000 and `SilentServer::accepting()` with TLS off. Call `connect()`, wait 800 ms with `tokio::time::sleep`, and call `authenticate()`. Assert that the `(login)` error arrives less than 600 ms after the wait ends, where a fresh deadline would take 1000 ms. Give it the line `/// Scenario: One deadline bounds every connection setup step`. Add the same test to task 5.4 with `SilentServer::after_websocket_upgrade()`.

## Design Depth

Checked without objection: decision [1] is the only `Promotes to ADR: yes` entry. Its Rationale names rule-2 criteria 2 and 4 and states the `speq decision-log show` result. Its Decision holds no signature, path, or flag. Its `Architecture:` line names § Components, § Data Flow, and § Constraints, and the delta contains all three. The delta's BASE `3e25a7edb471df5f4aa3f61f698f9b0545e3bf28` matches the blob hash of `specs/architecture.md`, and each CHANGED block copies the current section with only the planned edits. No decision contradicts `server-enforced-query-timeout`, `client-give-up-terminates-connection` (a login timeout drops the socket), `export-timer-stays-opt-in` (the setup deadline is not the export timer), or `transport-terminate-no-round-trip` (`close()` still delegates its teardown to `terminate()`). `SetupDeadline` passes the Quick Diagnostic as decision [2] records. The rewrite in decision [3] matches tokio-tungstenite 0.28.0 `connect.rs` and `tls.rs`: TCP connect, then rustls `wrap_stream`, then `client_async_with_config`.

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: decision-log.md entry [7]; plan.md task 2.1; connection-management/session-and-lifecycle/spec.md § "Terminate a connection whose in-flight response is no longer trusted"
- Issue: The transport core now hardcodes which higher-level caller terminated the transport, because its error text names an export. The scenario's GIVEN stays generic ("the driver has given up on an in-flight request"), and the Background of the same spec requires any client-side give-up on a running query to terminate the connection. The first caller of `terminate()` outside export therefore makes the recorded THEN step false, not only the error text. Decision [7] records that the text must change in that case, but not that the scenario must change too.
- Fix: Put the export-specific error requirement in a scenario whose GIVEN is the export case, for example the split scenario proposed in the `[REQUIREMENT_CONFLICT]` finding with GIVEN "an export timeout terminated the transport before the EXPORT response was read". Keep the generic scenario for the socket, disconnect, and failure steps. Extend decision [7] Consequences: a new caller of `terminate()` revises both the error text and that scenario.

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: connection-management/connection-timeout/spec.md § Background and § "WebSocket server that never answers the upgrade"; plan.md tasks 7.1 and 7.3; architecture.md § Constraints
- Issue: One setup step has two names. The error text and `SetupStep` say "WebSocket handshake". The docs row, the changelog entry, the architecture constraint, the spec Background, and the scenario title say "WebSocket upgrade". A reader of `docs/setup-and-connect.md` then meets a step name in the error that the docs never use. Separately, the Background says "The caller sets the connection timeout in seconds with ... `ConnectionBuilder::connection_timeout()`", but that method takes a `Duration`.
- Fix: Use one name in every artifact. "WebSocket upgrade" needs edits only to the `SetupStep` Display text in task 1.1 and the expected messages in tasks 5.4 and 6.3, and it avoids a clash with `websocket-client/handshake`, whose handshake includes the login. In the Background, write "in seconds with the `timeout` or `connection_timeout` connection-string parameter, or as a `Duration` with `ConnectionBuilder::connection_timeout()`".
