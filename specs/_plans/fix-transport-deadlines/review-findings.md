# Code Review Findings: fix-transport-deadlines

## Summary
- Files reviewed: 18
- Total findings: 4 (standard: 4, expert: 0)

## Standard fixes

### src/transport/test_support.rs

#### [SHRINKABLE] Loopback test fixtures and the terminated-message check are copied into five test modules
- Location: src/transport/native/mod.rs lines 1240-1255 and 1356-1361; src/transport/websocket.rs lines 771-786 and 958-963; src/transport/lifecycle.rs lines 191-192 and 259-269; src/transport/tls.rs line 452; src/transport/http_transport.rs lines 1423, 2721, and 2744
- Issue: The native and WebSocket test modules each define the same `TEST_BOUND` (10 s), `DISCONNECT_BOUND` (5 s), `CONNECTION_TIMEOUT` (300 ms), `TERMINATED_CAUSE`, `silent_server_params`, and `credentials`. The `lifecycle.rs` tests repeat `TERMINATED_CAUSE` and `credentials`, and the `tls.rs` tests repeat `TEST_BOUND`. The `http_transport.rs` tests name the same 10-second bound `LOOPBACK_TEST_LIMIT` and write the 5-second disconnect bound as the literal `Duration::from_secs(5)` twice. The check that an error names the termination (contains `TERMINATED_CAUSE` and `reconnect`, contains no `Must`) exists three times: `assert_names_the_termination` in the `lifecycle.rs` tests and an inline loop in each transport's `operations_after_terminate_report_the_terminated_transport`. Decision-log entry [7] expects the terminated text to change once `terminate()` gets a second caller, and that change then has to edit three copies of the expected text.
- Fix: In src/transport/test_support.rs, add `pub(crate) const LOOPBACK_TEST_BOUND: Duration = Duration::from_secs(10);`, `pub(crate) const DISCONNECT_BOUND: Duration = Duration::from_secs(5);`, `pub(crate) const SILENT_SERVER_CONNECTION_TIMEOUT: Duration = Duration::from_millis(300);`, `pub(crate) fn test_credentials() -> Credentials` returning `Credentials::new("sys".to_string(), "exasol".to_string())`, `pub(crate) fn silent_server_params(server: &SilentServer) -> ConnectionParams` with the body of the copy in src/transport/native/mod.rs using `SILENT_SERVER_CONNECTION_TIMEOUT`, and `pub(crate) fn assert_names_the_termination(error: &TransportError)` with the body of the helper in the src/transport/lifecycle.rs tests and a private `TERMINATED_CAUSE` constant beside it. Delete the local copies of these constants and functions from the test modules of src/transport/native/mod.rs, src/transport/websocket.rs, src/transport/lifecycle.rs, and src/transport/tls.rs, and import the shared ones: `TEST_BOUND` becomes `LOOPBACK_TEST_BOUND`, `CONNECTION_TIMEOUT` becomes `SILENT_SERVER_CONNECTION_TIMEOUT`, and `credentials()` becomes `test_credentials()`. Keep `MUST_AUTHENTICATE` local to the src/transport/lifecycle.rs tests. In the src/transport/http_transport.rs tests, replace `LOOPBACK_TEST_LIMIT` with `LOOPBACK_TEST_BOUND`, and replace the two `finish_within(Duration::from_secs(5), server.wait_for_disconnect())` literals with `DISCONNECT_BOUND`. In both `operations_after_terminate_report_the_terminated_transport` tests, replace the `for error in [error, error_after_close]` loop with `assert_names_the_termination(&error)` and `assert_names_the_termination(&error_after_close)`. Run `cargo test --lib --features websocket transport::` and `cargo clippy --all-targets --all-features -- -W clippy::all`.

### src/export/csv.rs

#### [REDUNDANT_COMMENT] The `# Timeout behavior` doc of `export_to_callback` states the span of `options.timeout_ms` twice
- Location: line 440
- Issue: Lines 435-436 state that `options.timeout_ms` bounds "SQL execution, tunnel transfer, and the callback". Line 440 opens the next paragraph with "A configured bound spans SQL execution, tunnel transfer, and the callback.", which repeats that statement.
- Fix: In src/export/csv.rs, in the `# Timeout behavior` section of the `export_to_callback` doc comment, replace "A configured bound spans SQL execution, tunnel transfer, and the callback. When it elapses, the transport is terminated only if" with "When a configured bound elapses, the transport is terminated only if". Leave the rest of that paragraph unchanged.

### src/transport/tls.rs

#### [UNTESTED_ERROR_PATH] `client_handshake` has no test for a host that is not a valid server name
- Location: lines 80-81
- Issue: `client_handshake` returns `TransportError::TlsError("Invalid server name: ...")` when `ServerName::try_from(host)` fails. Both transports now reach this branch through the shared TLS step. Neither of the two `handshake` tests nor any transport test exercises it.
- Fix: In src/transport/tls.rs, module `tests::handshake`, add `#[tokio::test] async fn client_handshake_fails_with_a_tls_error_for_a_host_that_is_not_a_server_name()`. Inside `finish_within(TEST_BOUND, async { ... })`, start `SilentServer::accepting().await`, open `TcpStream::connect((server.host.as_str(), server.port))`, call `client_handshake(tcp, "not a host name", client_config(None, false)).await`, and assert that the result matches `Err(TransportError::TlsError(message)) if message.starts_with("Invalid server name")`. Give the test no Scenario line.

### src/transport/websocket.rs

#### [UNTESTED_ERROR_PATH] No test pins that a rejected certificate on the WebSocket transport returns `TransportError::TlsError`
- Location: lines 487-498 (`connect`, TLS branch)
- Issue: `CHANGELOG.md` announces a behavior change: a failed TLS handshake on the WebSocket transport now returns `TransportError::TlsError` instead of `TransportError::WebSocketError`. The WebSocket tests cover only the TLS handshake timeout. The `tls.rs` tests cover `client_handshake` by itself. A later change to the error mapping in `WebSocketTransport::connect` would therefore pass every test.
- Fix: In the src/transport/websocket.rs tests, add `const REJECTION_TIMEOUT: Duration = Duration::from_secs(5);` and `#[tokio::test] async fn connect_reports_a_rejected_server_certificate_as_a_tls_error()`. Inside `finish_within(TEST_BOUND, async { ... })`, start `SilentServer::after_tls().await` and call `WebSocketTransport::new().connect(&ConnectionParams::new(server.host.clone(), server.port).with_timeout(REJECTION_TIMEOUT.as_millis() as u64))`. `ConnectionParams::new` keeps TLS and certificate validation on, and no native root certificate signs the self-signed certificate, so the handshake fails before the deadline. Assert that the error matches `TransportError::TlsError(_)`. Give the test no Scenario line.

## Expert fixes
[none]
