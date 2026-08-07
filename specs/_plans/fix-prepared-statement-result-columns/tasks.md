# Tasks: fix-prepared-statement-result-columns

## Phase 2: Implementation (Group A)
- [x] 2.1 Correct the native vcFlag bitmasks and guard the outbound byte [expert]
- [x] 2.2 Add `result_columns` to `PreparedStatementHandle`
- [x] 2.3 Give the `resultType` discriminator one owner in `src/transport/messages.rs`

## Phase 2: Implementation (Group B)
- [x] 2.4 Replace the sentinel with `NativeResponse::PreparedStatement`, rewrite its tests, and wire the native call site [expert]
- [x] 2.5 Wire the WebSocket transport to that owner

## Phase 2: Implementation (Group C)
- [x] 2.6 Add the native result-columns integration tests
- [x] 2.7 Add the WebSocket parity integration test

## Phase 2: Implementation (Group D)
- [x] 2.8 Bump the version and record it

## Phase 3: Verification
- [x] 3.1 Run automated checklist (build/test/lint/format/coverage)
- [x] 3.2 Scenario coverage audit
- [x] 3.3 Manual verification commands

## Phase 4: Review Fixes
- [x] 4.1 Reject an exception sub-result in `parse_handle_only_at` instead of reporting an empty successful prepare, and propagate sub-result warnings on the returned envelope [expert]
- [x] 4.2 Extract `connect_native_with_test_table` and `drop_native_schema` helpers in tests/integration_tests.rs and replace the triplicated setup/teardown in the three native result-column tests
- [x] 4.3 Drop the `WHERE ID = ?` parameter from `test_prepared_result_columns_native_derived_select_list` and assert `num_params == 0`, updating the spec delta if observed metadata changed
- [x] 4.4 Replace the discarded `CREATE SCHEMA` result with `.expect(...)` in `test_prepared_non_ascii_varchar_parameter_round_trip`
- [x] 4.5 Pin `test_prepared_non_ascii_varchar_parameter_round_trip` to the native transport via `#[cfg(feature = "native")]` and `get_test_connection_with_transport("native")`
- [x] 4.6 Name the received variant in the `_ =>` arms of `native_result_to_query_result` and `fetch_results`, and update the pinned test assertions
- [x] 4.7 Delete the two redundant match-arm comments in src/transport/websocket.rs
