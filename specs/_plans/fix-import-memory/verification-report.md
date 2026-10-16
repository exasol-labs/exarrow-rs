# Verification Report: fix-import-memory

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | The Parquet CSV path and multi-file CSV import now stream through the tunnel. All checks pass. The 8.x manual row was not run. |
| Code review | 9 findings, 9 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ (memory and docs rows; 8.x row not run) |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | 90.04 (production lines, floor 80.0) |
| Integration | not measured (per AGENTS.md) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit | 1709 | 1709 | 0 |
| Unit (ffi) | 1830 | 1830 | 0 |
| Integration (import_export_tests) | 75 | 74 | 1 |
| Driver manager | 71 | 71 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| parquet-io integration filters | ✓ |
| Parquet memory: peak 35.4 MB vs 27.4 MB baseline, +8 MB against about 44 MB of CSV | ✓ |
| parallel-import integration filters | ✓ |
| CSV memory: peak 22.2 MB vs 20.6 MB baseline, +1.6 MB against about 43 MB of CSV | ✓ |
| Docs build (`cargo doc --no-deps`) | ✓ |
| Exasol 8.x image, conversion-error tests | not run: the port 8563 container belongs to another project and was not replaced |

## Tool Evidence

### Linter

```
cargo +1.92 clippy --all-targets --all-features -- -W clippy::all: 0 warnings
```

### Formatter

```
cargo +1.92 fmt --all -- --check: clean
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| import-export | parquet-io | CSV-path Parquet import sends CSV one RecordBatch at a time | `src/import/parquet.rs`, `tests/import_export_tests.rs` | `test_stream_parquet_csv_writes_first_batch_before_decoding_last_row_group`, `test_parquet_import_csv_path_streams_multi_row_group_file` | Pass |
| import-export | parquet-io | CSV-path Parquet import that fails on a later RecordBatch imports no rows | `src/import/parquet.rs`, `tests/import_export_tests.rs` | `test_stream_parquet_csv_conversion_error_withholds_the_terminating_chunk`, `test_parquet_import_csv_path_conversion_error_in_later_batch_imports_no_rows` | Pass |
| import-export | parallel-import | Parallel CSV import streams each file in pieces | `src/import/csv.rs`, `tests/import_export_tests.rs` | `test_stream_file_to_connection_sends_a_piece_before_reading_the_next`, `test_parallel_csv_import_streams_large_files` | Pass |
| import-export | parallel-import | Parallel CSV import compresses each file while streaming it | `src/import/csv.rs`, `tests/import_export_tests.rs` | `test_stream_encoder_pieces_decompress_to_the_input`, `test_parallel_csv_import_compresses_while_streaming` | Pass |
| import-export | parallel-import | Parallel CSV import fails before running IMPORT when a file cannot be opened | `src/import/csv.rs`, `tests/import_export_tests.rs` | `test_import_from_files_missing_file_fails_before_connecting`, `test_parallel_csv_import_missing_file_fails_before_import` | Pass |
| import-export | parallel-import | Import multiple Parquet files in parallel | `src/import/parquet.rs`, `tests/import_export_tests.rs` | `test_import_from_parquet_files_returns_zero_when_every_file_is_row_less`, `test_parallel_parquet_import_csv_path_streams_multi_row_group_files` | Pass |
| import-export | parallel-import | Parallel CSV-path Parquet import that fails on a value imports no rows | `src/import/parallel.rs`, `tests/import_export_tests.rs` | `test_join_stream_handles_keeps_conversion_error`, `test_parallel_parquet_import_csv_path_conversion_error_imports_no_rows` | Pass |
| import-export | parallel-import | Parquet conversion is parallelized (recorded) | `src/import/parquet.rs` | `test_parquet_csv_producers_start_together_on_concurrent_gets` | Pass |
| import-export | parallel-import | Fail-fast on streaming error (recorded) | `src/import/csv.rs` | `test_csv_file_task_read_error_names_the_path` | Pass |

## Notes

- Tested against Exasol 2025.2.1 on localhost:8563, a container of another project, reused without stopping it.
- The 8.x manual check (server that picks the CSV path by default) is open; the 2025.2.1 tests confirm that Exasol fails the IMPORT when the terminating chunk is missing.
- Memory was measured with VmHWM and getrusage, because `/usr/bin/time` is not installed.
- Version is 0.21.4 (origin/main is 0.21.3).
