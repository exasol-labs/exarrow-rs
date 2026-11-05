# Tasks: fix-native-timestamp-ltz-type

## Phase 2: Implementation (Group A)
- [x] 1.1 Handshake unit test
- [x] 1.2 Add ATTR_TSUTC_ENABLED to build_login_packet
- [x] 2.1 result_parser unit test
- [x] 2.2 fill_timestamp_column decodes 124 like 125
- [x] 2.3 data_type_to_wire_type TSL -> T_TIMESTAMP, tests, dead arms
- [x] 2.4 results.rs doc comment
- [x] 3.1 tests/common cleanup
- [x] 3.2 update callers
- [x] 4.1 start docker, build release ffi cdylib
- [x] 4.2 ws vs native integration test
- [x] 4.3 text parameter tests
- [x] 4.4 driver manager test
- [x] 4.5 run on latest then 2025.2.1 [expert]

## Phase 2: Implementation (Group B)
- [x] 5.1 docs/type-mapping.md
- [x] 5.2 Cargo.toml version 0.32.0 (plan says 0.30.0: outdated, main is 0.31.0)
- [x] 5.3 CHANGELOG ## 0.32.0 Breaking entry

## Phase 3: Verification
- [x] 3.1 Run checks from plan Verification

## Phase 4: Review Fixes
- [x] 4.1 Delete `single_pass_timestamp_utc_has_utc_timezone` from `src/transport/native/result_parser.rs`
- [x] 4.2 Add `LOGIN_HEADER_LEN`, `LOGIN_ATTRIBUTE_COUNT`, and `login_attributes` helper to the `handshake.rs` tests and use them in the two login packet tests
- [x] 4.3 Rename `test_ws_local_time_zone_timestamp_result_matches_native` to `test_ws_local_time_zone_timestamp_type_matches_native_and_value_differs_outside_utc` and update the three plan.md rows
- [x] 4.4 Add `BERLIN_SESSION_UTC_INSTANT` constant to the renamed WebSocket test and drop the inline comment
- [x] 4.5 Move the text parameter test body into `assert_local_time_zone_text_parameter_reads_in_session_time_zone` in `tests/common/mod.rs` and call it from both tests
