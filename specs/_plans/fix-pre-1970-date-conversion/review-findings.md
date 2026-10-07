# Code Review Findings: fix-pre-1970-date-conversion

## Summary
- Files reviewed: 12
- Total findings: 5 (standard: 5, expert: 0)

## Standard fixes

### src/types/conversion.rs

#### [CONTEXTLESS_ERROR] Out-of-range timestamp error names a converted value in the wrong unit
- Location: lines 124-138, `format_timestamp_micros` and `format_timestamp`
- Issue: `format_timestamp` passes the range error of `format_timestamp_micros` through unchanged. For any unit other than `Microsecond`, that error names the value after conversion and labels it as microseconds. A `Timestamp(Second)` value 253402300800 (10000-01-01) fails with "Timestamp value 253402300800000000 microseconds is outside ...", but the caller supplied 253402300800 seconds. The spec scenarios require an error that names the value. The existing tests pass only because they use `Second` `i64::MAX`, which fails earlier in the overflow branch of `timestamp_unit_to_micros` with the original value.
- Fix: In src/types/conversion.rs, change `format_timestamp` so that it replaces the error of `format_timestamp_micros` with `format!("Timestamp value {value} in unit {unit:?} is outside 0001-01-01 00:00:00 to 9999-12-31 23:59:59.999999, the range Exasol accepts")`. Keep the overflow error of `timestamp_unit_to_micros` unchanged. Add the test `test_format_timestamp_names_the_value_in_its_own_unit` to the test module: `format_timestamp(&TimeUnit::Second, 253_402_300_800)` returns an `Err` whose message contains `253402300800 in unit Second` and does not contain `microseconds`.

#### [MISSING_DESIGN_INTENT] `format_timestamp` doc comment omits the rounding and the overflow failure
- Location: line 135
- Issue: The doc comment says only that the function formats any time unit "like `format_timestamp_micros`". It does not state that a nanosecond value is floored to whole microseconds, which drops the sub-microsecond digits of a TIMESTAMP(7) to TIMESTAMP(9) value. It also does not state that a second or millisecond value fails when its conversion to microseconds overflows `i64`. The callers in `src/import/arrow.rs`, `src/import/parquet.rs`, and `src/adbc_ffi.rs` depend on both behaviors. Only the inline comment inside the private `timestamp_unit_to_micros` records the first one.
- Fix: In src/types/conversion.rs, rewrite the doc comment of `format_timestamp` in at most two lines. State that it converts the value to whole microseconds and floors a nanosecond value toward the earlier instant, so sub-microsecond digits are dropped. State that it fails when the conversion overflows `i64` or the instant is outside the years 1 to 9999.

#### [PERFORMANCE_ISSUE] Exhaustive round-trip test takes 96% of the unit-test run
- Location: line 782, `test_format_date32_round_trips_every_exasol_day`
- Issue: Measured on this branch, the test alone runs in 5.97 s (`cargo test --lib types::conversion::tests::test_format_date32_round_trips_every_exasol_day -- --exact`). The whole `cargo test --lib` run of 1598 tests finishes in 6.20 s. The day-count test `test_ymd_to_days_matches_chrono_for_every_exasol_date` covers the same 3,652,059 days in 0.40 s, because it allocates no `String`. CI runs the lib tests at least twice, in `cargo llvm-cov --lib` under coverage instrumentation and in `cargo test --lib --features websocket` (`.github/workflows/ci.yml` lines 137 and 141). The parse side is already checked against chrono on every day. The round trip therefore adds coverage only for the day offset `DAYS_FROM_CE_TO_EPOCH` and the year filter of `format_date32`, and every month boundary shows both.
- Fix: In src/types/conversion.rs, rename `test_format_date32_round_trips_every_exasol_day` to `test_format_date32_round_trips_every_month_boundary`. For every year from 1 to 9999 and every month from 1 to 12, compute `first` as the signed day difference from `chrono::NaiveDate::from_ymd_opt(1970, 1, 1)` to `chrono::NaiveDate::from_ymd_opt(year, month, 1)`. For each `day` in `[first - 1, first]`, skip -719163, and assert `parse_date_to_days(&format_date32(day).unwrap()) == Ok(day)` with an assertion message that names the day. Run the test and confirm that it finishes in under 0.5 s.

#### [REDUNDANT_COMMENT] Moved test restates its arithmetic and sits outside its section
- Location: lines 347-348
- Issue: `test_parse_timestamp_to_micros_hours_and_minutes_only` carries the inline comment `// 01:30 without seconds is 5400 seconds`, which restates the assertion. `AGENTS.md` § Code style forbids comments that restate the code. The test also sits at the top of the test module, while its sibling tests sit under `// Tests for parse_timestamp_to_micros` at line 513.
- Fix: In src/types/conversion.rs, delete the comment line in `test_parse_timestamp_to_micros_hours_and_minutes_only`. Move the test into the `// Tests for parse_timestamp_to_micros` section, directly after `test_parse_timestamp_to_micros_with_short_fraction`.

### tests/import_export_tests.rs

#### [SHRINKABLE] Row assertions index rows without a length check
- Location: lines 3622-3626 (`test_arrow_round_trip_keeps_pre_1970_dates_and_timestamps`) and lines 3713-3714 (`test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps`)
- Issue: The CSV-path test reads `rows[0][1]` and `rows[1][1]` without checking the row count. A missing row therefore fails with an index-out-of-bounds panic, not with an assertion that shows the stored rows. The round-trip test uses a length assertion plus a five-line zip loop for the same check, and a mismatch there does not show which row failed.
- Fix: In tests/import_export_tests.rs, in `test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps`, replace the two `assert_eq!(rows[..][1], ...)` lines with `let stored: Vec<&str> = rows.iter().map(|row| row[1].as_str()).collect();` and `assert_eq!(stored, ["1969-12-31 23:59:59.500000", "1969-12-31 23:59:59.999999"]);`. In `test_arrow_round_trip_keeps_pre_1970_dates_and_timestamps`, replace the `assert_eq!(rows.len(), expected.len());` line and the zip loop with `let stored: Vec<(&str, &str)> = rows.iter().map(|row| (row[1].as_str(), row[2].as_str())).collect();` and `assert_eq!(stored, expected);`.

## Expert fixes
[none]
