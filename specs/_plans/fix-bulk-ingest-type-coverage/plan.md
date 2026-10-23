# Plan: fix-bulk-ingest-type-coverage

## Summary

ADBC bulk ingest checks the target catalog and every bound column type before its first SQL statement, so a rejected ingest never drops, creates, or writes a table, and it ingests Date64, Decimal256, string dictionaries, Utf8View, BinaryView, and FixedSizeBinary end to end (#111, #109). One Arrow-to-CSV conversion serves RecordBatch import, ADBC ingest, and CSV-path Parquet import, and `TypeMapper::arrow_to_exasol` maps only the types that this conversion converts.

## Context

- #111: `TypeMapper::arrow_to_exasol` (`src/types/mapping.rs:254`) maps Date64, Decimal256, and Interval to an Exasol column type, but `format_value` (`src/import/arrow.rs:332`) cannot convert them. In create mode the table stays behind empty; in replace mode the old table is already dropped when the import fails.
- `arrow_to_exasol` passes any decimal precision and scale through, so `Decimal128(38, s)` produces `DECIMAL(38,s)` DDL, which Exasol rejects after replace mode has dropped the table.
- `execute_bulk_ingest` (`src/adbc_ffi.rs:2065`) runs `DROP TABLE` before `CREATE TABLE` in replace mode and checks column types only while it builds the DDL. Append mode checks no column type before the import.
- `arrow_to_exasol` also feeds Parquet auto-create (`src/types/infer.rs:302`, `:330`). CSV-path Parquet import has its own value formatter (`src/import/parquet.rs:867`), which drops the sign of a negative decimal below 1 and quotes on `,` instead of the configured separator.
- #109: `execute_bulk_ingest` never reads `adbc.ingest.target_catalog`, so any catalog value is ignored. Exasol has one catalog, `EXA`, and `catalog_is_exasol` (`src/adbc_ffi.rs:516`) already checks it case-insensitively for GetObjects.
- The architecture delta (`architecture.md` in this plan directory) changes the ADBC C ABI interface and adds two constraints.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Bulk Ingestion | CHANGED | `adbc-driver/bulk-ingestion/spec.md` |
| Boundaries and Validation | CHANGED | `type-mapping/boundaries-and-validation/spec.md` |
| Arrow RecordBatch Import/Export | CHANGED | `import-export/arrow-recordbatch/spec.md` |
| Parquet I/O | CHANGED | `import-export/parquet-io/spec.md` |

## Impact

- ADBC ingest with a column type that the driver cannot ingest fails with `InvalidArguments` before any SQL statement. Replace mode keeps the existing table and its rows, and create mode leaves no table. In append mode the status changes from `Internal` to `InvalidArguments`.
- ADBC ingest accepts Date64, Decimal256 up to precision 36, Dictionary with Utf8 or LargeUtf8 values, Utf8View, BinaryView, and FixedSizeBinary.
- Breaking: an `adbc.ingest.target_catalog` value other than `EXA` in any letter case fails with `InvalidArguments`. Before, the driver ignored it.
- Changed: the public `TypeMapper::arrow_to_exasol` rejects every Interval unit and any decimal with a precision above 36, a negative scale, or a scale above the precision, and it maps the six added types. A Parquet auto-create import of such an Interval or decimal column fails with `ImportError::SchemaInferenceError` before `CREATE TABLE`. CSV-path Parquet auto-create now creates a column for Utf8View, string dictionary, BinaryView, and FixedSizeBinary fields instead of failing schema inference. Native Parquet auto-create still rejects these four types with `ImportError::SchemaInferenceError` before `CREATE TABLE`, and its message now names `with_native_parquet(Some(false))`.
- Fix: CSV-path Parquet import (servers before 2025.1.11, or `with_native_parquet(Some(false))`) keeps the sign of negative decimals, quotes values that contain the configured column separator, and writes floats without an exponent. Native Parquet import into an existing table does not change.

## Implementation Tasks

1. Type mapping and the shared CSV conversion
   - [ ] 1.1 In `TypeMapper::arrow_to_exasol` (`src/types/mapping.rs`), map Utf8View and Dictionary with an integer key and Utf8 or LargeUtf8 values to `VARCHAR(2000000)`, map BinaryView and FixedSizeBinary to `VARCHAR(2000000)`, accept Decimal128 and Decimal256 only with precision up to 36 and scale from 0 to the precision, and remove the Interval arm. Rewrite its doc comment to state that it maps only types that the import CSV conversion converts. Add the unit tests for both `type-mapping/boundaries-and-validation` scenarios and replace `test_arrow_decimal256_to_exasol`.
   - [ ] 1.2 In `format_value` (`src/import/arrow.rs`), convert Date64 as the day that holds the millisecond value through `conversion::format_date32`, Decimal256 as its decimal text with the column scale, including a negative scale, as `format_decimal128` does for Decimal128, a Dictionary with Utf8 or LargeUtf8 values by looking up its key in the values array and formatting that value, Utf8View as Utf8, and BinaryView and FixedSizeBinary as lowercase hex.
   - [ ] 1.3 In `src/import/arrow.rs`, add a `pub(crate)` schema check that returns `ImportError::ConversionError` naming the first column and Arrow type that `format_value` cannot convert. Add the unit test for the `import-export/arrow-recordbatch` scenario. It also asserts, over sample arrays of every type that 1.1 maps or rejects, that the check accepts exactly the types `format_value` converts, and that `arrow_to_exasol` accepts a subset that differs only in Decimal128 and Decimal256 with a precision above 36 or a scale outside 0 to the precision.
   - [ ] 1.4 Make `record_batch_to_csv` in `src/import/parquet.rs` keep its signature and format each row with the row formatter of `src/import/arrow.rs`, made `pub(crate)`, using `CsvWriterOptions` built from the separator, delimiter, and null value of `ParquetImportOptions`. Delete `format_arrow_value` and its Parquet-local helpers. Rewrite `test_format_timestamp_before_epoch_in_every_unit` and `test_format_arrow_value_rejects_dates_and_timestamps_outside_exasol_range` to convert through `record_batch_to_csv` of `src/import/parquet.rs`, rename them to `test_record_batch_to_csv_formats_pre_epoch_timestamps_in_every_unit` and `test_record_batch_to_csv_rejects_dates_and_timestamps_outside_exasol_range`, and keep their `/// Scenario:` lines. Delete only the other unit tests that call the deleted helpers, and move any assertion that `src/import/arrow.rs` tests do not already cover. Keep the CSV-path streaming and value-range tests passing [expert]
   - [ ] 1.5 Add the integration test for the `import-export/parquet-io` scenario to `tests/import_export_tests.rs`, writing the Parquet file with `ArrowWriter` so the Arrow schema carries the dictionary type.
   - [ ] 1.6 In `import_from_parquet` and `import_from_parquet_files` (`src/import/parquet.rs`), when `use_native` and `create_table_if_not_exists` are both set, read the Arrow schema of each file from its Parquet footer and reject a Utf8View, Dictionary, BinaryView, or FixedSizeBinary field before `CREATE TABLE` with `ImportError::SchemaInferenceError`. The message names the file, the column, and its Arrow type, and names `with_native_parquet(Some(false))` as the way to import the file. Keep this check in `src/import/parquet.rs`, which owns the path choice, and keep the signatures of `infer_schema_from_parquet` and `infer_schema_from_parquet_files`. Add a unit test that calls both functions with `use_native` set and a closure that records every SQL statement, and asserts the error and an empty record; `import_from_parquet_files` needs two files, because it delegates a single file to `import_from_parquet`. Add the integration test for the second `import-export/parquet-io` scenario to `tests/import_export_tests.rs`, writing each file with `ArrowWriter`.
2. ADBC bulk ingest checks
   - [ ] 2.1 In `execute_bulk_ingest` (`src/adbc_ffi.rs`), read `adbc.ingest.target_catalog`. When it is set and is not a string that `catalog_is_exasol` accepts, fail with `AdbcStatus::InvalidArguments` and a message that names the value and states that Exasol has only the catalog `EXA`.
   - [ ] 2.2 In `execute_bulk_ingest`, check the bound schema next to the other argument checks, before the connection lookup and before any SQL: in append mode with the schema check from 1.3, and in create, create-append, and replace modes by building every DDL statement with `generate_create_table_ddl`. Map the schema check error to `AdbcStatus::InvalidArguments` with a message that names the column and its Arrow type.
   - [ ] 2.3 Add unit tests in `src/adbc_ffi.rs` on a statement without a connection, so an `InvalidArguments` result proves that the check runs before any SQL: an uningestible type in every mode, a `Decimal128(38, 2)` column in each create mode, a non-`EXA` catalog (`BOGUS`, empty string, integer value), and `EXA` or `exa`, which reaches the missing-connection error.
   - [ ] 2.4 Add the six `adbc-driver/bulk-ingestion` integration tests to `tests/driver_manager_tests.rs`, each in its own schema from `generate_unique_test_name`, with one `/// Scenario:` line per scenario.
3. Documentation, changelog, and version
   - [ ] 3.1 Add an "Arrow to Exasol Type Conversions" section to `docs/type-mapping.md`: a table of each accepted Arrow type and its Exasol column type, the note that binary types are stored as lowercase hex text in `VARCHAR(2000000)` because Exasol has no binary column type, the precision limit of 36 with the append exception, the rejected types, the note that Parquet auto-create uses the same mapping except that native Parquet auto-create rejects Utf8View, string dictionary, BinaryView, and FixedSizeBinary columns and that `with_native_parquet(Some(false))` imports such a file, and the note that the hex text rule does not apply to native Parquet import, because Exasol's Parquet reader loads those values.
   - [ ] 3.2 Add a "Bulk ingestion" subsection to `docs/driver-manager.md` after "Parameters and row counts": a Python `cursor.adbc_ingest` example with `mode` and `db_schema_name`, the rule that `catalog_name` accepts only `EXA`, the column type check before any SQL statement, and a link to the new type table.
   - [ ] 3.3 Bump `version` in `Cargo.toml` from 0.28.0 to 0.29.0 and update the `exarrow-rs` entry in `Cargo.lock`.
   - [ ] 3.4 Add a `## 0.29.0` section to `CHANGELOG.md` with the entries of the Impact section, referencing #111 and #109.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Type mapping and CSV conversion | 1.1-1.6 | none | spec deltas `type-mapping/boundaries-and-validation`, `import-export/arrow-recordbatch`, `import-export/parquet-io`; `src/types/mapping.rs`, `src/types/infer.rs` (Parquet inference caller), `src/import/arrow.rs`, `src/import/parquet.rs`, `tests/import_export_tests.rs` |
| B: ADBC ingest checks | 2.1-2.4 | A (uses the schema check from 1.3 and the mapping from 1.1) | spec delta `adbc-driver/bulk-ingestion`; `src/adbc_ffi.rs` (`execute_bulk_ingest`, `generate_create_table_ddl`, `catalog_is_exasol`), `tests/driver_manager_tests.rs` |
| C: Docs and release | 3.1-3.4 | A, B | Impact section of this plan; `docs/type-mapping.md`, `docs/driver-manager.md`, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/import/parquet.rs` `format_arrow_value` (~line 867) and its Parquet-local float, decimal, and escape helpers | Replaced by the conversion in `src/import/arrow.rs` (task 1.4) |
| Test | `src/import/parquet.rs` unit tests that call only the deleted helpers (~lines 1203-1262, 1928-1951) | Their functions are deleted; covered by `src/import/arrow.rs` tests |
| Test | `src/types/mapping.rs` `test_arrow_decimal256_to_exasol` | Asserts that `Decimal256(40, 5)` maps to `DECIMAL(40,5)`, which the plan rejects |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| A bound column type that the driver cannot ingest fails before any SQL statement | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_uningestible_column_type_before_any_sql` |
| A bound column type that the driver cannot ingest fails before any SQL statement | Unit | `src/adbc_ffi.rs` | `test_bulk_ingest_rejects_uningestible_column_type_in_every_mode_without_a_connection` |
| A Decimal column with a precision above 36 is appended but never used to create a table | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_appends_decimal_precision_above_36_but_never_creates_it` |
| A Decimal column with a precision above 36 is appended but never used to create a table | Unit | `src/adbc_ffi.rs` | `test_bulk_ingest_rejects_decimal_precision_above_36_when_creating_without_a_connection` |
| Date64, Decimal256, string dictionary, and Utf8View columns are ingested | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_stores_date64_decimal256_dictionary_and_utf8view` |
| Binary column types are stored as lowercase hexadecimal text | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_stores_binary_types_as_hex_text` |
| Target catalog EXA is accepted in any letter case | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_accepts_target_catalog_exa_in_any_case` |
| Target catalog EXA is accepted in any letter case | Unit | `src/adbc_ffi.rs` | `test_bulk_ingest_passes_target_catalog_exa_without_a_connection` |
| A target catalog other than EXA fails before any SQL statement | Integration | `tests/driver_manager_tests.rs` | `test_bulk_ingest_rejects_other_target_catalog_before_any_sql` |
| A target catalog other than EXA fails before any SQL statement | Unit | `src/adbc_ffi.rs` | `test_bulk_ingest_rejects_other_target_catalog_without_a_connection` |
| String, binary, date, timestamp, boolean, and decimal type mappings for DDL generation | Unit | `src/types/mapping.rs` | `test_arrow_to_exasol_maps_string_binary_date_timestamp_boolean_and_decimal_types` |
| Arrow types without an Exasol column type are rejected | Unit | `src/types/mapping.rs` | `test_arrow_to_exasol_rejects_types_without_an_exasol_column_type` |
| RecordBatch CSV conversion covers every Arrow type that has an Exasol column type | Unit | `src/import/arrow.rs` | `test_csv_conversion_covers_every_mapped_arrow_type` |
| CSV-path Parquet import converts values as RecordBatch import does | Integration | `tests/import_export_tests.rs` | `test_parquet_import_csv_path_converts_values_as_record_batch_import` |
| Native Parquet import with table creation rejects Utf8View, dictionary, BinaryView, and FixedSizeBinary columns | Integration | `tests/import_export_tests.rs` | `test_parquet_import_native_path_rejects_csv_only_types_before_create_table` |
| Native Parquet import with table creation rejects Utf8View, dictionary, BinaryView, and FixedSizeBinary columns | Unit | `src/import/parquet.rs` | `test_native_parquet_auto_create_rejects_csv_only_types_before_any_sql` |
| CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch (regression, task 1.4) | Unit | `src/import/parquet.rs` | `test_record_batch_to_csv_formats_pre_epoch_timestamps_in_every_unit` |
| CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range (regression, task 1.4) | Unit | `src/import/parquet.rs` | `test_record_batch_to_csv_rejects_dates_and_timestamps_outside_exasol_range` |

### Manual Testing

Run against the local Exasol container from `AGENTS.md` after `cargo build --release --features ffi` and `pip install adbc-driver-manager pyarrow`, from the repository root.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Bulk Ingestion, Boundaries and Validation | Script M1 below | The four lines listed under M1 |
| Bulk Ingestion (#109) | Script M2 below | The two lines listed under M2 |
| Arrow RecordBatch Import/Export, Parquet I/O | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests test_parquet_import_csv_path_converts_values_as_record_batch_import -- --test-threads=1` | 1 passed |

M1 ingests a Date64 column, then tries a Time64 column in replace mode and a `Decimal128(38, 2)` column in create mode:

```bash
python3 - <<'EOF'
import datetime as dt, decimal, pyarrow as pa, adbc_driver_manager.dbapi as d
c = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="ExarrowDriverInit",
              db_kwargs={"uri": "exasol://sys:exasol@localhost:8563?validateservercertificate=0"}, autocommit=True)
cur = c.cursor()
cur.execute("CREATE SCHEMA IF NOT EXISTS PLAN_CHECK")
print(cur.adbc_ingest("D64", pa.table({"c": pa.array([dt.date(2020, 1, 1)], pa.date64())}), mode="create", db_schema_name="PLAN_CHECK"))
cur.execute("SELECT c FROM PLAN_CHECK.D64"); print(cur.fetchall())
cur.adbc_ingest("KEEP", pa.table({"c": pa.array([1], pa.int32())}), mode="replace", db_schema_name="PLAN_CHECK")
for name, table, mode in [("KEEP", pa.table({"c": pa.array([1], pa.time64("us"))}), "replace"),
                          ("DEC38", pa.table({"n": pa.array([decimal.Decimal("1.50")], pa.decimal128(38, 2))}), "create")]:
    try:
        cur.adbc_ingest(name, table, mode=mode, db_schema_name="PLAN_CHECK")
    except Exception as e:
        print(e)
cur.execute("SELECT COUNT(*) FROM PLAN_CHECK.KEEP"); print(cur.fetchall())
EOF
```

Expected: `1`; `[(datetime.date(2020, 1, 1),)]`; an `INVALID_ARGUMENT` error naming column `c` and `Time64`; an `INVALID_ARGUMENT` error naming column `n` and `Decimal128(38, 2)`; `[(1,)]`.

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
