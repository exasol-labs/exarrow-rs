# Tasks: fix-bulk-ingest-type-coverage

## Phase 2: Implementation (Group A)
- [x] 1.1 Type mapping in `arrow_to_exasol` and `to_ddl_type`, with unit tests
- [x] 1.2 INTERVAL text writer in `src/types/conversion.rs`, `parse_interval_months` in `i64`
- [x] 1.3a Single formatter dispatch and schema check in `src/import/arrow.rs` [expert]
- [x] 1.3b New formatters in the dispatch (Date64, Decimal256, negative scale, Dictionary, Utf8View, BinaryView, FixedSizeBinary, Interval)
- [x] 1.4 Unit tests in `src/import/arrow.rs`
- [x] 1.5 `record_batch_to_csv` uses the import CSV conversion; delete Parquet-local formatter [expert]
- [x] 1.6 Integration test: CSV-path Parquet import converts values
- [x] 1.7 Native Parquet auto-create allow-list check
- [x] 1.8 Integration test: interval values via RecordBatch import
- [x] 1.9 Reject Interval from a Parquet file without an Arrow schema

## Phase 2: Implementation (Group B)
- [x] 2.1 Target catalog check in `execute_bulk_ingest`
- [x] 2.2 Column type check before any SQL in `execute_bulk_ingest`
- [x] 2.3 Unit tests in `src/adbc_ffi.rs`
- [x] 2.4 Seven driver manager integration tests

## Phase 2: Implementation (Group C)
- [x] 3.1 `docs/type-mapping.md` Arrow to Exasol section
- [x] 3.2 `docs/driver-manager.md` Bulk ingestion subsection
- [x] 3.3 Bump version to 0.29.0 in `Cargo.toml` and `Cargo.lock`
- [x] 3.4 `CHANGELOG.md` 0.29.0 section

## Phase 3: Verification
- [x] 5.1 Build, unit tests (default, websocket, ffi), lint, format
- [x] 5.2 Integration suites against Docker Exasol
- [x] 5.3 Coverage floors
- [x] 5.4 Manual scripts M1, M2

## Phase 4: Review Fixes
- [x] 4.1 In `src/types/conversion.rs`, add the constants `MILLIS_PER_SECOND`, `MILLIS_PER_MINUTE`, `MILLIS_PER_HOUR`, `SECONDS_PER_MINUTE`, `MINUTES_PER_HOUR`, use them in `format_interval_day_to_second`, and use `MILLIS_PER_DAY` and `NANOS_PER_MILLISECOND` in the tests that repeat `86_400_000` and `1_000_000`
- [x] 4.2 Move Date64 formatting into `src/types/conversion.rs` as `pub fn format_date64`, reduce `format_date64` in `src/import/arrow.rs` to a call to it, delete the local `MILLIS_PER_DAY`, and add `test_format_date64_floors_to_the_day_that_holds_it_and_rejects_dates_outside_exasol_range`
- [x] 4.3 Delete `format_decimal128` in `src/import/arrow.rs`, call `format_decimal_text` from `format_decimal128_value`, and rename `test_format_decimal128` to `test_format_decimal_text_places_the_decimal_point_by_scale`
- [x] 4.4 In `src/import/parquet.rs`, replace `check_footer_before_create` with `check_native_auto_create_footer` and `check_csv_auto_create_footer` sharing a private `rejected_footer` helper, update the callers and the test
- [x] 4.5 In `src/import/parquet.rs`, add `test_csv_path_parquet_import_accepts_a_file_without_arrow_schema_and_without_intervals`
- [x] 4.6 In the `src/import/parquet.rs` tests, change `write_parquet` to take `(path, batch, options)` and update every caller
- [x] 4.7 In `tests/driver_manager_tests.rs`, add `IngestTarget` and change `ingest_batch` to take `(conn, target, batch)`, updating every caller
- [x] 4.8 In `tests/driver_manager_tests.rs`, delete `table_count` and replace the COUNT(*) assertions with `integers_from`
