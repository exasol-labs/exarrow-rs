# Code Review Findings: fix-bulk-ingest-type-coverage

## Summary
- Files reviewed: 11
- Total findings: 8 (standard: 8, expert: 0)
- `cargo clippy --all-targets --all-features -- -D warnings` reports no issues on the working tree.
- The changes match the plan's tasks and scenarios. No finding changes observable behavior. The findings cover named constants, a duplicated DATE conversion, a pass-through helper, a boolean flag parameter, one missing boundary test, and test helper shape.

## Standard fixes

### src/types/conversion.rs

#### [MAGIC_NUMBER] The INTERVAL DAY TO SECOND writer splits the time part with bare literals
- Location: lines 431-437 (`format_interval_day_to_second`), line 1550 (`duration_millis` in tests), line 1586 (round-trip assertion in `test_interval_text_writer_writes_exasol_interval_text`)
- Issue: `format_interval_day_to_second` divides the time part by the literals `3_600_000`, `60_000`, `1_000`, and `60`. The interval parser in the same file uses the named constants `NANOS_PER_HOUR` and `NANOS_PER_MINUTE`. The test helper `duration_millis` and the day-time round-trip assertion repeat `86_400_000` and `1_000_000`, although the module defines `MILLIS_PER_DAY` and `NANOS_PER_MILLISECOND`.
- Fix: In src/types/conversion.rs, add the module constants `MILLIS_PER_SECOND: u64 = 1_000`, `MILLIS_PER_MINUTE: u64 = 60 * MILLIS_PER_SECOND`, `MILLIS_PER_HOUR: u64 = 60 * MILLIS_PER_MINUTE`, `SECONDS_PER_MINUTE: u64 = 60`, and `MINUTES_PER_HOUR: u64 = 60`. In `format_interval_day_to_second`, write the four fields as `time / MILLIS_PER_HOUR`, `time / MILLIS_PER_MINUTE % MINUTES_PER_HOUR`, `time / MILLIS_PER_SECOND % SECONDS_PER_MINUTE`, and `time % MILLIS_PER_SECOND`. In the tests module, replace `86_400_000` with `MILLIS_PER_DAY` and `1_000_000` with `NANOS_PER_MILLISECOND` in `duration_millis` and in the expression `i64::from(days) * 86_400_000 + milliseconds`. Leave the scenario input values, such as `(0, 86_400_000)`, unchanged. Run the conversion.rs unit tests.

### src/import/arrow.rs

#### [INFORMATION_COUPLING] Date64 conversion duplicates the day length and the DATE range message of the types component
- Location: lines 551-567 (`format_date64`)
- Issue: `format_date64` defines its own `MILLIS_PER_DAY` and its own message "is outside 0001-01-01 to 9999-12-31, the range Exasol accepts". src/types/conversion.rs already owns `MILLIS_PER_DAY` (line 15), `format_date32` with the same range message (line 116), and `format_timestamp`, which converts a unit value and renames the range error in the same way. The architecture assigns the conversion of DATE values between Arrow counts and Exasol text to the types component. A change to either the day length or the range message now needs an edit in both modules.
- Fix: In src/types/conversion.rs, add `pub fn format_date64(milliseconds: i64) -> Result<String, String>` directly after `format_date32`, with a doc comment stating that it formats the day that holds the instant as `YYYY-MM-DD` and fails outside 0001-01-01 to 9999-12-31. Its body floors with `milliseconds.div_euclid(MILLIS_PER_DAY)`, converts the day count with `i32::try_from`, calls `format_date32`, and maps both failures to `format!("Date64 value {milliseconds} is outside 0001-01-01 to 9999-12-31, the range Exasol accepts")`. Move the comment `// Flooring keeps a pre-epoch value on the day that holds it.` into that function. In src/import/arrow.rs, reduce `format_date64` to reading the value with `array.as_primitive::<Date64Type>().value(row_idx)` and returning `conversion::format_date64(millis).map_err(ImportError::ConversionError)`, and delete its local `MILLIS_PER_DAY`. Add the unit test `test_format_date64_floors_to_the_day_that_holds_it_and_rejects_dates_outside_exasol_range` to src/types/conversion.rs: assert that `-1` becomes `1969-12-31`, `-86_400_001` becomes `1969-12-30`, and `1_577_836_800_000` becomes `2020-01-01`, and that `253_402_300_800_000` and `i64::MAX` fail with a message that contains the value. Keep the existing src/import/arrow.rs tests unchanged and run them.

#### [SHRINKABLE] `format_decimal128` is a pass-through to `format_decimal_text`
- Location: line 673 (`format_decimal128`), line 608 (`format_decimal128_value`), line 1068 (`test_format_decimal128`)
- Issue: The whole body of `format_decimal128` is `format_decimal_text(value.to_string(), scale)`. `format_decimal256_value` calls `format_decimal_text` directly, so the two decimal formatters reach the same helper through different paths.
- Fix: In src/import/arrow.rs, delete `format_decimal128`. In `format_decimal128_value`, return `Ok(format_decimal_text(decimals.value(row_idx).to_string(), decimals.scale()))`. Rename `test_format_decimal128` to `test_format_decimal_text_places_the_decimal_point_by_scale` and change each assertion to call `format_decimal_text(<value>.to_string(), <scale>)` with the same expected text, for example `format_decimal_text(12345.to_string(), 2)` equals `"123.45"`. Run the arrow.rs unit tests.

### src/import/parquet.rs

#### [BOOLEAN_FLAG_PARAMETER] `check_footer_before_create` takes a flag that selects one of two checks
- Location: line 319 (`check_footer_before_create`), callers at lines 268 and 809, test call at line 2936
- Issue: The parameter `use_native: bool` decides whether the function runs `check_native_auto_create_types` or `check_intervals_keep_month_part`. These are two different rules for two import paths behind one name.
- Fix: In src/import/parquet.rs, replace `check_footer_before_create` with two functions, `check_native_auto_create_footer(path: &Path) -> Result<(), ImportError>` and `check_csv_auto_create_footer(path: &Path) -> Result<(), ImportError>`. Each one calls `read_footer(path)`, runs its check (`check_native_auto_create_types(footer.schema())` or `check_intervals_keep_month_part(&footer)`), and maps the reason with one private helper `rejected_footer(path: &Path, reason: String) -> ImportError` that returns `ImportError::SchemaInferenceError(format!("Parquet file '{}': {reason}", path.display()))`. Give each function a doc comment that states its rule and that it runs before `CREATE TABLE`. In `import_from_parquet` and `import_from_parquet_files`, call `check_native_auto_create_footer` when `use_native` is set and `check_csv_auto_create_footer` otherwise. Rename `test_check_footer_before_create_names_a_file_whose_footer_cannot_be_read` to `test_auto_create_footer_checks_name_a_file_whose_footer_cannot_be_read` and assert the same error for both functions on each path. Run the parquet.rs unit tests.

#### [MISSING_BOUNDARY_TEST] No test shows that a file without an Arrow schema and without an Interval column still imports on the CSV path
- Location: line 392 (`check_intervals_keep_month_part`), tests at lines 2860-2925
- Issue: Every test that writes a Parquet file without an Arrow schema puts an Interval column in it. Parquet writers outside Arrow usually store no Arrow schema in the footer. A change that rejected every such file, for example by dropping the Interval filter in `check_intervals_keep_month_part`, would pass every current test.
- Fix: In the tests module of src/import/parquet.rs, add `test_csv_path_parquet_import_accepts_a_file_without_arrow_schema_and_without_intervals`. Write `id_and_x(Arc::new(StringArray::from(vec!["a"])))` with `without_arrow_schema()`. Open it with `open_parquet_csv_reader(std::fs::File::open(&path).expect("open"), &ParquetImportOptions::default())`, render it with `render_parquet_csv`, and assert the CSV text `"1,a\n"`. Then call `import_from_parquet(record_and_fail_sql(&log), "T", &path, auto_create_options(), false)` and assert `ImportError::SqlError` and that the first recorded statement starts with `CREATE TABLE`. Run the new test.

#### [TOO_MANY_ARGUMENTS] Test helper `write_parquet` takes four arguments
- Location: line 2659 (`write_parquet` in tests), callers at lines 2735, 2767, 2840, 2868, 2874, 2913, 2947, 2976
- Issue: `write_parquet(dir, name, batch, options)` takes four arguments. `dir` and `name` only form the target path, which the integration test helper `write_parquet_with(path, batch, options)` in tests/import_export_tests.rs already takes as one argument.
- Fix: In the tests module of src/import/parquet.rs, change the signature to `fn write_parquet(path: std::path::PathBuf, batch: &RecordBatch, options: parquet::arrow::arrow_writer::ArrowWriterOptions) -> std::path::PathBuf`, write the file at `path`, and return `path`. At every caller, pass `temp_dir.path().join("<name>")` in place of the `dir` and `name` arguments. Run the parquet.rs unit tests.

### tests/driver_manager_tests.rs

#### [TOO_MANY_ARGUMENTS] Test helper `ingest_batch` takes five arguments
- Location: line 4279 (`ingest_batch`), callers at lines 4412, 4482, 4548, 4598, 4655, 4683, 4716
- Issue: `ingest_batch(conn, target, mode, catalog, batch)` takes five arguments. `target`, `mode`, and `catalog` together describe one ingest destination.
- Fix: In tests/driver_manager_tests.rs, add the struct `IngestTarget<'a> { table: &'a str, mode: &'a str, catalog: Option<OptionValue> }` with a one-line doc comment, and change the helper to `fn ingest_batch<C: AdbcConnection>(conn: &mut C, target: IngestTarget, batch: RecordBatch) -> Result<Option<i64>, adbc_core::error::Error>`, setting `adbc.ingest.target_table` from `target.table`, `adbc.ingest.mode` from `target.mode`, and `adbc.ingest.target_catalog` from `target.catalog` when it is `Some`. Update every caller to pass an `IngestTarget` with the same values. Run `cargo build --release --features ffi`, then the seven bulk-ingest tests that end the file.

#### [SHRINKABLE] `table_count` and the COUNT(*) assertions duplicate the existing `integers_from` helper
- Location: line 4350 (`table_count`), uses at lines 4433, 4436, 4623, 4668, 4693, 4731
- Issue: `table_count` returns a row count as a `String`, and four assertions read `SELECT COUNT(*)` results as text through `first_column_text`. The file already has `integers_from(conn, sql)`, which returns the values of a one-column integer query as `Vec<i64>`.
- Fix: In tests/driver_manager_tests.rs, delete `table_count`. Replace `assert_eq!(table_count(&mut conn, &schema_name), "1")` with `assert_eq!(integers_from(&mut conn, format!("SELECT COUNT(*) FROM EXA_ALL_TABLES WHERE TABLE_SCHEMA = '{schema_name}'")), vec![1])`, and the `"0"` assertion at line 4731 with the same call compared against `vec![0]`. Replace each `first_column_text(&mut conn, "<SELECT COUNT(*) ...>")` comparison at lines 4433, 4623, 4668, and 4693 with `integers_from(&mut conn, <the same SQL as a String>)` compared against `vec![<n>]`. Keep `first_column_text` for `column_types_of` and the boolean interval comparison. Run `cargo build --release --features ffi`, then the seven bulk-ingest tests that end the file.

## Expert fixes
[none]
