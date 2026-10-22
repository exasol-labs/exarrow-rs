# Code Review Findings: fix-result-set-type-fidelity

## Summary
- Files reviewed: 18
- Total findings: 16 (standard: 16, expert: 0)

## Standard fixes

### src/query/results.rs

#### [CONTEXTLESS_ERROR] Unsupported Arrow type error does not name the column
- Location: line 569 (`other` arm of `json_column_to_array`), caller at line 490
- Issue: The `other` arm returns `ConversionError::ArrowError("Arrow type {other} is not a result set column type")`. `ColumnConversionError::into_conversion_error` passes `Other` errors through unchanged, so the message never names the column. Every other failure of this function names the column. The variant `ArrowError` also labels a schema problem as an Arrow library failure, while `ConversionError::SchemaMismatch` exists for this case.
- Fix: In src/query/results.rs, change `json_column_to_array(data_type: &DataType, values: &[&Value])` to `json_column_to_array(field: &Field, values: &[&Value])` and match on `field.data_type()`. In the `other` arm, return `ConversionError::SchemaMismatch(format!("Column {} has Arrow type {other}, which no result set column maps to", field.name())).into()`. In `column_major_to_record_batch` (line 490), pass `field` instead of `field.data_type()`. In `test_column_major_to_record_batch_unsupported_type_returns_error`, also assert that the error message contains `large_text`.

#### [REDUNDANT_COMMENT] Test comment restates the assertion
- Location: line 2042
- Issue: The inline comment `// LargeUtf8 is no result column type, so the conversion names it in an error` repeats what the test name and the assertion already state.
- Fix: In src/query/results.rs, delete the comment line at line 2042 in `test_column_major_to_record_batch_unsupported_type_returns_error`.

#### [SHRINKABLE] Redundant `srid: None` in a struct update
- Location: line 1497
- Issue: `test_exasol_type_of_interval_day_to_second` sets `srid: None` inside `DataType { fraction: Some(6), srid: None, ..plain_type("INTERVAL DAY TO SECOND") }`. `plain_type` already sets `srid: None`.
- Fix: In src/query/results.rs, delete the line `srid: None,` at line 1497 in `test_exasol_type_of_interval_day_to_second`.

### src/arrow_conversion/converter.rs

#### [SHRINKABLE] Redundant `srid: None` in a struct update
- Location: line 894
- Issue: `test_parse_exasol_type_interval_day_to_second_uses_fraction_as_precision` sets `srid: None` inside a struct update from `scalar_type(...)`. `scalar_type` already sets `srid: None` (line 345).
- Fix: In src/arrow_conversion/converter.rs, delete the line `srid: None,` at line 894 in `test_parse_exasol_type_interval_day_to_second_uses_fraction_as_precision`.

### src/arrow_conversion/builders.rs

#### [OUTDATED_COMMENT] Test banners name deleted functions
- Location: lines 1135-1137 and 1174-1176
- Issue: The banners `Tests for build_interval_year_to_month_array` and `Tests for build_interval_day_to_second_array` name functions that this change deleted. AGENTS.md also forbids banners.
- Fix: In src/arrow_conversion/builders.rs, delete both three-line banner blocks (lines 1135-1137 and lines 1174-1176).

#### [DUPLICATE_TEST] Length-only interval builder tests duplicate the value test
- Location: lines 1140, 1149, 1158, 1179, 1188, 1197, 1206
- Issue: `test_build_interval_year_to_month_array_positive`, `_negative`, `_with_nulls`, `test_build_interval_day_to_second_array_with_fractional_seconds`, `_without_fractional_seconds`, `_negative`, and `_with_nulls` all call the one function `build_interval_array` and assert only the array length and null count. `test_build_interval_array_stores_month_day_nano_values` (line 1215) covers both text forms and a NULL with value assertions, and the `parse_interval` tests in src/types/conversion.rs cover every text form these tests feed.
- Fix: In src/arrow_conversion/builders.rs, delete the seven tests `test_build_interval_year_to_month_array_positive`, `test_build_interval_year_to_month_array_negative`, `test_build_interval_year_to_month_array_with_nulls`, `test_build_interval_day_to_second_array_with_fractional_seconds`, `test_build_interval_day_to_second_array_without_fractional_seconds`, `test_build_interval_day_to_second_array_negative`, and `test_build_interval_day_to_second_array_with_nulls`.

#### [VAGUE_TEST_NAME] Test name refers to a deleted function
- Location: line 1167
- Issue: `test_build_interval_year_to_month_array_invalid_not_string` tests `build_interval_array`, which serves both interval kinds. The name refers to the deleted `build_interval_year_to_month_array`.
- Fix: In src/arrow_conversion/builders.rs, rename `test_build_interval_year_to_month_array_invalid_not_string` to `test_build_interval_array_rejects_a_value_that_is_not_a_string`.

### src/types/conversion.rs

#### [DUPLICATE_TEST] Moved interval tests repeat other tests in the same module
- Location: lines 1229, 1350, 1356, 1362
- Issue: `test_parse_interval_ds_invalid_hours`, `_invalid_minutes`, and `_invalid_seconds` assert only `is_err()` for the inputs that `test_parse_interval_ds_reports_invalid_hours`, `_minutes`, and `_seconds` (lines 1395-1412) check with the exact error message. `test_parse_interval_ym` checks one positive and one negative YEAR TO MONTH value, which `test_parse_interval_reads_exasol_wire_text_of_both_forms` (line 1429) also checks.
- Fix: In src/types/conversion.rs, delete the tests `test_parse_interval_ym`, `test_parse_interval_ds_invalid_hours`, `test_parse_interval_ds_invalid_minutes`, and `test_parse_interval_ds_invalid_seconds`.

#### [VAGUE_TEST_NAME] Test names contradict or omit the expected behavior
- Location: lines 1238, 1272-1276
- Issue: `test_parse_interval_ym_invalid_format` asserts that the unsigned text `01-06` parses, so its name states the opposite of the behavior. Its two inline comments (`// Missing sign` and `// This should still work since we trim the sign`) describe trimming, which `parse_interval` no longer does. `test_parse_interval_ds` names no condition and no expected behavior.
- Fix: In src/types/conversion.rs, rename `test_parse_interval_ym_invalid_format` to `test_parse_interval_reads_unsigned_text_as_positive`, replace its body with `assert_eq!(months_of("01-06"), 18);`, and delete its two inline comments. Rename `test_parse_interval_ds` to `test_parse_interval_reads_every_day_to_second_component`.

#### [REDUNDANT_COMMENT] Inline test comments restate the test name
- Location: lines 1337, 1369, 1421
- Issue: `// Only first 9 digits should be used`, `// A time segment with fewer than two colon-separated fields adds no nanoseconds`, and `// The sign applies to both components even when days are zero` repeat the test name and the assertion of `test_parse_interval_ds_with_fractional_12_digits`, `test_parse_interval_ds_time_without_colon_is_ignored`, and `test_parse_interval_ds_negative_nanos_only`.
- Fix: In src/types/conversion.rs, delete the three inline comments at lines 1337, 1369, and 1421.

### src/transport/native/result_parser.rs

#### [UNTESTED_ERROR_PATH] Truncated and non-UTF-8 INTERVAL values have no test
- Location: line 744 (`fill_interval_column`)
- Issue: `fill_interval_column` fails with `Interval data truncated` when the length prefix exceeds the buffer and with `Invalid UTF-8 in INTERVAL column {name}: ...` when the bytes are not UTF-8. No test reaches either path. The string and binary fillers have tests for the same paths (`string_value_longer_than_the_buffer_is_rejected`, `non_utf8_string_value_is_rejected`, `binary_value_longer_than_the_buffer_is_rejected`).
- Fix: In src/transport/native/result_parser.rs tests, add `interval_value_longer_than_the_buffer_is_rejected`: columns `vec![meta("IV", T_INTERVAL_DAY, None, None)]`, data `[1u8]` followed by `10i32.to_le_bytes()` and `b"ab"`, call `build_batch_from_wire(&data, &mut offset, &columns, 1)`, and assert `protocol_message(err) == "Interval data truncated"`. Add `non_utf8_interval_value_is_rejected`: the same column, data `[1u8]` followed by `2i32.to_le_bytes()` and `[0xFF, 0xFE]`, and assert that `protocol_message(err)` starts with `"Invalid UTF-8 in INTERVAL column IV: "`. Give both tests the line `/// Scenario: Direct binary to Arrow conversion for interval types`.

### tests/native_protocol_tests.rs

#### [TOO_MANY_ARGUMENTS] Schema assertion helper takes four arguments
- Location: line 526 (`assert_data_type_in_all_three_schemas`), callers at lines 564 and 623
- Issue: `assert_data_type_in_all_three_schemas(conn, sql, names, expected)` takes four arguments. Every field of both caller queries has the expected type, so `names` adds nothing.
- Fix: In tests/native_protocol_tests.rs, remove the `names` parameter from `assert_data_type_in_all_three_schemas`, assert `expected` for every field of each of the three schemas by iterating `schema.fields()`, update the doc comment to say "every field", and drop the `&["YM", "DS"]` and `&[name]` arguments at both call sites.

#### [MISSING_DOC_COMMENT] Test lacks the Scenario line the plan maps to it
- Location: line 556 (`test_native_interval_results_are_month_day_nano`)
- Issue: The plan's Scenario Coverage table maps "Date and time types mapping" to this test, and AGENTS.md requires one `/// Scenario:` line per implemented scenario. The test carries no line for that scenario.
- Fix: In tests/native_protocol_tests.rs, add the line `/// Scenario: Date and time types mapping` to the doc lines above `test_native_interval_results_are_month_day_nano`.

#### [SHRINKABLE] Metadata schema and batch fetch is copied into three test targets
- Location: tests/native_protocol_tests.rs line 504 (`metadata_schema_and_batches`), tests/websocket_integration_tests.rs line 2525 (`metadata_schema_and_batch`), tests/integration_tests.rs lines 4227-4236 (inline in `test_native_type_parameters_in_field_metadata`)
- Issue: The same sequence (`execute`, `metadata().expect("a SELECT has metadata").schema.clone()`, `fetch_all`) appears in two new helpers and one new inline block. The test files hold six more copies of it. The Rule of Three calls for one shared helper.
- Fix: In tests/common/mod.rs, add `#[allow(dead_code)] pub async fn metadata_schema_and_batches(conn: &mut Connection, sql: &str) -> (arrow::datatypes::SchemaRef, Vec<RecordBatch>)` with the body of the helper at tests/native_protocol_tests.rs line 504 and a one-line doc comment. Delete the helper in tests/native_protocol_tests.rs and import the common one. In tests/websocket_integration_tests.rs, make `metadata_schema_and_batch` call `common::metadata_schema_and_batches`, then keep its one-batch assertion and return. In tests/integration_tests.rs, replace the inline block at lines 4227-4236 with `let (metadata_schema, batches) = common::metadata_schema_and_batches(&mut conn, sql).await;`.

### tests/websocket_integration_tests.rs

#### [TOO_MANY_ARGUMENTS] Two-row assertion helper takes four arguments
- Location: line 2551 (`assert_two_row_result`), with `two_row_query` (line 2517) and `assert_equals_native` (line 2580)
- Issue: `assert_two_row_result(conn, literal_cast, type_sql, expected)` takes four arguments. `literal_cast` and `type_sql` always travel together through `two_row_query`, `assert_two_row_result`, and `assert_equals_native`.
- Fix: In tests/websocket_integration_tests.rs, add `struct TwoRowCase { literal_cast: &'static str, type_sql: &'static str }` with a method `fn query(&self, filter: &str) -> String` that holds the body of `two_row_query`, and delete `two_row_query`. Change `assert_two_row_result` to `(conn: &mut Connection, case: &TwoRowCase, expected: &DataType)` and `assert_equals_native` to `(ws_batch: &RecordBatch, case: &TwoRowCase)`. In the four `test_ws_*_result_equals_native` tests, replace the `LITERAL` and `TYPE_SQL` constants with one `const CASE: TwoRowCase` and pass `&CASE`.

#### [MISSING_DOC_COMMENT] Test lacks the Scenario line the plan maps to it
- Location: line 2626 (`test_ws_interval_year_to_month_result_equals_native`)
- Issue: The plan's Scenario Coverage table maps "Date and time types mapping" to this test, and AGENTS.md requires one `/// Scenario:` line per implemented scenario. The test carries no line for that scenario.
- Fix: In tests/websocket_integration_tests.rs, add the line `/// Scenario: Date and time types mapping` to the doc lines above `test_ws_interval_year_to_month_result_equals_native`.

## Expert fixes
[none]
