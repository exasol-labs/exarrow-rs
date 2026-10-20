# Plan: fix-result-set-schema-metadata

## Summary

Every result-set schema, on the native and the WebSocket transport, carries the Exasol type name, precision, scale, and size of each column as `exasol:` Arrow field metadata. The plan also specifies that every result-set field stays nullable, because Exasol reports no column nullability, and documents the metadata keys.

## Context

- GitHub issue exasol-labs/exarrow-rs#102 reports that result-set schemas carry no `exasol:` field metadata. A read of the code at 8662452 confirms the claim for every result-set schema. No test was run.
- `ResultSet::build_schema` (`src/query/results.rs`) builds `Field::new(name, type, true)` with no metadata. It produces the result set metadata schema, the schema of every WebSocket batch and of every zero-row batch, and the schema that ADBC `execute_schema` returns.
- The native transport builds its own batch fields in `src/transport/native/result_parser.rs`, also without metadata. `ResultSet` passes those batches through unchanged, and the ADBC RecordBatchReader takes its schema from the first batch.
- `SchemaBuilder` (`src/types/schema.rs`) attaches metadata through `TypeMapper::create_field_metadata`: `exasol:type` (serialized `ExasolType`), `exasol:precision` and `exasol:scale` (DECIMAL), `exasol:size` (CHAR, VARCHAR), and `exasol:srid` (GEOMETRY with a known SRID). Only `arrow_conversion` uses it. No key holds a plain type name.
- The scenarios "Column metadata" and "Schema documentation" in `type-mapping/exasol-to-arrow` do not match the code. The Background of `query-execution/result-sets` already claims "full schema metadata", and the scenario "NULL handling" already matches the code, because every field is nullable.
- Exasol reports no column nullability in result set metadata on either protocol, so the issue's nullability item becomes a specified fact and no code change. See decision [2].
- On the native transport, batch fields for INTERVAL, GEOMETRY, and HASHTYPE columns are Utf8, as `native-client/type-conversion` specifies, while `TypeMapper` maps them to Interval and Binary. Metadata therefore goes onto the batch's own fields. See decision [1].
- The accepted ADR `zero-row-result-sets-carry-column-schema` builds the zero-row batch from the column schema, so that batch carries the field metadata too. The plan conforms to it and to every other accepted ADR.
- Out of scope: `get_table_schema` and `get_parameter_schema`, which describe tables and parameters, not result sets, and the Parquet and Arrow export schemas.
- Not changed, observed while reading: the WebSocket path converts INTERVAL, GEOMETRY, and HASHTYPE values to strings (`json_column_to_array`) under a schema field of type Interval or Binary, which suggests that such result sets with rows fail. This is unverified and is a candidate for a separate issue. The integration tests of this plan use no such column on the WebSocket transport.
- Architecture: the plan changes the Data Flow and Interfaces sections, see `architecture.md` in this plan directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| type-mapping/exasol-to-arrow | CHANGED | `type-mapping/exasol-to-arrow/spec.md` |
| query-execution/result-sets | CHANGED | `query-execution/result-sets/spec.md` |
| adbc-driver/statement-and-results | CHANGED | `adbc-driver/statement-and-results/spec.md` |
| adbc-driver/get-parameter-schema | CHANGED | `adbc-driver/get-parameter-schema/spec.md` |

## Impact

- Every result-set field gains Arrow field metadata: in `ResultSet::metadata`, in every RecordBatch from `fetch_all`, the result set iterator, and `Connection::query`, in the ADBC RecordBatchReader, and in ADBC `execute_schema`. Python, Polars, and other ADBC driver-manager users can read the Exasol type of a column.
- Arrow data types and values do not change.
- Possibly breaking for Rust callers: arrow-rs compares field metadata in `Field` and `Schema` equality, so a comparison of a result batch schema with a hand-built schema without metadata now fails. The CHANGELOG entry states this.
- The new key `exasol:type_name` also appears in schemas that `SchemaBuilder` builds.

## Implementation Tasks

1. Field metadata keys (`src/types/`)
   - [ ] 1.1 Add `ExasolType::type_name()`, which returns the Exasol type name without parameters (`TIMESTAMP WITH LOCAL TIME ZONE` for a timestamp with local time zone), and make `TypeMapper::create_field_metadata` write it under `exasol:type_name`. Unit tests in `src/types/mapping.rs` cover every `ExasolType` variant.
   - [ ] 1.2 Extend the `SchemaBuilder` unit tests in `src/types/schema.rs` to assert `exasol:type_name`, `exasol:precision`, `exasol:scale`, and `exasol:size`.
2. Result-set schema and batches (`src/query/results.rs`)
   - [ ] 2.1 Make `ResultSet::build_schema` resolve each `ColumnInfo` with `exasol_type_of` and build the schema with `SchemaBuilder`, with every column nullable. Remove `exasol_datatype_to_arrow`. Unit test: `build_schema` of DECIMAL(10,2), VARCHAR(50), and CHAR(3) columns returns nullable fields with the expected keys.
   - [ ] 2.2 In `payload_to_record_batch` and in the Arrow arm of `column_major_to_record_batch`, rebuild an Arrow payload batch with fields that keep the batch field's name, data type, and nullability and take the metadata of the result-set schema field at the same position. Reuse the column arrays without copying them. Return a `ConversionError` when the batch and the schema differ in column count. Unit tests: a Utf8 batch column under an Interval schema field stays Utf8 and gains `exasol:type_name`; each output column is `Arc::ptr_eq` to its input; a fetch page from a mock transport carries the metadata.
   - [ ] 2.3 Add to `tests/common/mod.rs` the DDL and one-row INSERT for table `M` of the scenario "Column metadata", and an assertion helper that checks the expected keys, values, and nullability of each field of `M`.
   - [ ] 2.4 Add the native tests `test_result_set_fields_carry_exasol_column_metadata`, `test_native_interval_field_keeps_utf8_and_carries_exasol_metadata`, `test_zero_row_result_batch_carries_field_metadata`, and `test_every_fetched_batch_carries_the_result_set_field_metadata` (with `fetch_all` and with the iterator over `multi_fetch_query`) to `tests/integration_tests.rs`.
   - [ ] 2.5 Add the WebSocket tests `test_ws_result_set_fields_carry_exasol_column_metadata`, `test_ws_zero_row_result_batch_carries_field_metadata`, and `test_ws_every_fetched_batch_carries_the_result_set_field_metadata` to `tests/websocket_integration_tests.rs`.
   - [ ] 2.6 Add a "Field metadata" section to `docs/type-mapping.md`: a table of each `exasol:` key, the column types that carry it, and an example value; the note that every result-set field is nullable because Exasol reports no nullability; and the use of `TypeMapper::from_field_metadata` to restore the `ExasolType`.
3. ADBC surfaces (`tests/driver_manager_tests.rs`)
   - [ ] 3.1 Add `test_reader_schema_carries_exasol_field_metadata` for the scenario "RecordBatchReader schema carries Exasol field metadata".
   - [ ] 3.2 Extend `assert_execute_schema_reports_columns_without_running` to assert that the field `AMOUNT` of the `execute_schema` result carries `exasol:precision` `10` and `exasol:scale` `2`, and that every field carries `exasol:type_name`. The existing equality check against the zero-row reader schema then covers the field metadata. Update the unit test `execute_schema_maps_result_columns_without_executing` in `src/adbc_ffi.rs` to expect `exasol:type_name`, `exasol:type`, `exasol:precision` `18`, and `exasol:scale` `0` on `N`, and `exasol:type_name`, `exasol:type`, and `exasol:size` `50` on `NAME`. The test runs under `cargo test --lib --features ffi`.
   - [ ] 3.3 Add a CHANGELOG.md entry under the header of the version that this PR bumps to: result-set schemas carry `exasol:` field metadata on both transports, the new key `exasol:type_name`, and the schema-equality note from Impact.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: result-set field metadata | 1.1-1.2, 2.1-2.6 | none | spec deltas `type-mapping/exasol-to-arrow`, `query-execution/result-sets`; `src/types/mapping.rs`, `src/types/schema.rs`, `src/query/results.rs`, `tests/common/mod.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `docs/type-mapping.md` |
| B: ADBC schema surfaces | 3.1-3.3 | A (needs the metadata in result-set schemas) | spec deltas `adbc-driver/statement-and-results`, `adbc-driver/get-parameter-schema`; `tests/driver_manager_tests.rs`, `src/adbc_ffi.rs` (`execute`, `execute_schema`, `reader_over_batches`, and the unit test `execute_schema_maps_result_columns_without_executing`), `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/query/results.rs` `ResultSet::exasol_datatype_to_arrow` | `build_schema` maps types through `SchemaBuilder` |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| type-mapping/exasol-to-arrow: Column metadata (native) | Integration | `tests/integration_tests.rs` | `test_result_set_fields_carry_exasol_column_metadata` |
| type-mapping/exasol-to-arrow: Column metadata (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_ws_result_set_fields_carry_exasol_column_metadata` |
| type-mapping/exasol-to-arrow: Schema documentation (native) | Integration | `tests/integration_tests.rs` | `test_result_set_fields_carry_exasol_column_metadata` (shares table `M`) |
| type-mapping/exasol-to-arrow: Schema documentation (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_ws_result_set_fields_carry_exasol_column_metadata` (shares table `M`) |
| type-mapping/exasol-to-arrow: Result set fields are nullable when Exasol reports no nullability (native) | Integration | `tests/integration_tests.rs` | `test_result_set_fields_carry_exasol_column_metadata` (shares table `M`) |
| type-mapping/exasol-to-arrow: Result set fields are nullable when Exasol reports no nullability (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_ws_result_set_fields_carry_exasol_column_metadata` (shares table `M`) |
| type-mapping/exasol-to-arrow: Field metadata on the native transport keeps the decoded Arrow data type | Integration | `tests/integration_tests.rs` | `test_native_interval_field_keeps_utf8_and_carries_exasol_metadata` |
| query-execution/result-sets: Zero-row result set preserves schema (native) | Integration | `tests/integration_tests.rs` | `test_zero_row_result_batch_carries_field_metadata` |
| query-execution/result-sets: Zero-row result set preserves schema (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_ws_zero_row_result_batch_carries_field_metadata` |
| query-execution/result-sets: Every batch carries the field metadata of the result set schema (native) | Integration | `tests/integration_tests.rs` | `test_every_fetched_batch_carries_the_result_set_field_metadata` |
| query-execution/result-sets: Every batch carries the field metadata of the result set schema (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_ws_every_fetched_batch_carries_the_result_set_field_metadata` |
| adbc-driver/statement-and-results: RecordBatchReader schema carries Exasol field metadata | Integration | `tests/driver_manager_tests.rs` | `test_reader_schema_carries_exasol_field_metadata` |
| adbc-driver/get-parameter-schema: Execute schema returns the result columns of a query without executing it | Integration, Unit | `tests/driver_manager_tests.rs`; `src/adbc_ffi.rs` (unit, `--features ffi`) | `test_execute_schema_reports_columns_without_running_the_query`, `test_execute_schema_reports_columns_without_running_the_query_websocket`; `execute_schema_maps_result_columns_without_executing` |

### Manual Testing

The commands assume a local `exasol/docker-db` on port 8563, a release build from `cargo build --release --features 'ffi websocket'`, and Python with `adbc-driver-manager` and `pyarrow`. `PY` stands for this prefix: `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri':'exasol://sys:exasol@localhost:8563?validateservercertificate=0'}); cur=c.cursor();`

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| type-mapping/exasol-to-arrow | `PY cur.execute(\"SELECT CAST(1.5 AS DECIMAL(10,2)) AS AMOUNT, CAST('a' AS CHAR(3)) AS CODE, CAST(TIMESTAMP '2024-01-02 03:04:05' AS TIMESTAMP WITH LOCAL TIME ZONE) AS TSL\"); print(cur.fetch_arrow_table().schema)"` | `AMOUNT` lists `exasol:type_name: 'DECIMAL'`, `exasol:precision: '10'`, `exasol:scale: '2'`; `CODE` lists `exasol:type_name: 'CHAR'`, `exasol:size: '3'`; `TSL` lists `exasol:type_name: 'TIMESTAMP WITH LOCAL TIME ZONE'`; every field has an `exasol:type` entry |
| type-mapping/exasol-to-arrow (WebSocket) | The type-mapping command with `?validateservercertificate=0&transport=websocket` in the URI | The same field metadata as over the native transport |
| query-execution/result-sets | `PY cur.execute('SELECT t.v AS V FROM VALUES BETWEEN 1 AND 300000 AS t(v)'); print({b.schema.field(0).metadata[b'exasol:type_name'] for b in cur.fetch_record_batch()})"` | `{b'DECIMAL'}` |
| adbc-driver/statement-and-results | `PY cur.execute(\"SELECT CAST(1.5 AS DECIMAL(10,2)) AS AMOUNT, CAST('x' AS VARCHAR(20)) AS NAME\"); print(cur.fetch_record_batch().schema)"` | `AMOUNT` lists `exasol:type_name: 'DECIMAL'`, `exasol:precision: '10'`, `exasol:scale: '2'`; `NAME` lists `exasol:type_name: 'VARCHAR'`, `exasol:size: '20'` |
| adbc-driver/get-parameter-schema | `PY print(cur.adbc_execute_schema(\"SELECT CAST(1.5 AS DECIMAL(10,2)) AS AMOUNT\"))"` | `AMOUNT: decimal128(10, 2)` with `exasol:precision: '10'` and `exasol:scale: '2'` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build && cargo build --no-default-features --features websocket && cargo build --release --features 'ffi websocket'` | Exit 0 |
| Unit tests | `cargo test --lib && cargo test --lib --features websocket && cargo test --lib --features ffi` | 0 failures |
| Native integration tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| WebSocket integration tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors and 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Production line coverage at least 80 percent, no file below 50 percent |
