# Verification Report: fix-parquet-import-seekable-files

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Native Parquet import from files serves range requests from the open file in pieces of at most 64 KiB. Peak memory for a 277 MB file fell from 287 MiB (origin/main) to 10 MiB. |
| Code review | 9 findings: 9 fixed |

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
| Unit (production lines) | 90.96 |
| Integration | not measured |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test --lib`) | 1759 | 1759 | 0 |
| Unit (`--features ffi`) | 1880 | 1880 | 0 |
| Unit (`--no-default-features --features websocket`) | 1564 | 1564 | 0 |
| Integration (`import_export_tests`, Exasol 2025.2.1, `REQUIRE_EXASOL=1`, `--test-threads=1`) | 79 | 78 | 1 |

### Manual Tests

| Test | Result |
|------|--------|
| Single file, native: `benchmark --operation import-parquet --size 1gb` (277 MB Parquet, 4,294,967 rows). Peak RSS measured with Python `resource.getrusage` because `/usr/bin/time` is not installed. This branch: 10 MiB. Build of origin/main (57e68e9): 287 MiB. | ✓ |
| `test_parquet_import_native_path_serves_multi_row_group_file`: passed, peak RSS 31 MiB (debug test binary) | ✓ |
| `test_parallel_parquet_import_native_path_serves_multi_row_group_files`: passed, peak RSS 31 MiB, the same as the one-file run | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings: exit 0
```

### Formatter

```
cargo fmt --all -- --check: exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| import-export | parquet-io | Native Parquet import on Exasol 2025.1.11+ | `tests/import_export_tests.rs` | `test_parquet_import_native_path_serves_multi_row_group_file` | Pass |
| import-export | parquet-io | Native Parquet import on Exasol 2025.1.11+ | `src/transport/http_transport.rs` | `test_serve_parquet_range_requests_reads_only_the_requested_range_in_pieces` | Pass |
| import-export | parquet-io | Native Parquet import fails when the file becomes shorter during the import | `src/transport/http_transport.rs` | `test_serve_parquet_range_requests_fails_when_the_file_ends_inside_a_range` | Pass |
| import-export | parallel-import | Native parallel Parquet import on Exasol 2025.1.11+ | `tests/import_export_tests.rs` | `test_parallel_parquet_import_native_path_serves_multi_row_group_files` | Pass |
| import-export | parallel-import | Fail-fast on Parquet conversion error | `src/import/parallel.rs` | `test_stream_parquet_files_parallel_names_the_file_that_cannot_be_opened` | Pass |
| import-export | parallel-import | Fail-fast on Parquet conversion error | `src/import/parallel.rs` | `test_stream_parquet_files_parallel_names_the_file_whose_serving_fails` | Pass |

## Notes

- The multi-file memory check uses small fixtures (400,000 rows per file). It shows that a second file adds no resident memory, but it cannot distinguish this branch from origin/main. The 1 GB single-file run does.
- The benchmark did not create its `benchmark` schema because its setup skips the first statement of `schema.sql`. The schema was created with `exapump sql` before the run. This is a benchmark tooling issue outside this plan.
- Two review-fix tests (size probe failure, missing file before any SQL) cover behavior that already existed and passed on first run.
- The multi-file run on origin/main was not repeated. The comparison covers the single-file case.
