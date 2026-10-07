# Verification Report: fix-pre-1970-date-conversion

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Pre-1970 DATE and TIMESTAMP values convert to the correct day on every read and write path, and every checklist command exits 0 on the final code. |
| Code review | 5 findings, 5 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | 86.80 (production lines; floors 80.0 total and 50.0 per file pass) |
| Integration | not measured (AGENTS.md: integration coverage is not fed to Sonar) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test --lib`) | 1599 | 1599 | 0 |
| Unit (WebSocket, FFI parameter binding) | exit 0 | exit 0 | 0 |
| Integration (native, WebSocket, import/export, driver manager) | exit 0 each | exit 0 each | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `SELECT DATE '1968-01-01', DATE '0001-01-01', TIMESTAMP '1950-06-15 00:00:00'` through the cdylib, native and WebSocket | ✓ both print `date(1968, 1, 1)`, `date(1, 1, 1)`, `datetime(1950, 6, 15, 0, 0)` |
| `adbc_ingest` of -43200000000 µs, then `TO_CHAR` | ✓ `1969-12-31 12:00:00.000000` |
| `test_parquet_export_keeps_pre_1970_dates_and_timestamps` | ✓ passes against Docker Exasol |
| `test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps` | ✓ passes against Docker Exasol |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -W clippy::all: exit 0, no warnings
python3 scripts/check_ci_test_targets.py: exit 0
```

### Formatter

```
cargo fmt --all -- --check: exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| type-mapping | exasol-to-arrow | Pre-1970 DATE and TIMESTAMP query results keep their calendar day | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `src/query/results.rs`, `src/transport/native/result_parser.rs` | `test_pre_1970_dates_and_timestamps_keep_their_calendar_day`, `test_ws_pre_1970_dates_and_timestamps_keep_their_calendar_day`, `test_column_major_to_record_batch_keeps_pre_1970_dates_and_timestamps`, `single_pass_dates_and_timestamps_before_1970_keep_their_calendar_day` | Pass |
| type-mapping | exasol-to-arrow | DATE and TIMESTAMP values count days in the proleptic Gregorian calendar | `src/types/conversion.rs` | the seven tests of plan task 1.1 | Pass |
| type-mapping | boundaries-and-validation | Lossless conversion validation | `src/types/conversion.rs` | `test_ymd_to_days_julian_only_leap_day_reads_as_march_first` | Pass |
| type-mapping | arrow-to-exasol | Parameter binding formats pre-epoch DATE and TIMESTAMP values and rejects values outside Exasol's range | `src/adbc_ffi.rs` | `test_arrow_value_to_parameter_formats_pre_epoch_values`, `test_arrow_value_to_parameter_rejects_values_outside_exasol_range` | Pass |
| import-export | parquet-export | Export keeps pre-1970 DATE and TIMESTAMP values | `tests/import_export_tests.rs` | `test_parquet_export_keeps_pre_1970_dates_and_timestamps` | Pass |
| import-export | arrow-recordbatch | Pre-1970 DATE and TIMESTAMP values round-trip through RecordBatch import and export | `tests/import_export_tests.rs` | `test_arrow_round_trip_keeps_pre_1970_dates_and_timestamps` | Pass |
| import-export | arrow-recordbatch | RecordBatch import formats pre-epoch timestamps of every time unit as times before the epoch | `src/import/arrow.rs` | `test_format_timestamp_before_epoch_in_every_unit` | Pass |
| import-export | arrow-recordbatch | RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range | `src/import/arrow.rs` | `test_format_value_rejects_dates_and_timestamps_outside_exasol_range` | Pass |
| import-export | parquet-io | CSV-path Parquet import keeps pre-epoch timestamps with fractional seconds | `tests/import_export_tests.rs` | `test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps` | Pass |
| import-export | parquet-io | CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch | `src/import/parquet.rs` | `test_format_timestamp_before_epoch_in_every_unit` | Pass |
| import-export | parquet-io | CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range | `src/import/parquet.rs` | `test_format_arrow_value_rejects_dates_and_timestamps_outside_exasol_range` | Pass |

## Notes

- The new integration tests were not run against the old code, so they were not seen failing first. The unit tests of tasks 1.1, 2.1, and 4.3 were seen failing before their fixes.
- The local toolchain is 1.91, and CI pins 1.92.0. Formatting drift is possible.
- The release version is set by the orchestrator at the version bump, after this report.
