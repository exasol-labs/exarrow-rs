# Tasks: fix-result-set-type-fidelity

## Phase 2: Implementation (Group A)
- [x] 1.1 Shared `parse_interval` in `src/types/conversion.rs`; delete the four builders.rs parsers
- [x] 1.2 Native INTERVAL filler and GEOMETRY/HASHTYPE Binary in `fill_column_builder`
- [x] 1.3 `interval_fraction` on `NativeColumnMeta`, passed into `DataType::fraction`
- [x] 1.4 `tests/native_protocol_tests.rs`: INTERVAL and GEOMETRY/HASHTYPE tests
- [x] 1.5 `tests/integration_tests.rs`: `test_native_type_parameters_in_field_metadata`

## Phase 2: Implementation (Group B)
- [x] 2.1 `json_column_to_array`: Interval and Binary arms, error for other types
- [x] 2.2 `DataType.srid`, resolvers read it
- [x] 2.3 `payload_to_record_batch` doc and test rewrite
- [x] 2.4 `tests/websocket_integration_tests.rs`: four `*_result_equals_native` tests
- [x] 2.5 `test_ws_type_parameters_in_field_metadata` and HASHTYPE test change

## Phase 2: Implementation (Group C)
- [x] 3.1 `docs/type-mapping.md`
- [x] 3.2 Bump `Cargo.toml` to 0.28.0
- [x] 3.3 `CHANGELOG.md` 0.28.0 section

## Phase 3: Verification
- [x] 4.1 Build, unit, integration, driver manager tests
- [x] 4.2 Clippy and fmt
- [x] 4.3 Coverage
- [x] 4.4 Scenario coverage audit and manual testing

## Phase 4: Review Fixes
- [x] 4.5 `src/query/results.rs`: change `json_column_to_array` to take `&Field`, return `ConversionError::SchemaMismatch` naming the column in the `other` arm, pass `field` from `column_major_to_record_batch`, and assert `large_text` in `test_column_major_to_record_batch_unsupported_type_returns_error`
- [x] 4.6 `src/query/results.rs`: delete the restating comment in `test_column_major_to_record_batch_unsupported_type_returns_error`
- [x] 4.7 `src/query/results.rs`: delete `srid: None,` in `test_exasol_type_of_interval_day_to_second`
- [x] 4.8 `src/arrow_conversion/converter.rs`: delete `srid: None,` in `test_parse_exasol_type_interval_day_to_second_uses_fraction_as_precision`
- [x] 4.9 `src/arrow_conversion/builders.rs`: delete both stale test banner blocks
- [x] 4.10 `src/arrow_conversion/builders.rs`: delete the seven length-only interval builder tests
- [x] 4.11 `src/arrow_conversion/builders.rs`: rename `test_build_interval_year_to_month_array_invalid_not_string` to `test_build_interval_array_rejects_a_value_that_is_not_a_string`
- [x] 4.12 `src/types/conversion.rs`: delete `test_parse_interval_ym` and the three `test_parse_interval_ds_invalid_*` tests
- [x] 4.13 `src/types/conversion.rs`: rename and rewrite `test_parse_interval_ym_invalid_format`, rename `test_parse_interval_ds`
- [x] 4.14 `src/types/conversion.rs`: delete three redundant inline test comments
- [x] 4.15 `src/transport/native/result_parser.rs`: add `interval_value_longer_than_the_buffer_is_rejected` and `non_utf8_interval_value_is_rejected`
- [x] 4.16 `tests/native_protocol_tests.rs`: drop the `names` parameter of `assert_data_type_in_all_three_schemas`
- [x] 4.17 `tests/native_protocol_tests.rs`: add `/// Scenario: Date and time types mapping` to `test_native_interval_results_are_month_day_nano`
- [x] 4.18 `tests/common/mod.rs`: add shared `metadata_schema_and_batches` and use it in the three test targets
- [x] 4.19 `tests/websocket_integration_tests.rs`: introduce `TwoRowCase` replacing `two_row_query` and the paired arguments
- [x] 4.20 `tests/websocket_integration_tests.rs`: add `/// Scenario: Date and time types mapping` to `test_ws_interval_year_to_month_result_equals_native`
