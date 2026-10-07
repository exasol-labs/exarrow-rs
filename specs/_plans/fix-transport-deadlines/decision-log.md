# Decision Log: fix-transport-deadlines

## Interview

Headless run. No live interview took place. The orchestrator brief is the only input, summarized here as Q/A pairs.

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

## Design Decisions

### [1] The transport owns the connection-setup deadline

- **Decision:** The connection timeout is one deadline that the transport starts in `connect()` and that also bounds `authenticate()`. The transport names the step that was running when the deadline elapsed. The Connection does not wrap `connect()` and `authenticate()` in a timer of its own.
- **Alternatives:** (a) `Connection::connect_with_transport` computes the deadline and wraps both calls in `tokio::time::timeout_at`, as issue #76 proposes. Rejected: the Connection sees only two calls, so it cannot name the TLS handshake or the WebSocket upgrade. A dropped `authenticate()` future also leaves the transport in the `Connected` state with a half-read socket, and every caller would have to remember to tear it down. (b) An independent timeout per step. Rejected: a stalled setup could then take up to three or four times the configured value.
- **Rationale:** /speq:adr-rules rule-2 criterion 2: the rule binds every `TransportProtocol` implementation, because `connect()` must start the deadline and `authenticate()` must spend only what remains. Criterion 4: it rejects the issue's own proposal, and a later reader who changes `connect_with_transport` needs the reason. `speq decision-log show` lists no ADR about the connection timeout. `server-enforced-query-timeout` bans client-side timers in query execution only, and connection setup runs no query. Independently of the file-ownership limit in this plan, only the transport knows which setup step is running.
- **Consequences:** The `TransportProtocol::connect` and `TransportProtocol::authenticate` doc comments state the shared deadline. A transport that calls `authenticate()` long after `connect()` has less time for login. A login that runs out of time closes the transport's socket, see entry [4].
- **Architecture:** § Components, § Data Flow, § Constraints
- **Promotes to ADR:** yes

### [2] One shared setup-deadline module serves both transports and the tunnel

- **Decision:** A crate-private module `src/transport/deadline.rs` holds `SetupDeadline` and the `SetupStep` vocabulary. `SetupDeadline::start(label, budget)` fixes the deadline. `run(step, future)` awaits the future with `tokio::time::timeout_at` and, on elapse, returns `TransportError::IoError("<label> timeout after <budget>ms (<step>)")`. `has_elapsed()` tells a caller whether a failed step ran out of time. The native transport, the WebSocket transport, and `HttpTransportClient` use it with the labels `Connection` and `HTTP tunnel setup`.
- **Alternatives:** (a) Inline `tokio::time::timeout_at` calls and `format!` error texts in each of the three modules. Rejected: the step names and the error format are user-visible text, and three copies drift apart. (b) A `TransportError::Timeout` variant. Rejected, see entry [13].
- **Rationale:** Quick Diagnostic for the new module. One sentence: it bounds a sequence of async steps with one shared budget and names the step that ran out. Calling it is easier than reimplementing it: callers do not compute remaining time or format errors. Changing its internals forces no edit elsewhere, because callers pass only a label, a step, and a future. Its doc comment states why the step is named inside the module that runs the steps. It is the one owner of the step vocabulary and the timeout message format. Its boundary is one struct and one enum. It takes no shortcut that needs a follow-up. It depends only on `tokio` time and `TransportError`, which the transport layer already uses.
- **Consequences:** `src/transport/mod.rs` declares the module unconditionally, because `http_transport` is built without either transport feature. `SetupDeadline` is `Copy`, so a transport stores it between `connect()` and `authenticate()`.
- **Promotes to ADR:** no

### [3] The WebSocket transport runs TCP connect, TLS handshake, and WebSocket upgrade as separate steps

- **Decision:** `WebSocketTransport::connect` stops calling `connect_async_tls_with_config`. It opens the TCP connection with `TcpStream::connect((host, port))`, runs the TLS handshake with `tokio_rustls::TlsConnector` and the same rustls client configuration as today, wraps the result in `MaybeTlsStream::Rustls` or `MaybeTlsStream::Plain`, and runs the upgrade with `tokio_tungstenite::client_async_with_config`. Each call runs as its own step under the one deadline. The WebSocket configuration keeps unlimited frame and message sizes, and Nagle's algorithm stays enabled, as today.
- **Alternatives:** Keep `connect_async_tls_with_config` under one step. Rejected: that future runs three steps, so the error could not say whether the TLS handshake or the upgrade stalled.
- **Rationale:** The brief asks for the step name in the error. `connect_async_tls_with_config` performs these same three calls internally (tokio-tungstenite 0.28.0 `connect.rs` and `tls.rs`), and `MaybeTlsStream::Rustls` holds the `tokio_rustls` 0.26 stream that the crate already depends on, so the stream type `WebSocketStream<MaybeTlsStream<TcpStream>>` does not change.
- **Consequences:** The `Connector` import goes away. A TLS-enabled server that never answers fails with `(TLS handshake)`, and a plain server that never answers the upgrade fails with `(WebSocket handshake)`.
- **Promotes to ADR:** no

### [4] A login that runs out of time closes the socket and leaves the transport closed

- **Decision:** When the login step fails and the deadline has elapsed, the transport drops its socket, clears its session, and sets its state to `Closed` before it returns the timeout error. It does not set `Terminated`.
- **Alternatives:** (a) Keep the socket and the `Connected` state. Rejected: the dropped login future leaves unread bytes on the socket, so a retried `authenticate()` would read the wrong response. (b) Set `Terminated`. Rejected: the terminated error names an export as the cause, which would be false here.
- **Rationale:** ADR `client-give-up-terminates-connection` requires a client-side give-up to release the connection rather than abandon an in-flight request. `connect_with_transport` drops the transport after any authenticate error, so this changes only what a direct `TransportProtocol` caller sees.
- **Promotes to ADR:** no

### [5] A login timeout surfaces as `ConnectionError::AuthenticationFailed` for now

- **Decision:** This plan leaves the error mapping in `connect_with_transport` unchanged. A login timeout reaches the caller as `ConnectionError::AuthenticationFailed("Network I/O error: Connection timeout after <N>ms (login)")`. A timeout in the TCP connect, the TLS handshake, or the WebSocket upgrade reaches the caller as `ConnectionError::ConnectionFailed`. The follow-up maps a login timeout to `ConnectionError::Timeout` or `ConnectionError::ConnectionFailed` in `src/adbc/connection.rs`.
- **Alternatives:** Change the mapping in `src/adbc/connection.rs` now. Rejected: another worktree owns that file.
- **Rationale:** The error message names the timeout and the step, which is what issue #76 asks for. The variant is a known inaccuracy, recorded in plan.md § Impact and in the PR description.
- **Consequences:** The PR description lists the follow-up, together with the tunnel follow-up of entry [9].
- **Promotes to ADR:** no

### [6] A `Terminated` state and one guard helper per transport report the termination

- **Decision:** Both transports add `ConnectionState::Terminated`, which only `terminate()` sets. A private helper `require_state(expected, message)` returns `Ok` when the state matches, the shared terminated error when the state is `Terminated`, and `TransportError::ProtocolError(message)` otherwise. Every pre-flight guard calls it: `connect`, `authenticate`, `execute_query`, `fetch_results`, `close_result_set`, `create_prepared_statement`, `execute_prepared_statement`, `close_prepared_statement`, `set_autocommit`, and `set_query_timeout`. The existing guard messages stay.
- **Alternatives:** (a) Have `Connection` check `is_closed()` before every dispatch, as issue #56 also suggests. Rejected: `src/adbc/connection.rs` is out of bounds, and a direct `TransportProtocol` caller would still see the authentication message. (b) A `TransportError::Terminated` variant. Rejected, see entry [13]. (c) Add a terminated check to each guard by hand. Rejected: the brief prefers a helper, and ten copies of one rule drift.
- **Rationale:** The helper keeps the termination rule in one place per transport. The guard call sites still change, but each becomes one line.
- **Promotes to ADR:** no

### [7] The terminated error names the export and says "reconnect"

- **Decision:** `src/transport/protocol.rs` exposes a crate-private constructor that returns `TransportError::ProtocolError("Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation")`. Both transports use it.
- **Alternatives:** (a) "reopen the connection", the wording of the brief. Rejected in favour of "reconnect before the next operation", which `ExportError::Timeout` and `docs/import-export.md` already use for the same remedy. (b) A generic text without the export. Rejected: issue #56 asks the error to name the cause, and the export timeout in `export_to_callback` is the only caller of `terminate()` apart from `close()`.
- **Rationale:** One constructor keeps one text for both transports. The caller sees "Query execution failed: Protocol error: Transport was terminated after ..." at the `Connection` level.
- **Consequences:** A future caller of `terminate()`, such as a cancel implementation, must revise this text and the `session-and-lifecycle` scenario "Operations after an export timeout name the termination", because the transport then has more than one cause of termination.
- **Promotes to ADR:** no

### [8] `close()` keeps delegating its teardown to `terminate()` and then records `Closed`

- **Decision:** `close()` returns `Ok` without I/O when the state is `Disconnected`, `Closed`, or `Terminated`. Otherwise it sends the disconnect as today, calls `terminate()`, and sets the state to `Closed`.
- **Alternatives:** A private teardown helper that takes the target state, called by both methods. Rejected: it changes the delegation that ADR `transport-terminate-no-round-trip` records in its Consequences, for no gain.
- **Rationale:** The ADR states that `close()` delegates its teardown to `terminate()`. Without the final state assignment, a graceful close would report itself as terminated and blame an export. Returning early on `Terminated` keeps the termination cause after a later `close()`, which `test_csv_export_explicit_timeout_terminates_connection` performs.
- **Promotes to ADR:** no

### [9] Tunnel setup has a fixed 30-second default, and `connect_with_timeout` takes an explicit bound

- **Decision:** `HttpTransportClient::connect_with_timeout(host, port, use_tls, setup_timeout)` runs the TCP connect, the EXA handshake, and the TLS handshake under one `SetupDeadline` labelled `HTTP tunnel setup`. `HttpTransportClient::connect` keeps its signature and delegates with a private 30-second constant. Certificate generation stays between the EXA handshake and the TLS handshake, outside any step. `perform_handshake` stays unchanged. No caller changes.
- **Alternatives:** (a) Arm the export timer before tunnel setup. Rejected: ADR `export-timer-stays-opt-in` keeps that timer unset by default, so setup would stay unbounded by default, and imports have no timer at all. (b) Derive the bound from the connection timeout. Deferred: the import and export entry points get their options from `src/adbc/connection.rs`, which this plan must not edit.
- **Rationale:** The fixed bound turns every stalled tunnel setup into an error with the smallest change, and it covers imports and exports at once. `connect_with_timeout` is the entry point that the follow-up and the tests use. A stalled tunnel TLS handshake, as in issue #53, now fails after 30 seconds instead of hanging. The root cause of #53 stays out of scope.
- **Consequences:** Follow-up: pass the connection timeout from `src/adbc/connection.rs` to `connect_with_timeout` for every import and export, and record it in the PR description. `ExportError::Timeout` keeps its meaning, because the tunnel setup error is `ExportError::HttpTransportError` and is raised before any request is sent.
- **Promotes to ADR:** no

### [10] Spec organization

- **Decision:** A new feature `connection-management/connection-timeout` holds the connection-timeout scenarios. The scenario "Connection timeout" moves out of `connection-management/session-and-lifecycle`, whose Background points to the new feature. The tunnel bound adds two scenarios to `import-export/http-transport` (7 to 9). `import-export/csv-export-timeout` changes its description, Background, and the scenario "No client-side export timeout by default", which said that no client-side timer wraps the export at all.
- **Alternatives:** (a) Keep "Connection timeout" in `session-and-lifecycle` and change it in place. Rejected: the feature has 10 scenarios, the limit, so the step-specific cases do not fit. (b) Put the step cases into `native-client/handshake` and `websocket-client/handshake`. Rejected: the connection timeout is one cross-transport contract, and splitting it repeats the shared deadline rule in two features.
- **Rationale:** `connection-management` goes from 6 to 7 features. `session-and-lifecycle` stays at 10 scenarios: it loses "Connection timeout" and gains "Operations after an export timeout name the termination" (Review Findings entry [1]). `import-export` keeps 14 features. No test carries the `/// Scenario: Connection timeout` line, so no test doc line changes.
- **Promotes to ADR:** no

### [11] Tests run against loopback fakes in the library, plus public-API tests without Exasol

- **Decision:** Each transport and the tunnel get library unit tests that connect to loopback fakes in `src/transport/test_support.rs`: a server that accepts and never writes, one that completes TLS and then stays silent, and, with the `websocket` feature, one that completes the WebSocket upgrade and then stays silent. Each test runs under an outer `tokio::time::timeout` of 10 seconds that panics, with a setup bound of 300 milliseconds. `tests/integration_tests.rs` and `tests/websocket_integration_tests.rs` add tests that open a connection through `Connection::from_params` with `timeout=1` against a silent loopback server. These tests need no Exasol and do not call `skip_if_no_exasol!`. The 30-second tunnel default is tested through `export_to_callback` with the clock paused only after the fake server has received the magic packet, the pattern of `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset`. `SetupDeadline` is tested with `#[tokio::test(start_paused = true)]` and no I/O.
- **Alternatives:** (a) Public-API tests only. Rejected: SonarQube Cloud reads only the unit-test coverage, so the new transport code needs library tests. (b) Pause the clock before any socket I/O. Rejected: tokio advances a paused clock whenever the runtime is idle, so the deadline could fire during the loopback connect and name the wrong step.
- **Rationale:** The library tests cover every step on both transports. The public-API tests reproduce issue #76 through the connection string, including the mapping in `connect_with_transport`.
- **Promotes to ADR:** no

### [12] Changelog and version

- **Decision:** `CHANGELOG.md` gets a `## [Unreleased]` section above `## 0.18.0`, with `Fix:` entries for #76, #57, and #56 and an `Added:` entry for `HttpTransportClient::connect_with_timeout`. `Cargo.toml` keeps its version.
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
- **Direction change:** The step now reads "every subsequent operation on that transport except `close` MUST fail instead of reading the abandoned response, and `close` SHALL succeed without a protocol round-trip", and the scenario keeps three AND steps. The export-specific error text, the rule that a closed transport keeps reporting the termination, and the closed-versus-terminated rule move into the new scenario "Operations after an export timeout name the termination", whose GIVEN is the export case. `session-and-lifecycle` stays at 10 scenarios. The `/// Scenario:` lines in tasks 4.5, 5.4, and 6.4 and the § Scenario Coverage rows follow the split. Decision [7] Consequences now names the new scenario as well.
- **Promotes to ADR:** no

### [2] [plan-review] "Connection timeout default and maximum" had no implementing task

- **Finding:** The scenario cited three existing tests in `src/connection/params.rs`, none of which carries a `/// Scenario:` line. `test_builder_validation_timeout` checked only `is_err()`, not `ConnectionError::InvalidParameter`, and `test_parse_connection_timeout_param` checked neither the default nor the maximum.
- **Direction change:** New task 6.5 adds the Scenario line to `test_builder_default_values` and `test_builder_validation_timeout`, makes the latter assert `ConnectionError::InvalidParameter` with `parameter == "connection_timeout"`, and adds `test_parse_connection_timeout_default_and_maximum` for the connection-string path (default 30 seconds, 300 accepted, 301 rejected). § Scenario Coverage replaces `test_parse_connection_timeout_param` with the new test, and the group A Knowledge column lists `src/connection/params.rs`.
- **Promotes to ADR:** no
