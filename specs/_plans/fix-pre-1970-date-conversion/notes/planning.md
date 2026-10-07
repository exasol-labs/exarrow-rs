# Planning notes: fix-pre-1970-date-conversion

## Files and symbols checked

- `src/types/conversion.rs`: `parse_date_to_days`, `ymd_to_days`, `ymd_hms_nanos_to_micros`, `parse_timestamp_to_micros`, `parse_time_of_day_to_micros`, `parse_seconds_field_to_micros`, `fractional_seconds_to_units`, test module list
- `src/query/results.rs`: constants at lines 23-27, `column_major_to_record_batch`, `json_column_to_array`, `parse_date_to_days`, `parse_timestamp_to_micros`, `parse_time_of_day_to_micros`, `parse_seconds_to_micros`, `fractional_seconds_to_micros`, test sections for the parsers (about lines 1214-1393), `test_column_major_to_record_batch_*` tests
- `src/transport/native/result_parser.rs`: `read_packed_date_days`, `read_timestamp_micros`, native date test using `ymd_to_days(2024, 1, 2)`, `single_pass_timestamp_column_decodes_to_microseconds`
- `src/arrow_conversion/builders.rs`: `build_date_array`, `build_timestamp_array`, wrappers around the conversion parsers
- `src/export/arrow.rs`: `build_date32_array`, `build_timestamp_array`, `ArrowExportOptions::schema`, `export_to_record_batches` (requires explicit schema)
- `src/export/parquet.rs`: date and timestamp wrappers around the conversion parsers
- `src/import/arrow.rs`: `format_value` (Date32, Timestamp arms), `format_timestamp`, `format_date32`, `days_to_ymd`, `format_timestamp_micros`
- `src/import/parquet.rs`: `format_date32` (chrono), `format_timestamp`, `test_format_timestamp_nanosecond_unit_truncates_to_micros`
- `src/adbc_ffi.rs`: Date32 and Timestamp parameter binding (lines about 1800-1852), bulk ingest calling `import_from_record_batch` (about line 2189)
- `src/adbc/connection.rs`: public import and export method list
- `Cargo.toml`, `Cargo.lock` (chrono 0.4.45)
- `tests/common/mod.rs`: `skip_if_no_exasol!`, `get_test_connection_with_transport`, `is_exasol_available`
- `tests/integration_tests.rs`: datetime type test (about line 1480)
- `tests/websocket_integration_tests.rs`: `get_ws_connection`, datetime type test (about line 1235)
- `tests/import_export_tests.rs`: `test_arrow_round_trip`, `test_parquet_export_preserves_schema`, helpers `read_parquet_file`, `typed_column`, `row_of_id`, `create_table`, `table_source`, `write_small_parquet`, forced-CSV Parquet tests
- `tests/python/test_driver_integration.py`: driver path, entrypoint, URI
- `.github/workflows/ci.yml`: integration job test commands
- `CHANGELOG.md`, `CONTRIBUTING.md` § Releasing, `docs/type-mapping.md`, `specs/architecture.md`
- Specs: `type-mapping/exasol-to-arrow`, `type-mapping/arrow-to-exasol`, `type-mapping/boundaries-and-validation`, `native-client/type-conversion`, `native-client/zero-copy-fetch`, `arrow-conversion/type-converters`, `import-export/arrow-recordbatch`, `import-export/parquet-io`, `import-export/parquet-export`
- Branch `feat/fix-paged-fetch-position` (read with `git show`): `plan.md`, `decision-log.md`
- GitHub issue exasol-labs/exarrow-rs#84

## Searches run

- `speq search query "date before 1970 calendar"`: index build hung, stopped after 45 s, no result
- grep in `specs/` for `date32`, `1970`, `epoch`, `days since`, `microseconds since`, `calendar`, `gregorian`, `julian`, `leap`, `1582`, `pre-epoch`, `historical`, `0001-01-01`, `9999-12-31`, `timestamp`
- `speq decision-log show`: ten ADRs, grep for `calendar`, `date`, `timestamp`, `chrono`
- grep in `src/`, `tests/`, `benches/`, `examples/` for `1969`, `1901`, `1601`, `days_from_year`, `div_euclid`, `rem_euclid`, `86400`, `unsigned_abs`, `chrono::`, `ymd_to_days`, `days_to_ymd`, `parse_date_to_days`, `parse_timestamp_to_micros`, `ymd_hms_nanos_to_micros`, `format_timestamp`, `* 365`, `% 400`, `719468`, `/ 1_000`, `% 1_000_000`, `Date32`, `Timestamp*Array`
- `exapump sql` against `exasol-test` (2026.1.0): `DAYS_BETWEEN` for 1968-01-01, 0001-01-01, 9999-12-31, 1582-10-04, 1582-10-10, 1582-10-15, 1900-03-01, 1600-03-01, 0300-03-01, 1000-01-01; `TO_CHAR(ADD_DAYS(...))` around 1582-10-04, 1500-02-28, and day -719162/-719164; `SECONDS_BETWEEN` for 1950-06-15; `EXA_PARAMETERS` NLS formats; `TIMESTAMP(6)` cast
- Python `datetime` (run with `-I`): expected day and microsecond values; full-range comparison of the current and the floor-division formula
