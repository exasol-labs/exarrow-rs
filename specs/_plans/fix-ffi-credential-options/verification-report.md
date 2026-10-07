# Verification Report: fix-ffi-credential-options

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All checklist steps exit 0, all unit, driver manager, integration, and import/export tests pass, and the three manual checks give the expected output. |
| Code review | 5 findings, 5 fixed |

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
| Unit | 86.96 production lines (measured by the implementer before the review fixes; `src/connection/params.rs` 97.6%) |
| Integration | Not measured, per AGENTS.md (the `ffi` feature deadlocks under cargo-llvm-cov) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`--lib`) | 1614 | 1614 | 0 |
| Unit (`--lib --features ffi`) | 1718 | 1718 | 0 |
| Unit (`--lib --features websocket`) | 1645 | 1645 | 0 |
| Driver manager (`--include-ignored`) | 49 | 49 | 0 |
| Integration | 79 | 78 | 1 |
| Import/export | 62 | 61 | 1 |

The loopback-only run of `cargo test --lib --features ffi` (1717 tests, before the review fixes) passed with no server.

### Manual Tests

| Test | Result |
|------|--------|
| Option credentials with `client_name=dbt@ci` print `('SYS',)` | ✓ |
| Missing username raises `Username is required` and omits the password | ✓ |
| Parse error contains `position 1` and omits `Qx7` and `Kp9` | ✓ |

The implementer ran the manual checks with adbc-driver-manager 1.12.0 in a throwaway virtual environment, before the review fixes.

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -W clippy::all: exit 0, 0 warnings
```

### Formatter

```
cargo fmt --all -- --check: exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| connection-management | credential-sources | Option credentials take precedence over URI credentials | `tests/driver_manager_tests.rs` | `test_driver_manager_option_credentials_override_uri_credentials` | Pass |
| connection-management | credential-sources | Option credentials take precedence over URI credentials | `src/connection/params.rs` | `test_parse_with_credentials_options_override_userinfo` | Pass |
| connection-management | credential-sources | A single credential option replaces only its own field | `tests/driver_manager_tests.rs` | `test_driver_manager_password_option_keeps_uri_username` | Pass |
| connection-management | credential-sources | A single credential option replaces only its own field | `src/connection/params.rs` | `test_parse_with_credentials_password_option_keeps_userinfo_username` | Pass |
| connection-management | credential-sources | Option password reaches the server verbatim | `tests/driver_manager_tests.rs` | `test_driver_manager_option_password_reaches_server_verbatim` | Pass |
| connection-management | credential-sources | Option password reaches the server verbatim | `src/connection/params.rs` | `test_parse_with_credentials_keeps_option_password_verbatim` | Pass |
| connection-management | credential-sources | Option password reaches the server verbatim | `src/adbc_ffi.rs` | `test_ffi_database_option_password_is_verbatim` | Pass |
| connection-management | credential-sources | An at sign in a query parameter value does not change the host | `tests/driver_manager_tests.rs` | `test_driver_manager_at_sign_in_query_value_keeps_host` | Pass |
| connection-management | credential-sources | An at sign in a query parameter value does not change the host | `src/connection/params.rs` | `test_parse_with_credentials_at_sign_in_query_value_keeps_host` | Pass |
| connection-management | credential-sources | An at sign in a query parameter value does not change the host | `src/adbc_ffi.rs` | `test_ffi_database_at_sign_in_query_value_keeps_host` | Pass |
| connection-management | credential-sources | Missing username is rejected | `tests/driver_manager_tests.rs` | `test_driver_manager_missing_username_is_rejected` | Pass |
| connection-management | credential-sources | Missing username is rejected | `src/connection/params.rs` | `test_parse_with_credentials_requires_username` | Pass |
| connection-management | credential-sources | Missing username is rejected | `src/adbc_ffi.rs` | `test_ffi_database_without_username_rejects_connection` | Pass |
| connection-management | credential-sources | URI credentials apply when no credential option is set | `tests/driver_manager_tests.rs` | `test_driver_manager_uri_credentials_apply_without_options` | Pass |
| connection-management | credential-sources | URI credentials apply when no credential option is set | `src/connection/params.rs` | `test_parse_with_credentials_decodes_userinfo_without_options` | Pass |
| connection-management | credential-sources | Query credentials fill only what the userinfo omits | `src/connection/params.rs` | `test_parse_query_credentials_fill_only_what_userinfo_omits` | Pass |
| connection-management | credential-sources | Query user applies when only the password option is set | `src/connection/params.rs` | `test_parse_with_credentials_query_user_with_password_option` | Pass |
| connection-management | credential-sources | Query user applies when only the password option is set | `src/adbc_ffi.rs` | `test_ffi_database_query_user_with_password_option` | Pass |
| connection-management | credential-sources | Password stays out of Debug output | `src/connection/params.rs` | `test_debug_redacts_password_from_every_source` | Pass |
| connection-management | credential-sources | URI parse errors do not repeat URI values | `tests/driver_manager_tests.rs` | `test_driver_manager_uri_parse_error_omits_uri_values` | Pass |
| connection-management | credential-sources | URI parse errors do not repeat URI values | `src/connection/params.rs` | `test_parse_errors_do_not_repeat_uri_values` | Pass |
| connection-management | credential-sources | URI parse errors do not repeat URI values | `src/connection/params.rs` | `test_parse_error_for_unencoded_password_omits_password` | Pass |
| connection-management | credential-sources | URI parse errors do not repeat URI values | `src/adbc_ffi.rs` | `test_ffi_database_parse_error_omits_uri_values` | Pass |

## Notes

- The websocket-only build warns that `ymd_hms_nanos_to_micros` in `src/types/conversion.rs` is unused. The function and the warning exist at HEAD, and this change does not touch the file.
- The local default toolchain is 1.91.1 and CI pins 1.92.0. The implementer ran fmt and clippy on 1.92 as well, and both pass.
- The two ignored tests are reasoned `#[ignore]` attributes in files this change does not touch.
- Behavior change by design (decision-log entry [11]): `exasol://alice@host?password=pw` now logs in with `pw`.
