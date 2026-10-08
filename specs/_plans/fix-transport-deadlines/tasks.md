# Tasks: fix-transport-deadlines

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [x] tested-green
- [ ] recorded
- [ ] pr-ready


## Phase 2: Implementation (Group A: Control-connection deadline and terminated transport)
- [x] 1.1 Add the crate-private module `src/transport/deadline.rs` and declare it in `src/transport/mod.rs` without a feature gate, because `http_transport` is built without either (full text: plan.md task 1.1)
- [x] 1.2 Add unit tests in `src/transport/deadline.rs` with `#[tokio::test(start_paused = true)]` and no I/O (full text: plan.md task 1.2)
- [x] 2.1 Add the crate-private module `src/transport/lifecycle.rs` and declare it in `src/transport/mod.rs` under `#[cfg(any(feature = "native", feature = "websocket"))]`, because (full text: plan.md task 2.1)
- [x] 2.2 Add to `src/transport/lifecycle.rs` the crate-private trait `LifecycleSteps` (`#[async_trait]`, `Send`) with the four steps that differ per transport: `fn lifecycle_mut(& (full text: plan.md task 2.2) [expert]
- [x] 2.3 Update the doc comments of `TransportProtocol::connect`, `authenticate`, and `terminate` in `src/transport/protocol.rs` (full text: plan.md task 2.3)
- [x] 2.4 Add unit tests to `src/transport/lifecycle.rs` with `#[tokio::test(start_paused = true)]` and no I/O (full text: plan.md task 2.4)
- [x] 3.1 Add `SilentServer` next to `FakeExasolServer`, reusing `bind_loopback` (full text: plan.md task 3.1)
- [x] 3.2 Move the TLS step that both transports repeat into `src/transport/tls.rs`, which already holds `FingerprintVerifier` and `NoVerifier` (`decision-log.md` entry [6]) (full text: plan.md task 3.2)
- [x] 3.3 Add unit tests to `src/transport/tls.rs` under the feature gate of task 3.2 (full text: plan.md task 3.3)
- [x] 4.1 In `connect()`, replace the state guard and the `tokio::time::timeout` wrapper with `let deadline = self.lifecycle.begin_connect(params.timeout_ms)?` (full text: plan.md task 4.1)
- [x] 4.2 Adopt the shared lifecycle of task 2.2 (`decision-log.md` entry [6]) (full text: plan.md task 4.2)
- [x] 4.3 Add unit tests in `src/transport/native/mod.rs`, each under an outer `tokio::time::timeout` of 10 seconds that panics, with `timeout_ms` 300 and `validate_server_certific (full text: plan.md task 4.3)
- [x] 5.1 Split `connect()` into three steps under `let deadline = self.lifecycle.begin_connect(params.timeout_ms)?` (`decision-log.md` entry [3]) (full text: plan.md task 5.1) [expert]
- [x] 5.2 Adopt the shared lifecycle as task 4.2 describes, with these WebSocket steps: `login_exchange` holds today's login exchange and stores `session_info`; `send_disconnect` s (full text: plan.md task 5.2)
- [x] 5.3 Add unit tests in `src/transport/websocket.rs` with the settings of task 4.3 (full text: plan.md task 5.3)
- [x] 6.1 Add a helper to `tests/common/mod.rs` that binds `127.0.0.1:0`, accepts connections in a spawned task, holds them open without writing, and returns the port and the task  (full text: plan.md task 6.1)
- [x] 6.2 Add to `tests/integration_tests.rs`, without `skip_if_no_exasol!`, because no Exasol is involved: `test_connection_timeout_fails_a_silent_tls_server_at_the_tls_handshake` (full text: plan.md task 6.2)
- [x] 6.3 Add `test_ws_connection_timeout_fails_a_silent_server_at_the_websocket_upgrade` to `tests/websocket_integration_tests.rs` with `&tls=false&transport=websocket`, expecting (full text: plan.md task 6.3)
- [x] 6.4 In `test_csv_export_explicit_timeout_terminates_connection` (`tests/integration_tests.rs`), bind the error of the follow-up `conn.query("SELECT 1")` and assert that its D (full text: plan.md task 6.4)
- [x] 6.5 In `src/connection/params.rs`, add `/// Scenario: Connection timeout default and maximum` to `test_builder_default_values` and `test_builder_validation_timeout` (full text: plan.md task 6.5)
- [x] 7.1 In `docs/setup-and-connect.md`, change the `connection_timeout` row of the Parameters table to "Time limit in seconds for opening a connection: TCP connect, TLS handshake (full text: plan.md task 7.1)
- [x] 7.2 In `docs/import-export.md` § Export Timeout, extend the `transport_terminated: true` bullet: a statement issued on that connection afterwards fails with "Transport was te (full text: plan.md task 7.2)
- [x] 7.3 Add a `## [Unreleased]` section above `## 0.18.0` in `CHANGELOG.md` (`decision-log.md` entry [12]) with three entries (full text: plan.md task 7.3)

## Phase 2: Implementation (Group B: Tunnel setup deadline)
- [x] 8.1 Add `pub async fn connect_with_timeout(host: &str, port: u16, use_tls: bool, setup_timeout: Duration) -> Result<Self, TransportError>` (`decision-log.md` entry [9]) (full text: plan.md task 8.1)
- [x] 8.2 Add `FakeExasolServer::silent_before_handshake()` to `src/transport/test_support.rs`: it accepts one connection, reads the magic packet, signals the test through a `tokio (full text: plan.md task 8.2)
- [x] 8.3 Add unit tests in `src/transport/http_transport.rs` under an outer `tokio::time::timeout` of 10 seconds (full text: plan.md task 8.3)
- [x] 9.1 Rewrite the `# Timeout behavior` section of the `export_to_callback` doc: tunnel setup comes first and is bounded by the 30-second setup deadline of `HttpTransportClient: (full text: plan.md task 9.1)
- [x] 9.2 Add unit tests `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` (options without `timeout_ms`) and `test_export_to_callback_bounds_a_stalled_tunnel_s (full text: plan.md task 9.2)
- [x] 9.3 Add `/// Scenario: No client-side export timeout by default` to `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset` and confirm that it still passes,  (full text: plan.md task 9.3)
- [x] 9.4 Add the unit test `test_import_from_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` to `src/import/csv.rs` (full text: plan.md task 9.4)
- [x] 10.1 In `docs/import-export.md`, add a section `## Tunnel Setup Timeout` before `## Supported Formats`: every import and export opens its HTTP tunnel under one 30-second deadl (full text: plan.md task 10.1)
- [x] 10.2 Add to the `## [Unreleased]` section of `CHANGELOG.md`: a `Fix:` line stating that HTTP tunnel setup for imports and exports (TCP connect, EXA handshake, TLS handshake) i (full text: plan.md task 10.2)

## Phase 3: Verification
- [x] 3.1 Run plan Verification Checklist (plan.md § Verification)
- [x] 3.2 Scenario coverage audit and manual testing

## Phase 4: Review Fixes
- [x] 4.1 In `src/transport/test_support.rs`, add the shared `LOOPBACK_TEST_BOUND`, `DISCONNECT_BOUND`, `SILENT_SERVER_CONNECTION_TIMEOUT`, `test_credentials()`, `silent_server_params()`, and `assert_names_the_termination()`; delete the local copies from the test modules of `src/transport/native/mod.rs`, `websocket.rs`, `lifecycle.rs`, `tls.rs`, and `http_transport.rs`, and use the shared ones (including the two terminated-transport tests)
- [x] 4.2 In `src/export/csv.rs`, replace the repeated first sentence of the `export_to_callback` `# Timeout behavior` paragraph with "When a configured bound elapses, the transport is terminated only if"
- [x] 4.3 In `src/transport/tls.rs` `tests::handshake`, add `client_handshake_fails_with_a_tls_error_for_a_host_that_is_not_a_server_name`
- [x] 4.4 In `src/transport/websocket.rs` tests, add `REJECTION_TIMEOUT` and `connect_reports_a_rejected_server_certificate_as_a_tls_error`
