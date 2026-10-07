# Decision Log: fix-transport-deadlines

## Interview

The first planning pass ran headless, with the orchestrator brief as its only input. The first five Q/A pairs summarize that brief. The next four Q/A pairs are the user's answers after plan review round 1. The last three Q/A pairs are the user's decisions in the second user review. Where an answer conflicts with an earlier one, the later answer applies.

**Q:** What is in scope?
**A:** Issues #76, #57, and #56 in one PR. The orchestrator verified all three claims against the code. Issue #53 is out of scope and gets a separate PR. No `Cargo.toml` version bump.

**Q:** What does #76 require?
**A:** `connection_timeout` (`params.timeout_ms`) wraps only `TcpStream::connect` in `NativeTcpTransport::connect`. The TLS handshake and `authenticate()` have no bound, and the same holds in `src/transport/websocket.rs`. Add one shared deadline for TCP connect, TLS handshake, and login. `src/adbc/connection.rs` and `src/adbc_ffi.rs` belong to another worktree and must not change, so the issue's proposal in `connect_with_transport` is not implemented. Each transport records a deadline (`Instant` plus `timeout_ms`) in `connect()` and applies the remaining budget to the TLS handshake and to `authenticate()`. The error text names the step, for example "Connection timeout after 3000ms (TLS handshake)" or "(login)". Keep the 30-second default and the 300-second maximum. Tests use a local server that accepts and never answers, with TLS on and off, and one that completes TLS and then stays silent at login. The tests fail fast under an outer timeout and never hang CI. `docs/setup-and-connect.md` states that the timeout covers TCP connect, TLS handshake, and login.

**Q:** What does #57 require?
**A:** `HttpTransportClient::connect` has no bound at the TCP connect, at `perform_handshake` (`read_exact`), or at the TLS handshake. Keep the `connect(host, port, use_tls)` signature so the callers in `src/export/csv.rs`, `src/import/csv.rs`, `src/import/parquet.rs`, and `src/import/parallel.rs` stay unchanged. Add `connect_with_timeout(host, port, use_tls, Duration)`. `connect()` uses a 30-second default constant. One deadline covers the three steps, and the error names the step. Plumbing the connection timeout into the tunnel needs `src/adbc/connection.rs`, so it is a follow-up for the PR and the decision log and is not implemented. Also fix the false "only client-side bound on the whole operation" sentence in the `export_to_callback` doc, the `CsvExportOptions::timeout_ms` field doc, and `docs/import-export.md` so they say that tunnel setup is bounded separately.

**Q:** What does #56 require?
**A:** `terminate()` leaves the state `Closed`, so the guards report "Must authenticate before executing queries". Add a terminated state to the native transport and the same to the WebSocket transport, so the pre-flight guards return an error that names the cause: the transport was terminated after an export gave up on an in-flight response, and the caller must reopen the connection. Prefer a small helper over editing about ten guards by hand. Update `test_csv_export_explicit_timeout_terminates_connection` in `tests/integration_tests.rs` to assert the message.

**Q:** What about the changelog and the version?
**A:** Add concise user-facing entries under `## [Unreleased]` in `CHANGELOG.md` that name the new timeouts and the new error messages. Do not bump the version.

**Q:** Which spec limits apply?
**A:** At most 10 scenarios per feature and 8 features per domain. `import-export` already has 14 features, so it gets no new feature. Scenarios go into existing features under the limit. `connection-management` has 6 features, so a new feature is allowed there.

**Q:** Which name does the WebSocket setup step use?
**A:** "WebSocket upgrade" everywhere: the `SetupStep` Display text in task 1.1, the expected messages in tasks 5.3 and 6.3, the specs, the docs, the changelog, and the architecture. The Background states that the connection timeout is set in seconds with the `timeout` or `connection_timeout` connection-string parameter, or as a `Duration` with `ConnectionBuilder::connection_timeout()`.

**Q:** Does the terminated-transport error stay export-specific?
**A:** Yes. Decision [7] Consequences states that a new caller of `terminate()` revises both the error text and the export scenario.

**Q:** Does a login timeout keep surfacing as `ConnectionError::AuthenticationFailed`?
**A:** Yes, as a known limitation. Decision [5] stays unchanged, and `src/adbc/connection.rs` stays out of scope.

**Q:** Does HTTP tunnel setup keep the fixed 30-second bound?
**A:** No. Tie tunnel setup to the connection timeout in this plan. Work out from the code how import and export callers can obtain the connection timeout without editing `src/adbc/connection.rs` or `src/adbc_ffi.rs`. If that is impossible, say so in the decision log, pick the smallest approach, and flag it as an open question. The message becomes `HTTP tunnel setup timeout after <connection timeout ms>ms (<step>)`, and `connect_with_timeout` stays. Record the #53 interaction as a consequence of decision [9] and in the follow-up list for the PR description. (The last Q/A pair below replaces this answer.)

**Q:** Does this plan change query timeouts?
**A:** No. Query timeouts stay either user-defined or absent, and the database owns them. State this in plan.md § Context and § Impact, and keep every task away from query-timeout behavior: `set_query_timeout` gets only the shared state guard. A connection timeout that covers setup (TCP, TLS, WebSocket upgrade, login, tunnel setup) is fine.

**Q:** May the native and WebSocket tasks state the same rules twice?
**A:** No. The Terminated state, the state guard, the `close()` rule, and the login-with-deadline-and-teardown rule move into shared transport-core code in `src/transport/`, next to `deadline.rs`. Check both transports for what is identical and what must stay per transport, and invent no abstraction beyond the shared duplicated logic. `ConnectionState` keeps its existing variants, and the public API stays unchanged. Tests: one set of shared unit tests plus a thin check per transport.

**Q:** Does tunnel setup follow the connection timeout?
**A:** No. This answer replaces the earlier answer about the tunnel bound. Tunnel setup uses one fixed 30-second bound for imports and exports. Remove `TransportProtocol::connection_timeout()`, the task parts that exist only for it, and the transports' recording of the timeout for the tunnel. Collapse the two `http-transport` tunnel scenarios back into "Tunnel setup is bounded by 30 seconds by default". Keep `connect_with_timeout(host, port, use_tls, Duration)`. Decision [9] keeps two consequences: the 30 seconds do not follow the connection timeout yet, and the #53 fix decides whether the TLS handshake stays under the deadline. The PR description lists three follow-ups: the login error mapping, the tunnel bound following the connection timeout, and #53.

## Design Decisions

### [1] The transport owns the connection-setup deadline

- **Decision:** The connection timeout is one deadline that the transport starts in `connect()` and that also bounds `authenticate()`. The transport names the step that was running when the deadline elapsed. The Connection does not wrap `connect()` and `authenticate()` in a timer of its own.
- **Alternatives:** (a) `Connection::connect_with_transport` computes the deadline and wraps both calls in `tokio::time::timeout_at`, as issue #76 proposes. Rejected: the Connection sees only two calls, so it cannot name the TLS handshake or the WebSocket upgrade. A dropped `authenticate()` future also leaves the transport in the `Connected` state with a half-read socket, and every caller would have to remember to tear it down. (b) An independent timeout per step. Rejected: a stalled setup could then take up to three or four times the configured value.
- **Rationale:** /speq:adr-rules rule-2 criterion 2: the rule binds every `TransportProtocol` implementation, because `connect()` must start the deadline and `authenticate()` must spend only what remains. Criterion 4: it rejects the issue's own proposal, and a later reader who changes `connect_with_transport` needs the reason. `speq decision-log show` lists no ADR about the connection timeout. `server-enforced-query-timeout` bans client-side timers in query execution only, and connection setup runs no query. Independently of the file-ownership limit in this plan, only the transport knows which setup step is running.
- **Consequences:** The `TransportProtocol::connect` and `TransportProtocol::authenticate` doc comments state the shared deadline. A transport that calls `authenticate()` long after `connect()` has less time for login. A login that runs out of time closes the transport's socket, see entry [4]. Query execution keeps its server-enforced timeout.
- **Architecture:** § Components, § Data Flow, § Constraints
- **Promotes to ADR:** yes

### [2] One shared setup-deadline module serves both transports and the tunnel

- **Decision:** A crate-private module `src/transport/deadline.rs` holds `SetupDeadline` and the `SetupStep` vocabulary. `SetupDeadline::start(label, budget)` fixes the deadline. `run(step, future)` awaits the future with `tokio::time::timeout_at` and, on elapse, returns `TransportError::IoError("<label> timeout after <budget>ms (<step>)")`. `has_elapsed()` tells a caller whether a failed step ran out of time. The shared connection lifecycle (entry [6]) uses it with the label `Connection` for both transports, and `HttpTransportClient` uses it with the label `HTTP tunnel setup`.
- **Alternatives:** (a) Inline `tokio::time::timeout_at` calls and `format!` error texts in each of the three modules. Rejected: the step names and the error format are user-visible text, and three copies drift apart. (b) A `TransportError::Timeout` variant. Rejected, see entry [13].
- **Rationale:** Quick Diagnostic for the new module. One sentence: it bounds a sequence of async steps with one shared budget and names the step that ran out. Calling it is easier than reimplementing it: callers do not compute remaining time or format errors. Changing its internals forces no edit elsewhere, because callers pass only a label, a step, and a future. Its doc comment states why the step is named inside the module that runs the steps. It is the one owner of the step vocabulary and the timeout message format. Its boundary is one struct and one enum. It takes no shortcut that needs a follow-up. It depends only on `tokio` time and `TransportError`, which the transport layer already uses.
- **Consequences:** `src/transport/mod.rs` declares the module unconditionally, because `http_transport` is built without either transport feature. `SetupDeadline` is `Copy`, so `ConnectionLifecycle` stores it between `connect()` and `authenticate()`.
- **Promotes to ADR:** no

### [3] The WebSocket transport runs TCP connect, TLS handshake, and WebSocket upgrade as separate steps

- **Decision:** `WebSocketTransport::connect` stops calling `connect_async_tls_with_config`. It opens the TCP connection with `TcpStream::connect((host, port))`, runs the TLS handshake through the shared TLS step in `src/transport/tls.rs` (entry [6]), which builds the same rustls client configuration as today, wraps the result in `MaybeTlsStream::Rustls` or `MaybeTlsStream::Plain`, and runs the upgrade with `tokio_tungstenite::client_async_with_config`. Each call runs as its own step under the one deadline. The WebSocket configuration keeps unlimited frame and message sizes, and Nagle's algorithm stays enabled, as today.
- **Alternatives:** Keep `connect_async_tls_with_config` under one step. Rejected: that future runs three steps, so the error could not say whether the TLS handshake or the upgrade stalled.
- **Rationale:** The brief asks for the step name in the error. `connect_async_tls_with_config` performs these same three calls internally (tokio-tungstenite 0.28.0 `connect.rs` and `tls.rs`), and `MaybeTlsStream::Rustls` holds the `tokio_rustls` 0.26 stream that the crate already depends on, so the stream type `WebSocketStream<MaybeTlsStream<TcpStream>>` does not change.
- **Consequences:** The `Connector` import goes away. A TLS-enabled server that never answers fails with `(TLS handshake)`, and a plain server that never answers the upgrade fails with `(WebSocket upgrade)`.
- **Promotes to ADR:** no

### [4] A login that runs out of time closes the socket and leaves the transport closed

- **Decision:** When the login step fails and the deadline has elapsed, the shared login function (entry [6]) runs the shared terminate step, which drops the socket and the session, and then records `Closed` before it returns the timeout error. The state does not stay `Terminated`.
- **Alternatives:** (a) Keep the socket and the `Connected` state. Rejected: the dropped login future leaves unread bytes on the socket, so a retried `authenticate()` would read the wrong response. (b) Leave the state `Terminated`. Rejected: the terminated error names an export as the cause, which would be false here.
- **Rationale:** ADR `client-give-up-terminates-connection` requires a client-side give-up to release the connection rather than abandon an in-flight request. `connect_with_transport` drops the transport after any authenticate error, so this changes only what a direct `TransportProtocol` caller sees.
- **Promotes to ADR:** no

### [5] A login timeout surfaces as `ConnectionError::AuthenticationFailed` for now

- **Decision:** This plan leaves the error mapping in `connect_with_transport` unchanged. A login timeout reaches the caller as `ConnectionError::AuthenticationFailed("Network I/O error: Connection timeout after <N>ms (login)")`. A timeout in the TCP connect, the TLS handshake, or the WebSocket upgrade reaches the caller as `ConnectionError::ConnectionFailed`. The follow-up maps a login timeout to `ConnectionError::Timeout` or `ConnectionError::ConnectionFailed` in `src/adbc/connection.rs`.
- **Alternatives:** Change the mapping in `src/adbc/connection.rs` now. Rejected: another worktree owns that file.
- **Rationale:** The error message names the timeout and the step, which is what issue #76 asks for. The variant is a known inaccuracy, recorded in plan.md § Impact and in the PR description.
- **Consequences:** The PR description lists the follow-up, together with the two follow-ups of entry [9].
- **Promotes to ADR:** no

### [6] One shared connection lifecycle and one shared TLS step serve both transports

- **Decision:** A crate-private module `src/transport/lifecycle.rs` owns the state rules that both transports repeat today. It holds one `ConnectionState` enum, which replaces the two identical private enums, keeps the variants `Disconnected`, `Connected`, `Authenticated`, and `Closed`, and adds `Terminated`. `ConnectionLifecycle` holds the state and the stored setup deadline. Its `require(expected, message)` guard returns `Ok` for the expected state, the terminated error for `Terminated`, and `TransportError::ProtocolError(message)` otherwise, and every pre-flight guard of both transports calls it with the current message. Its `begin_connect` and `connected` methods start and store the connection deadline. A crate-private trait `LifecycleSteps` names the four steps that differ per transport: access to the lifecycle, the login exchange, the disconnect I/O, and the release of the stream and the session. Three generic functions over that trait run the shared rules: `terminate`, `authenticate_within_deadline` (entry [4]), and `close_gracefully` (entry [8]). Only these functions set `Authenticated`, `Closed`, and `Terminated`. The TLS step, which both transports run identically, moves into `src/transport/tls.rs` next to the verifiers it selects: `client_config` builds the rustls client configuration from the certificate fingerprint and the validation setting, and `client_handshake` runs the TLS handshake on a TCP stream and maps a failure to `TransportError::TlsError`. Each transport runs `client_handshake` under its connection deadline and wraps the resulting stream itself. The TCP connect with its socket options, the WebSocket upgrade, the stream types, and the protocol messages stay in each transport.
- **Alternatives:** (a) Have `Connection` check `is_closed()` before every dispatch, as issue #56 also suggests. Rejected: `src/adbc/connection.rs` is out of bounds, and a direct `TransportProtocol` caller would still see the authentication message. (b) A `TransportError::Terminated` variant. Rejected, see entry [13]. (c) One `require_state` helper and one copy of the close and login rules in each transport. Rejected by the user: the plan and the code stated each rule twice, and two copies drift apart. (d) A shared base transport that also owns the stream, the TCP connect, and the WebSocket upgrade. Rejected: the stream types, the socket options, the upgrade, the login exchange, and the disconnect I/O differ per transport. (e) Closures instead of a trait for the per-transport steps. Rejected: the login future borrows the transport mutably while the shared function also needs the lifecycle and the release step afterwards, and trait methods express that sequence without boxed higher-ranked closures. (f) Keep the TLS configuration and handshake in each transport. Rejected: the two blocks differ only in how they wrap the TLS stream, so every TLS change, such as a custom root store, would need two edits, and the user's rule of no duplicated code between the transports covers this step. (g) Share the TLS step with the HTTP tunnel as well. Rejected: the tunnel's TLS client presents a client certificate, uses the fixed server name `exasol`, and reports its own error text.
- **Rationale:** Quick Diagnostic for the new module. One sentence: it owns the connection state of a transport and the rules for moving between states. Calling it is easier than reimplementing it: a transport supplies four small steps and delegates `authenticate()`, `close()`, `terminate()`, `is_connected()`, and its guards. Changing a state rule forces no edit in the transports. The doc comments state that the module owns the state rules and that the transports supply only the steps. Each state rule has one owner, and private transitions keep `Terminated` reachable only through `terminate`. The boundary is one enum, one struct, one trait, and three functions. It takes no shortcut that needs a follow-up. It depends inward only, on `deadline.rs`, `Credentials`, `SessionInfo`, and `TransportError`. The TLS step adds two functions to an existing module and no new boundary: `tls.rs` already owns the verifiers, so it also owns the choice between them. The callers keep the deadline, the socket options, and the stream wrapping, so `tls.rs` stays independent of `deadline.rs` and of the stream types.
- **Consequences:** No public type changes, because both old enums were private. On the WebSocket transport, a TLS failure that is not a timeout returns `TransportError::TlsError` instead of `TransportError::WebSocketError`, as on the native transport, and task 7.3 adds a `Changed:` changelog line for it. `tls.rs` now depends on `error`. The module is declared only when the `native` or the `websocket` feature is on. The existing transport unit tests that read or write `state` use `ConnectionLifecycle::state()` and the test constructor `ConnectionLifecycle::in_state`. One set of unit tests against a test double covers the shared rules, and each transport keeps one login test against a loopback server and one terminated check.
- **Promotes to ADR:** no

### [7] The terminated error names the export and says "reconnect"

- **Decision:** `src/transport/lifecycle.rs` holds a private function that returns `TransportError::ProtocolError("Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation")`. `ConnectionLifecycle::require` is its only caller, so both transports return the same text.
- **Alternatives:** (a) "reopen the connection", the wording of the brief. Rejected in favour of "reconnect before the next operation", which `ExportError::Timeout` and `docs/import-export.md` already use for the same remedy. (b) A generic text without the export. Rejected: issue #56 asks the error to name the cause, and the export timeout in `export_to_callback` is the only caller of `terminate()` apart from `close()`.
- **Rationale:** One function in the module that owns the state keeps one text for both transports. The caller sees "Query execution failed: Protocol error: Transport was terminated after ..." at the `Connection` level.
- **Consequences:** A future caller of `terminate()`, such as a cancel implementation, must revise both this error text and the `session-and-lifecycle` scenario "Operations after an export timeout name the termination", whose GIVEN and THEN name the export as the cause, because the transport then has more than one cause of termination. Until then the text stays export-specific, as the user decided after review round 1.
- **Promotes to ADR:** no

### [8] `close()` keeps delegating its teardown to `terminate()` and then records `Closed`

- **Decision:** The shared function `close_gracefully` returns `Ok` without I/O when the state is `Disconnected`, `Closed`, or `Terminated`. Otherwise it runs the transport's disconnect step, then the shared `terminate` function, which `TransportProtocol::terminate()` of each transport also runs, and then records `Closed`. Each transport's `close()` delegates to it.
- **Alternatives:** (a) A private teardown helper that takes the target state, called by both methods. Rejected: it changes the delegation that ADR `transport-terminate-no-round-trip` records in its Consequences, for no gain. (b) Keep the close sequence in each transport. Rejected, see entry [6] alternative (c).
- **Rationale:** The ADR states that `close()` delegates its teardown to `terminate()`. The shared function keeps that order for both transports. Without the final state assignment, a graceful close would report itself as terminated and blame an export. Returning early on `Terminated` keeps the termination cause after a later `close()`, which `test_csv_export_explicit_timeout_terminates_connection` performs.
- **Promotes to ADR:** no

### [9] Tunnel setup has a fixed 30-second bound for imports and exports, and `connect_with_timeout` takes an explicit bound

- **Decision:** One `SetupDeadline` labelled `HTTP tunnel setup` bounds the TCP connect, the EXA handshake, and the TLS handshake of a tunnel. `HttpTransportClient::connect_with_timeout(host, port, use_tls, setup_timeout)` takes the bound. `HttpTransportClient::connect` keeps its signature and applies a private 30-second constant, `TUNNEL_SETUP_TIMEOUT`. Every import and export keeps calling `connect`, so every tunnel setup has the same 30-second bound. Certificate generation stays between the EXA handshake and the TLS handshake, outside any step. `perform_handshake` and every caller stay unchanged.
- **Alternatives:** (a) Arm the export timer before tunnel setup. Rejected: ADR `export-timer-stays-opt-in` keeps that timer unset by default, so setup would stay unbounded by default, and imports have no timer at all. (b) Bound an export's tunnel by the connection timeout through a new provided method `TransportProtocol::connection_timeout()`, and give imports the 30-second default. Rejected by the user: it adds a trait method and treats imports and exports differently, while imports still cannot obtain the connection timeout without a change to `src/adbc/connection.rs`. (c) A setup-timeout field in every import and export options struct. Rejected: `Connection` fills in only the host and the port of those options, so no `Connection` caller would get the connection timeout, and the field adds public API that has no effect until `src/adbc/connection.rs` sets it. (d) A constant shared with the 30-second default of `ConnectionParams`. Rejected: `http_transport` is the only user of the bound, so a private constant keeps the value in its one owner, and the follow-up replaces it with the configured connection timeout.
- **Rationale:** The fixed bound turns every stalled tunnel setup into an error with the smallest change, and it covers imports and exports at once. `connect_with_timeout` is the entry point that the follow-up and the tests use.
- **Consequences:** The 30 seconds do not follow the connection timeout yet. Follow-up for the PR description: pass the configured connection timeout from `src/adbc/connection.rs` to `connect_with_timeout` for every import and export. That follow-up amends every text that states the 30 seconds: the `import-export/http-transport` scenario "Tunnel setup is bounded by 30 seconds by default", the `http_transport` line of `specs/architecture.md` § Components, the tunnel line of § Constraints, and the doc texts of tasks 7.1, 9.1, and 10.1. Follow-up for the PR description: the fix for #53 decides whether the tunnel TLS handshake stays under the tunnel setup deadline, because #53's draft diagnosis proposes to run that handshake together with the IMPORT statement; that fix amends the `import-export/http-transport` scenario "Tunnel setup fails with the step named when the peer stops answering" and the tunnel line of `specs/architecture.md` § Constraints to match. A stalled tunnel TLS handshake, as in #53, now fails after 30 seconds instead of hanging, and the root cause of #53 stays out of scope. `ExportError::Timeout` keeps its meaning, because the tunnel setup error is `ExportError::HttpTransportError` and is raised before any request is sent.
- **Promotes to ADR:** no

### [10] Spec organization

- **Decision:** A new feature `connection-management/connection-timeout` holds the connection-timeout scenarios. The scenario "Connection timeout" moves out of `connection-management/session-and-lifecycle`, whose Background points to the new feature. The tunnel bound adds two scenarios to `import-export/http-transport` (7 to 9). `import-export/csv-export-timeout` changes its description, Background, and the scenario "No client-side export timeout by default", which said that no client-side timer wraps the export at all.
- **Alternatives:** (a) Keep "Connection timeout" in `session-and-lifecycle` and change it in place. Rejected: the feature has 10 scenarios, the limit, so the step-specific cases do not fit. (b) Put the step cases into `native-client/handshake` and `websocket-client/handshake`. Rejected: the connection timeout is one cross-transport contract, and splitting it repeats the shared deadline rule in two features.
- **Rationale:** `connection-management` goes from 6 to 7 features. `session-and-lifecycle` stays at 10 scenarios: it loses "Connection timeout" and gains "Operations after an export timeout name the termination" (Review Findings entry [1]). `import-export` keeps 14 features. No test carries the `/// Scenario: Connection timeout` line, so no test doc line changes.
- **Promotes to ADR:** no

### [11] Tests run against loopback fakes and test doubles in the library, plus public-API tests without Exasol

- **Decision:** The shared lifecycle rules are tested once in `src/transport/lifecycle.rs` against a test double with `#[tokio::test(start_paused = true)]` and no I/O. Each transport and the tunnel get library unit tests that connect to loopback fakes in `src/transport/test_support.rs`: a server that accepts and never writes, one that completes TLS and then stays silent, and, with the `websocket` feature, one that completes the WebSocket upgrade and then stays silent. Each loopback test runs under an outer `tokio::time::timeout` of 10 seconds that panics, with a setup bound of 300 milliseconds. `tests/integration_tests.rs` and `tests/websocket_integration_tests.rs` add tests that open a connection through `Connection::from_params` with `timeout=1` against a silent loopback server. These tests need no Exasol and do not call `skip_if_no_exasol!`. The 30-second tunnel bound is tested through `export_to_callback` and `import_from_callback` with the clock paused only after the fake server has received the magic packet, the pattern of `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset`. `SetupDeadline` is tested with `#[tokio::test(start_paused = true)]` and no I/O.
- **Alternatives:** (a) Public-API tests only. Rejected: SonarQube Cloud reads only the unit-test coverage, so the new transport code needs library tests. (b) Pause the clock before any socket I/O. Rejected: tokio advances a paused clock whenever the runtime is idle, so the deadline could fire during the loopback connect and name the wrong step.
- **Rationale:** The double tests cover every shared state rule once. The loopback tests cover every connect step on both transports and show that each transport's login and release steps act on the real socket. The public-API tests reproduce issue #76 through the connection string, including the mapping in `connect_with_transport`.
- **Promotes to ADR:** no

### [12] Changelog and version

- **Decision:** `CHANGELOG.md` gets a `## [Unreleased]` section above `## 0.18.0`, with `Fix:` entries for #76, #57, and #56, a `Changed:` entry for the TLS error of the WebSocket transport (entry [6]), and an `Added:` entry for `HttpTransportClient::connect_with_timeout`. `Cargo.toml` keeps its version.
- **Alternatives:** none.
- **Rationale:** The brief forbids a version bump, and `AGENTS.md` § Changelog puts entries of a PR without a bump under `## [Unreleased]`.
- **Promotes to ADR:** no

### [13] Timeouts and the terminated error keep the existing `TransportError` variants

- **Decision:** Setup timeouts use `TransportError::IoError`, as the existing TCP connect timeout does. The terminated error uses `TransportError::ProtocolError`, as the existing guards do. `TransportError` gets no new variant.
- **Alternatives:** New `Timeout` and `Terminated` variants. Rejected: `TransportError` is public and not `#[non_exhaustive]`, so a new variant breaks external exhaustive matches, and `connect_with_transport` would still flatten it into a string.
- **Rationale:** The change stays non-breaking, and the message carries the information the issues ask for.
- **Promotes to ADR:** no

### [14] The #57 doc fix also covers the `timeout_ms()` builder doc

- **Decision:** Besides the `export_to_callback` module doc, the `CsvExportOptions::timeout_ms` field doc, and `docs/import-export.md`, the builder method `CsvExportOptions::timeout_ms()` changes from "Bounds the whole export with a client-side timer" to a text that names SQL execution, tunnel transfer, and the callback.
- **Alternatives:** Leave the builder doc. Rejected: it makes the same false claim that issue #57 reports.
- **Rationale:** The claim is the same defect in a fourth place.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] The terminate scenario contradicted itself about `close`

- **Finding:** The changed scenario "Terminate a connection whose in-flight response is no longer trusted" required every subsequent operation to fail and also required a graceful close to succeed. `close` is a subsequent operation, so no test could satisfy both steps. The scenario had six AND steps.
- **Direction change:** The step now reads "every subsequent operation on that transport except `close` MUST fail instead of reading the abandoned response, and `close` SHALL succeed without a protocol round-trip", and the scenario keeps three AND steps. The export-specific error text, the rule that a closed transport keeps reporting the termination, and the closed-versus-terminated rule move into the new scenario "Operations after an export timeout name the termination", whose GIVEN is the export case. `session-and-lifecycle` stays at 10 scenarios. The `/// Scenario:` lines of the tests and the § Scenario Coverage rows follow the split. Decision [7] Consequences now names the new scenario as well.
- **Promotes to ADR:** no

### [2] [plan-review] "Connection timeout default and maximum" had no implementing task

- **Finding:** The scenario cited three existing tests in `src/connection/params.rs`, none of which carries a `/// Scenario:` line. `test_builder_validation_timeout` checked only `is_err()`, not `ConnectionError::InvalidParameter`, and `test_parse_connection_timeout_param` checked neither the default nor the maximum.
- **Direction change:** New task 6.5 adds the Scenario line to `test_builder_default_values` and `test_builder_validation_timeout`, makes the latter assert `ConnectionError::InvalidParameter` with `parameter == "connection_timeout"`, and adds `test_parse_connection_timeout_default_and_maximum` for the connection-string path (default 30 seconds, 300 accepted, 301 rejected). § Scenario Coverage replaces `test_parse_connection_timeout_param` with the new test, and the group A Knowledge column lists `src/connection/params.rs`.
- **Promotes to ADR:** no

### [3] [plan-review] One setup step had two names

- **Finding:** The error text and `SetupStep` said "WebSocket handshake", while the docs row, the changelog, the architecture, the spec Background, and the scenario title said "WebSocket upgrade". The Background also said that `ConnectionBuilder::connection_timeout()` takes seconds, but it takes a `Duration`.
- **Direction change:** "WebSocket upgrade" is the one name. The variant is `SetupStep::WebSocketUpgrade` with the `Display` text `WebSocket upgrade` (task 1.1). Tasks 5.1, 5.3, and 6.3, § Impact, § Scenario Coverage, § Manual Testing, decision [3], and the `connection-management/connection-timeout` scenarios use it. The Background now reads "in seconds with the `timeout` or `connection_timeout` connection-string parameter, or as a `Duration` with `ConnectionBuilder::connection_timeout()`".
- **Promotes to ADR:** no

### [4] [plan-review] The terminated error names the export inside the transport core

- **Finding:** The transport core hardcodes which caller terminated the transport, so the first caller of `terminate()` outside export makes both the error text and the recorded scenario false.
- **Direction change:** The text stays export-specific, as the user decided. The export case already has its own scenario, "Operations after an export timeout name the termination" (entry [1]). Decision [7] Consequences states that a new caller of `terminate()` revises both the error text and that scenario.
- **Promotes to ADR:** no

### [5] [plan-review] The fix for #53 interacts with the tunnel setup deadline

- **Finding:** Issue #53's draft diagnosis proposes to run the tunnel TLS handshake together with the IMPORT statement. That would move the TLS step out of the setup phase that the `http-transport` scenario and the architecture constraint put under the tunnel setup deadline, and decision [9] recorded no follow-up for it.
- **Direction change:** Decision [9] Consequences and the follow-up list in plan.md § Impact state that the #53 fix decides whether the TLS handshake stays under the tunnel setup deadline, and that it amends the scenario "Tunnel setup fails with the step named when the peer stops answering" and the tunnel line of § Constraints to match.
- **Promotes to ADR:** no

### [6] [plan-review] The build and test matrix had three unstated facts

- **Finding:** The task 6.1 helper is dead code in three of the five test targets that compile `tests/common/mod.rs`. Only the `websocket`-gated transport constructs the WebSocket step variant, so the default build reports it as never constructed. § Checklist omitted the native transport smoke test that CI runs.
- **Direction change:** Task 6.1 marks the helper `#[allow(dead_code)]`. Task 1.1 gates `SetupStep::WebSocketUpgrade` and its `Display` arm with `#[cfg(feature = "websocket")]`. § Checklist gains the `native_transport_smoke_test` row.
- **Promotes to ADR:** no

### [7] [plan-review] A setup budget that overflows the clock panicked

- **Finding:** `SetupDeadline::start` added the budget to the current `Instant`, which panics on overflow, so `connect_with_timeout` with `Duration::MAX` would panic instead of meaning "no practical limit".
- **Direction change:** Task 1.1 uses `Instant::checked_add` and falls back to a far-future deadline about 30 years ahead, as `tokio::time::timeout` does. Task 1.2 adds `a_budget_that_overflows_the_clock_does_not_panic`.
- **Promotes to ADR:** no

### [8] [plan-review] `authenticate()` left the stored deadline handling open

- **Finding:** The login tasks did not say whether `authenticate()` takes the deadline out of its field or what it does when the field is empty. A retried `authenticate()` after a rejected password could then panic.
- **Direction change:** `authenticate()` copies the stored deadline and leaves it stored, so a retry gets only the time that remains. An empty field returns `TransportError::ProtocolError("Must connect before authenticating")`. The shared function `authenticate_within_deadline` implements both rules (task 2.2).
- **Promotes to ADR:** no

### [9] [plan-review] No transport test measured the budget that connect and login share

- **Finding:** The transport tests asserted only a lower bound on the elapsed time, so a transport that started a fresh deadline in `authenticate()` passed every planned test.
- **Direction change:** `a_login_gets_only_the_time_left_after_connect` measures the shared budget: 1000 ms, a wait of 800 ms after `connected`, and the `(login)` error 1000 ms after `begin_connect`. It carries `/// Scenario: One deadline bounds every connection setup step` and runs once against the shared lifecycle (task 2.4), which both transports use for `authenticate()`.
- **Promotes to ADR:** no

### [10] [user-review] HTTP tunnel setup follows the connection timeout

- **Finding:** The user rejected the fixed 30-second tunnel bound and asked to tie tunnel setup to the connection timeout in this plan, without edits to `src/adbc/connection.rs` or `src/adbc_ffi.rs`.
- **Direction change:** Decision [9] bounded an export's tunnel setup by the connection timeout that its transport reported through a new provided method `TransportProtocol::connection_timeout()`, and imports used the 30-second default. Review Findings entry [13] reverses this change.
- **Promotes to ADR:** no

### [11] [user-review] Query timeouts stay unchanged

- **Finding:** The user stated that this plan does not modify query timeouts: a query timeout stays either user-defined or absent, and the database owns it.
- **Direction change:** plan.md § Summary, § Context, and § Impact state that query timeouts do not change and that the connection timeout and the tunnel setup deadline bound setup steps only. Tasks 4.2 and 5.2 give `set_query_timeout` only the shared state guard, and task 2.3 keeps its doc comment and behavior. Task 7.1 states in `docs/setup-and-connect.md` that the connection timeout does not bound query execution. The `connection-management/connection-timeout` description states that query execution has its own timeout. Decision [1] Consequences notes that query execution keeps its server-enforced timeout.
- **Promotes to ADR:** no

### [12] [user-review] Shared transport-core code replaces the per-transport copies of the state rules

- **Finding:** The native and WebSocket tasks each stated the Terminated state, the `require_state` guard, the `close()` rule, and the login-with-deadline-and-teardown rule, so the plan and the code would hold each rule twice.
- **Direction change:** New module `src/transport/lifecycle.rs` (tasks 2.1, 2.2, and 2.4) holds `ConnectionState` with `Terminated`, `ConnectionLifecycle` with the `require` guard and the stored deadline, the trait `LifecycleSteps`, and the shared functions `terminate`, `authenticate_within_deadline`, and `close_gracefully`. The terminated error moves from `src/transport/protocol.rs` into that module. Tasks 4.2 and 5.2 replace each transport's private enum and `state` field, supply the four transport-specific steps, and delegate. The connect tasks 4.1 and 5.1 use `begin_connect` and `connected`, so the `Connection` label has one owner. One set of shared unit tests (task 2.4) replaces the per-transport rule tests, and each transport keeps a TLS handshake test, one login test against a loopback server, and one terminated check (tasks 4.3 and 5.3). Decisions [2], [4], [6], [7], [8], and [11], § Scenario Coverage, § Parallelization, § Dead Code Removal, and the architecture delta § Components follow. `ConnectionState` keeps its four existing variants, and no public type changes.
- **Promotes to ADR:** no

### [13] [user-review] Tunnel setup returns to one fixed 30-second bound

- **Finding:** The user reversed Review Findings entry [10]: tunnel setup uses one fixed 30-second bound for imports and exports.
- **Direction change:** `TransportProtocol::connection_timeout()`, `DEFAULT_CONNECTION_TIMEOUT`, the transports' `connection_timeout` fields, and their tests are removed from the plan. Task 8.1 delegates `HttpTransportClient::connect` with a private `TUNNEL_SETUP_TIMEOUT` of 30 seconds and keeps `connect_with_timeout(host, port, use_tls, Duration)`. Task 9.1 changes only docs. Tasks 9.2 and 9.4 test the 30-second bound for an export and an import on a paused clock. In `import-export/http-transport`, the scenario "Tunnel setup is bounded by 30 seconds by default" replaces the export and import scenarios. The `csv-export-timeout` Background, the architecture § Components, § Data Flow, and § Constraints, plan.md § Summary, § Context, and § Impact with its three follow-ups, tasks 7.1, 10.1, and 10.2, and decisions [9], [10], [11], and [12] follow.
- **Promotes to ADR:** no

### [14] [plan-review] Both transports kept identical copies of the TLS configuration and handshake

- **Finding:** `src/transport/native/mod.rs:790-820` and `src/transport/websocket.rs:421-448` build the same rustls client configuration with the same three verifier branches, and task 5.1 wrote a second copy of the native TLS handshake. Every later TLS change would then need two edits. The § Context bullet stated that the connect steps differ per transport, although the TLS step is identical. The user's standing rule of no duplicated code between the native and WebSocket transports confirms that the rule covers this step.
- **Direction change:** New task 3.2 moves the TLS step into `src/transport/tls.rs` as the crate-private functions `client_config` and `client_handshake`, and task 3.3 adds their regression tests against `SilentServer::after_tls()`, which task 3.1 extends with `certificate_der()`. Tasks 4.1 and 5.1 call `client_handshake` as `SetupStep::TlsHandshake` and wrap the result in `NativeStream::Tls` or `MaybeTlsStream::Rustls`. On the WebSocket transport, a TLS failure that is not a timeout now returns `TransportError::TlsError`, which § Impact and a `Changed:` line in task 7.3 state. § Summary, § Context, § Dead Code Removal, § Scenario Coverage, § Parallelization, decisions [3] and [6], and the `tls` line of the architecture delta § Components follow.
- **Promotes to ADR:** no

### [15] [plan-review] A lost line break merged two architecture constraints

- **Finding:** In the architecture delta § Constraints, the constraint "The native transport fetches up to the server's maximum data message size per batch, with a 64 MiB fallback" was appended to the end of the tunnel constraint instead of standing as its own bullet. Recording the delta would have removed it as a bullet.
- **Direction change:** The tunnel bullet ends after "names the step that ran out", and the native fetch constraint is its own bullet again. Apart from the two new constraints, § Constraints matches `specs/architecture.md`.
- **Promotes to ADR:** no

### [16] [plan-review] The follow-up for the tunnel bound named one of the places that state 30 seconds

- **Finding:** The value 30 seconds appears in the `http-transport` scenario, the `csv-export-timeout` Background, the architecture § Components and § Constraints, and the doc texts of tasks 7.1, 9.1, and 10.1. The follow-up that ties the tunnel bound to the connection timeout named only the `http-transport` scenario. No `csv-export-timeout` scenario depends on the value.
- **Direction change:** The `csv-export-timeout` Background refers to "the tunnel setup deadline specified in `import-export/http-transport`" without the value. The follow-up in decision [9] Consequences and in plan.md § Impact now also names the `http_transport` line of architecture § Components, the tunnel line of § Constraints, and the doc texts of tasks 7.1, 9.1, and 10.1.
- **Promotes to ADR:** no
