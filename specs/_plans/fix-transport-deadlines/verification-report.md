# Verification Report: fix-transport-deadlines

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All checklist commands exit 0, all 41 scenario-coverage tests pass, and the three silent-server manual tests fail after 3 seconds with the expected step name. |
| Code review | 4 findings, 4 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit (production lines, default features) | 88.05 (floor 80.0, per-file floor 50.0 passed) |
| Integration | Not measured (FFI feature cannot run under cargo-llvm-cov) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test --lib`) | 1639 | 1639 | 0 |
| Unit (`--lib --features websocket`) | 1675 | 1675 | 0 |
| `integration_tests` | 81 | 80 | 1 |
| `websocket_integration_tests` | 51 | 51 | 0 |
| `import_export_tests` | 62 | 61 | 1 |
| `native_protocol_tests` | 14 | 14 | 0 |
| `native_transport_smoke_test` | 4 | 4 | 0 |
| `driver_manager_tests` | 49 | 49 | 0 |

### Manual Tests

Silent TCP server on 127.0.0.1:18001, connection through `adbc_driver_manager` and the release cdylib (`ffi websocket`).

| Test | Result |
|------|--------|
| `timeout=3` with TLS: error after 3 s, `Connection timeout after 3000ms (TLS handshake)` | ✓ |
| `timeout=3&tls=false`: error after 3 s, `Connection timeout after 3000ms (login)` | ✓ |
| `timeout=3&tls=false&transport=websocket`: error after 3 s, `Connection timeout after 3000ms (WebSocket upgrade)` | ✓ |
| `cargo test --lib transport::lifecycle`, `connect_with_timeout_`, `bounds_a_stalled_tunnel_setup`, `test_export_to_callback_` | ✓ (part of the unit runs above) |
| `test_csv_export_explicit_timeout_terminates_connection` against Exasol | ✓ (part of `integration_tests`) |

## Tool Evidence

### Linter

```
cargo +1.92 clippy --all-targets --all-features -- -W clippy::all: exit 0, 0 warnings
cargo +1.92 build, cargo +1.92 build --release --features ffi: 0 warnings
cargo +1.92 test --no-default-features --features websocket --tests --no-run: exit 0
```

### Formatter

```
cargo +1.92 fmt --all -- --check: no changes
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| connection-management | connection-timeout | One deadline bounds every connection setup step | `src/transport/deadline.rs` | `a_later_step_gets_only_the_time_left_by_earlier_steps` | Pass |
| connection-management | connection-timeout | One deadline bounds every connection setup step | `src/transport/deadline.rs` | `an_elapsed_deadline_names_the_label_the_budget_and_the_step` | Pass |
| connection-management | connection-timeout | One deadline bounds every connection setup step | `src/transport/deadline.rs` | `a_step_error_before_the_deadline_is_returned_unchanged` | Pass |
| connection-management | connection-timeout | One deadline bounds every connection setup step | `src/transport/lifecycle.rs` | `a_login_gets_only_the_time_left_after_connect` | Pass |
| connection-management | connection-timeout | One deadline bounds every connection setup step | `src/transport/lifecycle.rs` | `a_login_error_before_the_deadline_is_returned_and_keeps_the_transport_connected` | Pass |
| connection-management | connection-timeout | Connection timeout default and maximum | `src/connection/params.rs` | `test_builder_default_values` (existing, gains the Scenario line) | Pass |
| connection-management | connection-timeout | Connection timeout default and maximum | `src/connection/params.rs` | `test_builder_validation_timeout` (existing, asserts `InvalidParameter`) | Pass |
| connection-management | connection-timeout | Connection timeout default and maximum | `src/connection/params.rs` | `test_parse_connection_timeout_default_and_maximum` | Pass |
| connection-management | connection-timeout | Server that never answers the TLS handshake | `src/transport/native/mod.rs` | `connect_fails_at_the_tls_handshake_when_the_server_never_answers` | Pass |
| connection-management | connection-timeout | Server that never answers the TLS handshake | `src/transport/websocket.rs` | `connect_fails_at_the_tls_handshake_when_the_server_never_answers` | Pass |
| connection-management | connection-timeout | Server that never answers the TLS handshake | `tests/integration_tests.rs` | `test_connection_timeout_fails_a_silent_tls_server_at_the_tls_handshake` | Pass |
| connection-management | connection-timeout | Server that never answers the login | `src/transport/lifecycle.rs` | `a_login_that_runs_out_of_time_releases_the_connection_and_records_closed` | Pass |
| connection-management | connection-timeout | Server that never answers the login | `src/transport/native/mod.rs` | `authenticate_fails_at_login_when_the_server_goes_silent_after_tls` | Pass |
| connection-management | connection-timeout | Server that never answers the login | `src/transport/websocket.rs` | `authenticate_fails_at_login_when_the_server_goes_silent_after_the_upgrade` | Pass |
| connection-management | connection-timeout | Server that never answers the login | `tests/integration_tests.rs` | `test_connection_timeout_fails_a_silent_server_at_login_without_tls` | Pass |
| connection-management | connection-timeout | WebSocket server that never answers the upgrade | `src/transport/websocket.rs` | `connect_fails_at_the_websocket_upgrade_when_the_server_never_answers` | Pass |
| connection-management | connection-timeout | WebSocket server that never answers the upgrade | `tests/websocket_integration_tests.rs` | `test_ws_connection_timeout_fails_a_silent_server_at_the_websocket_upgrade` | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `src/transport/lifecycle.rs` | `terminate_releases_the_connection_without_io_and_records_terminated` | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `src/transport/lifecycle.rs` | `close_after_terminate_succeeds_without_io_and_keeps_reporting_termination` | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `src/transport/native/mod.rs` | `terminate_drops_the_session_and_reports_the_transport_disconnected` (existing) | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `src/transport/native/mod.rs` | `operations_after_terminate_report_the_terminated_transport` | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `src/transport/websocket.rs` | `terminate_drops_the_session_and_reports_the_transport_disconnected` (existing) | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `src/transport/websocket.rs` | `operations_after_terminate_report_the_terminated_transport` | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `tests/integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection` (updated) | Pass |
| connection-management | session-and-lifecycle | Terminate a connection whose in-flight response is no longer trusted | `tests/websocket_integration_tests.rs` | `test_terminate_marks_websocket_transport_disconnected` (updated) | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `src/transport/lifecycle.rs` | `every_guard_names_the_termination_on_a_terminated_transport` | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `src/transport/lifecycle.rs` | `close_after_terminate_succeeds_without_io_and_keeps_reporting_termination` | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `src/transport/lifecycle.rs` | `close_disconnects_an_open_transport_and_records_closed_not_terminated` | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `src/transport/native/mod.rs` | `operations_after_terminate_report_the_terminated_transport` | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `src/transport/websocket.rs` | `operations_after_terminate_report_the_terminated_transport` | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `tests/integration_tests.rs` | `test_csv_export_explicit_timeout_terminates_connection` (updated) | Pass |
| connection-management | session-and-lifecycle | Operations after an export timeout name the termination | `tests/websocket_integration_tests.rs` | `test_terminate_marks_websocket_transport_disconnected` (updated) | Pass |
| import-export | http-transport | Tunnel setup fails with the step named when the peer stops answering | `src/transport/http_transport.rs` | `connect_with_timeout_fails_at_the_exa_handshake_when_the_peer_never_answers` | Pass |
| import-export | http-transport | Tunnel setup fails with the step named when the peer stops answering | `src/transport/http_transport.rs` | `connect_with_timeout_fails_at_the_tls_handshake_when_the_peer_goes_silent_after_the_handshake` | Pass |
| import-export | http-transport | Tunnel setup fails with the step named when the peer stops answering | `src/transport/deadline.rs` | `an_elapsed_deadline_names_the_label_the_budget_and_the_step` | Pass |
| import-export | http-transport | EXA tunneling handshake in client mode (regression) | `src/transport/http_transport.rs` | `connect_with_timeout_returns_the_internal_address_within_the_bound` | Pass |
| import-export | csv-export-timeout | Tunnel setup is bounded by 30 seconds by default | `src/export/csv.rs` | `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` | Pass |
| import-export | csv-export-timeout | Tunnel setup is bounded by 30 seconds by default | `src/export/csv.rs` | `test_export_to_callback_bounds_a_stalled_tunnel_setup_even_with_an_export_timeout` | Pass |
| import-export | csv-export-timeout | Tunnel setup is bounded by 30 seconds by default | `src/import/csv.rs` | `test_import_from_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` | Pass |
| import-export | csv-export-timeout | No client-side export timeout by default | `src/export/csv.rs` | `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset` (existing) | Pass |
| import-export | csv-export-timeout | No client-side export timeout by default | `src/export/csv.rs` | `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` | Pass |

## Notes

- Verified with toolchain 1.92 (CI pin). The local default is 1.91.
- On the WebSocket transport, a TLS handshake failure is now `TransportError::TlsError`. A TCP connect or upgrade failure that is not a timeout stays `WebSocketError`, without tungstenite's `IO error: ` prefix.
- A login timeout reaches the Python caller as `Authentication failed: ... (login)`, because `connect_with_transport` maps every login error to `AuthenticationFailed` (decision-log entry [5]).
- `--no-default-features` builds without the `websocket` feature show dead-code warnings and a test compile error in `src/transport/tls.rs`. Not a CI configuration, and not checked against the pre-change tree.
- Pre-existing doc issues left alone: a broken anchor `import-export.md#websocket-tls-vs-http-transport-tls` in `docs/setup-and-connect.md`, and a § Schema and Session Behavior claim that a missing schema fails `connect()`.
