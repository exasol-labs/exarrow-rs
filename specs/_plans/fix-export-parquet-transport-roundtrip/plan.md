# Plan: fix-export-parquet-transport-roundtrip

## Summary

The transport Parquet export stops re-serializing rows to CSV and takes its column names and types from the export source's prepared-statement metadata. Values that contain separators or line breaks export intact, and text values keep their whitespace on both the transport path and the CSV-bytes path.

## Context

- Issue exasol-labs/exarrow-rs#58 reports three defects in `export_to_parquet_via_transport` (`src/export/parquet.rs`) in exarrow-rs 0.16.0. `exapump export --format parquet` reaches it when no split limit is set.
- The function receives correctly parsed rows from `export_to_list`, joins each row with the column separator and no quoting, and re-parses the result with `csv_to_record_batches`. A value that contains the separator changes the field count. A value that contains a line break splits the record, because `csv_to_record_batches` splits records with `csv_str.lines()`.
- The function builds a `col0..colN: Utf8` schema, so DECIMAL, DATE, TIMESTAMP, and BOOLEAN columns are written as strings without their names. The recorded scenario "Export preserves schema" in `import-export/parquet-io` requires a schema derived from Exasol metadata.
- `parse_csv_value` trims every field. A whitespace-only VARCHAR value becomes NULL when `null_value` is unset. The trim also affects the public CSV-bytes entry points `export_to_parquet(csv_data, schema, ...)`, `export_to_parquet_stream`, and `csv_to_record_batches`.
- An export that returns zero rows writes no file. exapump's split path calls `export_to_parquet` with `SELECT 1 WHERE FALSE` and expects an empty output file.
- Issue #60 is closed. `TransportProtocol::create_prepared_statement` returns a `PreparedStatementHandle` whose `result_columns: Vec<ColumnInfo>` carries the result-set column names and types on both transports (`prepared-statements/result-columns`). `close_prepared_statement` releases the handle.
- `export_to_record_batches` (`src/export/arrow.rs`) builds Arrow arrays directly from the parsed rows, but it requires a caller-supplied schema and pads short rows with NULLs.
- Exasol writes NULL as an empty field and stores an empty string as NULL. Exasol writes DATE and TIMESTAMP values in the session's NLS formats.
- The plan changes the export component and adds a data flow step. See the architecture delta, `architecture.md` in this plan directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| import-export/parquet-io | CHANGED | `import-export/parquet-io/spec.md` |

## Impact

- A Parquet file written by `Connection::export_to_parquet` now has the source's column names and Arrow types such as `Decimal128`, `Date32`, `Timestamp`, `Boolean`, and `Float64`, where it had `col0..colN: Utf8`. Downstream readers that relied on `col0` names or string types must use the real names and types.
- Values that contain the column separator, the column delimiter, or a line break now export intact instead of failing with `Expected N columns, found M` or landing in the wrong column.
- Text values keep leading and trailing whitespace. A whitespace-only value stays a value instead of becoming NULL.
- An export with zero rows now writes a Parquet file with the schema and zero rows. Before, it wrote no file.
- INTERVAL, GEOMETRY, and HASHTYPE columns export as Utf8 text. `exasol_types_to_arrow_schema` now returns Utf8 for these types instead of an error.
- TIMESTAMP WITH LOCAL TIME ZONE columns export as `Timestamp(Microsecond, None)` holding the wall-clock value in the session time zone, where they exported as Utf8 text. `exasol_types_to_arrow_schema` now returns `Timestamp(Microsecond, None)` for this type instead of `Timestamp(Microsecond, Some("UTC"))`. A schema built with the old mapping failed batch construction in the CSV-bytes entry points, because the timestamp builder returns no time zone.
- Behavior change: the transport export ignores `ParquetExportOptions::null_value`, because the driver passes no NULL clause to the EXPORT statement.
- Behavior change: the CSV-bytes entry points no longer trim fields. A typed field with surrounding whitespace now fails with `ParquetExportError::CsvParse`.
- A transport Parquet export now needs Exasol's default session formats for typed values. A session that sets a custom `NLS_DATE_FORMAT`, `NLS_TIMESTAMP_FORMAT`, or `NLS_NUMERIC_CHARACTERS` gets a conversion error that names the column and the value.
- A transport Parquet export makes one prepare and one close round trip before the EXPORT statement.
- No public function signature changes. exapump 0.12.0 picks up the fix without code changes.

## Dependencies

- Issue #60 (closed): prepared-statement result-set column metadata on the native and WebSocket transports. This plan uses `TransportProtocol::create_prepared_statement`, `PreparedStatementHandle::result_columns`, and `TransportProtocol::close_prepared_statement`.

## Implementation Tasks

1. Export source description and type lookup

- [ ] 1.1 In `src/query/export.rs`, add a crate-visible `ExportSource` method that returns the SELECT statement whose result set the source exports: `SELECT <columns joined with ", ", or *> FROM [schema.]name` for `ExportSource::Table`, and the query text for `ExportSource::Query`. Render `[schema.]name` through one private helper that `ExportQuery::source_clause` also uses, so the EXPORT statement and the SELECT statement resolve the same object. Add unit tests for a table with and without a schema and a column list, and for a query source.
- [ ] 1.2 In `src/query/results.rs`, move the `DataType`-to-`ExasolType` match out of `ResultSet::exasol_datatype_to_arrow` into a crate-visible function, and make `exasol_datatype_to_arrow` call it. The existing `ResultSet` tests stay unchanged and pass.
- [ ] 1.3 In `src/export/parquet.rs`, change the private `exasol_type_to_arrow` so INTERVAL YEAR TO MONTH, INTERVAL DAY TO SECOND, GEOMETRY, and HASHTYPE map to `Utf8`, and `ExasolType::Timestamp { with_local_time_zone: true }` maps to `Timestamp(Microsecond, None)` instead of the `Some("UTC")` that `types::conversion::exasol_type_to_arrow` returns. Replace `test_exasol_type_to_arrow_rejects_types_parquet_cannot_hold` with a test that asserts `Utf8` for all four through `exasol_types_to_arrow_schema`. Replace `test_exasol_type_to_arrow_timestamp_with_tz` with a test that asserts `Timestamp(Microsecond, None)` for TIMESTAMP WITH LOCAL TIME ZONE through `exasol_types_to_arrow_schema`. Update the doc comment of `exasol_types_to_arrow_schema`.

2. Shared row conversion and the CSV-bytes path

- [ ] 2.1 In `src/export/csv.rs`, make `parse_csv` `pub(crate)` and change it to iterate the input with a peekable `chars()` iterator instead of collecting a `Vec<char>`. The existing `parse_csv` and `export_to_list` tests stay unchanged and pass.
- [ ] 2.2 In `src/export/parquet.rs`, add the shared converter that turns parsed rows (`&[Vec<String>]`), a schema, an optional `null_value`, and a batch size into `Vec<RecordBatch>`. It fails with `ParquetExportError::CsvParse` naming the row when a row's field count differs from the schema's column count. It reads an empty field as NULL when `null_value` is `None`, reads a field equal to `null_value` as NULL, and keeps every other field verbatim with no trimming. It builds each column with the existing `build_array_from_csv_column`.
- [ ] 2.3 Rewrite `csv_to_record_batches` on `parse_csv` and the shared converter. Map a `parse_csv` error to `ParquetExportError::CsvParse` with its row. Skip the first parsed row when `with_column_names` is set. Keep returning one empty batch for empty or header-only input. Delete `parse_csv_line`, `parse_csv_value`, and `csv_chunk_to_record_batch`.
- [ ] 2.4 Move the `ArrowWriter` part of `export_to_parquet_stream` into a private helper that writes a list of batches with a schema and a compression codec, so the transport export reuses it. The helper writes a readable file for an empty list of batches.
- [ ] 2.5 Port the unit tests of the deleted functions (`test_parse_csv_line_*`, `test_parse_csv_value_*`) to tests of `csv_to_record_batches` with the same inputs and expectations, except that no test expects trimming. Add the unit tests for "CSV-bytes export keeps field whitespace", "CSV-bytes export reads only the null_value marker as NULL", and "CSV-bytes export accepts line breaks inside quoted fields".

3. Transport export

- [ ] 3.1 In `src/export/parquet.rs`, add a private async function that derives the export schema from a transport and an `ExportSource`. It prepares the SELECT statement from task 1.1 with `TransportProtocol::create_prepared_statement` and maps a failure to `ExportError::SqlExecutionError`. It calls `close_prepared_statement` whenever the prepare succeeded, also when it then rejects the source. It returns `ExportError::SqlExecutionError` stating that the export source produces no result set when `result_columns` is empty. Otherwise it maps each `ColumnInfo` through the function from task 1.2 and `exasol_types_to_arrow_schema`, with the column name as the field name. A type lookup failure becomes `ParquetExportError::Schema` naming the Exasol type, and reaches the caller through the mapping in decision-log entry [7]. A close failure is returned as `ExportError::TransportError`. When both a rejection and the close fail, the rejection error is returned.
- [ ] 3.2 Rewrite `export_to_parquet_via_transport`: derive the schema (task 3.1), run `export_to_list` with the shared CSV options, convert the rows with the shared converter (task 2.2) and `null_value: None`, then create the output file and write the batches (task 2.4). Return the row count. For zero rows, write a file with the schema and zero rows. Create the file only after every batch converts. Map errors as decision-log entry [7] states. Delete the `col0` schema construction and the `row.join` re-serialization.
- [ ] 3.3 Rewrite the unit tests `test_export_to_parquet_via_transport_writes_the_tunnel_rows_as_parquet` and `test_export_to_parquet_via_transport_writes_no_file_for_an_empty_export` against mocked result-set metadata. The first asserts named, typed fields and a tunnel row whose quoted field contains the separator. The second asserts a schema-only file. Add a unit test with a `mockall::Sequence` for "Export releases the schema prepared statement before the EXPORT statement runs" that covers the success order and the no-result-set path. Add a unit test asserting that a prepare failure returns `ExportError::SqlExecutionError` and never calls `execute_query`.
- [ ] 3.4 Add integration tests to `tests/import_export_tests.rs` for the transport scenarios listed under Scenario Coverage. Each test carries one `/// Scenario: <title>` line per scenario it implements and follows the file's `#[ignore]` and `skip_if_no_exasol!()` convention. `test_parquet_export_preserves_schema` runs `ALTER SESSION SET TIME_ZONE = 'EUROPE/BERLIN'` on its connection before it creates, fills, and exports `T`. It checks each non-NULL `TS_LTZ` value read back against the wall-clock time in the text that `Connection::export_csv_to_list` returns for the same row on that connection.
- [ ] 3.5 Run `test_parquet_export_preserves_schema` against the Docker database (decision-log entry [10]). If Exasol writes the `PRICE` values `0.50` and `-0.50` without the leading zero, extend `parse_decimal_to_i128` in `src/types/conversion.rs` to accept an empty integer part (`.5`, `-.5`) and add a unit test for both forms. If a typed builder rejects another default text form, extend that builder and add a unit test for the form.

4. Documentation

- [ ] 4.1 Update the doc comments of the `src/export/parquet.rs` module (its numbered architecture steps), `export_to_parquet_via_transport`, `export_to_parquet`, `export_to_parquet_stream` (no trimming), `ParquetExportOptions::null_value` (applies only to the CSV-bytes entry points), and `Connection::export_to_parquet` in `src/adbc/connection.rs`.
- [ ] 4.2 In `docs/import-export.md` § Parquet Export, state that the file carries the source's column names and types, that INTERVAL, GEOMETRY, and HASHTYPE columns are written as Utf8 text, that TIMESTAMP WITH LOCAL TIME ZONE columns are written as timestamps without a time zone that hold the session-local value, that an empty export writes a schema-only file, and that typed values require Exasol's default session formats.
- [ ] 4.3 Add the `CHANGELOG.md` entries under `## [Unreleased]` that decision-log entry [11] lists.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Parquet export conversion | 1.1-1.3, 2.1-2.5, 3.1-3.5, 4.1-4.3 | none | spec delta `import-export/parquet-io`; `src/export/parquet.rs`, `src/export/csv.rs` (`parse_csv`), `src/query/export.rs` (`ExportSource`, `ExportQuery::source_clause`), `src/query/results.rs` (`exasol_datatype_to_arrow`), `src/types/conversion.rs` (`parse_decimal_to_i128`), `src/transport/protocol.rs` (`PreparedStatementHandle`), `src/transport/test_support.rs` (`MockTransport`, `FakeExasolServer`), `src/adbc/connection.rs` (`export_to_parquet` doc), `tests/import_export_tests.rs`, `docs/import-export.md`, `CHANGELOG.md` |

- One group: both entry points share the converter in `src/export/parquet.rs`, so a split would give two agents the same file and the same spec delta.
- Order inside the group: tasks 1.x and 2.1-2.4 before 3.1-3.2, tests with their tasks, 3.5 after 3.4, and 4.x last.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/export/parquet.rs` `parse_csv_line` | Replaced by `export::csv::parse_csv` and the shared converter |
| Function | `src/export/parquet.rs` `parse_csv_value` | The shared converter applies the NULL rule without trimming |
| Function | `src/export/parquet.rs` `csv_chunk_to_record_batch` | Replaced by the shared converter |
| Code block | `src/export/parquet.rs` `export_to_parquet_via_transport`: the `col0` schema construction and the `row.join` CSV re-serialization | Replaced by metadata-derived schema and direct conversion |
| Test | `src/export/parquet.rs` `test_parse_csv_line_*`, `test_parse_csv_value_*` | Functions removed; inputs ported to `csv_to_record_batches` tests (task 2.5) |
| Test | `src/export/parquet.rs` `test_exasol_type_to_arrow_rejects_types_parquet_cannot_hold` | Replaced by the Utf8 mapping test (task 1.3) |
| Test | `src/export/parquet.rs` `test_exasol_type_to_arrow_timestamp_with_tz` | Asserts the UTC label that task 1.3 removes; replaced by the no-time-zone mapping test (task 1.3) |
| Test | `src/export/parquet.rs` `test_export_to_parquet_via_transport_writes_no_file_for_an_empty_export` | Replaced by the schema-only file test (task 3.3) |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Export preserves schema | Integration | `tests/import_export_tests.rs` | `test_parquet_export_preserves_schema` |
| Query export names fields after the select list | Integration | `tests/import_export_tests.rs` | `test_parquet_export_names_fields_after_select_list` |
| Values containing the separator, the delimiter, or a line break export intact | Integration | `tests/import_export_tests.rs` | `test_parquet_export_keeps_values_with_separators_and_line_breaks` |
| Exported text values keep their whitespace | Integration | `tests/import_export_tests.rs` | `test_parquet_export_keeps_text_whitespace` |
| Columns without a typed CSV conversion export as text | Integration | `tests/import_export_tests.rs` | `test_parquet_export_writes_untyped_columns_as_text` |
| Columns without a typed CSV conversion export as text (`exasol_types_to_arrow_schema` clause) | Unit | `src/export/parquet.rs` | `test_exasol_types_to_arrow_schema_maps_text_only_types_to_utf8` |
| Empty export writes a Parquet file that carries the schema | Integration | `tests/import_export_tests.rs` | `test_parquet_export_empty_result_writes_schema_only_file` |
| Export source that produces no result set is rejected before the export runs | Integration | `tests/import_export_tests.rs` | `test_parquet_export_rejects_source_without_result_set` |
| Export releases the schema prepared statement before the EXPORT statement runs | Unit | `src/export/parquet.rs` | `test_export_to_parquet_via_transport_closes_the_schema_statement_before_the_export` |
| A value that does not match its column type fails the export | Integration | `tests/import_export_tests.rs` | `test_parquet_export_fails_on_value_outside_default_session_format` |
| CSV-bytes export keeps field whitespace | Unit | `src/export/parquet.rs` | `test_export_to_parquet_stream_keeps_field_whitespace` |
| CSV-bytes export reads only the null_value marker as NULL | Unit | `src/export/parquet.rs` | `test_export_to_parquet_stream_reads_null_value_marker_as_null` |
| CSV-bytes export accepts line breaks inside quoted fields | Unit | `src/export/parquet.rs` | `test_csv_to_record_batches_accepts_line_breaks_in_quoted_fields` |

- The prepared-statement ordering scenario uses a unit test because the order of transport calls is not observable from outside the driver. The test runs the real export against `MockTransport` and the in-process `FakeExasolServer` tunnel.
- The CSV-bytes scenarios are pure in-memory conversions, so unit tests cover them.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| import-export/parquet-io (transport export) | `EXASOL_USER=sys EXASOL_PASSWORD=exasol EXASOL_VALIDATE_CERT=false cargo run --example import_export` | The "Export to Parquet" section prints a batch schema with the fields `PRODUCT_ID` (`Decimal128(18, 0)`), `PRODUCT_NAME` (`Utf8`), and `PRICE` (`Decimal128(10, 2)`), and no `col0` field |
| import-export/parquet-io (issue #58 reproduction) | `REQUIRE_EXASOL=1 cargo test --test import_export_tests test_parquet_export_keeps_values_with_separators_and_line_breaks -- --ignored --nocapture` | The export of `Smith, John` and the line-break values succeeds, and the test passes |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (WebSocket) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Unit test | `cargo test --lib` | 0 failures |
| Integration test | `REQUIRE_EXASOL=1 cargo test --test import_export_tests -- --ignored` | 0 failures, no skips |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Check passes: total production coverage at least 80%, every file at least 50% |
