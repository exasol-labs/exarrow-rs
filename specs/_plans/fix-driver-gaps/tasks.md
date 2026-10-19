# Tasks: fix-driver-gaps

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [x] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: Native protocol version check)
- [x] 1.1 Add a native login fake to `src/transport/test_support.rs`
- [x] 1.2 Write the failing test `login_rejects_a_server_protocol_version_below_14_before_sending_the_password`
- [x] 1.3 Write the test `login_accepts_server_protocol_versions_from_14_to_the_client_maximum`
- [x] 1.4 Reject a reported protocol version below 14 in `login_exchange`

## Phase 2: Implementation (Group B: Version capability and session settings)
- [x] 2.1 Write the failing `Session` unit tests for `supports_at_least`
- [x] 2.2 Memoize the parsed release version and add `Session::supports_at_least`
- [x] 2.3 Write the test `capability_checks_issue_no_sql`
- [x] 2.4 Remove `enable_keepalive` and `keepalive_interval` from `SessionConfig`

## Phase 2: Implementation (Group C: Conversion error context)
- [x] 3.1 Add `column_name`, `value`, and `target_type` to the two `ConversionError` variants and add the crate-private error and conversion method
- [x] 3.2 Write the failing tests in `src/arrow_conversion/converter.rs`
- [x] 3.3 Move the builders to the crate-private error and thread the column name through `build_array`
- [x] 3.4 Name the column in the WebSocket DECIMAL conversion error in `src/query/results.rs`

## Phase 2: Implementation (Group D: Release notes)
- [x] 4.1 Set the version to 0.25.0 and add the `## 0.25.0` changelog section

## Phase 3: Verification
- [x] 5.1 Run the plan's Checklist commands
- [x] 5.2 Confirm every scenario in the Scenario Coverage table has a passing test
- [x] 5.3 Run the Manual Testing commands

## Phase 4: Review Fixes
- [x] 4.2 In `src/arrow_conversion/builders.rs` `build_array`, compute the target type with `TypeMapper::exasol_to_arrow(exasol_type, true)?` before `build_typed_array` and attach the column context without the `Err(unsupported) => unsupported` arm
- [x] 4.3 In `src/query/results.rs` `json_to_decimal_array`, replace the overflow `if`/`else` closure body with `ColumnConversionError::Value { row, reason: failure.reason, overflow: failure.overflow }`
- [x] 4.4 In `src/connection/session.rs`, add `/// Scenario: Reject an unparsable release_version` above `supports_at_least_is_false_for_an_unparsable_release_version`
- [x] 4.5 In `src/adbc/connection.rs`, add `/// Scenario: Capability check is in-process and round-trip-free` above `capability_checks_issue_no_sql`
- [x] 4.6 In `src/transport/test_support.rs`, list `NativeLoginServer`, `LoginExchange`, `raw_public_key`, and `test_phrase` in the module doc
- [x] 4.7 In `src/transport/test_support.rs`, extract `read_login_packet` and `login_response` with the constant `LOGIN_PACKET_PREFIX_LEN` out of `NativeLoginServer::reporting_protocol_version`
- [x] 4.8 In `src/transport/native/mod.rs`, add `negotiated_protocol_version`, which rejects a reported protocol version below 14 or outside the `i32` range, and call it from `login_exchange` before `build_auth_message` [expert]
- [x] 4.9 Add the field `message` to `ConversionError::NumericOverflow`, carry the overflow reason into it in `into_conversion_error`, name the integer-part overflow in `parse_string_to_decimal`, and update the tests and the 0.25.0 CHANGELOG entry [expert]
