# Plan: fix-bulk-ingest-type-coverage

## Summary

ADBC bulk ingest checks the target catalog and every bound column type before its first SQL statement, with one column type rule in every mode, so an ingest that these checks reject never drops, creates, or writes a table. It converts the bound values after the DDL, as it does today, so a value that the import CSV conversion rejects fails a replace-mode ingest after the old table is dropped. It ingests Date64, Decimal256, string dictionaries, Utf8View, BinaryView, FixedSizeBinary, and every Interval unit end to end (#111, #109). The import CSV conversion, the conversion of Arrow values to CSV text in `src/import/arrow.rs`, serves RecordBatch import, Arrow IPC import, ADBC ingest, and CSV-path Parquet import, and `TypeMapper::arrow_to_exasol` maps only the types that the import CSV conversion converts.

## Context

- #111: `TypeMapper::arrow_to_exasol` (`src/types/mapping.rs:254`) maps Date64, Decimal256, and Interval to an Exasol column type, but `format_value` (`src/import/arrow.rs:332`) cannot convert them. In create mode the table stays behind empty; in replace mode the old table is already dropped when the import fails.
- `arrow_to_exasol` maps every Interval unit to INTERVAL DAY TO SECOND(3), which is wrong for Interval(YearMonth), and `ExasolType::to_ddl_type` writes the interval types without a leading precision, so a created column holds at most 99 years or days.
- `arrow_to_exasol` passes any decimal precision and scale through, so `Decimal128(38, s)` produces `DECIMAL(38,s)` DDL, which Exasol rejects after replace mode has dropped the table.
- `execute_bulk_ingest` (`src/adbc_ffi.rs:2065`) runs `DROP TABLE` before `CREATE TABLE` in replace mode and checks column types only while it builds the DDL. Append mode checks no column type before the import.
- `arrow_to_exasol` also feeds Parquet auto-create (`src/types/infer.rs:302`, `:330`). CSV-path Parquet import has its own value formatter (`src/import/parquet.rs:867`), which drops the sign of a negative decimal below 1 and quotes on `,` instead of the configured separator.
- #109: `execute_bulk_ingest` never reads `adbc.ingest.target_catalog`, so any catalog value is ignored. Exasol has one catalog, `EXA`, and `catalog_is_exasol` (`src/adbc_ffi.rs:516`) already checks it case-insensitively for GetObjects.
- The architecture delta (`architecture.md` in this plan directory) changes the import and types components and the ADBC C ABI interface, and adds three constraints.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Bulk Ingestion | CHANGED | `adbc-driver/bulk-ingestion/spec.md` |
| Boundaries and Validation | CHANGED | `type-mapping/boundaries-and-validation/spec.md` |
| Arrow RecordBatch Import/Export | CHANGED | `import-export/arrow-recordbatch/spec.md` |
| Arrow RecordBatch DATE and TIMESTAMP Conversion | CHANGED | `import-export/arrow-recordbatch-dates/spec.md` |
| Parquet I/O | CHANGED | `import-export/parquet-io/spec.md` |

## Impact

- ADBC ingest with a column type that the driver cannot ingest fails with `InvalidArguments` before any SQL statement, in every mode. Replace mode keeps the existing table and its rows, and create mode leaves no table. In append mode the status changes from `Internal` to `InvalidArguments`.
- ADBC ingest accepts Date64, Decimal256 up to precision 36, Dictionary with Utf8 or LargeUtf8 values, Utf8View, BinaryView, FixedSizeBinary, Interval(YearMonth), Interval(DayTime), and Interval(MonthDayNano). RecordBatch import, Arrow IPC import, and CSV-path Parquet import convert the same types.
- Breaking: appending Decimal128 with precision above 36 now fails with InvalidArguments before any SQL. The same holds for a negative scale or a scale above the precision.
- Breaking: an `adbc.ingest.target_catalog` value other than `EXA` in any letter case fails with `InvalidArguments`. Before, the driver ignored it.
- Changed: the public `TypeMapper::arrow_to_exasol` newly maps Utf8View, string dictionaries, BinaryView, and FixedSizeBinary, so CSV-path Parquet auto-create creates a column for these four types instead of failing schema inference. It maps Interval(YearMonth) to INTERVAL YEAR TO MONTH instead of INTERVAL DAY TO SECOND. It rejects any decimal with a precision above 36, a negative scale, or a scale above the precision, so a Parquet auto-create import of such a decimal column fails with `ImportError::SchemaInferenceError` before `CREATE TABLE`.
- Changed: native Parquet auto-create accepts only Boolean, the signed and unsigned integer types, Float32, Float64, Utf8, LargeUtf8, Binary, LargeBinary, Date32, Date64, Timestamp, Decimal128, and Decimal256 fields. It rejects every other field, such as Utf8View, string dictionary, BinaryView, FixedSizeBinary, or Interval, with `ImportError::SchemaInferenceError` before `CREATE TABLE`, and the message names `with_native_parquet(Some(false))`. Before, it created an INTERVAL DAY TO SECOND(3) column for every Interval unit.
- Changed: CSV-path Parquet import fails with `ImportError::SchemaInferenceError` on an Interval field from a file whose footer holds no Arrow schema, before `CREATE TABLE` or `IMPORT`. The Parquet reader reads such a column as Interval(DayTime) and drops its month part, so converting it would store wrong values. Before, CSV-path Parquet import could not convert any Interval column.
- Changed: the public `ExasolType::to_ddl_type` writes `INTERVAL YEAR(9) TO MONTH` and `INTERVAL DAY(9) TO SECOND(p)`, so a column that ADBC ingest or CSV-path Parquet auto-create creates holds up to 999,999,999 years or days.
- Changed: an interval value that Exasol cannot store unchanged fails the import with `ImportError::ConversionError`: an Interval(MonthDayNano) value with a month component, a day-time value with a fraction of a millisecond, or a day-time value of more than 999,999,999 days. ADBC ingest converts values after its DDL, so such a value, or a date outside Exasol's range, fails a replace-mode ingest after the old table is dropped and leaves an empty table in create mode. Only the type and catalog checks guarantee that a rejected ingest changes no table.
- Fix: CSV-path Parquet import (servers before 2025.1.11, or `with_native_parquet(Some(false))`) keeps the sign of negative decimals, quotes values that contain the configured column separator, and writes floats without an exponent. Native Parquet import into an existing table does not change.
- Fix: a query that reads the INTERVAL YEAR TO MONTH value `-178956970-08`, the smallest month count that Arrow holds, returns it instead of failing.
- Follow-up outside this repository: after the release, update the driver page in adbc-drivers/exasol to list the ingestible Arrow types and the target catalog `EXA` as supported.

## Implementation Tasks

1. Type mapping and the import CSV conversion
   - [ ] 1.1 In `TypeMapper::arrow_to_exasol` (`src/types/mapping.rs`), map Utf8View and Dictionary with an integer key and Utf8 or LargeUtf8 values to `VARCHAR(2000000)`, map BinaryView and FixedSizeBinary to `VARCHAR(2000000)`, accept Decimal128 and Decimal256 only with precision up to 36 and scale from 0 to the precision, map Interval(YearMonth) to `ExasolType::IntervalYearToMonth`, and map Interval(DayTime) and Interval(MonthDayNano) to `ExasolType::IntervalDayToSecond { precision: 3 }`. In `ExasolType::to_ddl_type`, write `INTERVAL YEAR(9) TO MONTH` and `INTERVAL DAY(9) TO SECOND(p)`, and update `test_to_ddl_type_interval`. Rewrite the doc comment of `arrow_to_exasol` to state that it maps only types that the import CSV conversion converts. Add the unit tests for both `type-mapping/boundaries-and-validation` scenarios and replace `test_arrow_decimal256_to_exasol`.
   - [ ] 1.2 In `src/types/conversion.rs`, next to `parse_interval`, add the INTERVAL text writer. The year-month writer takes an `i32` month count and computes in `i64`, so `i32::MIN` becomes `-178956970-08`. The day-time writer takes days and a time part, adds them as days of 86,400,000 ms in checked `i64` arithmetic, and fails for more than 999,999,999 days. The MonthDayNano writer fails for a month component or for nanoseconds that are not whole milliseconds, and then uses the day-time writer. Each error names the value. Add unit tests with the values of both interval scenarios of `import-export/arrow-recordbatch`, and assert that `parse_interval` reads every written text back to the same duration. Make `parse_interval_months` compute the month count in `i64`, apply the sign, and convert it to `i32` with a checked conversion, so `-178956970-08` parses to `i32::MIN`. Include `i32::MIN` in the round-trip assertion.
   - [ ] 1.3a In `src/import/arrow.rs`, replace the `match` in `format_value` with one dispatch: a function that resolves the value formatter of a column from its `DataType` once, or returns `ImportError::ConversionError` that names the type. `format_row` uses the formatters that it returns. Add a `pub(crate)` schema check that calls the function for every field and names the first column and Arrow type that it rejects. Add no new types: the dispatch converts exactly the types that `format_value` converts today, and the existing `src/import/arrow.rs` tests pass unchanged [expert]
   - [ ] 1.3b Add the new formatters to the dispatch of 1.3a: Date64 floors the millisecond value to days with `div_euclid(86_400_000)` and converts the day count to `i32` with a checked conversion that fails with `ImportError::ConversionError`, then uses `conversion::format_date32`. Decimal256 writes its decimal text with the column scale. For a negative scale, Decimal128 and Decimal256 both write the integer text followed by one zero per unit of negative scale, with no multiplication. A Dictionary with Utf8 or LargeUtf8 values looks up its key in the values array and formats that value. Utf8View formats as Utf8. BinaryView and FixedSizeBinary format as lowercase hex. The three Interval units use the writer from 1.2.
   - [ ] 1.4 Add unit tests in `src/import/arrow.rs`: one for "The import CSV conversion covers every Arrow type that has an Exasol column type", including the full digit strings for `Decimal128(38, -5)` holding 10^37 and `Decimal256(76, -10)` holding 10^76 - 1; one for "The import CSV conversion rejects interval values that INTERVAL DAY TO SECOND cannot hold", which calls `import_from_record_batch` with a closure that records every SQL statement and asserts an empty record; and one that compares `arrow_to_exasol` with the dispatch function from 1.3a over the type list of the first scenario and the rejected types of `type-mapping/boundaries-and-validation`, and asserts that `arrow_to_exasol` accepts a subset that differs only in Decimal128 and Decimal256 outside Exasol's DECIMAL limits. Add a test for "RecordBatch import formats a pre-epoch Date64 value as the day that holds it", and add the two Date64 values to `test_format_value_rejects_dates_and_timestamps_outside_exasol_range`.
   - [ ] 1.5 Make `record_batch_to_csv` in `src/import/parquet.rs` keep its signature and format each row with the row formatter of `src/import/arrow.rs`, made `pub(crate)`, using `CsvWriterOptions` built from the separator, delimiter, and null value of `ParquetImportOptions`. Delete `format_arrow_value` and its Parquet-local helpers. Rewrite `test_format_timestamp_before_epoch_in_every_unit` and `test_format_arrow_value_rejects_dates_and_timestamps_outside_exasol_range` of `src/import/parquet.rs` to convert through `record_batch_to_csv`, rename them to `test_record_batch_to_csv_formats_pre_epoch_timestamps_in_every_unit` and `test_record_batch_to_csv_rejects_dates_and_timestamps_outside_exasol_range`, and keep their `/// Scenario:` lines. Delete only the other unit tests that call the deleted helpers, and move any assertion that `src/import/arrow.rs` tests do not already cover. Keep the CSV-path streaming and value-range tests passing [expert]
   - [ ] 1.6 Add the integration test for "CSV-path Parquet import converts values as RecordBatch import does" to `tests/import_export_tests.rs`, writing the Parquet file with `ArrowWriter` so the Arrow schema carries the dictionary type.
   - [ ] 1.7 In `import_from_parquet` and `import_from_parquet_files` (`src/import/parquet.rs`), when `use_native` and `create_table_if_not_exists` are both set, read the Arrow schema of each file from its Parquet footer and check every field against an allow-list named once in `src/import/parquet.rs`. The allow-list holds the Arrow types that native auto-create accepted before this plan, minus Interval: Boolean, the signed and unsigned integer types, Float32, Float64, Utf8, LargeUtf8, Binary, LargeBinary, Date32, Date64, Timestamp, Decimal128, and Decimal256. Every other field fails before `CREATE TABLE` with `ImportError::SchemaInferenceError`, so a type that a later change maps in `arrow_to_exasol` stays rejected on the native path until it is added to the allow-list. The message names the file, the column, and its Arrow type, and names `with_native_parquet(Some(false))` as the way to import the file. Keep this check in `src/import/parquet.rs`, which owns the path choice, and keep the signatures of `infer_schema_from_parquet` and `infer_schema_from_parquet_files`. Add a unit test that calls both functions with `use_native` set and a closure that records every SQL statement, and asserts the error and an empty record; `import_from_parquet_files` needs two files, because it delegates a single file to `import_from_parquet`. Add the integration test for the native `import-export/parquet-io` scenario to `tests/import_export_tests.rs`, writing each file with `ArrowWriter`.
   - [ ] 1.8 Add the integration test for "RecordBatch import writes Arrow interval values as Exasol INTERVAL text" to `tests/import_export_tests.rs`. It creates `T`, imports the RecordBatch with `import_from_record_batch`, and checks each stored value with a SQL comparison against the `CAST` value of the scenario.
   - [ ] 1.9 In `src/import/parquet.rs`, reject a field whose Arrow type is Interval when the Parquet footer's key-value metadata holds no `parquet::arrow::ARROW_SCHEMA_META_KEY` entry, because the Parquet reader then reads a Parquet INTERVAL column as Interval(DayTime) and drops its month part. Fail with `ImportError::SchemaInferenceError` that names the column and states that the column would lose its month part; a check that knows the file path also names the file. Run the check in `open_parquet_csv_reader`, which serves the single-file, multi-file, and stream CSV paths; `open_parquet_csv_file` keeps adding the file path to its errors. On the CSV path with `create_table_if_not_exists`, also run it before `CREATE TABLE` in `import_from_parquet` and `import_from_parquet_files`, in the footer pass of task 1.7, which then runs whenever `create_table_if_not_exists` is set and applies the allow-list on the native path and this check on the CSV path. Add a unit test that writes the file with `ArrowWriterOptions::with_skip_arrow_metadata(true)`, calls both functions with a closure that records every SQL statement, and asserts the error and an empty record; `import_from_parquet_files` needs two files. Add the integration test for the `import-export/parquet-io` scenario "CSV-path Parquet import rejects an INTERVAL column from a file without an Arrow schema" to `tests/import_export_tests.rs`.
2. ADBC bulk ingest checks
   - [ ] 2.1 In `execute_bulk_ingest` (`src/adbc_ffi.rs`), read `adbc.ingest.target_catalog`. When it is set and is not a string that `catalog_is_exasol` accepts, fail with `AdbcStatus::InvalidArguments` and a message that names the value and states that Exasol has only the catalog `EXA`.
   - [ ] 2.2 In `execute_bulk_ingest`, check the bound schema next to the other argument checks, before the connection lookup and before any SQL, with the same rule in every mode: map every field with `TypeMapper::arrow_to_exasol` and run the schema check from 1.3a. Map either error to `AdbcStatus::InvalidArguments` with a message that names the column and its Arrow type. The create modes then build their DDL with `generate_create_table_ddl`.
   - [ ] 2.3 Add unit tests in `src/adbc_ffi.rs` on a statement without a connection, so an `InvalidArguments` result proves that the check runs before any SQL: each uningestible type of the first `adbc-driver/bulk-ingestion` scenario, including `Decimal128(38, 2)` and `Decimal128(10, -2)`, in every mode, a non-`EXA` catalog (`BOGUS`, empty string, integer value), and `EXA` or `exa`, which reaches the missing-connection error.
   - [ ] 2.4 Add the seven `adbc-driver/bulk-ingestion` integration tests to `tests/driver_manager_tests.rs`, each in its own schema from `generate_unique_test_name`, with one `/// Scenario:` line per scenario. The interval test reads the created column types from `EXA_ALL_COLUMNS`.
3. Documentation, changelog, and version
   - [ ] 3.1 Add an "Arrow to Exasol Type Conversions" section to `docs/type-mapping.md`: a table of each accepted Arrow type and its Exasol column type, including the three Interval units; the note that binary types are stored as lowercase hex text in `VARCHAR(2000000)` because Exasol has no binary column type; the DECIMAL precision limit of 36, which ADBC ingest applies in every mode, while RecordBatch import into an existing table still converts larger decimals; the interval values that fail, and the note that a value read from an INTERVAL YEAR TO MONTH column needs a cast to Interval(YearMonth) before it is written back; the rejected types; the note that Parquet auto-create uses the same mapping, except that native Parquet auto-create accepts only the types it accepted before this plan minus Interval, so it rejects Utf8View, string dictionary, BinaryView, FixedSizeBinary, and Interval columns, and that `with_native_parquet(Some(false))` imports such a file; the note that CSV-path Parquet import rejects an Interval column from a file written without its Arrow schema, because the Parquet reader cannot tell the interval unit and would drop the month part; and the note that the hex text rule does not apply to native Parquet import, because Exasol's Parquet reader loads those values.
   - [ ] 3.2 Add a "Bulk ingestion" subsection to `docs/driver-manager.md` after "Parameters and row counts": a Python `cursor.adbc_ingest` example with `mode` and `db_schema_name`, the rule that `catalog_name` accepts only `EXA`, the catalog and column type checks before any SQL statement, which guarantee that a rejected catalog or column type changes no table, the warning that values convert after the DDL, so a value which the import CSV conversion rejects, such as a date outside 0001-01-01 to 9999-12-31 or an Interval(MonthDayNano) value with a month component, fails a replace-mode ingest after the old table is dropped and leaves an empty table in create mode, and a link to the new type table.
   - [ ] 3.3 Bump `version` in `Cargo.toml` from 0.28.0 to 0.29.0 and update the `exarrow-rs` entry in `Cargo.lock`.
   - [ ] 3.4 Add a `## 0.29.0` section to `CHANGELOG.md` with the entries of the Impact section except the follow-up bullet, referencing #111 and #109.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Type mapping and the import CSV conversion | 1.1-1.9 | none | spec deltas `type-mapping/boundaries-and-validation`, `import-export/arrow-recordbatch`, `import-export/arrow-recordbatch-dates`, `import-export/parquet-io`; `src/types/mapping.rs`, `src/types/conversion.rs` (`parse_interval`), `src/types/infer.rs` (Parquet inference caller), `src/import/arrow.rs`, `src/import/parquet.rs`, `tests/import_export_tests.rs` |
| B: ADBC ingest checks | 2.1-2.4 | A (uses the schema check from 1.3a and the mapping from 1.1) | spec delta `adbc-driver/bulk-ingestion`; `src/adbc_ffi.rs` (`execute_bulk_ingest`, `generate_create_table_ddl`, `catalog_is_exasol`, `to_adbc_error`), `tests/driver_manager_tests.rs` |
| C: Docs and release | 3.1-3.4 | A, B | Impact section of this plan; `docs/type-mapping.md`, `docs/driver-manager.md`, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/import/parquet.rs` `format_arrow_value` (~line 867) and its Parquet-local float, decimal, and escape helpers | Replaced by the import CSV conversion in `src/import/arrow.rs` (task 1.5) |
| Test | `src/import/parquet.rs` unit tests that call only the deleted helpers (~lines 1203-1262, 1928-1951), except the two tests that task 1.5 rewrites | Their functions are deleted; covered by `src/import/arrow.rs` tests |
| Test | `src/types/mapping.rs` `test_arrow_decimal256_to_exasol` | Asserts that `Decimal256(40, 5)` maps to `DECIMAL(40,5)`, which the plan rejects |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| A bound column type that the driver cannot ingest fails before any SQL statement | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_uningestible_column_type_before_any_sql` |
| A bound column type that the driver cannot ingest fails before any SQL statement | Unit | `src/adbc_ffi.rs` | `test_bulk_ingest_rejects_uningestible_column_type_in_every_mode_without_a_connection` |
| Date64, Decimal256, string dictionary, and Utf8View columns are ingested | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_stores_date64_decimal256_dictionary_and_utf8view` |
| Binary column types are stored as lowercase hexadecimal text | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_stores_binary_types_as_hex_text` |
| Interval columns are ingested | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_creates_and_stores_interval_columns` |
| An Interval(MonthDayNano) value with a month component fails the ingest | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_month_day_nano_value_with_months` |
| Target catalog EXA is accepted in any letter case | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_accepts_target_catalog_exa_in_any_case` |
| Target catalog EXA is accepted in any letter case | Unit | `src/adbc_ffi.rs` | `test_bulk_ingest_passes_target_catalog_exa_without_a_connection` |
| A target catalog other than EXA fails before any SQL statement | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_other_target_catalog_before_any_sql` |
| A target catalog other than EXA fails before any SQL statement | Unit | `src/adbc_ffi.rs` | `test_bulk_ingest_rejects_other_target_catalog_without_a_connection` |
| String, binary, date, timestamp, interval, boolean, and decimal type mappings for DDL generation | Unit | `src/types/mapping.rs` | `test_arrow_to_exasol_maps_string_binary_date_timestamp_interval_boolean_and_decimal_types` |
| Arrow types without an Exasol column type are rejected | Unit | `src/types/mapping.rs` | `test_arrow_to_exasol_rejects_types_without_an_exasol_column_type` |
| The import CSV conversion covers every Arrow type that has an Exasol column type | Unit | `src/import/arrow.rs` | `test_import_csv_conversion_covers_every_mapped_arrow_type` |
| RecordBatch import writes Arrow interval values as Exasol INTERVAL text | Unit | `src/types/conversion.rs` | `test_interval_text_writer_writes_exasol_interval_text` |
| RecordBatch import writes Arrow interval values as Exasol INTERVAL text | Integration | `tests/import_export_tests.rs` | `test_record_batch_import_stores_interval_values` |
| The import CSV conversion rejects interval values that INTERVAL DAY TO SECOND cannot hold | Unit | `src/import/arrow.rs` | `test_import_from_record_batch_rejects_interval_values_that_day_to_second_cannot_hold` |
| RecordBatch import formats a pre-epoch Date64 value as the day that holds it | Unit | `src/import/arrow.rs` | `test_format_date64_before_epoch_as_the_day_that_holds_it` |
| RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range | Unit | `src/import/arrow.rs` | `test_format_value_rejects_dates_and_timestamps_outside_exasol_range` (extended, task 1.4) |
| CSV-path Parquet import converts values as RecordBatch import does | Integration | `tests/import_export_tests.rs` | `test_parquet_import_csv_path_converts_values_as_record_batch_import` |
| Native Parquet import with table creation rejects Utf8View, dictionary, BinaryView, FixedSizeBinary, and interval columns | Integration | `tests/import_export_tests.rs` | `test_parquet_import_native_path_rejects_csv_only_types_before_create_table` |
| Native Parquet import with table creation rejects Utf8View, dictionary, BinaryView, FixedSizeBinary, and interval columns | Unit | `src/import/parquet.rs` | `test_native_parquet_auto_create_rejects_csv_only_types_before_any_sql` |
| CSV-path Parquet import rejects an INTERVAL column from a file without an Arrow schema | Integration | `tests/import_export_tests.rs` | `test_parquet_import_csv_path_rejects_interval_without_arrow_schema` |
| CSV-path Parquet import rejects an INTERVAL column from a file without an Arrow schema | Unit | `src/import/parquet.rs` | `test_csv_path_parquet_import_rejects_interval_without_arrow_schema_before_any_sql` |
| CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch (regression, task 1.5) | Unit | `src/import/parquet.rs` | `test_record_batch_to_csv_formats_pre_epoch_timestamps_in_every_unit` |
| CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range (regression, task 1.5) | Unit | `src/import/parquet.rs` | `test_record_batch_to_csv_rejects_dates_and_timestamps_outside_exasol_range` |

### Manual Testing

Run against the local Exasol container from `AGENTS.md` after `cargo build --release --features ffi` and `pip install adbc-driver-manager pyarrow`, from the repository root.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Bulk Ingestion, Boundaries and Validation, Arrow RecordBatch DATE and TIMESTAMP Conversion | Script M1 below | The eight lines listed under M1 |
| Bulk Ingestion (#109) | Script M2 below | The three lines listed under M2 |
| Arrow RecordBatch Import/Export, Parquet I/O | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1 test_parquet_import_csv_path_converts_values_as_record_batch_import test_record_batch_import_stores_interval_values test_parquet_import_csv_path_rejects_interval_without_arrow_schema` | 3 passed |

M1 ingests a Date64 column and an Interval(MonthDayNano) column, then tries a Time64 column in replace mode, a `Decimal128(38, 2)` column in append mode, and an Interval(MonthDayNano) value with a month component in append mode:

```bash
python3 - <<'EOF'
import datetime as dt, decimal, pyarrow as pa, adbc_driver_manager.dbapi as d
c = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="ExarrowDriverInit",
              db_kwargs={"uri": "exasol://sys:exasol@localhost:8563?validateservercertificate=0"}, autocommit=True)
cur = c.cursor()
cur.execute("CREATE SCHEMA IF NOT EXISTS PLAN_CHECK")
print(cur.adbc_ingest("D64", pa.table({"c": pa.array([dt.date(2020, 1, 1)], pa.date64())}), mode="create", db_schema_name="PLAN_CHECK"))
cur.execute('SELECT "c" FROM PLAN_CHECK.D64'); print(cur.fetchall())
iv = pa.array([(0, 2, 46_210_123_000_000)], pa.month_day_nano_interval())
print(cur.adbc_ingest("IV", pa.table({"i": iv}), mode="create", db_schema_name="PLAN_CHECK"))
cur.execute("""SELECT "i" = CAST('2 12:50:10.123' AS INTERVAL DAY(9) TO SECOND(3)) FROM PLAN_CHECK.IV"""); print(cur.fetchall())
cur.adbc_ingest("KEEP", pa.table({"c": pa.array([1], pa.int32())}), mode="replace", db_schema_name="PLAN_CHECK")
for name, table, mode in [("KEEP", pa.table({"c": pa.array([1], pa.time64("us"))}), "replace"),
                          ("KEEP", pa.table({"c": pa.array([decimal.Decimal("1.50")], pa.decimal128(38, 2))}), "append"),
                          ("IV", pa.table({"i": pa.array([(1, 0, 0)], pa.month_day_nano_interval())}), "append")]:
    try:
        cur.adbc_ingest(name, table, mode=mode, db_schema_name="PLAN_CHECK")
    except Exception as e:
        print(e)
cur.execute("SELECT COUNT(*) FROM PLAN_CHECK.KEEP"); print(cur.fetchall())
EOF
```

Expected: `1`; `[(datetime.date(2020, 1, 1),)]`; `1`; `[(True,)]`; an `INVALID_ARGUMENT` error naming column `c` and `Time64`; an `INVALID_ARGUMENT` error naming column `c` and `Decimal128(38, 2)`; an `INTERNAL` error stating that Exasol INTERVAL DAY TO SECOND has no month component; `[(1,)]`.

M2 ingests with the catalog `exa`, then with the catalog `BOGUS`:

```bash
python3 - <<'EOF'
import pyarrow as pa, adbc_driver_manager.dbapi as d
c = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="ExarrowDriverInit",
              db_kwargs={"uri": "exasol://sys:exasol@localhost:8563?validateservercertificate=0"}, autocommit=True)
cur = c.cursor()
cur.execute("CREATE SCHEMA IF NOT EXISTS PLAN_CHECK")
t = pa.table({"c": pa.array([1], pa.int32())})
print(cur.adbc_ingest("CAT_OK", t, mode="create", catalog_name="exa", db_schema_name="PLAN_CHECK"))
try:
    cur.adbc_ingest("CAT_BAD", t, mode="create", catalog_name="BOGUS", db_schema_name="PLAN_CHECK")
except Exception as e:
    print(e)
cur.execute("SELECT COUNT(*) FROM EXA_ALL_TABLES WHERE TABLE_SCHEMA = 'PLAN_CHECK' AND TABLE_NAME = 'CAT_BAD'"); print(cur.fetchall())
EOF
```

Expected: `1`; an `INVALID_ARGUMENT` error stating that Exasol has only the catalog `EXA`; `[(0,)]`.

Clean up with `exapump sql 'DROP SCHEMA PLAN_CHECK CASCADE'`.

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release --features ffi` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Unit tests (websocket) | `cargo test --lib --features websocket` | 0 failures |
| Unit tests (ffi) | `cargo test --lib --features ffi` | 0 failures |
| Driver manager tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Import/export tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Floors pass |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
