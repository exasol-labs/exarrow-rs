# Verification Report: fix-result-set-type-fidelity

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | WebSocket queries with INTERVAL, GEOMETRY, and HASHTYPE columns return the same Arrow types and values as native queries, and the checklist is green. |
| Code review | 16 findings, 16 fixed |

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
| Unit | 91.16 (production lines, floor 80) |
| Integration | not measured (CI measures unit coverage only) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit, default features | 1782 | 1782 | 0 |
| Unit, `websocket` | 1827 | 1827 | 0 |
| Unit, `ffi` | 1903 | 1903 | 0 |
| Integration, `native_protocol_tests` | 18 | 18 | 0 |
| Integration, `websocket_integration_tests` | 68 | 68 | 0 |
| Integration, `integration_tests` | 97 | 96 | 1 |
| Integration, `driver_manager_tests` | 75 | 75 | 0 |

The ignored test in `integration_tests` carried its `#[ignore]` before this plan.

### Manual Tests

| Test | Result |
|------|--------|
| Native: `I` and `J` are `month_day_nano_interval`, `G` and `H` are `binary`, `J` shows `precision` 6, `G` shows no `exasol:srid`, `H` shows `byte_size` 16, row is months 14 and days -3 with nanoseconds -14706789000000, `b'POINT (1 2)'`, `b'550e8400e29b11d4a716446655440000'` | ✓ |
| WebSocket: no error, same Arrow types and same row as native, `J` shows `precision` 6, `G` shows `exasol:srid` `4326` | ✓ |
| Month limit, native: `BIG` query raises an error that names `BIG` | ✓ |
| Month limit, WebSocket: `BIG` query raises an error that names `BIG` | ✓ |

## Tool Evidence

### Linter

```
cargo +1.92 clippy --all-targets --all-features -- -D warnings: exit 0
```

### Formatter

```
cargo +1.92 fmt --all -- --check: exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| native-client | type-conversion | Direct binary to Arrow conversion for interval types | `src/transport/native/result_parser.rs` | `single_pass_interval_columns_decode_to_month_day_nano` | Pass |
| native-client | type-conversion | Direct binary to Arrow conversion for binary and geometry types | `src/transport/native/result_parser.rs` | `single_pass_geometry_and_hashtype_columns_keep_their_text_bytes_in_binary` | Pass |
| type-mapping | exasol-to-arrow-dates | Date and time types mapping | `tests/native_protocol_tests.rs` | `test_native_interval_results_are_month_day_nano` | Pass |
| type-mapping | exasol-to-arrow-field-metadata | HASHTYPE field metadata carries the byte size of the column | `tests/websocket_integration_tests.rs` | `test_ws_hashtype_field_metadata_carries_the_declared_byte_size` | Pass |
| type-mapping | exasol-to-arrow-field-metadata | INTERVAL DAY TO SECOND field metadata carries the fractional second precision | `tests/integration_tests.rs` | `test_native_type_parameters_in_field_metadata` | Pass |
| type-mapping | exasol-to-arrow-field-metadata | GEOMETRY field metadata carries the SRID that Exasol reports | `tests/websocket_integration_tests.rs` | `test_ws_type_parameters_in_field_metadata` | Pass |
| type-mapping | exasol-to-arrow | GEOMETRY type handling | `tests/native_protocol_tests.rs` | `test_native_geometry_and_hashtype_results_are_binary` | Pass |
| type-mapping | exasol-to-arrow | INTERVAL type mapping | `tests/native_protocol_tests.rs` | `test_native_interval_results_are_month_day_nano` | Pass |
| type-mapping | exasol-to-arrow | INTERVAL YEAR TO MONTH value outside the Arrow month range | `tests/websocket_integration_tests.rs` | `test_ws_interval_year_to_month_result_equals_native` | Pass |
| type-mapping | exasol-to-arrow | HASHTYPE type mapping | `tests/websocket_integration_tests.rs` | `test_ws_hashtype_result_equals_native` | Pass |
| type-mapping | exasol-to-arrow | WebSocket INTERVAL, GEOMETRY, and HASHTYPE results equal native results | `tests/websocket_integration_tests.rs` | `test_ws_geometry_result_equals_native` | Pass |

The scenario "Field metadata on the native transport keeps the decoded Arrow data type" is a DELTA:REMOVED scenario. Its test was replaced in task 1.5.

## Notes

- The `websocket`-only build prints 8 `dead_code` warnings from date and timestamp helpers in `src/types/conversion.rs`. The helpers exist in HEAD and this plan does not change them. CI builds that configuration without `-D warnings`.
- The Parquet export still writes INTERVAL, GEOMETRY, and HASHTYPE as Utf8 text, as the plan states.
- Two review-fix tests, `interval_value_longer_than_the_buffer_is_rejected` and `non_utf8_interval_value_is_rejected`, are characterization tests of error paths that already existed.
- `YEAR TO MONTH` text is now recognized by its `-` between years and months, so a bare `+01` reads as a day-only DAY TO SECOND value. The largest accepted YEAR TO MONTH value is `+178956970-07`.
