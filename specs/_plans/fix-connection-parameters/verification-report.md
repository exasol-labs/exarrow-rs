# Verification Report: fix-connection-parameters

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Unknown query keys are rejected by name, `client_name` and `client_version` reach the login on both transports, `idle_timeout` is deprecated. Version 0.22.0. |
| Code review | 3 findings (standard: 3, expert: 0), 3 fixed |

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
| Unit (production lines, default features) | 89.57 (floor 80.0, per-file floor 50.0 passes) |

### Test Results

| Type | Passed | Ignored |
|------|--------|---------|
| Unit (default) | `cargo test --lib` exit 0 | 0 |
| Unit (websocket) | `cargo test --features websocket --lib` exit 0 | 0 |
| `integration_tests` | 88 | 1 |
| `native_protocol_tests` | 14 | 0 |
| `websocket_integration_tests` | 58 | 0 |
| `driver_manager_tests` | 66 | 0 |
| `import_export_tests` (REQUIRE_EXASOL=1, ffi) | 66 | 1 |

The WebSocket-only build (`--no-default-features --features websocket --tests --no-run`) compiles. The ignored tests carry an ignore reason in the source.

### Red evidence

- `unknown_query_parameters_are_rejected_by_name` and `idle_timeout_is_accepted_with_a_deprecation_warning` failed on the old code.
- Both `dbt 9.9` integration tests failed on the old code with `exarrow-rs 0.21.2`.

### Manual Tests (Python adbc_driver_manager against `exasol-test`)

| Test | Result |
|------|--------|
| `autocommit=false` fails with `Invalid connection parameter 'autocommit'` listing `query_timeout`, without the value | ✓ |
| `client_name=dbt&client_version=9.9` gives `('dbt 9.9',)` | ✓ |
| `user=` and `password=` in the query give `1` | ✓ |
| `idle_timeout=120` connects and gives `1` | ✓ |

## Tool Evidence

### Linter

`cargo clippy --all-targets --all-features -- -W clippy::all`: 0 warnings.

### Formatter

`cargo fmt --all -- --check`: clean.

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| connection-management | auth-and-security | Unknown connection-string parameter is rejected | `src/connection/params.rs` | `unknown_query_parameters_are_rejected_by_name` | Pass |
| connection-management | auth-and-security | Client name and version identify the session | `tests/integration_tests.rs` | `test_client_name_and_version_identify_the_session_native` | Pass |
| connection-management | auth-and-security | Client name and version identify the session | `tests/websocket_integration_tests.rs` | `test_client_name_and_version_identify_the_session_websocket` | Pass |
| connection-management | auth-and-security | Client name and version identify the session | `src/transport/native/handshake.rs` | `login_packet_identifies_the_user_and_the_driver` | Pass |
| connection-management | auth-and-security | Client name and version identify the session | `src/transport/websocket.rs` | `auth_request_reports_the_client_and_the_driver` | Pass |
| connection-management | auth-and-security | Default client identification | `tests/integration_tests.rs` | `test_default_client_identification` | Pass |
| connection-management | credential-sources | Query credentials fill only what the userinfo omits | `src/connection/params.rs` | `test_parse_query_credentials_fill_only_what_userinfo_omits` | Pass |
| connection-management | session-and-lifecycle | Idle timeout | `src/connection/params.rs` | `idle_timeout_is_accepted_with_a_deprecation_warning` | Pass |
| connection-management | session-and-lifecycle | Idle timeout with an invalid value | `src/connection/params.rs` | `test_parse_invalid_idle_timeout_value` | Pass |

## Notes

- The Docker container `exasol-test` was shared with run implement-94. No test failed from interference.
- The architecture delta was re-based onto the post-#93 `specs/architecture.md` (BASE hash updated, #93's five changed lines applied).
