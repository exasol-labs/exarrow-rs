# Verification Report: fix-bulk-ingest-type-coverage

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Every checklist command exits 0, every scenario has a passing test, and the manual scripts M1 and M2 match the expected output. |
| Code review | 8 findings, 8 fixed |

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
| Unit (production lines, `cargo llvm-cov --lib`, default features) | 91.48 |
| Integration | not measured (CI measures unit coverage only) |

The per-file floor of 50 percent and the production line floor of 80 percent pass. The lowest file is `src/connection/version.rs` at 80.00 percent.

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit, default features | 1796 | 1796 | 0 |
| Unit, `websocket` | 1841 | 1841 | 0 |
| Unit, `ffi` | 1922 | 1922 | 0 |
| Integration, driver manager (`ffi websocket`, `--include-ignored`) | 82 | 82 | 0 |
| Integration, import/export (`ffi`) | 83 | 82 | 1 |

### Manual Tests

| Test | Result |
|------|--------|
| M1: Date64 and Interval(MonthDayNano) ingest, then Time64 in replace mode, `Decimal128(38, 2)` in append mode, and a month-component interval value | ✓ |
| M2: catalog `exa` accepted, catalog `BOGUS` rejected, no `CAT_BAD` table | ✓ |
| Import/export tests for CSV-path Parquet values, interval values, and interval without Arrow schema | ✓ (part of the 82 passed) |

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
| adbc-driver | bulk-ingestion | A bound column type that the driver cannot ingest fails before any SQL statement | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_an_uningestible_column_type_before_any_sql` | Pass |
| adbc-driver | bulk-ingestion | A bound column type that the driver cannot ingest fails before any SQL statement | `src/adbc_ffi.rs` | `test_bulk_ingest_rejects_an_uningestible_column_type_in_every_mode_before_any_sql` | Pass |
| adbc-driver | bulk-ingestion | Date64, Decimal256, string dictionary, and Utf8View columns are ingested | `tests/driver_manager_tests.rs` | `test_bulk_ingest_creates_date64_decimal256_dictionary_and_utf8view_columns` | Pass |
| adbc-driver | bulk-ingestion | Binary column types are stored as lowercase hexadecimal text | `tests/driver_manager_tests.rs` | `test_bulk_ingest_stores_binary_columns_as_lowercase_hex_text` | Pass |
| adbc-driver | bulk-ingestion | Interval columns are ingested | `tests/driver_manager_tests.rs` | `test_bulk_ingest_creates_interval_columns_of_leading_precision_9` | Pass |
| adbc-driver | bulk-ingestion | An Interval(MonthDayNano) value with a month component fails the ingest | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_an_interval_value_with_a_month_component` | Pass |
| adbc-driver | bulk-ingestion | Target catalog EXA is accepted in any letter case | `tests/driver_manager_tests.rs` | `test_bulk_ingest_accepts_target_catalog_exa_in_any_letter_case` | Pass |
| adbc-driver | bulk-ingestion | Target catalog EXA is accepted in any letter case | `src/adbc_ffi.rs` | `test_bulk_ingest_accepts_target_catalog_exa_in_any_letter_case` | Pass |
| adbc-driver | bulk-ingestion | A target catalog other than EXA fails before any SQL statement | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_a_target_catalog_other_than_exa_before_any_sql` | Pass |
| adbc-driver | bulk-ingestion | A target catalog other than EXA fails before any SQL statement | `src/adbc_ffi.rs` | `test_bulk_ingest_rejects_a_target_catalog_other_than_exa_before_any_sql` | Pass |

The remaining scenarios of `type-mapping/boundaries-and-validation`, `import-export/arrow-recordbatch`, `import-export/arrow-recordbatch-dates`, and `import-export/parquet-io` each carry a `/// Scenario:` line on a unit or integration test in `src/types/mapping.rs`, `src/types/conversion.rs`, `src/import/arrow.rs`, `src/import/parquet.rs`, or `tests/import_export_tests.rs`. All of those suites pass.

## Notes

- The test names in the plan's Scenario Coverage table differ from 10 of the implemented names. The `/// Scenario:` lines match the spec titles exactly, so coverage is unaffected. The ten adbc-driver rows above list the implemented names.
- The websocket driver manager tests need a release library built with `--features 'ffi websocket'`. A library built with `ffi` only fails 3 of them with "Transport 'websocket' is not available".
- A cursor keeps the ingest target option after `adbc_ingest`, so a later `execute` on the same cursor fails with "No data bound for bulk ingestion". This behavior is unchanged by this plan. The manual scripts use a fresh cursor per statement.
- `main` was merged into the branch. It brought only the spec-only audit change from #113, with no conflicts.
- A native Parquet import into an auto-created Utf8View column succeeded against the Docker Exasol. The allow-list for native auto-create is a policy choice, not a fix for a known failure.
