# Plan: change-integer-result-columns-to-decimal

## Summary

On Exasol 2025.2.1, native result sets already return INTEGER and SMALLINT columns as Decimal128(18, 0) and Decimal128(9, 0), the types of the WebSocket transport and of the result set metadata schema. The plan corrects `docs/type-mapping.md`, `docs/queries.md`, and the specs `type-mapping/exasol-to-arrow` and `native-client/type-conversion`, which state Int64 and Int32 for these columns. It also aligns the decode of the `T_INTEGER` and `T_SMALLINT` wire types with the DECIMAL(18,0) and DECIMAL(9,0) that the native transport reports for them. Results on Exasol 2025.2.1 do not change.

## Context

- Exasol INTEGER and SMALLINT are aliases of DECIMAL(18,0) and DECIMAL(9,0). `TypeMapper::exasol_to_arrow` maps them to Decimal128(18, 0) and Decimal128(9, 0), so the WebSocket batches, the result set metadata schema of both transports, and ADBC `execute_schema` already use these types.
- The native protocol defines the `T_INTEGER` (5) wire type with 8-byte little-endian values and the `T_SMALLINT` (4) wire type with 4-byte values. Their column description carries no precision or scale (`parse_column_meta`), and `native_meta_to_data_type` reports DECIMAL(18,0) and DECIMAL(9,0) for them. No tested server sends either wire type for a result column.
- `fill_integer_column` and `fill_smallint_column` decode the `T_INTEGER` and `T_SMALLINT` wire types into Int64 and Int32 arrays, so a batch field of either wire type would differ from its result set metadata schema field, Decimal128(18, 0) or Decimal128(9, 0).
- `fill_decimal_column` and `fill_smalldecimal_column` read 8-byte and 4-byte values into Decimal128 arrays, with precision 18 and 9 and scale 0 when the column description has no precision.
- A wire probe on Exasol 2025.2.1 shows the native wire types of result columns. INTEGER, DECIMAL(10..18,0), and COUNT(*) arrive as `T_DECIMAL` (6). SMALLINT and DECIMAL(p<=9,0) arrive as `T_SMALLDECIMAL` (63). BIGINT and DECIMAL(p>18) arrive as `T_BIGDECIMAL` (64). TIMESTAMP WITH LOCAL TIME ZONE arrives as `T_TIMESTAMP` (21). This holds for CAST expressions, table columns, system-table columns, multi-fetch results, and prepared statements. The probe table is in `notes/planning.md`.
- No result column arrived as `T_INTEGER` (5) or `T_SMALLINT` (4), so native INTEGER and SMALLINT results are already Decimal128(18, 0) and Decimal128(9, 0). `SELECT CAST(5 AS INTEGER) I, CAST(5 AS SMALLINT) S` returns these types over the native and the WebSocket transport of the release driver. Other server versions were not probed.
- `get_int_value` in the FFI, two driver manager test helpers, and the benchmark read Int64 or Int32 arrays from query results. No production code outside the FFI reads native integer columns by type.
- Version 0.28.0 is unreleased (PR #108), so the CHANGELOG entry joins its section and the version stays 0.28.0.
- The data flow correction is in the architecture delta `architecture.md`.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Exasol to Arrow | CHANGED | `type-mapping/exasol-to-arrow/spec.md` |
| Native Result Set Type Conversion | CHANGED | `native-client/type-conversion/spec.md` |

## Impact

- Results on Exasol 2025.2.1 do not change: native and WebSocket INTEGER and SMALLINT columns stay Decimal128(18, 0) and Decimal128(9, 0).
- Docs fix: `docs/type-mapping.md`, `docs/queries.md`, and the specs `type-mapping/exasol-to-arrow` and `native-client/type-conversion` state Decimal128(18, 0) and Decimal128(9, 0) for native INTEGER and SMALLINT results instead of Int64 and Int32.
- Code alignment: the native transport decodes the `T_INTEGER` and `T_SMALLINT` wire types into the Decimal128 types of the DECIMAL(18,0) and DECIMAL(9,0) that it reports for them. No tested server sends these wire types for a result column.
- Not changed: the WebSocket results, the result set metadata schema, ADBC `execute_schema`, the `exasol:` field metadata, parameter binding, and the Parquet export types.

## Implementation Tasks

### Group A: Native INTEGER and SMALLINT decoding

- [ ] 1.1 In `tests/native_protocol_tests.rs`, add the regression test `test_native_integer_and_smallint_results_are_decimal128` with the one line `/// Scenario: INTEGER and SMALLINT type mapping`: over `get_test_connection()`, run `SELECT CAST(-999999999999999999 AS INTEGER) AS I, CAST(999999999 AS SMALLINT) AS S FROM DUAL` and the same query with `WHERE FALSE`, and assert Decimal128(18, 0) for `I` and Decimal128(9, 0) for `S` in the result set metadata schema, in the batch, and in the zero-row batch (per field, because `assert_data_type_in_all_three_schemas` takes one type for every field), and the Decimal128 values -999999999999999999 and 999999999. The test passes before 1.2, because Exasol sends these columns as `T_DECIMAL` and `T_SMALLDECIMAL`, and it keeps the documented types from regressing
- [ ] 1.2 Write the unit tests of this task first and confirm that they fail with Int64 and Int32: they feed synthetic `T_INTEGER` and `T_SMALLINT` bytes, which no tested Exasol server sends for a result column. Give each of them the line `/// Scenario: Direct binary to Arrow conversion for numeric types`. In `fill_column_builder` (`src/transport/native/result_parser.rs` l.595), route `T_INTEGER` to `fill_decimal_column` and `T_SMALLINT` to `fill_smalldecimal_column`, which build Decimal128(18, 0) from the 8-byte value and Decimal128(9, 0) from the 4-byte value when the column description has no precision. Delete `fill_integer_column` (l.664), `fill_smallint_column` (l.684), and the `Int32Builder` and `Int64Builder` imports if no code uses them, and state in the doc comment of `fill_column_builder` (l.589) that the `T_INTEGER` and `T_SMALLINT` wire types decode to the Decimal128 type of the DECIMAL(18,0) and DECIMAL(9,0) that `native_meta_to_data_type` reports. In the unit tests of the file, rename `single_pass_smallint_column_decodes_to_int32` (l.1958) to `single_pass_smallint_column_decodes_to_decimal128` and assert Decimal128(9, 0) with the values -7, 999999999, -999999999, and NULL; add `single_pass_integer_column_decodes_to_decimal128` with 999999999999999999, -999999999999999999, and NULL, asserting Decimal128(18, 0) and that the offset reaches the end of the data; change `single_pass_all_primitive_types_offsets_align`, `single_pass_empty_rows_produces_typed_empty_batch`, and `parse_fetch_to_record_batch_reads_row_count_prefix` to expect Decimal128(18, 0) and `Decimal128Array`; add `integer_wire_types_take_the_arrow_type_of_their_reported_decimal`, which asserts for `T_INTEGER` and `T_SMALLINT` that the batch field type equals `TypeMapper::exasol_to_arrow` of `crate::query::results::exasol_type_of(&native_meta_to_data_type(&meta))`
- [ ] 1.3 In `ResultSet::payload_to_record_batch` (`src/query/results.rs` l.405-412), replace the INTEGER and SMALLINT example in the doc comment with the remaining native wire type that decodes to another type: `T_TIMESTAMP_LOCAL_TZ` decodes to Timestamp(Microsecond, None) under a Timestamp(Microsecond, "UTC") field. Rewrite `test_payload_to_record_batch_keeps_int64_column_under_decimal_schema_field` (l.2782) as `test_payload_to_record_batch_keeps_timestamp_column_under_local_time_zone_schema_field`, with a Timestamp(Microsecond, None) batch under the metadata schema of a `TIMESTAMP WITH LOCAL TIME ZONE` column. Replace the test helper `int64_batch` (l.2675) with `decimal_batch`, which builds a Decimal128(18, 0) column, the type that the native transport sends for DECIMAL(18,0), update its callers (l.2764, l.2941, l.2959), and change the Int64 assertion of `test_payload_to_record_batch_keeps_arrow_columns_and_adds_field_metadata` (l.2773) to Decimal128(18, 0)

### Group B: Cross-transport tests and integer consumers

- [ ] 2.1 In `tests/websocket_integration_tests.rs`, add `test_ws_integer_result_equals_native` and `test_ws_smallint_result_equals_native` after `test_ws_hashtype_result_equals_native`. Each uses a `TwoRowCase`: `CAST(-999999999999999999 AS INTEGER)` with type `INTEGER`, and `CAST(999999999 AS SMALLINT)` with type `SMALLINT`. Each calls `assert_two_row_result` over `get_ws_connection` with Decimal128(18, 0) or Decimal128(9, 0), asserts the Decimal128 value of the first row, and calls `assert_equals_native` in a `#[cfg(feature = "native")]` block
- [ ] 2.2 Delete `test_integer_to_arrow_int64` (`tests/integration_tests.rs` l.1281-1349) and `test_ws_integer_to_arrow_int64` (`tests/websocket_integration_tests.rs` l.998-1060), whose permissive type checks 1.1 and 2.1 replace. Add the line `/// Scenario: Non-decimal numeric type mapping` to `test_double_type_conversion` (`tests/integration_tests.rs` l.1716) and `test_ws_double_type_conversion` (`tests/websocket_integration_tests.rs` l.1405)
- [ ] 2.3 In `get_int_value` (`src/adbc_ffi.rs` l.961-976), delete the Int32 and Int64 arms and change the doc comment to state that Exasol returns whole numbers as Decimal128 on both transports. In the unit tests, remove the Int32 and Int64 cases from `get_int_value_reads_every_numeric_column_type_exasol_may_report` (l.3082), assert in `get_int_value_reports_null_and_unsupported_columns_as_absent` (l.3099) that an Int32 or Int64 value reads as absent, and build the array of `get_int_value_reads_the_requested_row_not_the_first` (l.3120) as a `Decimal128Array`
- [ ] 2.4 Keep only the Decimal128 branch in `integers_in` (`tests/driver_manager_tests.rs` l.136-152) and in the value loop at l.2479-2492, and in the COUNT reader of `benches/rust/benchmark.rs` (l.398-406), deleting its Int64 branch and the comment above it

### Group C: Documentation and release

- [ ] 3.1 In `docs/type-mapping.md`, set the INTEGER row (l.18) to `Decimal128(18, 0)` with the note "Alias of `DECIMAL(18, 0)`" and the SMALLINT row (l.19) to `Decimal128(9, 0)` with the note "Alias of `DECIMAL(9, 0)`"; delete the Field Metadata line on native INTEGER and SMALLINT precision (l.112), because both transports report DECIMAL(18,0) and DECIMAL(9,0); in the WebSocket Transport Differences table (l.116-123), replace the INTEGER and SMALLINT rows with a `TIMESTAMP WITH LOCAL TIME ZONE` row (native `Timestamp(Microsecond, None)`, WebSocket `Timestamp(Microsecond, "UTC")`), because Exasol sends that column as `T_TIMESTAMP` on the native wire (`type-mapping/exasol-to-arrow-field-metadata`), and set the TIMESTAMP WITH LOCAL TIME ZONE row of the type table (l.22) to `Timestamp(Microsecond, None)` with the note "`Timestamp(Microsecond, "UTC")` over WebSocket". If the manual test shows other TIMESTAMP WITH LOCAL TIME ZONE types, write the types it shows
- [ ] 3.2 In `docs/queries.md` (l.36-42), read the `age` column as a `Decimal128Array` instead of an `Int64Array`, with one sentence that an INTEGER column is Decimal128(18, 0), whose `value(i)` is the integer because the scale is 0
- [ ] 3.3 In `CHANGELOG.md`, add one Fix entry after the two Fix entries of the `## 0.28.0` section: the docs listed `Int64` and `Int32` for native INTEGER and SMALLINT results, and both transports return `Decimal128(18, 0)` and `Decimal128(9, 0)`. Do not change the version in `Cargo.toml`

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Native INTEGER and SMALLINT decoding | 1.1-1.3 | none | spec deltas `native-client/type-conversion`, `type-mapping/exasol-to-arrow` (scenario "INTEGER and SMALLINT type mapping", native half); architecture delta (Data Flow); `src/transport/native/result_parser.rs`, `src/transport/native/arrow_builder.rs`, `src/query/results.rs`, `tests/native_protocol_tests.rs`, `tests/common/mod.rs` |
| B: Cross-transport tests and integer consumers | 2.1-2.4 | A (2.1 compares with the native values that 1.2 produces; 2.3 and 2.4 assume no Int64 or Int32 result column) | spec delta `type-mapping/exasol-to-arrow` (WebSocket half and the equality scenario, "Non-decimal numeric type mapping"); `tests/websocket_integration_tests.rs`, `tests/integration_tests.rs`, `src/adbc_ffi.rs`, `tests/driver_manager_tests.rs`, `benches/rust/benchmark.rs` |
| C: Documentation and release | 3.1-3.3 | A | both spec deltas; `docs/type-mapping.md`, `docs/queries.md`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/transport/native/result_parser.rs`: `fill_integer_column`, `fill_smallint_column` | Replaced by `fill_decimal_column` and `fill_smalldecimal_column` in 1.2 |
| Test | `src/query/results.rs`: `test_payload_to_record_batch_keeps_int64_column_under_decimal_schema_field` and the helper `int64_batch` | The native transport no longer sends Int64 under a Decimal128 field; replaced in 1.3 |
| Test | `tests/integration_tests.rs`: `test_integer_to_arrow_int64`; `tests/websocket_integration_tests.rs`: `test_ws_integer_to_arrow_int64` | Their permissive type checks accept Int64; replaced by 1.1 and 2.1 |
| Code | `src/adbc_ffi.rs` `get_int_value`: the Int32 and Int64 arms | No result column of either transport is Int32 or Int64 after 1.2 |
| Test code | `tests/driver_manager_tests.rs`: the Int32 and Int64 branches of `integers_in` and of the value loop at l.2479-2492; `benches/rust/benchmark.rs`: the Int64 branch of the COUNT reader | Same reason |
| Doc | `docs/type-mapping.md` l.112 and the INTEGER and SMALLINT rows of the WebSocket Transport Differences table | Both transports return the same types and metadata for these columns |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Non-decimal numeric type mapping | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_double_type_conversion`, `test_ws_double_type_conversion` (unchanged apart from the `/// Scenario:` line) |
| INTEGER and SMALLINT type mapping | Integration | `tests/native_protocol_tests.rs`, `tests/websocket_integration_tests.rs` | `test_native_integer_and_smallint_results_are_decimal128`, `test_ws_integer_result_equals_native`, `test_ws_smallint_result_equals_native` |
| WebSocket INTEGER and SMALLINT results equal native results | Integration | `tests/websocket_integration_tests.rs` | `test_ws_integer_result_equals_native`, `test_ws_smallint_result_equals_native` |
| Direct binary to Arrow conversion for numeric types | Unit | `src/transport/native/result_parser.rs` | The 1.2 unit tests: `single_pass_integer_column_decodes_to_decimal128`, `single_pass_smallint_column_decodes_to_decimal128`, `single_pass_all_primitive_types_offsets_align`, `single_pass_empty_rows_produces_typed_empty_batch`, `parse_fetch_to_record_batch_reads_row_count_prefix`, `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` |

Each test carries one `/// Scenario: <title>` line per scenario it implements, with the title quoted verbatim, per `AGENTS.md`. The native halves of `test_ws_integer_result_equals_native` and `test_ws_smallint_result_equals_native` sit in `#[cfg(feature = "native")]` blocks, because `tests/websocket_integration_tests.rs` also builds without the `native` feature. The 1.2 unit tests cover pure decoding of wire bytes with no I/O. Test commands and the Exasol test environment are in `specs/mission.md` and `AGENTS.md` § Testing.

### Manual Testing

Start Exasol per `AGENTS.md` § Testing, then build the driver with `cargo build --release --features 'ffi websocket'`. The commands use Python with `adbc-driver-manager` and `pyarrow`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Exasol to Arrow, Native Result Set Type Conversion (native) | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri':'exasol://sys:exasol@localhost:8563?validateservercertificate=0'}); cur=c.cursor(); cur.execute(\"SELECT CAST(-999999999999999999 AS INTEGER) I, CAST(999999999 AS SMALLINT) S, CAST(NULL AS INTEGER) N, CAST(TIMESTAMP '2024-01-02 03:04:05' AS TIMESTAMP WITH LOCAL TIME ZONE) TSL\"); t=cur.fetch_arrow_table(); print(t.schema); print(t.to_pylist())"` | `I` and `N` are `decimal128(18, 0)` fields, `S` is a `decimal128(9, 0)` field, and `TSL` is a `timestamp[us]` field; the row holds `Decimal('-999999999999999999')`, `Decimal('999999999')`, and `None` |
| Exasol to Arrow (WebSocket) | The same command with `&transport=websocket` appended to the URI | `I`, `S`, and `N` have the same types and values as in the native run; `TSL` is a `timestamp[us, tz=UTC]` field |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` and `cargo build --release --features 'ffi websocket'` | Exit 0 |
| WebSocket-only test build | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit tests | `cargo test --lib`, `cargo test --lib --features websocket`, `cargo test --lib --features ffi` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1`, `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1`, `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `cargo build --release --features ffi`, then `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `scripts/strip_test_coverage.py strip` and `check` | Line coverage at least 80 percent, every file at least 50 percent |
