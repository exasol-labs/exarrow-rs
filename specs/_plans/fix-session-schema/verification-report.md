# Verification Report: fix-session-schema

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | #77 and #72 are implemented. Every check in the plan's Verification checklist passes. |
| Code review | 11 findings, 11 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ (WebSocket manual run replaced by integration tests, see Notes) |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit (production lines) | 87.77% (floor 80.0%, every file above 50.0%) |
| Integration | not measured (AGENTS.md: integration coverage is not fed to Sonar) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit | `cargo +1.92 test --lib` | 1666 | 0 |
| Unit | `--lib --features websocket` | 1710 | 0 |
| Unit | `--lib --features ffi` | 1780 | 0 |
| Integration | `integration_tests` | 85 | 1 (ignored before this work) |
| Integration | `websocket_integration_tests` | 56 | 0 |
| Integration | `native_protocol_tests` | 16 | 0 |
| Driver manager | `driver_manager_tests --include-ignored` | 65 | 0 |
| Import/export | `import_export_tests` | 61 | 1 (ignored before this work) |

All Exasol-backed suites ran with `REQUIRE_EXASOL=1` and `--test-threads=1` against `exasol-test`. The WebSocket-only test build (`--no-default-features --features websocket --tests --no-run`) builds.

### Manual Tests

| Test | Result |
|------|--------|
| URI `/ZZ_MixedCase`: `CURRENT_SCHEMA` and `adbc_current_db_schema` both `ZZ_MixedCase` | ✓ |
| URI `/ZZ_TYPO`: connect fails with "failed to set schema 'ZZ_TYPO' ... schema ZZ_TYPO not found" | ✓ |
| `db_schema` option set to `ZZ_MixedCase`, then `OPEN SCHEMA SYS`: reads `ZZ_MixedCase`, then `SYS` | ✓ |
| Failed statement, then `autocommit=false` and commit: `transaction ok` | ✓ |

## Tool Evidence

### Linter

```
cargo +1.92 clippy --all-targets --all-features -- -W clippy::all: exit 0, 0 warnings
```

### Formatter

```
cargo +1.92 fmt --all -- --check: exit 0, no changes
```

## Scenario Coverage

Every scenario title of the four delta specs appears verbatim in at least one `/// Scenario:` line in `src/` or `tests/` (grep audit). The removed scenario "Non-existent URI schema is a best-effort default" has no test, as intended.

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| connection-management | schema-activation | Schema in connection params is opened on connect | `src/adbc/connection.rs`, `tests/` | `connect_sets_the_uri_schema_as_a_session_attribute_after_login`, `test_uri_schema_is_opened_on_connect`, `test_websocket_uri_schema_is_opened_on_connect`, `test_ffi_uri_schema_is_opened_on_connect` | Pass |
| connection-management | schema-activation | Schema activation failure surfaces during connect | `src/adbc/connection.rs`, `tests/` | `connect_fails_and_closes_the_transport_for_any_rejected_uri_schema`, `test_connect_with_missing_uri_schema_fails`, `test_ffi_missing_uri_schema_fails_the_connection` | Pass |
| connection-management | schema-activation | URI schema name follows the server's case rule | `tests/` | `test_uri_schema_name_follows_the_server_case_rule`, `test_websocket_uri_schema_name_follows_the_server_case_rule` | Pass |
| connection-management | schema-activation | Set the current schema at runtime | `src/adbc/connection.rs`, `tests/` | `set_schema_sends_the_schema_attribute_and_reports_the_servers_name`, `test_set_schema_sets_the_server_current_schema` | Pass |
| connection-management | schema-activation | Current schema follows schema changes made in SQL | `src/adbc/connection.rs`, `tests/` | `current_schema_reports_the_transports_value_without_a_request`, `test_current_schema_follows_schema_changes_in_sql` | Pass |
| connection-management | schema-activation | ADBC db_schema option sets / reads / before the session exists | `tests/driver_manager_tests.rs`, `src/adbc_ffi.rs` | `test_ffi_db_schema_option_*`, `get_option_string_reports_*` | Pass |
| connection-management | session-and-lifecycle | A failed statement leaves the session usable | `src/adbc/connection.rs`, `tests/` | `failed_execute_statement_leaves_the_session_ready`, `test_failed_statement_leaves_the_session_usable`, `test_ffi_autocommit_off_after_a_failed_statement` | Pass |
| connection-management | session-and-lifecycle | A failed statement inside a transaction keeps the transaction | `src/adbc/connection.rs`, `tests/integration_tests.rs` | `failed_statement_inside_a_transaction_keeps_the_transaction`, `test_failed_statement_inside_a_transaction_keeps_the_transaction` | Pass |
| connection-management | session-and-lifecycle | An abandoned execution leaves the session state unchanged | `src/adbc/connection.rs` | `abandoned_execution_leaves_the_session_state_unchanged` | Pass |
| connection-management | session-and-lifecycle | A closed session rejects operations as closed | `src/adbc/connection.rs`, `src/connection/session.rs` | `closed_session_rejects_operations_as_closed`, `test_session_validate_ready` | Pass |
| connection-management | session-and-lifecycle | Starting a second transaction reports the active transaction | `src/adbc/connection.rs` | `begin_transaction_rejects_a_second_overlapping_transaction` | Pass |
| native-client | protocol | Set / Track the current schema attribute | `src/transport/native/mod.rs`, `tests/native_protocol_tests.rs` | `current_schema_change_reads_attribute_22_from_a_response`, `native_set_current_schema_*` | Pass |
| websocket-client | protocol | Set / Track the current schema attribute | `src/transport/websocket.rs` | `set_current_schema_*`, `response_attributes_update_the_current_schema` | Pass |

## Notes

- The #72 regression tests use a failing statement, not a missing URI schema, because #77 makes a missing URI schema fail at connect (decision [6]).
- Native `CLOSE SCHEMA` with a schema open returns an empty attribute 22, so `current_schema()` becomes `None` without an extra request. No gap.
- The WebSocket manual snippet (needs a separate `websocket` cdylib build) was not run. WebSocket behavior is covered by `websocket_integration_tests` (56 passed) and the WebSocket unit tests.
- The follow-up issue for the transport desync after a dropped execution (plan Dependencies) is not yet opened.
- The dbt adapter change must ship before or with this release (plan Dependencies).
- `ymd_hms_nanos_to_micros` is unused in the WebSocket-only build. The warning exists on main and is out of scope.
- The code review removed five `#[ignore]` attributes in `driver_manager_tests.rs` for schema tests. The suite now runs them without `--include-ignored`.
