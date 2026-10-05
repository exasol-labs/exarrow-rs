# Tasks: fix-export-parquet-transport-roundtrip

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1 Add ExportSource SELECT-statement method in src/query/export.rs with shared name helper and unit tests
- [x] 1.2 Extract DataType-to-ExasolType function in src/query/results.rs
- [x] 1.3 Change private exasol_type_to_arrow in src/export/parquet.rs (text-only types to Utf8, TS LTZ to Timestamp(Microsecond, None)) and replace two tests
- [x] 2.1 Add row iterator in src/export/csv.rs; reduce parse_csv to collect it; add unit test
- [x] 2.2 Add shared chunked row-to-batch converter in src/export/parquet.rs with row-offset unit test
- [x] 2.3 Rewrite csv_to_record_batches on the iterator and converter; delete parse_csv_line, parse_csv_value, csv_chunk_to_record_batch
- [x] 2.4 Extract ArrowWriter helper from export_to_parquet_stream
- [x] 2.5 Port deleted-function tests; add whitespace, null_value, and quoted line-break unit tests
- [x] 3.1 Add async schema derivation from prepared-statement metadata with close handling
- [x] 3.2 Rewrite export_to_parquet_via_transport (direct conversion, file cleanup on failure) with unit test
- [x] 3.3 Rewrite transport unit tests; add ordering and prepare-failure tests
- [x] 3.4 Add integration tests to tests/import_export_tests.rs
- [x] 3.5 Run test_parquet_export_preserves_schema against Docker; extend builders only if needed
- [x] 4.1 Update doc comments
- [x] 4.2 Update docs/import-export.md Parquet Export section
- [x] 4.3 Add CHANGELOG.md entries under [Unreleased]

## Phase 3: Verification
- [x] 5.1 Run build, unit tests, integration tests, lint, format, coverage

## Phase 4: Review Fixes
- [x] 4.1 In src/export/csv.rs, delete the `finished` field from `CsvRows`, its initializer in `csv_rows`, the guard at the top of `CsvRows::next`, and `self.finished = true;` after the loop; run `cargo test --lib export::csv`
- [x] 4.2 In src/export/csv.rs tests, add `test_csv_rows_ends_a_row_at_a_lone_carriage_return_outside_quotes` using `csv_rows("1,a\r2,b\r", ',', '"')`
- [x] 4.3 In src/export/parquet.rs, change the `/// Scenario:` line above `test_csv_to_record_batches_reports_the_data_row_of_a_bad_value_in_a_later_batch` to `/// Scenario: CSV-bytes export keeps field whitespace`
- [x] 4.4 In src/export/parquet.rs, replace the last sentence of the `# Errors` section of `export_to_parquet_via_transport` with the CsvParseError row 0 / IoError wording
- [x] 4.5 In src/export/parquet.rs tests, add `test_export_to_parquet_via_transport_returns_io_error_when_the_file_cannot_be_created` and `test_export_error_maps_a_parquet_writer_failure_to_csv_parse_error_at_row_zero`
- [x] 4.6 In src/export/parquet.rs tests, add `test_csv_to_record_batches_converts_every_row_when_batch_size_is_zero`
- [x] 4.7 In src/export/parquet.rs, delete `test_csv_to_record_batches_keeps_regular_text_verbatim`
- [x] 4.8 In src/export/parquet.rs, fold the two `export_schema` duplicate tests into the public-entry tests (extend `matches!`, add `expect_close_prepared_statement().never()`), then delete the two `export_schema` tests
- [x] 4.9 In src/adbc/connection.rs, replace the `# Errors` paragraph of `Connection::export_to_parquet` with the wording from the finding
- [x] 4.10 In docs/import-export.md line 231, add `export::parquet::export_to_parquet` to the `null_value` entry-point list
