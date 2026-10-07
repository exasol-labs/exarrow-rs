# Plan: fix-transport-deadlines

## Summary

Opening a connection and opening an HTTP tunnel each run under one deadline that bounds every setup step and names the step that ran out, so a peer that accepts the TCP connection and then stops answering fails the call instead of hanging it (#76, #57). After an export timeout terminates the transport, the next operation reports the termination instead of a missing authentication (#56).

## Context

- Issue #76: `NativeTcpTransport::connect` (`src/transport/native/mod.rs`) wraps only `TcpStream::connect` in `tokio::time::timeout(params.timeout_ms)`. The TLS handshake (`connector.connect`) and `authenticate()` have no bound. A server that accepts and stays silent hangs the caller with TLS on, with TLS off, and after a completed TLS handshake.
- `WebSocketTransport::connect` (`src/transport/websocket.rs`) wraps `connect_async_tls_with_config` in one timeout. That future runs the TCP connect, the TLS handshake, and the WebSocket upgrade, so those steps are bounded, but the error does not name the step. `authenticate()` has no bound.
- `ConnectionParams` (`src/connection/params.rs`) defaults the connection timeout to 30 seconds and rejects values above 300 seconds. `Connection::connect_with_transport` (`src/adbc/connection.rs`) converts it to `timeout_ms`, calls `transport.connect()` and then `transport.authenticate()`, and maps their errors to `ConnectionError::ConnectionFailed` and `ConnectionError::AuthenticationFailed`. Every connection path, including `Database::connect` and the ADBC FFI, goes through it.
- `src/adbc/connection.rs` and `src/adbc_ffi.rs` belong to another worktree and stay unchanged in this plan.
- `TransportProtocol::authenticate` takes only credentials, so a deadline that also bounds the login must be stored by the transport in `connect()`.
- Issue #57: `HttpTransportClient::connect(host, port, use_tls)` (`src/transport/http_transport.rs`) awaits `TcpStream::connect`, `perform_handshake` (a `read_exact` of the 24-byte EXA response), and the tunnel TLS handshake with no bound. Its callers are `src/export/csv.rs`, `src/import/csv.rs`, `src/import/parquet.rs`, and `src/import/parallel.rs`.
- `export_to_callback` (`src/export/csv.rs`) opens the tunnel before it arms `CsvExportOptions::timeout_ms`, so that timeout never covers tunnel setup. Its doc says `timeout_ms` "is the only client-side bound on the whole operation". The field doc, the builder doc ("Bounds the whole export"), and `docs/import-export.md` do not say that tunnel setup is outside the bound.
- Issue #53 reports that a `use_tls(true)` import hangs in the tunnel TLS handshake. Its root cause stays out of scope.
- Issue #56: `terminate()` sets `ConnectionState::Closed` in both transports, the same state a graceful `close()` sets. The pre-flight guards compare against `Connected` or `Authenticated` and return "Must authenticate before ..." errors, so `conn.query("SELECT 1")` after a terminating export timeout fails with "Must authenticate before executing queries".
- `close()` in both transports delegates its teardown to `terminate()`, as ADR `transport-terminate-no-round-trip` records. `export_to_callback` is the only other caller of `terminate()`.
- `TransportError` is public and not `#[non_exhaustive]`. The existing connect timeout returns `TransportError::IoError`, and the existing guards return `TransportError::ProtocolError`.
- SonarQube Cloud reads only unit-test coverage. CI runs `cargo test --lib --features websocket` for the WebSocket unit tests, and coverage runs with default features only.
- The plan changes § Components, § Data Flow, and § Constraints of the architecture, see the architecture delta `architecture.md` in this plan directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| connection-management/connection-timeout | NEW | `connection-management/connection-timeout/spec.md` |
| connection-management/session-and-lifecycle | CHANGED | `connection-management/session-and-lifecycle/spec.md` |
| import-export/http-transport | CHANGED | `import-export/http-transport/spec.md` |
| import-export/csv-export-timeout | CHANGED | `import-export/csv-export-timeout/spec.md` |

## Impact

- Behavior change: opening a connection to a server that accepts the TCP connection and then stops answering fails after the connection timeout (default 30 seconds) instead of hanging. The error message contains `Connection timeout after <ms>ms (<step>)` with the step `TCP connect`, `TLS handshake`, `WebSocket handshake`, or `login`.
- Behavior change: the connection timeout now covers the TLS handshake and the login, not only the TCP connect. A setup that took longer than the configured timeout in total, for example a slow login with `timeout=1`, now fails. The 30-second default and the 300-second maximum stay.
- Known limitation: a login that runs out of time reaches the caller as `ConnectionError::AuthenticationFailed` with the timeout message, because `connect_with_transport` maps every login error to that variant. The ADBC FFI reports it the same way. The follow-up in `src/adbc/connection.rs` is recorded in `decision-log.md` entry [5] and goes into the PR description.
- Behavior change: HTTP tunnel setup for every import and export fails after 30 seconds instead of hanging, with `HTTP tunnel setup timeout after 30000ms (<step>)` inside `ImportError::HttpTransportError` or `ExportError::HttpTransportError`. A `use_tls(true)` tunnel that stalls in its TLS handshake (#53) now fails after 30 seconds with the step `TLS handshake`. The 30 seconds do not follow the connection timeout yet. That follow-up is recorded in `decision-log.md` entry [9] and goes into the PR description.
- New public API: `HttpTransportClient::connect_with_timeout(host, port, use_tls, Duration)`. `HttpTransportClient::connect` keeps its signature.
- After an export timeout terminated the transport, the next operation fails with "Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation" instead of "Must authenticate before ...". At the `Connection` level it arrives as `QueryError::ExecutionFailed`, as before.
- No breaking change: `TransportProtocol`, `TransportError`, `ConnectionError`, `ExportError`, and `ImportError` keep their shape. The version stays, and the changelog entries go under `## [Unreleased]`.

## Implementation Tasks

### Group A: Control-connection deadline and terminated transport

1. Shared setup deadline (`src/transport/deadline.rs`, `src/transport/mod.rs`)

- [ ] 1.1 Add the crate-private module `src/transport/deadline.rs` and declare it in `src/transport/mod.rs` without a feature gate, because `http_transport` is built without either transport feature (`decision-log.md` entry [2]). It defines `SetupStep` with the variants `TcpConnect`, `TlsHandshake`, `WebSocketHandshake`, `Login`, and `ExaHandshake`, whose `Display` output is `TCP connect`, `TLS handshake`, `WebSocket handshake`, `login`, and `EXA handshake`. It defines `SetupDeadline` (`Clone`, `Copy`, `Debug`) holding a `tokio::time::Instant` deadline, the budget, and a `&'static str` label. `SetupDeadline::start(label, budget: Duration)` sets the deadline to now plus the budget. `async fn run<T>(&self, step: SetupStep, future)`, where the future yields `Result<T, TransportError>`, awaits `tokio::time::timeout_at`, returns the future's result unchanged when it finishes first, and otherwise returns `TransportError::IoError(format!("{label} timeout after {budget_ms}ms ({step})"))`. `has_elapsed()` reports whether the deadline has passed. The doc comments state why the module that runs the steps names the step.
- [ ] 1.2 Add unit tests in `src/transport/deadline.rs` with `#[tokio::test(start_paused = true)]` and no I/O. `a_later_step_gets_only_the_time_left_by_earlier_steps`: budget 1000 ms, step 1 sleeps 600 ms and returns `Ok`, step 2 never finishes; step 2 fails 1000 ms after the start, measured with `tokio::time::Instant`, and its error names step 2. `an_elapsed_deadline_names_the_label_the_budget_and_the_step`: label `Connection`, budget 3000 ms, a pending `TlsHandshake` step gives the Display text `Network I/O error: Connection timeout after 3000ms (TLS handshake)`; label `HTTP tunnel setup`, budget 300 ms, a pending `ExaHandshake` step gives `HTTP tunnel setup timeout after 300ms (EXA handshake)`. `a_step_error_before_the_deadline_is_returned_unchanged`: a step that returns `TransportError::TlsError` before the deadline yields that error. Each test carries `/// Scenario: One deadline bounds every connection setup step`. The second test also carries `/// Scenario: Tunnel setup fails with the step named when the peer stops answering`.

2. Transport core (`src/transport/protocol.rs`)

- [ ] 2.1 Add a crate-private function `terminated_transport_error() -> TransportError` that returns `TransportError::ProtocolError("Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation")` (`decision-log.md` entry [7]). Its doc comment states that the export timeout is the only caller of `terminate()` apart from `close()`, so a new caller must revise the text.
- [ ] 2.2 Update the doc comments of `TransportProtocol::connect`, `authenticate`, and `terminate`. `connect` starts the connection-timeout deadline, which bounds the TCP connect, the TLS handshake, and the WebSocket upgrade. `authenticate` is bounded by what remains of that deadline (`decision-log.md` entry [1]). After `terminate`, every other operation fails with the terminated-transport error, and `close` succeeds without I/O.

3. Loopback fakes (`src/transport/test_support.rs`)

- [ ] 3.1 Add `SilentServer` next to `FakeExasolServer`, reusing `bind_loopback`. `SilentServer::accepting()` accepts one connection, never writes, and reads until end of stream. `SilentServer::after_tls()` accepts one connection, completes a server-side TLS handshake with `TlsCertificate::generate()`, `to_server_config()`, and `tokio_rustls::TlsAcceptor`, then reads until end of stream without writing. `SilentServer::after_websocket_upgrade()` (`websocket` feature) completes `tokio_tungstenite::accept_async` on the plain connection, then reads until the client closes without sending a frame. `wait_for_disconnect()` waits until the peer task observed end of stream. `Drop` aborts the peer task. Add the new fake to the module doc's list of doubles.

4. Native transport (`src/transport/native/mod.rs`)

- [ ] 4.1 In `connect()`, start `SetupDeadline::start("Connection", Duration::from_millis(params.timeout_ms))` before the TCP connect. Run `TcpStream::connect` as `SetupStep::TcpConnect` and `connector.connect` as `SetupStep::TlsHandshake`, keeping today's error mapping for failures that are not timeouts. Store the deadline in a new field `setup_deadline: Option<SetupDeadline>` when the state becomes `Connected`. Remove the old `tokio::time::timeout` wrapper.
- [ ] 4.2 In `authenticate()`, move the three login phases into a private async method and run it as `SetupStep::Login` with the deadline taken from `setup_deadline`. When the login fails and `has_elapsed()` is true, drop the stream, clear the session, and set the state to `Closed` before returning the error (`decision-log.md` entry [4]). A login error that arrives before the deadline leaves the transport as today. [expert]
- [ ] 4.3 Add `ConnectionState::Terminated`. `terminate()` sets it. `close()` returns `Ok` without I/O for `Disconnected`, `Closed`, and `Terminated`, and otherwise sends the disconnect as today, calls `terminate()`, and sets the state to `Closed` (`decision-log.md` entry [8]). `is_connected()` stays unchanged.
- [ ] 4.4 Add a private method `require_state(&self, expected: ConnectionState, message: &str) -> Result<(), TransportError>` that returns `Ok` for the expected state, `terminated_transport_error()` for `Terminated`, and `TransportError::ProtocolError(message)` otherwise. Replace the guards of `connect`, `authenticate`, `execute_query`, `fetch_results`, `close_result_set`, `create_prepared_statement`, `execute_prepared_statement`, `close_prepared_statement`, `set_autocommit`, and `set_query_timeout` with calls that keep the current messages (`decision-log.md` entry [6]).
- [ ] 4.5 Add unit tests in `src/transport/native/mod.rs`, each under an outer `tokio::time::timeout` of 10 seconds that panics, with `timeout_ms` 300 and `validate_server_certificate` off. `connect_fails_at_the_tls_handshake_when_the_server_never_answers` (`SilentServer::accepting()`, TLS on): the error contains `Connection timeout after 300ms (TLS handshake)`, at least 300 ms passed, and the server sees end of stream within 5 seconds. `authenticate_fails_at_login_when_the_server_never_answers_without_tls` (`SilentServer::accepting()`, TLS off): `connect()` succeeds, `authenticate()` fails with `Connection timeout after 300ms (login)` after at least 300 ms in total, `is_connected()` is false, and the server sees end of stream. `authenticate_fails_at_login_when_the_server_goes_silent_after_tls` (`SilentServer::after_tls()`, TLS on): the same assertions. `every_operation_after_terminate_reports_the_terminated_transport`: from `Authenticated`, `terminate()`, then `connect`, `authenticate`, `execute_query`, `fetch_results`, `create_prepared_statement`, `set_autocommit`, and `set_query_timeout` each fail with a message that contains `Transport was terminated after an export gave up on an in-flight response` and `reconnect` and does not contain `Must authenticate`. `close_after_terminate_succeeds_and_keeps_reporting_termination`: `terminate()`, then `close()` returns `Ok`, then `execute_query` still reports the termination. `close_leaves_a_transport_closed_rather_than_terminated`: from `Connected` with no stream, `close()`, then `execute_query` fails with `Must authenticate before executing queries`. Each test carries the `/// Scenario:` line of the scenario it implements: the first test "Server that never answers the TLS handshake"; the two `authenticate_` tests "Server that never answers the login"; `every_operation_after_terminate_reports_the_terminated_transport` and `close_leaves_a_transport_closed_rather_than_terminated` "Operations after an export timeout name the termination"; `close_after_terminate_succeeds_and_keeps_reporting_termination` both "Terminate a connection whose in-flight response is no longer trusted" and "Operations after an export timeout name the termination". Add `/// Scenario: Terminate a connection whose in-flight response is no longer trusted` to the existing `terminate_drops_the_session_and_reports_the_transport_disconnected`.

5. WebSocket transport (`src/transport/websocket.rs`)

- [ ] 5.1 Split `connect()` into three steps under `SetupDeadline::start("Connection", ...)` (`decision-log.md` entry [3]). Run `TcpStream::connect((params.host.as_str(), params.port))` as `SetupStep::TcpConnect`. With TLS, build the rustls `ClientConfig` exactly as today (fingerprint verifier, native roots, or `NoVerifier`), run `tokio_rustls::TlsConnector::connect` with the host's `ServerName` as `SetupStep::TlsHandshake`, and wrap the result in `MaybeTlsStream::Rustls`; without TLS, use `MaybeTlsStream::Plain`. Run `tokio_tungstenite::client_async_with_config(params.to_websocket_url(), stream, Some(ws_config))` as `SetupStep::WebSocketHandshake`, keeping unlimited frame and message sizes and leaving Nagle's algorithm enabled as today. Store the deadline in a new field `setup_deadline`. Remove the `connect_async_tls_with_config` call and the `Connector` import. [expert]
- [ ] 5.2 In `authenticate()`, run the login exchange as `SetupStep::Login` with the stored deadline and apply the teardown rule of task 4.2.
- [ ] 5.3 Apply tasks 4.3 and 4.4 to the WebSocket transport: `ConnectionState::Terminated`, the `close()` rule, and a `require_state` helper for the same ten guards.
- [ ] 5.4 Add unit tests in `src/transport/websocket.rs` with the settings of task 4.5. `connect_fails_at_the_tls_handshake_when_the_server_never_answers` (`SilentServer::accepting()`, TLS on, expects `(TLS handshake)` and end of stream). `connect_fails_at_the_websocket_handshake_when_the_server_never_answers` (`SilentServer::accepting()`, TLS off, expects `(WebSocket handshake)` and end of stream). `authenticate_fails_at_login_when_the_server_goes_silent_after_the_upgrade` (`SilentServer::after_websocket_upgrade()`, TLS off, expects `(login)`, `is_connected()` false, and end of stream). `every_operation_after_terminate_reports_the_terminated_transport` and `close_leaves_a_transport_closed_rather_than_terminated` as in task 4.5. The existing `FakeWebSocketServer` tests keep passing and cover the plain connect path. Each test carries the `/// Scenario:` line of its task 4.5 counterpart; `connect_fails_at_the_websocket_handshake_when_the_server_never_answers` carries "WebSocket server that never answers the upgrade". Add `/// Scenario: Terminate a connection whose in-flight response is no longer trusted` to the existing `terminate_drops_the_session_and_reports_the_transport_disconnected` in this file.

6. Public-API and parameter tests (`tests/`, `src/connection/params.rs`)

- [ ] 6.1 Add a helper to `tests/common/mod.rs` that binds `127.0.0.1:0`, accepts connections in a spawned task, holds them open without writing, and returns the port and the task handle.
- [ ] 6.2 Add to `tests/integration_tests.rs`, without `skip_if_no_exasol!`, because no Exasol is involved: `test_connection_timeout_fails_a_silent_tls_server_at_the_tls_handshake` opens `exasol://sys:exasol@127.0.0.1:<port>?timeout=1&validateservercertificate=0` through `ConnectionParams::from_str` and `Connection::from_params` and asserts that the error's Display contains `Connection timeout after 1000ms (TLS handshake)` and that at least 1 second passed. `test_connection_timeout_fails_a_silent_server_at_login_without_tls` adds `&tls=false` and expects `Connection timeout after 1000ms (login)`. Both run under an outer `tokio::time::timeout` of 10 seconds that panics. Each carries its `/// Scenario:` line.
- [ ] 6.3 Add `test_ws_connection_timeout_fails_a_silent_server_at_the_websocket_handshake` to `tests/websocket_integration_tests.rs` with `&tls=false&transport=websocket`, expecting `Connection timeout after 1000ms (WebSocket handshake)`, with the same outer bound and its `/// Scenario:` line.
- [ ] 6.4 In `test_csv_export_explicit_timeout_terminates_connection` (`tests/integration_tests.rs`), bind the error of the follow-up `conn.query("SELECT 1")` and assert that its Display contains `Transport was terminated after an export gave up on an in-flight response` and `reconnect` and does not contain `Must authenticate`. In `test_terminate_marks_websocket_transport_disconnected` (`tests/websocket_integration_tests.rs`), assert the same for the error of `execute_query`. Add `/// Scenario: Terminate a connection whose in-flight response is no longer trusted` and `/// Scenario: Operations after an export timeout name the termination` to both.
- [ ] 6.5 In `src/connection/params.rs`, add `/// Scenario: Connection timeout default and maximum` to `test_builder_default_values` and `test_builder_validation_timeout`. Change `test_builder_validation_timeout` to assert `ConnectionError::InvalidParameter` with `parameter == "connection_timeout"` instead of only `is_err()`. Add `test_parse_connection_timeout_default_and_maximum` with the same Scenario line: `exasol://user@localhost` gives 30 seconds, `?timeout=300` gives 300 seconds, and `?timeout=301` fails with `ConnectionError::InvalidParameter` whose `parameter` is `connection_timeout`. No production code changes.

7. Documentation and changelog

- [ ] 7.1 In `docs/setup-and-connect.md`, change the `connection_timeout` row of the Parameters table to "Time limit in seconds for opening a connection: TCP connect, TLS handshake, WebSocket upgrade, and login together (max 300)". In § Timeouts, add a paragraph: the connection timeout is one deadline for the TCP connect, the TLS handshake, and the login (and the WebSocket upgrade on the WebSocket transport); a server that stops answering fails the connection with `Connection timeout after <ms>ms (<step>)`; a login that runs out of time is reported as `ConnectionError::AuthenticationFailed`.
- [ ] 7.2 In `docs/import-export.md` § Export Timeout, extend the `transport_terminated: true` bullet: a statement issued on that connection afterwards fails with "Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation".
- [ ] 7.3 Add a `## [Unreleased]` section above `## 0.18.0` in `CHANGELOG.md` (`decision-log.md` entry [12]) with two entries. A `Fix:` line: the connection timeout (`timeout` or `connection_timeout`, default 30 seconds) now bounds the TCP connect, the TLS handshake, the WebSocket upgrade, and the login together; a server that accepts the connection and then stops answering fails the connection with `Connection timeout after <ms>ms (<step>)` instead of hanging; a login that runs out of time is reported as `ConnectionError::AuthenticationFailed`; Fixes #76. A `Fix:` line: after an export timeout terminated the transport, the next operation fails with "Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation" instead of "Must authenticate before executing queries"; Fixes #56.

### Group B: Tunnel setup deadline

8. HTTP tunnel (`src/transport/http_transport.rs`, `src/transport/test_support.rs`)

- [ ] 8.1 Add `pub async fn connect_with_timeout(host: &str, port: u16, use_tls: bool, setup_timeout: Duration) -> Result<Self, TransportError>` (`decision-log.md` entry [9]). It starts `SetupDeadline::start("HTTP tunnel setup", setup_timeout)` and runs `TcpStream::connect` as `SetupStep::TcpConnect`, `perform_handshake` as `SetupStep::ExaHandshake`, and the TLS `connector.connect` as `SetupStep::TlsHandshake`, keeping today's error texts for failures that are not timeouts. Certificate generation stays between the EXA handshake and the TLS handshake. `connect(host, port, use_tls)` keeps its signature and delegates with a private constant `TUNNEL_SETUP_TIMEOUT` of 30 seconds. Both doc comments state that one deadline bounds the three steps, the 30-second default, and that `CsvExportOptions::timeout_ms` does not cover tunnel setup. `perform_handshake` and every caller stay unchanged.
- [ ] 8.2 Add `FakeExasolServer::silent_before_handshake()` to `src/transport/test_support.rs`: it accepts one connection, reads the magic packet, signals the test through a `tokio::sync::oneshot` channel or an equivalent awaitable method, and then reads until end of stream without writing.
- [ ] 8.3 Add unit tests in `src/transport/http_transport.rs` under an outer `tokio::time::timeout` of 10 seconds. `connect_with_timeout_fails_at_the_exa_handshake_when_the_peer_never_answers` (`SilentServer::accepting()`, 300 ms): the error contains `HTTP tunnel setup timeout after 300ms (EXA handshake)` and the peer sees end of stream within 5 seconds. `connect_with_timeout_fails_at_the_tls_handshake_when_the_peer_goes_silent_after_the_handshake` (`FakeExasolServer::silent_after_handshake()`, TLS on, 300 ms): the error contains `(TLS handshake)` and `wait_for_disconnect()` returns within 5 seconds. `connect_with_timeout_returns_the_internal_address_within_the_bound` (`FakeExasolServer::silent_after_handshake()`, TLS off, 5 seconds): `internal_address()` is `10.0.0.5:8563`. The first two carry `/// Scenario: Tunnel setup fails with the step named when the peer stops answering`. The third carries `/// Scenario: EXA tunneling handshake in client mode`.

9. Export (`src/export/csv.rs`)

- [ ] 9.1 Rewrite the `# Timeout behavior` section of the `export_to_callback` doc: tunnel setup comes first and is bounded by the 30-second setup deadline of `HttpTransportClient::connect`; `options.timeout_ms` bounds what follows (SQL execution, tunnel transfer, and the callback) and is unset by default; the tunnel read after setup has no timeout of its own. Keep the remaining paragraphs. Add to the `CsvExportOptions::timeout_ms` field doc that tunnel setup comes before this bound and has its own 30-second limit. Change the doc of the builder method `timeout_ms()` from "Bounds the whole export with a client-side timer" to a text that names SQL execution, tunnel transfer, and the callback (`decision-log.md` entry [14]).
- [ ] 9.2 Add unit tests `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` (options without `timeout_ms`) and `test_export_to_callback_bounds_a_stalled_tunnel_setup_even_with_an_export_timeout` (`timeout_ms(1_000)`). Each starts `export_to_callback` against `FakeExasolServer::silent_before_handshake()` with a `MockTransport` that expects no call, waits until the fake received the magic packet, then calls `tokio::time::pause()` and awaits the export, so the paused clock advances to the next timer (`decision-log.md` entry [11]). Each asserts `ExportError::HttpTransportError` whose message contains `HTTP tunnel setup timeout after 30000ms (EXA handshake)`. Each carries `/// Scenario: Tunnel setup is bounded by 30 seconds by default`. The first also carries `/// Scenario: No client-side export timeout by default`.
- [ ] 9.3 Add `/// Scenario: No client-side export timeout by default` to `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset` and confirm that it still passes, which shows that the setup deadline leaves no timer armed once the tunnel is set up.

10. Documentation and changelog

- [ ] 10.1 In `docs/import-export.md`, add a section `## Tunnel Setup Timeout` before `## Supported Formats`: every import and export opens its HTTP tunnel under one 30-second deadline across the TCP connect, the EXA handshake, and the TLS handshake; a stalled setup fails with `HTTP tunnel setup timeout after 30000ms (<step>)` inside `ImportError::HttpTransportError` or `ExportError::HttpTransportError`; neither the connection timeout nor `CsvExportOptions::timeout_ms` changes this bound. In § Export Timeout, change "a client-side bound on the whole export (SQL execution, HTTP transfer, and your callback, together)" to a bound on the export after tunnel setup (SQL execution, HTTP transfer, and your callback, together), with a link to the new section.
- [ ] 10.2 Add to the `## [Unreleased]` section of `CHANGELOG.md`: a `Fix:` line stating that HTTP tunnel setup for imports and exports (TCP connect, EXA handshake, TLS handshake) is bounded by 30 seconds and fails with `HTTP tunnel setup timeout after 30000ms (<step>)` instead of hanging, that `CsvExportOptions::timeout_ms` does not cover tunnel setup, and "Fixes #57"; and an `Added:` line for `HttpTransportClient::connect_with_timeout`, which opens a tunnel with a caller-chosen setup bound.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Control-connection deadline and terminated transport | 1.1-7.3 | none | spec deltas `connection-management/connection-timeout` and `connection-management/session-and-lifecycle`; architecture delta `architecture.md` (§ Components, § Data Flow, § Constraints); `src/transport/deadline.rs` (new), `src/transport/mod.rs`, `src/transport/protocol.rs` (`TransportProtocol` docs, terminated error), `src/transport/native/mod.rs` (`connect`, `authenticate`, guards, `close`, `terminate`), `src/transport/websocket.rs` (same), `src/transport/test_support.rs` (`SilentServer`), `src/connection/params.rs` (timeout tests only), `tests/common/mod.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `docs/setup-and-connect.md`, `docs/import-export.md` (§ Export Timeout bullet), `CHANGELOG.md` |
| B: Tunnel setup deadline | 8.1-10.2 | A (uses `src/transport/deadline.rs` and `SilentServer`; shares `src/transport/test_support.rs`, `docs/import-export.md`, and `CHANGELOG.md`) | spec deltas `import-export/http-transport` and `import-export/csv-export-timeout`; `src/transport/http_transport.rs` (`HttpTransportClient::connect`, `connect_with_timeout`), `src/transport/deadline.rs` (read only), `src/transport/test_support.rs` (`FakeExasolServer`), `src/export/csv.rs` (docs and `export_to_callback` tests), `docs/import-export.md`, `CHANGELOG.md` |

- Group A holds #76 and #56 together, because both change the state machine, the guards, and the `connect`/`authenticate` paths of the same two transport files and share `session-and-lifecycle`.
- Group B runs after group A. It needs the deadline module from task 1.1 and `SilentServer` from task 3.1, and it edits other sections of three files that group A also edits. The overlap is limited to those reads and appends, so a second agent orients on `http_transport` and the export docs alone.
- Order inside group A: 1.x, 2.x, 3.1, then the tests of 4.5 and 5.4 before their fixes (4.1-4.4, 5.1-5.3), then 6.x, then 7.x.
- Order inside group B: 8.2, the tests of 8.3 and 9.2, then 8.1, then 9.1, 9.3, and 10.x.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Code block | `src/transport/native/mod.rs` `connect`: the `tokio::time::timeout` around `TcpStream::connect` | Replaced by `SetupDeadline` (task 4.1) |
| Code block | `src/transport/websocket.rs` `connect`: the `connect_async_tls_with_config` call, its `tokio::time::timeout` wrapper, and the `Connector` values and import | Replaced by the three-step connect (task 5.1) |
| Code block | `src/transport/native/mod.rs` and `src/transport/websocket.rs`: the inline `if self.state != ...` guard blocks | Replaced by `require_state` calls (tasks 4.4 and 5.3) |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| One deadline bounds every connection setup step | Unit | `src/transport/deadline.rs` | `a_later_step_gets_only_the_time_left_by_earlier_steps` |
| One deadline bounds every connection setup step | Unit | `src/transport/deadline.rs` | `an_elapsed_deadline_names_the_label_the_budget_and_the_step` |
| One deadline bounds every connection setup step | Unit | `src/transport/deadline.rs` | `a_step_error_before_the_deadline_is_returned_unchanged` |
| Connection timeout default and maximum | Unit | `src/connection/params.rs` | `test_builder_default_values` (existing, gains the Scenario line) |
| Connection timeout default and maximum | Unit | `src/connection/params.rs` | `test_builder_validation_timeout` (existing, asserts `InvalidParameter`) |
| Connection timeout default and maximum | Unit | `src/connection/params.rs` | `test_parse_connection_timeout_default_and_maximum` |
| Server that never answers the TLS handshake | Integration (loopback, library) | `src/transport/native/mod.rs` | `connect_fails_at_the_tls_handshake_when_the_server_never_answers` |
| Server that never answers the TLS handshake | Integration (loopback, library) | `src/transport/websocket.rs` | `connect_fails_at_the_tls_handshake_when_the_server_never_answers` |
| Server that never answers the TLS handshake | Integration | `tests/integration_tests.rs` | `test_connection_timeout_fails_a_silent_tls_server_at_the_tls_handshake` |
| Server that never answers the login | Integration (loopback, library) | `src/transport/native/mod.rs` | `authenticate_fails_at_login_when_the_server_never_answers_without_tls` |
| Server that never answers the login | Integration (loopback, library) | `src/transport/native/mod.rs` | `authenticate_fails_at_login_when_the_server_goes_silent_after_tls` |
| Server that never answers the login | Integration (loopback, library) | `src/transport/websocket.rs` | `authenticate_fails_at_login_when_the_server_goes_silent_after_the_upgrade` |
| Server that never answers the login | Integration | `tests/integration_tests.rs` | `test_connection_timeout_fails_a_silent_server_at_login_without_tls` |
| WebSocket server that never answers the upgrade | Integration (loopback, library) | `src/transport/websocket.rs` | `connect_fails_at_the_websocket_handshake_when_the_server_never_answers` |
| WebSocket server that never answers the upgrade | Integration | `tests/websocket_integration_tests.rs` | `test_ws_connection_timeout_fails_a_silent_server_at_the_websocket_handshake` |
| Terminate a connection whose in-flight response is no longer trusted | Unit | `src/transport/native/mod.rs` | `terminate_drops_the_session_and_reports_the_transport_disconnected` (existing) |
| Terminate a connection whose in-flight response is no longer trusted | Unit | `src/transport/native/mod.rs` | `close_after_terminate_succeeds_and_keeps_reporting_termination` |
| Terminate a connection whose in-flight response is no longer trusted | Unit | `src/transport/websocket.rs` | `terminate_drops_the_session_and_reports_the_transport_disconnected` (existing) |
| Terminate a connection whose in-flight response is no longer trusted | Integration | `tests/integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection` (updated) |
| Terminate a connection whose in-flight response is no longer trusted | Integration | `tests/websocket_integration_tests.rs` | `test_terminate_marks_websocket_transport_disconnected` (updated) |
| Operations after an export timeout name the termination | Unit | `src/transport/native/mod.rs` | `every_operation_after_terminate_reports_the_terminated_transport` |
| Operations after an export timeout name the termination | Unit | `src/transport/native/mod.rs` | `close_after_terminate_succeeds_and_keeps_reporting_termination` |
| Operations after an export timeout name the termination | Unit | `src/transport/native/mod.rs` | `close_leaves_a_transport_closed_rather_than_terminated` |
| Operations after an export timeout name the termination | Unit | `src/transport/websocket.rs` | `every_operation_after_terminate_reports_the_terminated_transport` |
| Operations after an export timeout name the termination | Unit | `src/transport/websocket.rs` | `close_leaves_a_transport_closed_rather_than_terminated` |
| Operations after an export timeout name the termination | Integration | `tests/integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection` (updated) |
| Operations after an export timeout name the termination | Integration | `tests/websocket_integration_tests.rs` | `test_terminate_marks_websocket_transport_disconnected` (updated) |
| Tunnel setup fails with the step named when the peer stops answering | Integration (loopback, library) | `src/transport/http_transport.rs` | `connect_with_timeout_fails_at_the_exa_handshake_when_the_peer_never_answers` |
| Tunnel setup fails with the step named when the peer stops answering | Integration (loopback, library) | `src/transport/http_transport.rs` | `connect_with_timeout_fails_at_the_tls_handshake_when_the_peer_goes_silent_after_the_handshake` |
| Tunnel setup fails with the step named when the peer stops answering | Unit | `src/transport/deadline.rs` | `an_elapsed_deadline_names_the_label_the_budget_and_the_step` |
| EXA tunneling handshake in client mode (regression) | Integration (loopback, library) | `src/transport/http_transport.rs` | `connect_with_timeout_returns_the_internal_address_within_the_bound` |
| Tunnel setup is bounded by 30 seconds by default | Integration (loopback, library) | `src/export/csv.rs` | `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` |
| Tunnel setup is bounded by 30 seconds by default | Integration (loopback, library) | `src/export/csv.rs` | `test_export_to_callback_bounds_a_stalled_tunnel_setup_even_with_an_export_timeout` |
| No client-side export timeout by default | Integration (loopback, library) | `src/export/csv.rs` | `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset` (existing) |
| No client-side export timeout by default | Integration (loopback, library) | `src/export/csv.rs` | `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` |

- "Integration (loopback, library)" tests run real sockets against a loopback fake inside `cargo test --lib`, so their coverage reaches SonarQube Cloud (`decision-log.md` entry [11]). The WebSocket library tests run under `cargo test --lib --features websocket`.

### Manual Testing

The silent server below is the one from issue #76. Start it in a separate terminal before the connection commands, and stop it afterwards.

```bash
python3 -c 'import socket; s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", 18001)); s.listen(); c = []
while True: c.append(s.accept()[0])'
```

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| connection-management/connection-timeout | `cargo build --release --features 'ffi websocket' && time python3 -c "import adbc_driver_manager.dbapi as d; d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://sys:exasol@127.0.0.1:18001?timeout=3&validateservercertificate=0'})"` | An exception after about 3 seconds whose text contains `Connection timeout after 3000ms (TLS handshake)` |
| connection-management/connection-timeout | Same command with `?timeout=3&tls=false` | An exception after about 3 seconds that contains `Connection timeout after 3000ms (login)` |
| connection-management/connection-timeout | Same command with `?timeout=3&tls=false&transport=websocket` | An exception after about 3 seconds that contains `Connection timeout after 3000ms (WebSocket handshake)` |
| connection-management/session-and-lifecycle | `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`, wait until `exapump sql 'select 1'` returns `1`, then `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests test_csv_export_explicit_timeout_terminates_connection -- --test-threads=1` | The test passes, which asserts the terminated-transport message |
| import-export/http-transport | `cargo test --lib connect_with_timeout_ -- --nocapture` | 3 tests pass. The two failure tests finish in well under a second each |
| import-export/csv-export-timeout | `cargo test --lib test_export_to_callback_` | Every `export_to_callback` test passes, including the two tunnel-setup tests |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Build (WebSocket only) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Unit tests (WebSocket) | `cargo test --lib --features websocket` | 0 failures |
| Integration tests | `cargo test --features ffi --test integration_tests -- --test-threads=1` (Exasol running) | 0 failures |
| WebSocket integration tests | `cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` (Exasol running) | 0 failures |
| Import/export tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Native protocol tests | `cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1` (Exasol running) | 0 failures |
| Driver manager tests | `cargo build --release --features ffi && cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
