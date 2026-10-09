# Plan: fix-session-schema

## Summary

This plan fixes #77 and #72 in one PR. The driver sets the session's current schema as a protocol session attribute instead of running `OPEN SCHEMA` (#77). A connect fails when the URI schema does not exist. `Connection::current_schema()` and the ADBC `adbc.connection.db_schema` option report the server's current schema. A failed statement no longer leaves the session in a state that makes the next transaction start fail with "Connection is closed" (#72).

## Context

- `Connection::set_schema` runs `OPEN SCHEMA {name}` without quoting, so the server opens an existing `"ZZ_MixedCase"` as `ZZ_MIXEDCASE` (not found, swallowed) and rejects `zz-hyphen` as a syntax error (#77 rows 1 and 2, reproduced on this branch).
- Since #39 (0.12.5), `connect_with_transport` swallows a failed `OPEN SCHEMA` whose error text contains "not found", so a mistyped URI schema connects with no current schema and no warning (#77 row 3, reproduced). ADR `uri-specified-schema-is-best-effort-default` records that behavior.
- The FFI `adbc.connection.db_schema` option only writes and reads a field on `FfiConnection`. The server's current schema never changes, and a schema from the URI reads back as `NOT_FOUND` (#77 row 4, reproduced).
- Each of the five `execute_*` methods in `src/adbc/connection.rs` sets `SessionState::Executing` and restores the state only on success. `Session::begin_transaction` rejects the leaked `Executing` as `ConnectionError::ConnectionClosed` (#72, reproduced through a missing URI schema and through a failing `SELECT` on a healthy connection).
- `SessionState::Initializing`, `Idle`, and `Error` are never set outside tests. `validate_ready` accepts `Executing` through `is_active`, and `begin_transaction` rejects it through `can_execute`.
- A protocol spike against the `exasol-test` container on this branch (2026-10-09) established the server contract that the decision log relies on:
  - Native: attribute 22 (current schema) in the phase-1 login packet opens the schema; in the phase-2 password message the server ignores it. `CMD_SET_ATTRIBUTES` with attribute 22 opens the schema, and its response does not echo the attribute. `CMD_GET_ATTRIBUTES` reports attribute 22 only while a schema is open.
  - Native: the response to `OPEN SCHEMA` and `CREATE SCHEMA` carries attribute 22 with the new schema. The response to `DROP SCHEMA` of the current schema carries attribute 22 empty. The responses to `SELECT 1`, a failing `SELECT`, and `CLOSE SCHEMA` with no schema open carry no attribute 22.
  - Native, unverified: the spike ran `CLOSE SCHEMA` only after `DROP SCHEMA` had cleared the current schema. The native response to `CLOSE SCHEMA` with a schema open is therefore not established. The plan expects attribute 22 empty, as in the `DROP SCHEMA` response and in the WebSocket `CLOSE SCHEMA` response. `test_current_schema_follows_schema_changes_in_sql` verifies it.
  - WebSocket: `currentSchema` in the login credentials message opens the schema. `setAttributes` with `currentSchema: "sys"` opens `SYS`, and its response carries no attributes. `getAttributes` reports `currentSchema`, empty when no schema is open.
  - WebSocket: the `execute` responses to `OPEN SCHEMA`, `CREATE SCHEMA`, `DROP SCHEMA` of the current schema, and `CLOSE SCHEMA` carry a top-level `attributes.currentSchema`. The value is empty after `DROP SCHEMA` and `CLOSE SCHEMA`. The response to `SELECT 1` carries no `currentSchema`. The plan reviewer recorded these responses with a WebSocket probe (pyexasol 2.2.1) against the same container.
  - WebSocket: the top-level `attributes` object of a `getAttributes` response mixes value types, for example `autocommit: true`, `queryTimeout: 0`, and `currentSchema: ""`.
  - Both transports: a missing schema at login fails with SQL state `08004` and "Connection exception - schema X not found", the same SQL state as a wrong password ("Connection exception - authentication failed."). A missing schema in `setAttributes` fails with SQL state `42000` and "schema X not found", and the session keeps its previous current schema.
- The dbt Exasol adapter (`dbt-labs/dbt`, `crates/dbt-auth/src/exasol/mod.rs`) puts the target schema into the URI before the schema exists. #39 was made for that case.
- The branch is based on `main`. On `main`, `Cargo.toml` is at version 0.20.1, and the top header of `CHANGELOG.md` is `## 0.20.1`.
- The architecture changes in data flow, interfaces, and transport ownership: see `architecture.md` in this plan directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| connection-management/schema-activation | CHANGED | `connection-management/schema-activation/spec.md` |
| connection-management/session-and-lifecycle | CHANGED | `connection-management/session-and-lifecycle/spec.md` |
| native-client/protocol | CHANGED | `native-client/protocol/spec.md` |
| websocket-client/protocol | CHANGED | `websocket-client/protocol/spec.md` |

## Impact

- **Breaking:** a connection whose URI or `ConnectionParams` names a schema that does not exist fails at connect with a `ConnectionError` that names the schema. This reverts the 0.12.5 behavior from #39. A dbt run whose target schema does not exist yet, such as the first run on a new environment, fails at connect until the dbt adapter stops putting a not-yet-existing schema into the URI (see Dependencies).
- **Changed:** the server resolves a URI schema by exact name first, then by its upper-case form. `/myschema` still opens `MYSCHEMA`. When both `myschema` and `MYSCHEMA` exist, `/myschema` now opens `myschema`, where `OPEN SCHEMA myschema` opened `MYSCHEMA`.
- **Changed:** `Connection::current_schema()` reports the name of the schema the server opened, for example `MY_SCHEMA` for the URI schema `my_schema`, and follows `OPEN SCHEMA`, `CREATE SCHEMA`, `CLOSE SCHEMA`, and `DROP SCHEMA` in SQL.
- **Fixed:** an existing mixed-case or special-character URI schema, such as `ZZ_MixedCase` or `zz-hyphen`, becomes the current schema.
- **Fixed:** setting `adbc.connection.db_schema` changes the server's current schema, and reading it returns the server's value.
- **Fixed:** a failed statement no longer makes `begin_transaction()` or ADBC `AutoCommit=false` fail with "Transaction error: Connection is closed".
- **Breaking (Rust API):** `TransportProtocol` gains `set_current_schema`, `refresh_current_schema`, and `current_schema`, so an implementation outside the crate must add them. `SessionState` loses `Initializing`, `Executing`, `Idle`, and `Error`, `SessionState::is_active` is removed, and `Session::current_schema` and `Session::set_current_schema` are removed.
- **Changed (docs):** `docs/setup-and-connect.md` no longer lists `currentSchema` as a URI query parameter, because the driver never sends it.
- The version moves from 0.20.1 to 0.21.0.

## Dependencies

- Release ordering risk: the dbt adapter change in `dbt-labs/dbt` (connect without the schema, then `CREATE SCHEMA IF NOT EXISTS`, which also sets the current schema) must ship before or together with the driver release that contains this change. The change is in an external repository and is not part of this plan.
- Integration and driver manager tests need the `exasol-test` Docker container and a release cdylib from `cargo build --release --features ffi`.
- Task 1.10 is the only version bump of this PR. The version-bump step of the PR pipeline (`/speq:implement-pr` A3, `speq-implement-pr` step 3) MUST leave `Cargo.toml` at 0.21.0 and the `## 0.21.0` header of `CHANGELOG.md` unchanged, because this plan has no `workspace/version` delta and that step's Conventional Commits fallback would bump this `fix` plan a second time.
- Follow-up, tracked: issue #73 covers the remaining claim in `docs/setup-and-connect.md` § Session Attributes that the driver forwards unrecognized URI query parameters to the server. This plan deletes only the `currentSchema` row of that table (decision [9]).
- Follow-up, not yet tracked: a caller that drops an execution before it completes can leave an unread response on the transport (decision [4]). No issue tracks this transport desync today. The PR author opens a follow-up issue for it together with this PR.

## Migration

| Current | New |
|---------|-----|
| A missing URI schema connects with no current schema | A missing URI schema fails the connect; connect without a schema and run `CREATE SCHEMA IF NOT EXISTS` instead |
| `current_schema()` returns the URI or `set_schema()` text as given | `current_schema()` returns the name of the schema the server opened |
| `adbc.connection.db_schema` set only stores the value | The set changes the server's current schema and fails for a schema the server rejects |
| `SessionState::Executing`, `Idle`, `Error`, `Initializing` | Removed; the session is `Ready`, `InTransaction`, `Closing`, or `Closed` |
| `Session::current_schema()` | `Connection::current_schema()` |

## Implementation Tasks

### 1. #77: current schema as a session attribute

- [ ] 1.1 Transport contract: add `set_current_schema(&mut self, schema: &str) -> Result<(), TransportError>`, `refresh_current_schema(&mut self) -> Result<Option<String>, TransportError>` (one get-attributes request that records and returns the value), and `current_schema(&self) -> Option<String>` (the value the server last reported) to `TransportProtocol` in `src/transport/protocol.rs`. The doc comments state that the transport is the single owner of the server-reported current schema. Extend `MockTransport` and `StalledQueryTransport` in `src/transport/test_support.rs`. [expert]
- [ ] 1.2 Native transport (`src/transport/native/`): implement the current-schema attribute in these steps. [expert]
  - Add `ATTR_CURRENT_SCHEMA: u16 = 22` to `constants.rs`.
  - Every response passes through `receive_into_buf`, including the login responses that `receive_message` reads. `receive_into_buf` does not know which command a response answers. It therefore applies only the command-independent rule, through a pure helper that reads the response's attribute block and returns the change. A present value is recorded, and an empty value means none. An absent attribute 22 leaves the value unchanged.
  - The "absent means none" rule applies only to a `CMD_GET_ATTRIBUTES` response. The two call sites that send `CMD_GET_ATTRIBUTES` apply it after the receive: `refresh_current_schema` and the login's existing `CMD_GET_ATTRIBUTES` step in `login_exchange`. Both call a second pure helper that maps the parsed attribute set of a get-attributes response to the current schema, with an absent attribute 22 as none. The login step seeds the value this way.
  - Implement `set_current_schema` as `CMD_SET_ATTRIBUTES` with attribute 22, then `refresh_current_schema`. A rejected `CMD_SET_ATTRIBUTES` returns the server's error before the refresh, so the recorded value stays unchanged.
  - Do not add attribute 22 to either login message.
  - Add the unit tests of both helpers listed in Scenario Coverage.
- [ ] 1.3 WebSocket transport (`src/transport/websocket.rs`, `src/transport/messages.rs`): implement the current-schema attribute in these steps. [expert]
  - Add `SetAttributesRequest::current_schema`, a `getAttributes` request, and its response type.
  - `send_receive<T, R>` deserializes each response straight into `R` in one JSON parse. No response type in `messages.rs` has a top-level `attributes` field today. Add a response-attributes type that declares only `currentSchema` as an optional string. Serde skips the undeclared members, so the type accepts the mixed value types of a `getAttributes` response (`autocommit: true`, `queryTimeout: 0`, `currentSchema: ""`). Do not reuse the `HashMap<String, String>` pattern of `ExecuteResponseData.attributes`, because it rejects booleans and numbers.
  - Add an optional top-level `attributes` field of that type to every response type that `send_receive` reads: `PublicKeyResponse`, `LoginResponse`, `DisconnectResponse`, `ExecuteResponse`, `FetchResponse`, `CloseResultSetResponse`, `CreatePreparedStatementResponse`, `ClosePreparedStatementResponse`, `SetAttributesResponse`, and the new `getAttributes` response type.
  - Add a trait that exposes the response-attributes field, implement it for each of these types, and bound `R` in `send_receive` on it. After the parse, `send_receive` records the value: a present value is recorded, an empty value means none, and an absent member leaves the value unchanged. Do not wrap responses in a `serde(flatten)` envelope, because flatten buffers the whole response, and a large fetch response would be held twice.
  - Implement `set_current_schema` as `setAttributes`, then `refresh_current_schema` (`getAttributes`). A rejected `setAttributes` returns the server's error before the refresh, so the recorded value stays unchanged.
  - Add the unit tests with `FakeWebSocketServer` listed in Scenario Coverage. The fake `getAttributes` response holds mixed value types, as the server sends them.
- [ ] 1.4 Connection (`src/adbc/connection.rs`, `src/connection/session.rs`): `set_schema` validates the session, then calls `TransportProtocol::set_current_schema` and never sends `OPEN SCHEMA`. `current_schema()` returns the transport's recorded value without a server request. Add `pub(crate) async fn refresh_current_schema(&self) -> Result<Option<String>, QueryError>` for the FFI read. Remove `Session::current_schema`, `Session::set_current_schema`, the `current_schema` field, and `test_session_schema`. Replace `set_schema_opens_the_schema_and_records_it_on_the_session` with the unit tests in Scenario Coverage.
- [ ] 1.5 Connect (`Connection::connect_with_transport`): after authentication and the query-timeout push, set a URI or `ConnectionParams` schema through `set_schema`. On any error, shut the transport down and return `ConnectionError::ConnectionFailed` whose message names the schema and contains the server's message, with no inspection of the error text. Delete `schema_open_error_is_missing_schema` and its unit tests `schema_open_error_missing_schema_is_recognized` and `schema_open_error_unrelated_is_fatal`. Replace `connect_opens_the_schema_named_in_the_connection_uri`, `connect_survives_a_schema_from_the_uri_that_does_not_exist_yet`, and `connect_closes_the_transport_when_opening_the_uri_schema_fails_fatally` with the unit tests in Scenario Coverage.
- [ ] 1.6 FFI (`src/adbc_ffi.rs`, `FfiConnection` option handling only; bound execute stays untouched): `set_option(CurrentSchema)` checks the value type, calls `ensure_connected`, and calls `Connection::set_schema`. `get_option_string(CurrentSchema)` on an established session calls `Connection::refresh_current_schema` and reports `NOT_FOUND` when there is no current schema. Before the session exists, it returns `params.schema` without dialing, or `NOT_FOUND`. Remove the `current_schema` field and the unit test `set_option_stores_the_current_schema`. Add the unit test for the read before the session exists.
- [ ] 1.7 Rust integration tests for the schema-activation and native-client/protocol scenarios, as listed in Scenario Coverage: `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, and `tests/native_protocol_tests.rs`.
  - Rewrite `test_connect_with_nonexistent_uri_schema_succeeds` and `test_uri_schema_missing_is_best_effort_via_adbc` into failing-connect tests.
  - Each test creates schemas with unique names, quotes them where the case matters, and drops them afterwards.
  - Both case-rule tests create `"zz_both"` and `ZZ_BOTH` and assert that the URI schema `zz_both` opens `zz_both`.
  - Both `..._follows_schema_changes_in_sql` tests run `CLOSE SCHEMA` with a schema open and assert that `current_schema()` returns `None`. The native test verifies the unverified native `CLOSE SCHEMA` result in Context. If that response carries no attribute 22, the implementer reports the gap and does not add a server request, because the scenario forbids one.
  - Rewrite the section comment above `test_uri_schema_is_opened_on_connect` in `tests/integration_tests.rs` and the doc comment of that test, which describe `OPEN SCHEMA` on connect, to describe the set-attributes command.
- [ ] 1.8 Driver manager tests for the schema-activation scenarios in `tests/driver_manager_tests.rs`, as listed in Scenario Coverage, against a fresh `cargo build --release --features ffi`.
  - `test_ffi_db_schema_option_sets_the_server_current_schema` sets the option both at connection creation and after it.
  - Rewrite the doc comment of `test_ffi_uri_schema_is_opened_on_connect`, which describes "auto-OPEN-SCHEMA" and the `OPEN SCHEMA` await, to describe the set-attributes command.
- [ ] 1.9 Documentation: update `docs/setup-and-connect.md`, `README.md`, and `specs/mission.md`.
  - In `docs/setup-and-connect.md`, state that the URI schema must exist.
  - Explain the server's case rule with the `ZZ_MixedCase`, `zz-hyphen`, and `myschema`/`MYSCHEMA` examples.
  - Describe `set_schema()`, `current_schema()`, and the `adbc.connection.db_schema` option.
  - Remove every `OPEN SCHEMA` reference, including the connection-timeout paragraph.
  - Delete the `currentSchema` row from the § Session Attributes table, because the driver never sends that URI query parameter.
  - Update the Quick Start comment in `README.md`.
  - Update Core Capability 9 in `specs/mission.md` to "a default schema from the URI or connection parameters that must exist, set as a session attribute on connect".
- [ ] 1.10 Release metadata: bump the version from 0.20.1 to 0.21.0. Task 1.10 is the only version bump of this PR. The version-bump step of the PR pipeline MUST leave the 0.21.0 version and the `## 0.21.0` header unchanged (see Dependencies).
  - Set `version = "0.21.0"` in `Cargo.toml`.
  - Refresh `Cargo.lock` with `cargo build`.
  - Add a `## 0.21.0` section above `## 0.20.1` in `CHANGELOG.md` with these #77 entries. The last entry ends with "Fixes #77".
    - Breaking: a missing URI schema fails the connect, with the dbt note.
    - Breaking: `TransportProtocol` gains three methods.
    - Breaking: the `Session` schema methods are removed.
    - Changed: the server's case rule decides which schema a URI schema opens.
    - Changed: `current_schema()` reports the server's name for the schema.
    - Fix: mixed-case and special-character URI schemas become the current schema.
    - Fix: the `adbc.connection.db_schema` option sets and reads the server's current schema.
    - Fix: `current_schema()` follows schema changes made in SQL.

### 2. #72: session state survives a failed statement

- [ ] 2.1 `src/connection/session.rs`: reduce `SessionState` to `Ready`, `InTransaction`, `Closing`, and `Closed`, and remove `SessionState::is_active`. `validate_ready` accepts only `can_execute()` states and reports `Closing` and `Closed` as `ConnectionError::ConnectionClosed`. `begin_transaction` reports `ConnectionClosed` only for `Closing` and `Closed`, and reports an active transaction as "Transaction already active". Update `test_session_state_transitions`, `test_session_validate_ready`, and `test_session_state_checks` to the four states.
- [ ] 2.2 `src/adbc/connection.rs`: delete the five `set_state(SessionState::Executing)` calls and `update_session_state_after_query`, so no execution path changes the session state. `Connection::begin_transaction` checks the session state and the active-transaction flag before it sends `set_autocommit(false)`. Update `execute_statement_rejects_a_closed_session_as_an_invalid_state` to assert the "Connection is closed" text, and add the unit tests in Scenario Coverage.
- [ ] 2.3 Integration tests for the session-and-lifecycle scenarios, as listed in Scenario Coverage: `tests/integration_tests.rs` and `tests/driver_manager_tests.rs`.
- [ ] 2.4 Add the #72 entries to the `## 0.21.0` section of `CHANGELOG.md`: Fix (a failed statement no longer makes `begin_transaction()` or ADBC `AutoCommit=false` fail with "Connection is closed", ending with "Fixes #72") and Breaking (removed `SessionState` variants and `SessionState::is_active`).

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Current schema attribute (#77) | 1.1-1.10 | none | spec deltas `connection-management/schema-activation`, `connection-management/session-and-lifecycle` (Session attributes), `native-client/protocol`, `websocket-client/protocol`, plan `architecture.md`; `src/transport/protocol.rs`, `src/transport/test_support.rs`, `src/transport/messages.rs`, `src/transport/websocket.rs`, `src/transport/native/constants.rs`, `src/transport/native/mod.rs`, `src/adbc/connection.rs` (connect, `set_schema`, `current_schema`), `src/connection/session.rs` (current schema), `src/adbc_ffi.rs` (`FfiConnection` options), `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `tests/native_protocol_tests.rs`, `tests/driver_manager_tests.rs`, `docs/setup-and-connect.md`, `README.md`, `specs/mission.md`, `Cargo.toml`, `CHANGELOG.md` |
| B: Session state (#72) | 2.1-2.4 | A (shares `src/adbc/connection.rs`, `src/connection/session.rs`, the integration test files, and `CHANGELOG.md`; #72 lands after #77) | spec delta `connection-management/session-and-lifecycle` (Background and the five new scenarios); `src/connection/session.rs` (`SessionState`, `validate_ready`, `begin_transaction`), `src/adbc/connection.rs` (`execute_*` methods, `begin_transaction`), `src/transport/test_support.rs` (`StalledQueryTransport`), `tests/integration_tests.rs`, `tests/driver_manager_tests.rs`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/adbc/connection.rs` `schema_open_error_is_missing_schema` | The connect fails for every schema rejection, with no error-text matching |
| Test | `src/adbc/connection.rs` `schema_open_error_missing_schema_is_recognized`, `schema_open_error_unrelated_is_fatal`, `connect_survives_a_schema_from_the_uri_that_does_not_exist_yet` | Test the removed best-effort behavior |
| Method and field | `src/connection/session.rs` `Session::current_schema`, `Session::set_current_schema`, field `current_schema` | The transport records the server-reported current schema |
| Test | `src/connection/session.rs` `test_session_schema` | Tests the removed field |
| Field | `src/adbc_ffi.rs` `FfiConnection::current_schema` | The option reads and writes the server's value |
| Test | `src/adbc_ffi.rs` `set_option_stores_the_current_schema` | Tests the removed stub |
| Method | `src/adbc/connection.rs` `update_session_state_after_query` | Execution no longer changes the session state |
| Enum variants | `src/connection/session.rs` `SessionState::{Initializing, Executing, Idle, Error}` | Never set, or set only to be restored |
| Method | `src/connection/session.rs` `SessionState::is_active` | Same states as `can_execute` once the variants are gone |
| Test | `tests/integration_tests.rs` `test_connect_with_nonexistent_uri_schema_succeeds`, `test_uri_schema_missing_is_best_effort_via_adbc` | Rewritten as failing-connect tests |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| schema-activation: Schema in connection params is opened on connect | Unit | `src/adbc/connection.rs` | `connect_sets_the_uri_schema_as_a_session_attribute_after_login` |
| schema-activation: Schema in connection params is opened on connect | Integration | `tests/integration_tests.rs` | `test_uri_schema_is_opened_on_connect` |
| schema-activation: Schema in connection params is opened on connect | Integration | `tests/websocket_integration_tests.rs` | `test_websocket_uri_schema_is_opened_on_connect` |
| schema-activation: Schema in connection params is opened on connect | Integration | `tests/driver_manager_tests.rs` | `test_ffi_uri_schema_is_opened_on_connect` |
| schema-activation: Schema activation failure surfaces during connect | Unit | `src/adbc/connection.rs` | `connect_fails_and_closes_the_transport_for_any_rejected_uri_schema` |
| schema-activation: Schema activation failure surfaces during connect | Integration | `tests/integration_tests.rs` | `test_connect_with_missing_uri_schema_fails`, `test_uri_schema_missing_fails_via_adbc` |
| schema-activation: Schema activation failure surfaces during connect | Integration | `tests/websocket_integration_tests.rs` | `test_websocket_connect_with_missing_uri_schema_fails` |
| schema-activation: Schema activation failure surfaces during connect | Integration | `tests/driver_manager_tests.rs` | `test_ffi_missing_uri_schema_fails_the_connection` |
| schema-activation: URI schema name follows the server's case rule | Integration | `tests/integration_tests.rs` | `test_uri_schema_name_follows_the_server_case_rule` |
| schema-activation: URI schema name follows the server's case rule | Integration | `tests/websocket_integration_tests.rs` | `test_websocket_uri_schema_name_follows_the_server_case_rule` |
| schema-activation: Set the current schema at runtime | Unit | `src/adbc/connection.rs` | `set_schema_sends_the_schema_attribute_and_reports_the_servers_name`, `set_schema_returns_the_transports_error` |
| schema-activation: Set the current schema at runtime | Integration | `tests/integration_tests.rs` | `test_set_schema_sets_the_server_current_schema` |
| schema-activation: Set the current schema at runtime | Integration | `tests/websocket_integration_tests.rs` | `test_websocket_set_schema_sets_the_server_current_schema` |
| schema-activation: Current schema follows schema changes made in SQL | Unit | `src/adbc/connection.rs` | `current_schema_reports_the_transports_value_without_a_request` |
| schema-activation: Current schema follows schema changes made in SQL | Integration | `tests/integration_tests.rs` | `test_current_schema_follows_schema_changes_in_sql` |
| schema-activation: Current schema follows schema changes made in SQL | Integration | `tests/websocket_integration_tests.rs` | `test_websocket_current_schema_follows_schema_changes_in_sql` |
| schema-activation: ADBC db_schema option sets the server's current schema | Integration | `tests/driver_manager_tests.rs` | `test_ffi_db_schema_option_sets_the_server_current_schema`, `test_ffi_db_schema_option_rejects_a_missing_schema` |
| schema-activation: ADBC db_schema option reads the server's current schema | Integration | `tests/driver_manager_tests.rs` | `test_ffi_db_schema_option_reads_the_server_current_schema` |
| schema-activation: ADBC db_schema option before the session exists | Unit | `src/adbc_ffi.rs` | `get_option_string_reports_the_uri_schema_before_the_session_exists`, `get_option_string_reports_an_unset_schema_as_not_found` |
| session-and-lifecycle: Session attributes | Integration | `tests/integration_tests.rs` | `test_uri_schema_is_opened_on_connect`, `test_connect_with_missing_uri_schema_fails` |
| session-and-lifecycle: A failed statement leaves the session usable | Unit | `src/adbc/connection.rs` | `failed_execute_statement_leaves_the_session_ready`, `failed_prepared_executions_leave_the_session_ready`, `begin_transaction_succeeds_after_a_failed_statement` |
| session-and-lifecycle: A failed statement leaves the session usable | Integration | `tests/integration_tests.rs` | `test_failed_statement_leaves_the_session_usable` |
| session-and-lifecycle: A failed statement leaves the session usable | Integration | `tests/driver_manager_tests.rs` | `test_ffi_autocommit_off_after_a_failed_statement` |
| session-and-lifecycle: A failed statement inside a transaction keeps the transaction | Unit | `src/adbc/connection.rs` | `failed_statement_inside_a_transaction_keeps_the_transaction` |
| session-and-lifecycle: A failed statement inside a transaction keeps the transaction | Integration | `tests/integration_tests.rs` | `test_failed_statement_inside_a_transaction_keeps_the_transaction` |
| session-and-lifecycle: An abandoned execution leaves the session state unchanged | Unit | `src/adbc/connection.rs` | `abandoned_execution_leaves_the_session_state_unchanged` |
| session-and-lifecycle: A closed session rejects operations as closed | Unit | `src/adbc/connection.rs` | `closed_session_rejects_operations_as_closed`, `execute_statement_rejects_a_closed_session_as_an_invalid_state` |
| session-and-lifecycle: A closed session rejects operations as closed | Unit | `src/connection/session.rs` | `test_session_validate_ready` |
| session-and-lifecycle: Starting a second transaction reports the active transaction | Unit | `src/adbc/connection.rs` | `begin_transaction_rejects_a_second_overlapping_transaction` |
| native-client/protocol: Set the current schema attribute | Integration | `tests/native_protocol_tests.rs` | `native_set_current_schema_records_the_servers_name`, `native_set_current_schema_reports_a_rejected_schema` |
| native-client/protocol: Track the current schema attribute from responses | Unit | `src/transport/native/mod.rs` | `current_schema_change_reads_attribute_22_from_a_response`, `response_without_attribute_22_leaves_the_current_schema_unchanged`, `get_attributes_response_without_attribute_22_means_no_current_schema` |
| websocket-client/protocol: Set the current schema attribute | Unit | `src/transport/websocket.rs` | `set_current_schema_sends_set_attributes_then_records_the_servers_name`, `set_current_schema_returns_the_servers_rejection` |
| websocket-client/protocol: Track the current schema attribute from responses | Unit | `src/transport/websocket.rs` | `response_attributes_update_the_current_schema` |

- Every test in this table carries one `/// Scenario: <title>` line per scenario it is mapped to, as AGENTS.md requires. Each line quotes the scenario title of the spec delta verbatim, for example `/// Scenario: Set the current schema at runtime`. A reused test gets the lines too.
- Each transport keeps the recorded current schema when the server rejects a set. The transport tests assert that step of "Set the current schema at runtime": `set_current_schema_returns_the_servers_rejection` (WebSocket) and `native_set_current_schema_reports_a_rejected_schema` (native) assert that `current_schema()` keeps the earlier value after the rejection. `set_schema_returns_the_transports_error` runs against the mock transport, so it asserts only that `set_schema` returns the transport's error.
- Unit tests listed for `src/adbc/connection.rs` use the mock transport; the native helper and WebSocket tests use a pure function or `FakeWebSocketServer`, so none of them needs a database server.
- `failed_execute_statement_leaves_the_session_ready` covers a server error, an unbound parameter, and a rejected `queryTimeout` push.
- `failed_prepared_executions_leave_the_session_ready` covers `execute_prepared`, `execute_prepared_update`, `execute_batch_update`, and `execute_batch`, because the five execution methods share setup and assertions.

### Manual Testing

Run `cargo build --release --features ffi` first. The snippets below run the cdylib through the Python ADBC driver manager. `$URI_BASE` is `exasol://sys:exasol@localhost:8563`, and `$OPTS` is `?tls=1&validateservercertificate=0`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| schema-activation | `exapump sql 'CREATE SCHEMA IF NOT EXISTS "ZZ_MixedCase"'`, then snippet M1 with the URI `$URI_BASE/ZZ_MixedCase$OPTS` | `('ZZ_MixedCase',) ZZ_MixedCase` |
| schema-activation | Snippet M1 with the URI `$URI_BASE/ZZ_TYPO$OPTS` | The connect raises an error whose message contains `ZZ_TYPO` and `not found` |
| schema-activation | Snippet M2 | `ZZ_MixedCase`, then `SYS` |
| session-and-lifecycle | Snippet M3 | `failed as expected`, then `transaction ok`, with no "Connection is closed" error |
| native-client/protocol | Snippet M1 with the URI `$URI_BASE/ZZ_MixedCase$OPTS` (the default transport is native) | `('ZZ_MixedCase',) ZZ_MixedCase` |
| websocket-client/protocol | `cargo build --release --features 'ffi websocket'`, then snippet M1 with the URI `$URI_BASE/ZZ_MixedCase$OPTS&transport=websocket` | `('ZZ_MixedCase',) ZZ_MixedCase` |

Snippet M1 reads the URI from the shell variable `URI`:

```bash
uv run --with adbc-driver-manager --with pyarrow python - "$URI" <<'EOF'
import sys
import adbc_driver_manager.dbapi as d
c = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="AdbcDriverExasolInit", db_kwargs={"uri": sys.argv[1]})
cur = c.cursor()
cur.execute("SELECT CURRENT_SCHEMA")
print(cur.fetchone(), c.adbc_current_db_schema)
EOF
```

Snippet M2:

```bash
uv run --with adbc-driver-manager --with pyarrow python - <<'EOF'
import adbc_driver_manager.dbapi as d
c = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="AdbcDriverExasolInit", db_kwargs={"uri": "exasol://sys:exasol@localhost:8563?tls=1&validateservercertificate=0"})
c.adbc_connection.set_options(**{"adbc.connection.db_schema": "ZZ_MixedCase"})
print(c.adbc_current_db_schema)
cur = c.cursor()
cur.execute("OPEN SCHEMA SYS")
print(c.adbc_current_db_schema)
EOF
```

Snippet M3:

```bash
uv run --with adbc-driver-manager --with pyarrow python - <<'EOF'
import adbc_driver_manager.dbapi as d
c = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="AdbcDriverExasolInit", db_kwargs={"uri": "exasol://sys:exasol@localhost:8563?tls=1&validateservercertificate=0"}, autocommit=True)
cur = c.cursor()
try:
    cur.execute("SELECT * FROM ZZ_NO_SUCH_TABLE")
except Exception:
    print("failed as expected")
c.adbc_connection.set_options(**{"adbc.connection.autocommit": "false"})
cur.execute("SELECT 1")
c.commit()
print("transaction ok")
EOF
```

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo +1.92 build --release --features ffi` | Exit 0 |
| WebSocket-only build | `cargo +1.92 test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit tests | `cargo +1.92 test --lib`, `cargo +1.92 test --lib --features websocket`, `cargo +1.92 test --lib --features ffi` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo +1.92 test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| WebSocket integration tests | `REQUIRE_EXASOL=1 cargo +1.92 test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Native protocol tests | `REQUIRE_EXASOL=1 cargo +1.92 test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `cargo +1.92 build --release --features ffi && REQUIRE_EXASOL=1 cargo +1.92 test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Import and export tests | `REQUIRE_EXASOL=1 cargo +1.92 test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Lint | `cargo +1.92 clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo +1.92 fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Total at least 80.0%, every file at least 50.0% |
| Version | `grep '^version' Cargo.toml` and `grep -n '^## 0.21.0' CHANGELOG.md` | `version = "0.21.0"` and one `## 0.21.0` header above `## 0.20.1` |
