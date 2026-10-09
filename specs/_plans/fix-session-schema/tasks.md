# Tasks: fix-session-schema

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [x] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: current schema attribute, #77)
- [x] 1.1 Transport contract in `TransportProtocol` and test transports [expert]
- [x] 1.2 Native transport current-schema attribute [expert]
- [x] 1.3 WebSocket transport current-schema attribute [expert]
- [x] 1.4 Connection `set_schema` / `current_schema` / `refresh_current_schema`; remove Session schema
- [x] 1.5 Connect applies URI schema through `set_schema`; fail on any error
- [x] 1.6 FFI `CurrentSchema` option
- [x] 1.7 Rust integration tests (schema-activation, native protocol)
- [x] 1.8 Driver manager tests (schema-activation)
- [x] 1.9 Documentation (docs/setup-and-connect.md, README.md, specs/mission.md)
- [x] 1.10 Release metadata: 0.21.0, lockfile, CHANGELOG #77 entries

## Phase 2: Implementation (Group B: session state, #72)
- [x] 2.1 `SessionState` reduced to four states; `validate_ready` / `begin_transaction` errors
- [x] 2.2 Remove `set_state(Executing)` calls and `update_session_state_after_query`; `Connection::begin_transaction` ordering
- [x] 2.3 Integration tests for session-and-lifecycle scenarios
- [x] 2.4 CHANGELOG #72 entries in `## 0.21.0`

## Phase 3: Verification
- [ ] 3.1 Run the plan's Verification checklist

## Phase 4: Review Fixes
- [x] 4.1 In src/adbc/connection.rs, change the attribute above `abandoned_execution_leaves_the_session_state_unchanged` to `#[tokio::test(start_paused = true)]` and confirm it passes
- [x] 4.2 In src/adbc/connection.rs, add a doc comment and `# Errors` section above `pub async fn begin_transaction` describing the closed-session and active-transaction rejections before any server request
- [x] 4.3 In src/adbc/connection.rs, delete the three-line section banner above `fn failing_statement_transport`
- [x] 4.4 In src/connection/session.rs, delete `last_activity`, `update_activity` and its three calls, narrow the `Instant` import, and add `Session::update_activity` to the CHANGELOG 0.21.0 breaking entry
- [x] 4.5 In src/transport/native/mod.rs, delete the `// --- Tracking the current schema attribute ---` banner above `fn response_with_attributes`
- [x] 4.6 In src/transport/websocket.rs tests, add `refresh_current_schema_returns_the_servers_error` covering a rejected getAttributes response
- [x] 4.7 In src/transport/websocket.rs tests, delete the `// --- Current schema attribute ---` banner above `fn row_count_response`
- [x] 4.8 Compile tests/common/schema_activation.rs only into integration_tests and websocket_integration_tests via `#[path]`, drop `#![allow(dead_code)]` and the `pub mod` declaration
- [x] 4.9 In tests/common/schema_activation.rs, make `opened_schema` return a Result and drop schemas before asserting on connect failure; reword the module doc
- [x] 4.10 In tests/driver_manager_tests.rs, remove the `#[ignore]` attribute from the five schema-activation tests
- [x] 4.11 In tests/driver_manager_tests.rs, delete the `// adbc.connection.db_schema option tests` label above `fn get_test_uri_with_schema`
