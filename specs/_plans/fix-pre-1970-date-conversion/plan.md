# Plan: fix-pre-1970-date-conversion

## Summary

DATE and TIMESTAMP values before 1970 convert to the correct day in query results on both transports, in typed exports, and in Arrow and Parquet imports. One day-count function with floor division serves every read path, the WebSocket path's private copy is deleted, and unit tests check the function against chrono over Exasol's full DATE range. The Arrow-to-Exasol date and timestamp formatting of the imports and of ADBC parameter binding moves into the same module, with one error contract.

## Context

- Issue exasol-labs/exarrow-rs#84: `SELECT DATE '1968-01-01'` returns Date32 -730 (1968-01-02) instead of -731, and `TIMESTAMP '1950-06-15 00:00:00'` returns 1950-06-16. Both transports return the same wrong values.
- Root cause: the days-from-year formula `(year - 1970) * 365 + (year - 1969) / 4 - (year - 1901) / 100 + (year - 1601) / 400` uses Rust's integer `/`, which truncates toward zero. For years before 1970 the three leap-year terms are negative and need floor division.
- The formula exists twice. `ymd_to_days` in `src/types/conversion.rs` serves the native decoder (`read_packed_date_days` and `read_timestamp_micros` in `src/transport/native/result_parser.rs`), the public `ArrowConverter` (`src/arrow_conversion/builders.rs`), and the typed export conversions (`src/export/arrow.rs`, `src/export/parquet.rs`). A private copy, `ResultSet::parse_date_to_days` in `src/query/results.rs`, serves the WebSocket query results, together with private copies of the timestamp, time-of-day, and fraction parsers.
- Over the 3,652,059 days from 0001-01-01 to 9999-12-31, the current formula returns a wrong day count for 487,680 days. With floor division it matches Python's proleptic Gregorian `datetime.date` on every day (`decision-log.md` entry [1]).
- `format_timestamp_micros` in `src/import/arrow.rs` splits microseconds with `/`, `%`, and `unsigned_abs`. Importing 1969-12-31 12:00:00 stores 1970-01-01 12:00:00 (issue #84, confirmed against Docker). `import_from_record_batch`, `import_from_record_batches`, `import_from_arrow_ipc`, and ADBC bulk ingestion use it.
- `format_timestamp` in `src/import/parquet.rs` splits seconds and microseconds with `/` and `%`, so -0.5 s becomes +0.5 s. Only the CSV path uses it: servers below 2025.1.11, or `with_native_parquet(Some(false))`. Native Parquet import sends the file bytes unchanged.
- Both import formatters convert nanoseconds with `/ 1_000`, which moves a pre-epoch value with sub-microsecond digits toward the later instant.
- Date32 import is already correct, but by three unrelated routes: `days_to_ymd` in `src/import/arrow.rs` uses floor arithmetic, and `src/import/parquet.rs` and the FFI parameter binding in `src/adbc_ffi.rs` use chrono. The FFI timestamp binding uses `div_euclid` and `rem_euclid` but first converts every unit to nanoseconds, which overflows `i64` before 1677-09-21 and after 2262-04-11 (a release build wraps silently, so Microsecond 0001-01-01 is stored as 1754-08-30 22:43:41.128654, computed and not run against Exasol). The write side therefore keeps the calendar rule in six code sites with three error contracts: `import::arrow` is infallible, `import::parquet` returns an error for a timestamp chrono cannot represent and panics on a Date32 value outside chrono's range, and the FFI date binding can panic on an out-of-range day. Issue #84 itself came from two drifted copies of one rule, so this plan moves all six into `types::conversion` (`decision-log.md` entry [3]).
- Exasol labels dates before 1582-10-15 in the Julian calendar. On `exasol/docker-db` 2026.1.0, `ADD_DAYS(DATE '1582-10-04', 1)` is 1582-10-15, `1500-02-29` exists, and `DAYS_BETWEEN(DATE '0001-01-01', DATE '1970-01-01')` is -719164. chrono and pyarrow decode Date32 in the proleptic Gregorian calendar, where 0001-01-01 is day -719162 (`decision-log.md` entry [2]).
- The existing unit tests check dates from 1969 onward only, which the current formula gets right. The existing integration tests use dates from 1999 onward.
- Architecture: the calendar rule of `decision-log.md` entry [2] becomes a constraint on every read and write path. The architecture delta `architecture.md` in this plan directory adds it to § Constraints.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Exasol to Arrow | CHANGED | `type-mapping/exasol-to-arrow/spec.md` |
| Boundaries and Validation | CHANGED | `type-mapping/boundaries-and-validation/spec.md` |
| Parquet Export | CHANGED | `import-export/parquet-export/spec.md` |
| Arrow RecordBatch Import/Export | CHANGED | `import-export/arrow-recordbatch/spec.md` |
| Parquet I/O | CHANGED | `import-export/parquet-io/spec.md` |
| Arrow to Exasol | CHANGED | `type-mapping/arrow-to-exasol/spec.md` |

## Impact

- Query results on both transports, `ArrowConverter`, `export_to_parquet`, `export_to_record_batches`, `export_to_arrow_ipc`, and the CSV-bytes Parquet export entry points return the correct Date32 and Timestamp values for DATE and TIMESTAMP values before 1970. Values from 1970 onward do not change.
- `import_from_record_batch`, `import_from_record_batches`, `import_from_arrow_ipc`, ADBC bulk ingestion, and the CSV path of Parquet import store pre-1970 TIMESTAMP values at the correct instant. A pre-epoch value with sub-microsecond digits rounds them down to the microsecond.
- A Date32 value outside 0001-01-01 to 9999-12-31 and a Timestamp value outside 0001-01-01 00:00:00 to 9999-12-31 23:59:59.999999, or one that overflows when converted to microseconds, now fails with `ImportError::ConversionError` on both import paths and with `AdbcStatus::InvalidArguments` in parameter binding. Exasol rejects such text anyway. Before, the Arrow import formatted it with an out-of-range year, and the FFI date binding could panic. The FFI timestamp binding no longer wraps for a value before 1677 or after 2262, which it stored at the wrong instant.
- Applications that read or stored pre-1970 values through the driver get different values for the same data: the correct ones. No public API changes.
- A date before 1582-10-15 converts by its year, month, and day, so Arrow tools show the same date text as Exasol. A day difference across 1582-10-15 computed from Date32 values differs from Exasol's `DAYS_BETWEEN`, and February 29 of the twelve Julian-only leap years from 100 to 1500 converts to March 1. On import, a Date32 value for 1582-10-05 to 1582-10-14 becomes Exasol text that Exasol stores as 1582-10-15. `docs/type-mapping.md` states these effects.
- Breaking changes: none.

## Dependencies

None. The unit-test oracle uses chrono 0.4, which `Cargo.toml` already lists as a regular dependency.

## Implementation Tasks

1. Shared day count (`src/types/conversion.rs`)

- [ ] 1.1 Add these unit tests to the test module of `src/types/conversion.rs`. Each carries the line `/// Scenario: DATE and TIMESTAMP values count days in the proleptic Gregorian calendar`.
  - `test_ymd_to_days_matches_chrono_for_every_exasol_date`: iterate every `chrono::NaiveDate` from 0001-01-01 to 9999-12-31 and assert that `ymd_to_days(year, month, day)` equals the date's signed day difference from 1970-01-01. The assertion message names the date.
  - `test_parse_timestamp_to_micros_matches_chrono_for_every_exasol_year`: for each year from 1 to 9999, parse `format!("{year:04}-03-01 12:34:56.789012")` and assert that the result equals chrono's microsecond timestamp of the same date and time in UTC.
  - `test_parse_date_to_days_before_1970`: `1968-01-01` is -731, `1900-03-01` is -25508, `1600-03-01` is -135080, `0001-01-01` is -719162, and `9999-12-31` is 2932896.
  - `test_parse_date_to_days_century_leap_years`: `1600-02-29` is -135081, `1900-02-28` is -25509, `1900-03-01` is -25508, `2000-02-29` is 11016, and `2000-03-01` is 11017.
  - `test_parse_timestamp_to_micros_before_1970`: `1950-06-15 00:00:00` is -616896000000000, `1969-12-31 23:59:59.999999` is -1, `1969-12-31 12:00:00` is -43200000000, `0001-01-01 00:00:00` is -62135596800000000, and `9999-12-31 23:59:59.999999` is 253402300799999999.
  - `test_ymd_hms_nanos_to_micros_before_epoch`: `(1969, 12, 31, 23, 59, 59, 999_999_000)` is -1, and `(1950, 6, 15, 0, 0, 0, 0)` is -616896000000000.
  - `test_ymd_to_days_julian_only_leap_day_reads_as_march_first`: `ymd_to_days(1500, 2, 29)` and `ymd_to_days(1500, 3, 1)` both return -171605, and `parse_date_to_days("1500-02-29")` returns `Ok(-171605)`. This test also carries the line `/// Scenario: Lossless conversion validation`.
  Run the tests. All seven fail on the current formula, because each one asserts at least one value from before 1970 that the current formula gets wrong.
- [ ] 1.2 In `ymd_to_days`, replace `/` with `div_euclid` in the three leap-year terms. Rewrite its doc comment in at most two lines: it counts days in the proleptic Gregorian calendar for any year, and floor division keeps the leap-day count correct before 1970 (`decision-log.md` entries [1] and [2]). The tests of task 1.1 pass.

2. WebSocket result path (`src/query/results.rs`)

- [ ] 2.1 Add the unit test `test_column_major_to_record_batch_keeps_pre_1970_dates_and_timestamps`: a schema with a Date32 field and a `Timestamp(Microsecond, None)` field, and the JSON rows `["1968-01-01", "1950-06-15 00:00:00"]` and `["0001-01-01", "1969-12-31 23:59:59.999999"]`. `ResultSet::column_major_to_record_batch` returns the Date32 values -731 and -719162 and the timestamp values -616896000000000 and -1. The test carries the line `/// Scenario: Pre-1970 DATE and TIMESTAMP query results keep their calendar day`. It fails before task 2.2.
- [ ] 2.2 In `json_column_to_array`, convert DATE strings with `crate::types::conversion::parse_date_to_days(s).ok()` and TIMESTAMP strings with `crate::types::conversion::parse_timestamp_to_micros(s).ok()`. Delete `ResultSet::parse_date_to_days`, `parse_timestamp_to_micros`, `parse_time_of_day_to_micros`, `parse_seconds_to_micros`, and `fractional_seconds_to_micros`, and the constants `SECONDS_PER_MINUTE`, `SECONDS_PER_HOUR`, `SECONDS_PER_DAY`, `MICROS_PER_SECOND`, and `MICROS_FRACTION_DIGITS`. The existing tests `test_column_major_to_record_batch_with_date32`, `test_column_major_to_record_batch_with_timestamp`, `test_column_major_to_record_batch_invalid_date_becomes_null`, and `test_column_major_to_record_batch_invalid_timestamp_becomes_null` pass unchanged, and task 2.1's test passes.
- [ ] 2.3 Delete the unit tests in `src/query/results.rs` that call the removed functions or constants:
  - the sections "Tests for ResultSet::parse_date_to_days" and "Tests for ResultSet::parse_timestamp_to_micros" (24 tests);
  - the section "Tests for the parse_date_to_days month table": `test_parse_date_to_days_covers_every_month_of_a_common_year`, `test_parse_date_to_days_adds_the_leap_day_only_after_february`, `test_parse_timestamp_to_micros_always_has_a_date_part_to_parse`, `test_parse_timestamp_to_micros_ignores_a_trailing_third_space_part`, `test_parse_timestamp_to_micros_empty_fractional_part_contributes_nothing`, and `test_parse_timestamp_to_micros_rejects_non_numeric_time_parts`;
  - `test_parse_timestamp_to_micros_time_part_without_a_colon_adds_nothing`, which also uses `SECONDS_PER_DAY` and `MICROS_PER_SECOND`.
  Move `test_parse_timestamp_to_micros_hours_and_minutes_only` to the test module of `src/types/conversion.rs` and make it call `parse_timestamp_to_micros`, because that module has no test for a time of day without seconds. Every other deleted case has a counterpart there. Afterwards, `grep -n 'ResultSet::parse_date_to_days\|ResultSet::parse_timestamp_to_micros\|SECONDS_PER_\|MICROS_PER_SECOND\|MICROS_FRACTION_DIGITS' src/query/results.rs` prints nothing.

3. Native result decoder (`src/transport/native/result_parser.rs`)

- [ ] 3.1 Add the unit test `single_pass_dates_and_timestamps_before_1970_keep_their_calendar_day`: wire data for a `T_DATE` column with the packed values (1968, 1, 1) and (1, 1, 1), and for a `T_TIMESTAMP` column with (1950, 6, 15, 0, 0, 0, 0) and (1969, 12, 31, 23, 59, 59, 999_999_000). `build_batch_from_wire` returns the Date32 values -731 and -719162 and the timestamp values -616896000000000 and -1. Build the wire bytes the way the existing date and timestamp tests in the file do. The test carries the line `/// Scenario: Pre-1970 DATE and TIMESTAMP query results keep their calendar day`.
- [ ] 3.2 In the existing native date test that expects `crate::types::conversion::ymd_to_days(2024, 1, 2)`, replace that expected value with the literal 19724, so the test no longer derives its expectation from the code under test.

4. Write-side formatters (`src/types/conversion.rs`, `src/import/arrow.rs`, `src/import/parquet.rs`, `src/adbc_ffi.rs`)

- [ ] 4.1 In the test module of `src/types/conversion.rs`, add these unit tests:
  - `test_format_date32_round_trips_every_exasol_day`: for every day from -719162 to 2932896, `parse_date_to_days(&format_date32(day).unwrap())` returns `Ok(day)`. The assertion message names the day.
  - `test_format_date32_literals`: 0 is `1970-01-01`, 1 is `1970-01-02`, 365 is `1971-01-01`, -1 is `1969-12-31`, -731 is `1968-01-01`, -25567 is `1900-01-01`, 10956 is `1999-12-31`, 11016 is `2000-02-29`, 11017 is `2000-03-01`, 19737 is `2024-01-15`, -719162 is `0001-01-01`, and 2932896 is `9999-12-31`.
  - `test_format_date32_rejects_days_outside_exasol_range`: -719163, 2932897, `i32::MIN`, and `i32::MAX` return `Err`.
  - `test_format_timestamp_micros_literals`: 0 is `1970-01-01 00:00:00.000000`, 1000000 is `1970-01-01 00:00:01.000000`, 86400000000 is `1970-01-02 00:00:00.000000`, 123456 is `1970-01-01 00:00:00.123456`, -1 is `1969-12-31 23:59:59.999999`, -1000000 is `1969-12-31 23:59:59.000000`, -86400000000 is `1969-12-31 00:00:00.000000`, -43200000000 is `1969-12-31 12:00:00.000000`, -62135596800000000 is `0001-01-01 00:00:00.000000`, and 253402300799999999 is `9999-12-31 23:59:59.999999`.
  - `test_format_timestamp_micros_rejects_micros_outside_exasol_range`: 253402300800000000, -62135596800000001, `i64::MIN`, and `i64::MAX` return `Err`.
  - `test_format_timestamp_floors_every_unit`: `format_timestamp(&unit, value)` gives `1969-12-31 23:59:59.000000` for `Second` -1, `1969-12-31 23:59:59.999000` for `Millisecond` -1, `1969-12-31 23:59:59.999999` for `Microsecond` -1, `Nanosecond` -1, and `Nanosecond` -1000, `1969-12-31 23:59:59.999998` for `Nanosecond` -1001, and `1970-01-01 00:00:00.000001` for `Nanosecond` 1999.
  - `test_format_timestamp_rejects_overflow_and_out_of_range`: `Second` `i64::MAX`, `Millisecond` `i64::MIN`, and `Microsecond` 253402300800000000 return `Err`.
  The tests do not compile before task 4.2.
- [ ] 4.2 In `src/types/conversion.rs`, add three public functions with `Result<_, String>` returns, as the module's other functions have. `format_date32(days: i32)` builds the date with `chrono::NaiveDate::from_num_days_from_ce_opt(days + 719_163)` (chrono counts 0001-01-01 as day 1, so 1970-01-01 is day 719163 and a Date32 value `days` is day `days + 719163`; `checked_add` guards the sum), requires year 1 to 9999, and formats `%Y-%m-%d`. `format_timestamp_micros(micros: i64)` builds the instant with `chrono::DateTime::from_timestamp_micros`, requires year 1 to 9999, and formats `%Y-%m-%d %H:%M:%S%.6f`. `format_timestamp(unit: &TimeUnit, value: i64)` converts the value to microseconds and calls `format_timestamp_micros`. The conversion is private: it multiplies `Second` by 1000000 and `Millisecond` by 1000 with `checked_mul`, passes `Microsecond` through, and applies `div_euclid(1_000)` to `Nanosecond`, and its overflow error names the value. Each error message names the value. One comment states the why: the year limits are Exasol's range, and `div_euclid` rounds a pre-epoch nanosecond value down to the microsecond, as a positive value already is. The tests of task 4.1 pass.
- [ ] 4.3 Add these tests. They fail on the current code, or do not compile, until task 4.4.
  - `src/import/arrow.rs`: `test_format_timestamp_before_epoch_in_every_unit` (the value -1 in a `TimestampSecondArray`, `TimestampMillisecondArray`, `TimestampMicrosecondArray`, and `TimestampNanosecondArray`, passed through `format_timestamp`, gives `1969-12-31 23:59:59.000000`, `1969-12-31 23:59:59.999000`, `1969-12-31 23:59:59.999999`, and `1969-12-31 23:59:59.999999`; it carries the line `/// Scenario: RecordBatch import formats pre-epoch timestamps of every time unit as times before the epoch`) and `test_format_value_rejects_dates_and_timestamps_outside_exasol_range` (the four values of the scenario, passed through `format_value`, return `ImportError::ConversionError` whose message contains the value; it carries the line `/// Scenario: RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range`).
  - `src/import/parquet.rs`: `test_format_timestamp_before_epoch_in_every_unit` (the four strings above, and -500000 in a `TimestampMicrosecondArray` gives `1969-12-31 23:59:59.500000`; it carries the line `/// Scenario: CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch`) and `test_format_arrow_value_rejects_dates_and_timestamps_outside_exasol_range` (the four values of the scenario, passed through the function that `test_format_arrow_value_rejects_unsupported_type` calls, return `ImportError::ConversionError` whose message contains the value; it carries the line `/// Scenario: CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range`).
  - `src/adbc_ffi.rs`, test module: `test_arrow_value_to_parameter_formats_pre_epoch_values` (Date32 -731 gives `Parameter::String("1968-01-01")`, and a `Timestamp(Microsecond)` value -1 gives `Parameter::String("1969-12-31 23:59:59.999999")`) and `test_arrow_value_to_parameter_rejects_values_outside_exasol_range` (Date32 2932897, `Timestamp(Second)` `i64::MAX`, and `Timestamp(Microsecond)` 253402300800000000 each return an `Err` whose status is `AdbcStatus::InvalidArguments`; `Parameter` has no `PartialEq`, so assert the `Ok` cases with `matches!(.., Parameter::String(s) if s == ..)`). Both carry the line `/// Scenario: Parameter binding formats pre-epoch DATE and TIMESTAMP values and rejects values outside Exasol's range`.
- [ ] 4.4 Switch the callers to the shared functions and delete the copies.
  - `src/import/arrow.rs`: the Date32 arm of `format_value` calls `crate::types::conversion::format_date32`, and `format_timestamp` returns `Result<String, ImportError>`, reading the value of its unit, calling `crate::types::conversion::format_timestamp`, and mapping the `String` error to `ImportError::ConversionError`. Delete `format_date32`, `days_to_ymd`, and `format_timestamp_micros`, and the tests that call a deleted function: `test_format_date32`, `test_format_timestamp_micros`, `test_days_to_ymd_edge_cases`, `test_days_to_ymd_before_epoch`, `test_format_date32_before_epoch`, and `test_format_timestamp_micros_truncates_negative_day_offset`. Task 4.1 covers each of their cases, and the compiler lists any other test that calls a deleted function.
  - `src/import/parquet.rs`: the Date32 arm calls `format_date32` the same way, and `format_timestamp` keeps its array downcasts and calls `crate::types::conversion::format_timestamp`. Delete its `format_date32` and `test_format_date32`.
  - `src/adbc_ffi.rs`: the Date32 and Timestamp arms of `arrow_value_to_parameter` call the shared functions and map the `String` error to `AdbcError::with_message_and_status(message, AdbcStatus::InvalidArguments)`. The Timestamp arm reads the value of its unit, calls `format_timestamp`, and no longer builds nanoseconds or uses chrono directly.
  The tests of tasks 4.1 and 4.3 and the remaining existing tests of the three files pass, including `test_format_timestamp_nanosecond_unit_truncates_to_micros`.

5. Integration tests

- [ ] 5.1 In `tests/integration_tests.rs`, add `test_pre_1970_dates_and_timestamps_keep_their_calendar_day`. It connects with `get_test_connection_with_transport("native").await.expect(...)` and does not call `skip_if_no_exasol!()` (`decision-log.md` entry [6]). It runs the query of the scenario with the column aliases `D1` to `D5` and `T1` to `T3`, downcasts the columns to `Date32Array` and `TimestampMicrosecondArray`, and asserts the eight values of the scenario. It carries the line `/// Scenario: Pre-1970 DATE and TIMESTAMP query results keep their calendar day`.
- [ ] 5.2 In `tests/websocket_integration_tests.rs`, add `test_ws_pre_1970_dates_and_timestamps_keep_their_calendar_day` with the file's `get_ws_connection` helper and the same query, assertions, and scenario line as task 5.1.
- [ ] 5.3 In `tests/import_export_tests.rs`, add `test_parquet_export_keeps_pre_1970_dates_and_timestamps` with `#[tokio::test]` and `assert!(common::is_exasol_available(), ...)` as its first statement, and no `#[ignore]`, which CI rejects since fix-paged-fetch-position (`decision-log.md` entry [6]). It creates table `T` and inserts the two rows of the scenario "Export keeps pre-1970 DATE and TIMESTAMP values", calls `export_to_parquet` for the table, reads the file with `read_parquet_file`, maps rows with `row_of_id`, and asserts the `D` and `TS` values of the scenario. It uses the file's helpers `create_table`, `table_source`, `typed_column`, and `cleanup_schema`. It carries that scenario's line.
- [ ] 5.4 In `tests/import_export_tests.rs`, add `test_arrow_round_trip_keeps_pre_1970_dates_and_timestamps` with the attributes and first statement of task 5.3. It creates table `T` with `ID DECIMAL(18,0), D DATE, TS TIMESTAMP(6)`, imports the RecordBatch of the scenario "Pre-1970 DATE and TIMESTAMP values round-trip through RecordBatch import and export" with `import_from_record_batch`, and queries `SELECT ID, TO_CHAR(D, 'YYYY-MM-DD'), TO_CHAR(TS, 'YYYY-MM-DD HH24:MI:SS.FF6') FROM T ORDER BY ID` through the connection. It asserts the six strings of the scenario. It then calls `export_to_record_batches` for `SELECT ID, D, TS FROM T ORDER BY ID` with `ArrowExportOptions::default().with_schema(...)` and the RecordBatch's schema, and asserts that the `D` and `TS` values equal the imported values. It carries that scenario's line.
- [ ] 5.5 In `tests/import_export_tests.rs`, add `test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps` with the attributes and first statement of task 5.3. It writes a Parquet file with the fields `ID` (Int64) and `TS` (`Timestamp(Microsecond, None)`) and the rows `(1, -500000)` and `(2, -1)` into a `TempDir` with `parquet::arrow::ArrowWriter`, as `write_small_parquet` does. It creates table `T` with `ID DECIMAL(18,0), TS TIMESTAMP(6)`, imports the file with `import_from_parquet` and `ParquetImportOptions::default().with_native_parquet(Some(false))`, and asserts the `TO_CHAR` strings of the scenario "CSV-path Parquet import keeps pre-epoch timestamps with fractional seconds". It carries that scenario's line.

6. Documentation and changelog

- [ ] 6.1 In `docs/type-mapping.md` § Precision and Scale, add a subsection `### DATE and TIMESTAMP before 1970` after § TIMESTAMP. It states that Date32 counts days and Timestamp counts microseconds from 1970-01-01 00:00:00, both in the proleptic Gregorian calendar, applied to the year, month, and day that Exasol reports, the same calendar Arrow tools use. It states that Exasol labels dates before 1582-10-15 in the Julian calendar, so such a date shows the same text in Arrow tools as in Exasol, a day difference across 1582-10-15 differs from Exasol's `DAYS_BETWEEN`, and February 29 of 100, 200, 300, 500, 600, 700, 900, 1000, 1100, 1300, 1400, and 1500 converts to March 1. It states that the driver's import and parameter binding convert Arrow values to Exasol text with the same calendar, and that on import, a Date32 value for 1582-10-05 to 1582-10-14 becomes Exasol text that Exasol stores as 1582-10-15 (`decision-log.md` entry [2]).
- [ ] 6.2 In `CHANGELOG.md`, add a `## [Unreleased]` section above the newest release header (`decision-log.md` entry [7]) with four entries. A `Fix:` entry: DATE and TIMESTAMP values before 1970 now convert to the correct day in query results on both transports and in `export_to_parquet`, `export_to_record_batches`, and `export_to_arrow_ipc`. Many such values came back one day late, some years before 1902 one day early, and TIMESTAMP values shifted by 24 hours. Dates before 1582-10-15 convert by their year, month, and day, as `docs/type-mapping.md` describes. Fixes #84. A second `Fix:` entry: Arrow RecordBatch, Arrow IPC, and ADBC bulk-ingestion imports, and the CSV path of Parquet import, now store TIMESTAMP values before 1970 at the correct instant. 1969-12-31 12:00:00 was stored as 1970-01-01 12:00:00, and a fractional second before 1970 moved to the other side of the epoch. A third `Fix:` entry: ADBC parameter binding stores TIMESTAMP values before 1677-09-21 and after 2262-04-11 at the correct instant. A fourth entry, `Changed:`: a Date32 or Timestamp value outside 0001-01-01 to 9999-12-31 now fails with `ImportError::ConversionError` on both import paths and with `InvalidArguments` in parameter binding, where the Arrow import formatted an out-of-range year and the date binding could panic.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Pre-1970 temporal conversion | 1.1-1.2, 2.1-2.3, 3.1-3.2, 4.1-4.4, 5.1-5.5, 6.1-6.2 | none | spec deltas `type-mapping/exasol-to-arrow`, `type-mapping/boundaries-and-validation`, `import-export/parquet-export`, `import-export/arrow-recordbatch`, `import-export/parquet-io`, `type-mapping/arrow-to-exasol`; `src/types/conversion.rs` (`ymd_to_days`, `parse_date_to_days`, `parse_timestamp_to_micros`, `ymd_hms_nanos_to_micros`), `src/query/results.rs` (`json_column_to_array` and the private parsers it replaces), `src/transport/native/result_parser.rs` (`read_packed_date_days`, `read_timestamp_micros`, tests), `src/import/arrow.rs` (`format_value`, `format_date32`, `days_to_ymd`, `format_timestamp`, `format_timestamp_micros`), `src/import/parquet.rs` (`format_date32`, `format_timestamp`), `src/adbc_ffi.rs` (`arrow_value_to_parameter`), `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `tests/import_export_tests.rs`, `docs/type-mapping.md`, `CHANGELOG.md` |

- One group: every task follows from one fact, how a calendar date maps to an Arrow day count and back. The read-path and write-path tests share `tests/import_export_tests.rs`, and the round-trip test of task 5.4 needs the read-path fix of task 1.2 and the write-path fix of task 4.4. Two groups would share files and this mental model.
- Order: 1.1 and 1.2, then 2.1 to 2.3, 3.1 and 3.2, then 4.1 to 4.4, then 5.1 to 5.5 against a running Exasol, then 6.1 and 6.2.
- No task carries `[expert]`. Planning settled the arithmetic and verified it on every day of the range, so the tasks are mechanical edits with literal expected values.

### Release

This instruction is for the `/speq:implement-pr` orchestrator (`decision-log.md` entry [7]). At step A3, do not apply the Conventional Commits patch default. Set `version` in `Cargo.toml` to X.(Y+1).0. X.Y is the highest of the `vX.Y.Z` tags and of the versions that open pull requests set in `Cargo.toml` or name in a `CHANGELOG.md` header (read each with `ghbrk gh pr diff <number>`). Run `cargo build`. Rename `## [Unreleased]` in `CHANGELOG.md` to `## X.(Y+1).0`, and merge in any `[Unreleased]` entries already on `main`.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/query/results.rs` `ResultSet::parse_date_to_days`, `parse_timestamp_to_micros`, `parse_time_of_day_to_micros`, `parse_seconds_to_micros`, `fractional_seconds_to_micros` | Duplicates of `types::conversion` parsers; the copy of the day-count formula carried the defect (task 2.2) |
| Constant | `src/query/results.rs` `SECONDS_PER_MINUTE`, `SECONDS_PER_HOUR`, `SECONDS_PER_DAY`, `MICROS_PER_SECOND`, `MICROS_FRACTION_DIGITS` | Used only by the removed functions and by tests that task 2.3 deletes (task 2.2) |
| Test | `src/query/results.rs`: the sections "Tests for ResultSet::parse_date_to_days", "Tests for ResultSet::parse_timestamp_to_micros", and "Tests for the parse_date_to_days month table", and `test_parse_timestamp_to_micros_time_part_without_a_colon_adds_nothing` | Test removed functions and constants; `src/types/conversion.rs` covers each case, and the one missing case moves there (task 2.3) |
| Function | `src/import/arrow.rs` `format_date32`, `days_to_ymd`, `format_timestamp_micros`; `src/import/parquet.rs` `format_date32` | Replaced by the shared `types::conversion` functions (task 4.4) |
| Code | `src/import/parquet.rs` `format_timestamp` and `src/adbc_ffi.rs` `arrow_value_to_parameter`: the inline `/`, `%`, `unsigned_abs`, nanosecond, and chrono date and timestamp formatting | Replaced by calls to the shared functions (task 4.4) |
| Test | `src/import/arrow.rs` `test_format_date32`, `test_format_timestamp_micros`, `test_days_to_ymd_edge_cases`, `test_days_to_ymd_before_epoch`, `test_format_date32_before_epoch`, `test_format_timestamp_micros_truncates_negative_day_offset`; `src/import/parquet.rs` `test_format_date32` | Test removed functions or the truncating split; task 4.1 covers each case (task 4.4) |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Pre-1970 DATE and TIMESTAMP query results keep their calendar day | Integration | `tests/integration_tests.rs` | `test_pre_1970_dates_and_timestamps_keep_their_calendar_day` |
| Pre-1970 DATE and TIMESTAMP query results keep their calendar day | Integration | `tests/websocket_integration_tests.rs` | `test_ws_pre_1970_dates_and_timestamps_keep_their_calendar_day` |
| Pre-1970 DATE and TIMESTAMP query results keep their calendar day | Unit | `src/query/results.rs` | `test_column_major_to_record_batch_keeps_pre_1970_dates_and_timestamps` |
| Pre-1970 DATE and TIMESTAMP query results keep their calendar day | Unit | `src/transport/native/result_parser.rs` | `single_pass_dates_and_timestamps_before_1970_keep_their_calendar_day` |
| DATE and TIMESTAMP values count days in the proleptic Gregorian calendar | Unit | `src/types/conversion.rs` | `test_ymd_to_days_matches_chrono_for_every_exasol_date`, `test_parse_timestamp_to_micros_matches_chrono_for_every_exasol_year`, `test_parse_date_to_days_before_1970`, `test_parse_date_to_days_century_leap_years`, `test_parse_timestamp_to_micros_before_1970`, `test_ymd_hms_nanos_to_micros_before_epoch`, `test_ymd_to_days_julian_only_leap_day_reads_as_march_first` |
| Lossless conversion validation (new step: February 29 of a Julian-only leap year) | Unit | `src/types/conversion.rs` | `test_ymd_to_days_julian_only_leap_day_reads_as_march_first` |
| Export keeps pre-1970 DATE and TIMESTAMP values | Integration | `tests/import_export_tests.rs` | `test_parquet_export_keeps_pre_1970_dates_and_timestamps` |
| Pre-1970 DATE and TIMESTAMP values round-trip through RecordBatch import and export | Integration | `tests/import_export_tests.rs` | `test_arrow_round_trip_keeps_pre_1970_dates_and_timestamps` |
| RecordBatch import formats pre-epoch timestamps of every time unit as times before the epoch | Unit | `src/import/arrow.rs`, `src/types/conversion.rs` | `test_format_timestamp_before_epoch_in_every_unit`; `test_format_timestamp_micros_literals`, `test_format_timestamp_floors_every_unit` |
| RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range | Unit | `src/import/arrow.rs` | `test_format_value_rejects_dates_and_timestamps_outside_exasol_range` |
| CSV-path Parquet import keeps pre-epoch timestamps with fractional seconds | Integration | `tests/import_export_tests.rs` | `test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps` |
| CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch | Unit | `src/import/parquet.rs` | `test_format_timestamp_before_epoch_in_every_unit` |
| CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range | Unit | `src/import/parquet.rs` | `test_format_arrow_value_rejects_dates_and_timestamps_outside_exasol_range` |
| Parameter binding formats pre-epoch DATE and TIMESTAMP values and rejects values outside Exasol's range | Unit | `src/adbc_ffi.rs` | `test_arrow_value_to_parameter_formats_pre_epoch_values`, `test_arrow_value_to_parameter_rejects_values_outside_exasol_range` |

- "DATE and TIMESTAMP values count days in the proleptic Gregorian calendar" and the time-unit, range, and parameter-binding scenarios use unit tests only. They describe pure conversions with no I/O, and the full date range cannot be exercised through a database query in reasonable time. The integration tests check the same rule at the issue's examples and the range boundaries.
- The query-result scenario also has unit tests on each transport's decoding path, so a regression on one transport fails without Docker.
- "Lossless conversion validation" keeps its first two steps unchanged. This plan adds only the step for February 29 of a Julian-only leap year. The Julian-leap-day unit test covers that step through `ymd_to_days` and through `parse_date_to_days`, which can return an error.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| type-mapping/exasol-to-arrow | `cargo build --release --features 'ffi websocket'`, then in a scratch virtual environment with `pip install adbc-driver-manager pyarrow` (the packages CI installs): `python -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='AdbcDriverExasolInit', db_kwargs={'uri':'exasol://sys:exasol@localhost:8563?tls=true&validateservercertificate=0'}); cur=c.cursor(); cur.execute(\"SELECT DATE '1968-01-01' D, DATE '0001-01-01' E, TIMESTAMP '1950-06-15 00:00:00' T FROM DUAL\"); print(cur.fetch_arrow_table().to_pylist())"`, then the same with `&transport=websocket` appended to the URI | Both runs print `[{'D': datetime.date(1968, 1, 1), 'E': datetime.date(1, 1, 1), 'T': datetime.datetime(1950, 6, 15, 0, 0)}]` |
| import-export/arrow-recordbatch | In the same environment and connection (native URI): run `CREATE SCHEMA PRE1970_CHECK`, `OPEN SCHEMA PRE1970_CHECK`, and `CREATE TABLE T (TS TIMESTAMP(6))`, ingest `pyarrow.table({'TS': pyarrow.array([-43200000000], pyarrow.timestamp('us'))})` with `cur.adbc_ingest('T', table, mode='append')`, run `SELECT TO_CHAR(TS, 'YYYY-MM-DD HH24:MI:SS.FF6') FROM T`, then `DROP SCHEMA PRE1970_CHECK CASCADE` | The SELECT returns `1969-12-31 12:00:00.000000` |
| import-export/parquet-export | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests test_parquet_export_keeps_pre_1970_dates_and_timestamps -- --nocapture` | The Parquet file holds Date32 -731 and -719162 and timestamps -616896000000000 and -1, and the test passes |
| import-export/parquet-io | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps -- --nocapture` | Exasol returns `1969-12-31 23:59:59.500000` and `1969-12-31 23:59:59.999999`, and the test passes |

Exasol runs in Docker per `AGENTS.md` § Testing: `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`, ready when `exapump sql 'select 1'` returns `1`.

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (WebSocket only) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Unit test | `cargo test --lib` | 0 failures |
| Unit test (FFI parameter binding) | `cargo test --lib --features ffi arrow_value_to_parameter` | 0 failures |
| Unit test (WebSocket) | `cargo test --lib --features websocket` | 0 failures |
| Integration test (native) | `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| Integration test (WebSocket) | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Import/export test | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| CI test-target guard | `python3 scripts/check_ci_test_targets.py` | Exit 0: no bare `#[ignore]` in the new tests |
| Driver manager test | `REQUIRE_EXASOL=1 cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures, run after the FFI build |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Check passes: total production coverage at least 80%, every file at least 50% |
