# Plan: fix-result-set-schema-metadata

## Summary

On the native and the WebSocket transport, every field of the result set metadata schema and of each batch schema carries the Exasol type name, precision, scale, and size of its column as `exasol:` Arrow field metadata. The plan also derives the HASHTYPE byte size from the size that Exasol reports, specifies that every result set field stays nullable, because Exasol reports no column nullability, and documents the metadata keys.

## Context

- GitHub issue exasol-labs/exarrow-rs#102 reports that result sets carry no `exasol:` field metadata. A read of the code at 8662452 confirms the claim for the result set metadata schema and for every batch schema. No test was run.
- Terms: "the result set metadata schema" is `ResultSet::metadata().schema`, and "the batch schema" is the schema of one RecordBatch.
- `ResultSet::build_schema` (`src/query/results.rs`) builds `Field::new(name, type, true)` with no metadata. It produces the result set metadata schema, the batch schema of every WebSocket batch and of every zero-row batch, and the schema that ADBC `execute_schema` returns.
- The native transport builds its own batch fields in `src/transport/native/result_parser.rs`, also without metadata. `ResultSet` passes those batches through unchanged, and the ADBC RecordBatchReader takes its schema from the first batch.
- `SchemaBuilder` (`src/types/schema.rs`) attaches metadata through `TypeMapper::create_field_metadata`: `exasol:type` (serialized `ExasolType`), `exasol:precision` and `exasol:scale` (DECIMAL), `exasol:size` (CHAR, VARCHAR), and `exasol:srid` (GEOMETRY with a known SRID). Only `arrow_conversion` uses it. No key holds a plain type name.
- The scenarios "Column metadata" and "Schema documentation" in `type-mapping/exasol-to-arrow` do not match the code. The Background of `query-execution/result-sets` already claims "full schema metadata", and the scenario "NULL handling" already matches the code, because every field is nullable.
- Exasol reports no column nullability in result set metadata on either protocol, so the issue's nullability item becomes a specified fact and no code change. See decision [2].
- On the native transport, the batch schema and the result set metadata schema differ in Arrow data type for two groups of columns, as `native-client/type-conversion` specifies. INTERVAL, GEOMETRY, and HASHTYPE batch fields are Utf8, while `TypeMapper` maps these types to Interval and Binary. Columns of the INTEGER and SMALLINT wire types are Int64 and Int32 batch fields, while `native_meta_to_data_type` (`src/transport/native/arrow_builder.rs`) reports them as DECIMAL(18,0) and DECIMAL(9,0), which `TypeMapper` maps to Decimal128(18,0) and Decimal128(9,0). Metadata therefore goes onto the batch's own fields. See decision [1].
- `exasol_type_of` (`src/query/results.rs:132`) sets the `byte_size` of every HASHTYPE column to 16, although both transports report a size for a HASHTYPE column. Task 2.7 states how each transport reports it and derives the byte size from it.
- Three other values in the metadata do not reflect the declared column type, and this plan documents them without changing them: GEOMETRY carries no SRID, native INTERVAL DAY TO SECOND precision is always 3, and native integer-encoded DECIMAL columns report precision 18 or 9.
- The accepted ADR `zero-row-result-sets-carry-column-schema` builds the zero-row batch from the column schema, so that batch carries the field metadata too. The plan conforms to it and to every other accepted ADR.
- Out of scope: `get_table_schema` and `get_parameter_schema`, which describe tables and parameters, not result sets, and the Parquet and Arrow export schemas.
- Out of scope: on the WebSocket transport, a result set with rows in an INTERVAL, GEOMETRY, or HASHTYPE column fails, because `json_column_to_array` converts those values to strings under a schema field of type Interval or Binary. A run against Exasol 2025.2.1 on code identical to main confirms the failure, and exasol-labs/exarrow-rs#107 tracks it. The WebSocket tests of this plan use none of these three column types.
- Architecture: the plan changes the Data Flow and Interfaces sections, see `architecture.md` in this plan directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| type-mapping/exasol-to-arrow | CHANGED | `type-mapping/exasol-to-arrow/spec.md` |
| query-execution/result-sets | CHANGED | `query-execution/result-sets/spec.md` |
| adbc-driver/statement-and-results | CHANGED | `adbc-driver/statement-and-results/spec.md` |
| adbc-driver/get-parameter-schema | CHANGED | `adbc-driver/get-parameter-schema/spec.md` |

## Impact

- Every result set field gains Arrow field metadata: in `ResultSet::metadata`, in every RecordBatch from `fetch_all`, the result set iterator, and `Connection::query`, in the ADBC RecordBatchReader, and in ADBC `execute_schema`. Python, Polars, and other ADBC driver-manager users can read the Exasol type of a column.
- Arrow data types and values do not change.
- Breaking for Rust callers that compare schemas: arrow-rs compares field metadata in `Field` and `Schema` equality, so a comparison of a batch schema or of the result set metadata schema with a hand-built schema without metadata now fails. The CHANGELOG entry states this.
- The release is a minor version bump to 0.27.0, because Cargo treats a 0.x minor bump as breaking. Task 3.3 sets 0.27.0 in `Cargo.toml`, so the patch-bump step of implement-pr must skip.
- The new key `exasol:type_name` also appears in schemas that `SchemaBuilder` builds.

## Implementation Tasks

1. Field metadata keys (`src/types/`)
   - [ ] 1.1 Add `ExasolType::type_name()`, which returns the Exasol type name without parameters (`TIMESTAMP WITH LOCAL TIME ZONE` for a timestamp with local time zone), and make `TypeMapper::create_field_metadata` write it under `exasol:type_name`. Unit tests in `src/types/mapping.rs` cover every `ExasolType` variant.
   - [ ] 1.2 Extend the `SchemaBuilder` unit tests in `src/types/schema.rs` to assert `exasol:type_name`, `exasol:precision`, `exasol:scale`, and `exasol:size`.
2. Result set metadata schema and batch schemas (`src/query/results.rs`)
   - [ ] 2.1 Make `ResultSet::build_schema` resolve each `ColumnInfo` with `exasol_type_of` and build the schema with `SchemaBuilder`, with every column nullable. Remove `exasol_datatype_to_arrow`. Point the 19 unit tests that call `ResultSet::exasol_datatype_to_arrow` (18 at lines 1290 to 1463 and `test_exasol_datatype_to_arrow_named_timestamp_with_local_time_zone` near line 2710) at `exasol_type_of` followed by `TypeMapper::exasol_to_arrow`, rename them to `test_exasol_type_of_*`, and keep every default and error case they assert. Unit test: `build_schema` of DECIMAL(10,2), VARCHAR(50), and CHAR(3) columns returns nullable fields with the expected keys.
   - [ ] 2.2 In `payload_to_record_batch` only, rebuild an Arrow payload batch with fields that keep the batch field's name, data type, and nullability and take the metadata of the result set metadata schema field at the same position. Reuse the column arrays without copying them. Return a `ConversionError` when the batch schema and the result set metadata schema differ in column count. Remove the Arrow arm of `column_major_to_record_batch`, which has no production caller: `payload_to_record_batch` passes it the `ResultPayload::Json` rows, and its unit tests pass rows too. Rename `test_payload_to_record_batch_returns_arrow_payload_unchanged` to `test_payload_to_record_batch_keeps_arrow_columns_and_adds_field_metadata`. Unit tests: a Utf8 batch column under an Interval schema field stays Utf8 and gains `exasol:type_name`; an Int64 batch column under a Decimal128(18,0) schema field stays Int64 and gains `exasol:precision` `18`; each output column is `Arc::ptr_eq` to its input; a fetch page from a mock transport carries the metadata.
   - [ ] 2.3 Add to `tests/common/mod.rs` the DDL and one-row INSERT for table `M` of the scenario "Column metadata", and an assertion helper that checks the expected keys, values, and nullability of each field of `M`.
   - [ ] 2.4 Add the native tests `test_result_set_fields_carry_exasol_column_metadata`, `test_native_interval_field_keeps_utf8_and_carries_exasol_metadata`, `test_zero_row_result_batch_carries_field_metadata`, and `test_every_fetched_batch_carries_the_result_set_field_metadata` (with `fetch_all` and with the iterator over `multi_fetch_query`) to `tests/integration_tests.rs`.
   - [ ] 2.5 Add the WebSocket tests `test_ws_result_set_fields_carry_exasol_column_metadata`, `test_ws_zero_row_result_batch_carries_field_metadata`, and `test_ws_every_fetched_batch_carries_the_result_set_field_metadata` to `tests/websocket_integration_tests.rs`.
   - [ ] 2.6 Add a "Field metadata" section to `docs/type-mapping.md`: a table of each `exasol:` key, the column types that carry it, and an example value; the note that every result set field is nullable because Exasol reports no nullability; the use of `TypeMapper::from_field_metadata` to restore the `ExasolType`; and the values that do not reflect the declared column type: GEOMETRY carries no SRID, native INTERVAL DAY TO SECOND precision is always 3, and native integer-encoded DECIMAL columns report precision 18 or 9.
   - [ ] 2.7 Derive the HASHTYPE `byte_size` from the size that Exasol reports, instead of the fixed 16 in `exasol_type_of` (`src/query/results.rs:132`), and keep 16 when Exasol reports no size. Finding from a code and documentation read, with no server run: on the native transport, `parse_column_meta` (`src/transport/native/result_parser.rs:434`) reads HASHTYPE column metadata in the CHAR layout (flag byte, `maxLen`, `octetLen`) and keeps `maxLen`, and `native_meta_to_data_type` (`src/transport/native/arrow_builder.rs:104`) drops it with `size: None`; on the WebSocket transport, the WebSocket API v3 response type table lists `size` for HASHTYPE, which deserializes into `DataType.size`, and `exasol_type_of` ignores it. Neither source states whether the size counts bytes or hex characters, HASHTYPE values arrive as hex strings, and `parse_exasol_type` (`src/arrow_conversion/converter.rs:304`) reads the WebSocket size as bytes. Write the native integration test `test_hashtype_field_metadata_carries_the_declared_byte_size` in `tests/integration_tests.rs` first, and run the HASHTYPE `execute_schema` manual command over the WebSocket transport, to learn the unit of each reported value for HASHTYPE(32 BYTE); if a value changes with the session parameter `HASHTYPE_FORMAT`, derive the byte size from one that does not. Then `native_meta_to_data_type` sets `size` for `T_HASHTYPE` in the unit of the WebSocket `size`, and `exasol_type_of` converts that unit to bytes; if the WebSocket size counts hex characters, `parse_exasol_type` converts it the same way. Unit tests: `native_meta_to_data_type` of a `T_HASHTYPE` column with a `maxLen` sets `size`; `exasol_type_of` of a HASHTYPE `DataType` with the size reported for HASHTYPE(32 BYTE) returns `byte_size` 32, and without a size returns 16.
3. ADBC surfaces (`tests/driver_manager_tests.rs`)
   - [ ] 3.1 Add `test_reader_schema_carries_exasol_field_metadata` for the scenario "RecordBatchReader schema carries Exasol field metadata".
   - [ ] 3.2 Extend `assert_execute_schema_reports_columns_without_running` to assert that the field `AMOUNT` of the `execute_schema` result carries `exasol:precision` `10` and `exasol:scale` `2`, and that every field carries `exasol:type_name`. The existing equality check against the zero-row reader schema then covers the field metadata. Update the unit test `execute_schema_maps_result_columns_without_executing` in `src/adbc_ffi.rs` to expect `exasol:type_name`, `exasol:type`, `exasol:precision` `18`, and `exasol:scale` `0` on `N`, and `exasol:type_name`, `exasol:type`, and `exasol:size` `50` on `NAME`. The test runs under `cargo test --lib --features ffi`.
   - [ ] 3.3 Bump the version to 0.27.0 in `Cargo.toml` and in the `exarrow-rs` entry of `Cargo.lock`, and add a CHANGELOG.md entry under the header `## 0.27.0`: the result set metadata schema and every batch schema carry `exasol:` field metadata on both transports, the new key `exasol:type_name`, and the breaking schema-equality note from Impact. Because `Cargo.toml` then carries 0.27.0, the patch-bump step of implement-pr must skip.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: result set field metadata | 1.1-1.2, 2.1-2.7 | none | spec deltas `type-mapping/exasol-to-arrow`, `query-execution/result-sets`; `src/types/mapping.rs`, `src/types/schema.rs`, `src/query/results.rs`, `src/transport/native/arrow_builder.rs`, `src/transport/native/result_parser.rs` (`parse_column_meta`), `src/arrow_conversion/converter.rs` (`parse_exasol_type`), `tests/common/mod.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `docs/type-mapping.md` |
| B: ADBC schema surfaces | 3.1-3.3 | A (needs the metadata in the result set metadata schema and the batch schemas) | spec deltas `adbc-driver/statement-and-results`, `adbc-driver/get-parameter-schema`; `tests/driver_manager_tests.rs`, `src/adbc_ffi.rs` (`execute`, `execute_schema`, `reader_over_batches`, and the unit test `execute_schema_maps_result_columns_without_executing`), `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/query/results.rs` `ResultSet::exasol_datatype_to_arrow` | `build_schema` maps types through `SchemaBuilder` |
| Test | `src/query/results.rs` tests named `test_exasol_datatype_to_arrow_*` (19) | Point at `exasol_type_of` followed by `TypeMapper::exasol_to_arrow` and rename to `test_exasol_type_of_*`; do not delete, because they are the only tests of the `exasol_type_of` defaults and of its error for an unsupported type |
| Branch | `src/query/results.rs` Arrow arm of `column_major_to_record_batch`, with `test_column_major_to_record_batch_returns_arrow_payload_unchanged` | Unreachable: its only production caller is the `ResultPayload::Json` arm of `payload_to_record_batch` (`src/query/results.rs:411`), which handles Arrow payloads itself |

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
| type-mapping/exasol-to-arrow: HASHTYPE field metadata carries the byte size of the column | Integration, Unit | `tests/integration_tests.rs`; `src/query/results.rs` and `src/transport/native/arrow_builder.rs` (unit, WebSocket and native size derivation) | `test_hashtype_field_metadata_carries_the_declared_byte_size`; `test_exasol_type_of_hashtype_uses_reported_size`, `test_exasol_type_of_hashtype`, `native_meta_to_data_type_reports_hashtype_size` |
| query-execution/result-sets: Zero-row result set preserves schema (native) | Integration | `tests/integration_tests.rs` | `test_zero_row_result_batch_carries_field_metadata` |
| query-execution/result-sets: Zero-row result set preserves schema (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_ws_zero_row_result_batch_carries_field_metadata` |
| query-execution/result-sets: Every batch carries the field metadata of the result set metadata schema (native) | Integration | `tests/integration_tests.rs` | `test_every_fetched_batch_carries_the_result_set_field_metadata` |
| query-execution/result-sets: Every batch carries the field metadata of the result set metadata schema (WebSocket) | Integration | `tests/websocket_integration_tests.rs` | `test_ws_every_fetched_batch_carries_the_result_set_field_metadata` |
| adbc-driver/statement-and-results: RecordBatchReader schema carries Exasol field metadata | Integration | `tests/driver_manager_tests.rs` | `test_reader_schema_carries_exasol_field_metadata` |
| adbc-driver/get-parameter-schema: Execute schema returns the result columns of a query without executing it | Integration, Unit | `tests/driver_manager_tests.rs`; `src/adbc_ffi.rs` (unit, `--features ffi`) | `test_execute_schema_reports_columns_without_running_the_query`, `test_execute_schema_reports_columns_without_running_the_query_websocket`; `execute_schema_maps_result_columns_without_executing` |

### Manual Testing

The commands assume a local `exasol/docker-db` on port 8563, a release build from `cargo build --release --features 'ffi websocket'`, and Python with `adbc-driver-manager` and `pyarrow`. `PY` stands for this prefix: `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri':'exasol://sys:exasol@localhost:8563?validateservercertificate=0'}); cur=c.cursor();`

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| type-mapping/exasol-to-arrow | `PY cur.execute(\"SELECT CAST(1.5 AS DECIMAL(10,2)) AS AMOUNT, CAST('a' AS CHAR(3)) AS CODE, CAST(TIMESTAMP '2024-01-02 03:04:05' AS TIMESTAMP WITH LOCAL TIME ZONE) AS TSL\"); print(cur.fetch_arrow_table().schema)"` | `AMOUNT` lists `exasol:type_name: 'DECIMAL'`, `exasol:precision: '10'`, `exasol:scale: '2'`; `CODE` lists `exasol:type_name: 'CHAR'`, `exasol:size: '3'`; `TSL` lists `exasol:type_name: 'TIMESTAMP WITH LOCAL TIME ZONE'`; every field has an `exasol:type` entry |
| type-mapping/exasol-to-arrow (WebSocket) | The type-mapping command with `?validateservercertificate=0&transport=websocket` in the URI | The same field metadata as over the native transport |
| type-mapping/exasol-to-arrow (HASHTYPE) | `PY cur.execute(\"SELECT CAST('0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' AS HASHTYPE(32 BYTE)) AS H\"); print(cur.fetch_arrow_table().schema.field('H').metadata)"` | `exasol:type_name` is `HASHTYPE`, and `exasol:type` holds `"byte_size":32` |
| type-mapping/exasol-to-arrow (HASHTYPE, WebSocket) | With `?validateservercertificate=0&transport=websocket` in the URI: `PY print(cur.adbc_execute_schema(\"SELECT CAST('0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' AS HASHTYPE(32 BYTE)) AS H\").field('H').metadata)"`. `execute_schema` prepares the query and fetches no rows, so the failure that #107 tracks does not occur | `exasol:type` holds `"byte_size":32` |
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
