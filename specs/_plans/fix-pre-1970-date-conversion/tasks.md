# Tasks: fix-pre-1970-date-conversion

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: Pre-1970 temporal conversion)
- [x] 1.1 Add the seven unit tests of plan.md task 1.1 to src/types/conversion.rs; they fail on the current formula
- [x] 1.2 Replace `/` with `div_euclid` in the leap-year terms of `ymd_to_days`; rewrite its doc comment
- [x] 2.1 Add test_column_major_to_record_batch_keeps_pre_1970_dates_and_timestamps
- [x] 2.2 Use the shared parsers in json_column_to_array; delete the private parsers and constants
- [x] 2.3 Delete the tests of the removed parsers; move test_parse_timestamp_to_micros_hours_and_minutes_only to types::conversion
- [x] 3.1 Add single_pass_dates_and_timestamps_before_1970_keep_their_calendar_day
- [x] 3.2 Replace the derived expectation in the native date test with the literal 19724
- [x] 4.1 Add the types::conversion tests for format_date32, format_timestamp_micros, format_timestamp
- [x] 4.2 Add format_date32, format_timestamp_micros, format_timestamp to src/types/conversion.rs
- [x] 4.3 Add the import::arrow, import::parquet, and adbc_ffi tests
- [x] 4.4 Switch the callers to the shared functions; delete the copies and their tests
- [x] 4.5 Add the FFI parameter binding unit-test step to .github/workflows/ci.yml
- [x] 5.1 Add test_pre_1970_dates_and_timestamps_keep_their_calendar_day (native)
- [x] 5.2 Add test_ws_pre_1970_dates_and_timestamps_keep_their_calendar_day
- [x] 5.3 Add test_parquet_export_keeps_pre_1970_dates_and_timestamps
- [x] 5.4 Add test_arrow_round_trip_keeps_pre_1970_dates_and_timestamps
- [x] 5.5 Add test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps
- [x] 6.1 Add the DATE and TIMESTAMP before 1970 subsection to docs/type-mapping.md
- [x] 6.2 Add the four CHANGELOG.md entries under [Unreleased]

## Phase 4: Review Fixes
- [x] 4.6 In src/types/conversion.rs, make `format_timestamp` replace the range error of `format_timestamp_micros` with one that names the value in its own unit; add `test_format_timestamp_names_the_value_in_its_own_unit`
- [x] 4.7 In src/types/conversion.rs, rewrite the doc comment of `format_timestamp` to state the nanosecond flooring and the overflow and range failures
- [x] 4.8 In src/types/conversion.rs, replace the exhaustive `test_format_date32_round_trips_every_exasol_day` with `test_format_date32_round_trips_every_month_boundary`
- [x] 4.9 In src/types/conversion.rs, delete the restating comment of `test_parse_timestamp_to_micros_hours_and_minutes_only` and move the test after `test_parse_timestamp_to_micros_with_short_fraction`
- [x] 4.10 In tests/import_export_tests.rs, replace the indexed row assertions in the Arrow round-trip and Parquet CSV-path tests with whole-collection assertions

## Phase 5: Verification
- [x] 3.1 Run the plan's verification checklist
- [x] 3.2 Run the manual testing steps
