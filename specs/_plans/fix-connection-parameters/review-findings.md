# Code Review Findings: fix-connection-parameters

## Summary
- Files reviewed: 14
- Total findings: 3 (standard: 3, expert: 0)
- Evidence: `cargo clippy --all-targets --all-features -- -W clippy::all` reports 0 warnings. `cargo test --lib -- connection::params transport::protocol transport::native::handshake` passes 129 tests. `cargo test --features websocket --lib -- auth_request_reports` passes. Against the running `exasol-test` container, the three new integration tests pass: `test_client_name_and_version_identify_the_session_native`, `test_default_client_identification`, and `test_client_name_and_version_identify_the_session_websocket`.

## Standard fixes

### tests/integration_tests.rs

#### [VAGUE_TEST_NAME] The native client-identification test does not select the native transport
- Location: line 4025, `test_client_name_and_version_identify_the_session_native`
- Issue: The test name says it checks the native transport. The test builds its connection string from `get_test_connection_string()`, which sets no `transport` key, so the test runs on whatever transport is the default. `cargo test --no-default-features --features websocket --test integration_tests -- --list` lists this test, so in a WebSocket-only build it runs over WebSocket under a native name. The existing native tests in this file, such as `test_prepared_non_ascii_varchar_parameter_round_trip` (line 2247), carry `#[cfg(feature = "native")]` and select the transport with `get_test_connection_with_transport("native")`.
- Fix: In tests/integration_tests.rs, add `#[cfg(feature = "native")]` above `#[tokio::test]` on `test_client_name_and_version_identify_the_session_native`, build its connection string as `format!("{}&client_name=dbt&client_version=9.9", get_test_connection_string_with_transport("native"))`, and add `get_test_connection_string_with_transport` to the `#[cfg(feature = "native")] use common::{...}` import at line 87. Leave `test_default_client_identification` unchanged. Run `cargo test --test integration_tests -- client_name_and_version default_client_identification` and `cargo test --no-default-features --features websocket --tests --no-run`.

### src/transport/protocol.rs

#### [INFORMATION_LEAKAGE] The default client version is decided in two modules
- Location: `Credentials::new`, line 104; `ConnectionBuilder::build` in src/connection/params.rs, line 383
- Issue: The default client identity is "the driver name and the crate version". The change moves the name into the shared constant `DRIVER_NAME` in src/lib.rs, but the version default stays a separate `env!("CARGO_PKG_VERSION")` in both `Credentials::new` and `ConnectionBuilder::build`. A change to the default client version must edit both modules, and nothing enforces that they agree. `test_default_client_identification` checks only the `ConnectionBuilder` path.
- Fix: In src/lib.rs, below `DRIVER_NAME`, add `pub(crate) const DRIVER_VERSION: &str = env!("CARGO_PKG_VERSION");` with the doc comment `/// Driver version reported to the server at login, and the default client version.`. In src/transport/protocol.rs, import `crate::DRIVER_VERSION` and set `client_version: DRIVER_VERSION.to_string()` in `Credentials::new`. In src/connection/params.rs, import `crate::DRIVER_VERSION` and replace `.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())` in `ConnectionBuilder::build` with `.unwrap_or_else(|| DRIVER_VERSION.to_string())`. Run `cargo test --lib -- connection::params transport::protocol`.

### src/transport/native/handshake.rs

#### [MISSING_DESIGN_INTENT] `build_login_packet` takes the password but its doc does not say the packet excludes it
- Location: lines 25-31, doc comment of `build_login_packet`
- Issue: The public function now takes `&Credentials`, which holds the plaintext password. The first login packet is sent before any password encryption, and the function correctly writes only the username, the client name, the client version, the driver name, and the OS. The doc comment states the byte layout only, so a reader of the signature cannot tell that the password stays out of this packet. AGENTS.md requires that the driver never exposes connection passwords.
- Fix: In src/transport/native/handshake.rs, add one line to the doc comment of `build_login_packet`, after the first sentence: `/// It sends the username and the client identity from \`credentials\`, never the password, which \`build_auth_message\` sends RSA-encrypted.`. Change no code.

## Expert fixes
[none]
