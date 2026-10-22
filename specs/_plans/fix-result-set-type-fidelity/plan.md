# Plan: fix-result-set-type-fidelity

## Summary

Queries that return INTERVAL, GEOMETRY, or HASHTYPE columns return Arrow Interval(MonthDayNano) and Binary values with the same types and values on the native and the WebSocket transport, so WebSocket queries with these columns no longer fail (GitHub #107). WebSocket GEOMETRY fields carry `exasol:srid`, and native INTERVAL DAY TO SECOND fields report their declared fractional second precision in `exasol:type`.

## Context

- On the WebSocket transport, the result set metadata schema declares Interval(MonthDayNano) for INTERVAL and Binary for GEOMETRY and HASHTYPE, while `ResultSet::json_column_to_array` builds Utf8 arrays for them through its `_` arm, so `RecordBatch::try_new` rejects the batch (#107). Every #107 query fails this way against Docker Exasol (`notes/planning.md`, wire observations).
- The native wire carries INTERVAL, GEOMETRY, and HASHTYPE values as length-prefixed UTF-8 text, byte-identical to the WebSocket JSON string, such as `+01-02`, `+03 04:05:06.789`, `POINT (1 2)`, and `550e8400e29b11d4a716446655440000`. The native parser copies this text into Utf8 arrays, while the result set metadata schema and the zero-row batch declare Interval(MonthDayNano) and Binary.
- The base branch already attaches `exasol:type_name`, `exasol:type`, `exasol:precision`, `exasol:scale`, and `exasol:size` to every result set field on both transports, keeps every field nullable, and derives the HASHTYPE byte size from the reported size (`type-mapping/exasol-to-arrow-field-metadata`).
- Two type parameters are still lost: `messages::DataType` has no `srid` member, although the WebSocket `dataType` sends `srid` (0 for `GEOMETRY`, 4326 for `GEOMETRY(4326)`), and `parse_column_meta` reads the native INTERVAL DAY TO SECOND fraction and discards it, so `exasol_type_of` reports precision 3. The native column description has no SRID.
- Exasol allows INTERVAL YEAR(9) TO MONTH values up to 999,999,999 years, and the Arrow IntervalMonthDayNano month count is a 32-bit integer. The current `arrow_conversion` parser computes `years * 12 + months` without an overflow check.
- The component, data flow, and interface changes are in the architecture delta `architecture.md`.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Exasol to Arrow | CHANGED | `type-mapping/exasol-to-arrow/spec.md` |
| Exasol to Arrow DATE and TIMESTAMP | CHANGED | `type-mapping/exasol-to-arrow-dates/spec.md` |
| Exasol to Arrow Field Metadata | CHANGED | `type-mapping/exasol-to-arrow-field-metadata/spec.md` |
| Native Result Set Type Conversion | CHANGED | `native-client/type-conversion/spec.md` |

## Impact

- Fix: a WebSocket query that returns INTERVAL, GEOMETRY, or HASHTYPE values returns Interval(MonthDayNano) and Binary values, equal to the native values of the same query, instead of failing.
- Fix: WebSocket GEOMETRY fields carry `exasol:srid`, and native INTERVAL DAY TO SECOND fields carry the declared fractional second precision in `exasol:type` instead of 3.
- Breaking: native result sets return INTERVAL as Interval(MonthDayNano) instead of Utf8 text. An INTERVAL YEAR TO MONTH value above 178,956,970 years fails the query on both transports, and `ArrowConverter` returns an error for it instead of a wrapped month count.
- Breaking: native result sets return GEOMETRY and HASHTYPE as Binary instead of Utf8. Each Binary value holds the text bytes that Exasol sends: WKT for GEOMETRY, and the session's `HASHTYPE_FORMAT` text, hexadecimal by default, for HASHTYPE.
- Breaking: the public structs `transport::messages::DataType` and `transport::native::result_parser::NativeColumnMeta` gain the members `srid` and `interval_fraction`, so code that builds them with a struct literal must set them.
- Not changed: native GEOMETRY fields carry no `exasol:srid`, because the native protocol sends none. The Parquet export keeps writing these types as Utf8 text.

## Migration

| Current | New |
|---------|-----|
| Native INTERVAL column: Utf8 text, such as `+01-02` | Interval(MonthDayNano): months 14, days 0, nanoseconds 0 |
| Native GEOMETRY or HASHTYPE column: Utf8 text | Binary holding the same text bytes; decode them as UTF-8 to get the old string |
| WebSocket query with INTERVAL, GEOMETRY, or HASHTYPE: fails | Interval(MonthDayNano) and Binary values, equal to the native values |
| `DataType { type_name, precision, scale, size, character_set, with_local_time_zone, fraction }` | The same struct literal plus `srid: None` |
| `NativeColumnMeta { name, type_id, precision, scale, is_varchar, max_len }` | The same struct literal plus `interval_fraction: None` |

## Implementation Tasks

### Group A: Shared INTERVAL parser and native values

- [ ] 1.1 In `src/types/conversion.rs`, add `pub fn parse_interval(text: &str) -> Result<arrow::datatypes::IntervalMonthDayNano, String>`, built from `parse_interval_year_to_month`, `parse_interval_day_to_second`, `parse_interval_time_to_nanos`, and `parse_interval_seconds_to_nanos` of `src/arrow_conversion/builders.rs` (l.375-504): it reads YEAR TO MONTH text when the text after its sign holds a `-` and DAY TO SECOND text otherwise, applies the sign to every component, computes the month count with checked arithmetic so that a count outside the `i32` range returns an error, and accepts every form the current parsers accept. Delete the four functions and have both `arrow_conversion` interval builders call `parse_interval`, mapping the error to `ColumnConversionError::invalid_value` with the row. Move their unit tests and add cases for `+01-02`, `-01-02`, `+999999999-11` (error), `-03 04:05:06.789`, `+03 04:05:06` (no fraction), and `-999999999 23:59:59.999000000`
- [ ] 1.2 In `fill_column_builder` (`src/transport/native/result_parser.rs` l.588), route `T_INTERVAL_YEAR` and `T_INTERVAL_DAY` to a new filler that reads each length-prefixed value, decodes it as UTF-8 in place, parses it with `parse_interval`, and appends it to an `IntervalMonthDayNanoBuilder`, failing with a `TransportError::ProtocolError` whose message names the column and the value; route `T_GEOMETRY` and `T_HASHTYPE` to `fill_binary_column` (l.712). Update the doc comment of `fill_column_builder`, keep only `T_CHAR` and the unknown type id in `single_pass_string_like_types_all_decode_as_utf8` (l.1956), and add unit tests through `build_batch_from_wire` that feed the observed wire text (`+01-02`, `-03 04:05:06.789`, `+999999999-11`, `POINT (1 2)`, `550e8400e29b11d4a716446655440000`, and a NULL flag)
- [ ] 1.3 Add `pub interval_fraction: Option<i32>` to `NativeColumnMeta` (`src/transport/native/result_parser.rs` l.25) and fill it from the fraction that `parse_column_meta` reads for `T_INTERVAL_DAY` (l.477); in `native_meta_to_data_type` (`src/transport/native/arrow_builder.rs`), pass it into `fraction` for `T_INTERVAL_DAY`. Set `interval_fraction: None` in every other `NativeColumnMeta` literal, and extend the unit tests of both files with an INTERVAL DAY TO SECOND column of fraction 6
- [ ] 1.4 In `tests/native_protocol_tests.rs`, add `test_native_interval_results_are_month_day_nano` (the `YM` and `DS` query of "INTERVAL type mapping": assert Interval(MonthDayNano) in the result set metadata schema, in the batch, and in the batch of the same query with `WHERE FALSE`, and the month, day, and nanosecond values; then run the `BIG` query of "INTERVAL YEAR TO MONTH value outside the Arrow month range" and assert an error that names `BIG`) and `test_native_geometry_and_hashtype_results_are_binary` (the `G` and `H` queries of "GEOMETRY type handling" and "HASHTYPE type mapping", asserting Binary in all three schemas and the text bytes as values)
- [ ] 1.5 In `tests/integration_tests.rs`, replace `test_native_interval_field_keeps_utf8_and_carries_exasol_metadata` (l.4202) with `test_native_type_parameters_in_field_metadata` (`#[cfg(feature = "native")]`): run the `IV` query of "INTERVAL DAY TO SECOND field metadata carries the fractional second precision" and the `G1`/`G0` query of "GEOMETRY field metadata carries the SRID that Exasol reports" over `get_test_connection_with_transport("native")`, and assert the native half of both scenarios on the result set metadata schema and on the batch schema

### Group B: WebSocket values and the SRID

- [ ] 2.1 In `ResultSet::json_column_to_array` (`src/query/results.rs` l.515), add an `Interval(MonthDayNano)` arm that turns a JSON null into NULL, parses a JSON string with `parse_interval`, and fails the column with `ColumnConversionError::invalid_value` for a string that does not parse or a value that is not a string, and a `Binary` arm that stores the UTF-8 bytes of each JSON string unchanged and turns any other value into NULL. Replace the `_` arm (l.557-558) with a `ColumnConversionError` that names the Arrow type, and state the INTERVAL failure in the doc comment. Add unit tests through `column_major_to_record_batch` with a schema from `ResultSet::build_schema` for `+01-02`, `-03 04:05:06.789`, `+999999999-11` (an error that names the column), `POINT (1 2)`, `550e8400e29b11d4a716446655440000`, and NULL; keep `test_column_major_to_record_batch_unsupported_type_returns_error` (l.1991) passing and fix its comments
- [ ] 2.2 In `src/transport/messages.rs`, add `pub srid: Option<i32>` to `DataType` (l.379) with `#[serde(skip_serializing_if = "Option::is_none")]`, and set `srid: None` in every `DataType` struct literal in `src/`, `tests/`, `benches/`, and `examples/`. Have `exasol_type_of` (`src/query/results.rs` l.131) and `parse_exasol_type` (`src/arrow_conversion/converter.rs` l.302) build `ExasolType::Geometry { srid: data_type.srid }`. Add unit tests that deserialize `{"type":"GEOMETRY","size":2000000,"srid":4326}` and that resolve GEOMETRY with srid 4326 and without srid in both resolvers
- [ ] 2.3 In `ResultSet::payload_to_record_batch` (`src/query/results.rs` l.401-407), change the doc comment to name the INTEGER and SMALLINT wire types (Int64 and Int32 under Decimal128 fields) as the native batch types that differ from `TypeMapper`, and rewrite `test_payload_to_record_batch_keeps_utf8_column_under_interval_schema_field` (l.2734) as a test that keeps an Int64 column under a `DECIMAL(18,0)` field of the result set metadata schema
- [ ] 2.4 In `tests/websocket_integration_tests.rs`, add one test per type: `test_ws_interval_year_to_month_result_equals_native`, `test_ws_interval_day_to_second_result_equals_native`, `test_ws_geometry_result_equals_native`, and `test_ws_hashtype_result_equals_native`. Each runs the #107 query of its type (`YM`, `DS`, `G`, or `H` of the "Exasol to Arrow" scenarios) with a second row that casts NULL to the same type, ordered by a row-number column, over `get_ws_connection`; it asserts the Interval(MonthDayNano) or Binary field in the result set metadata schema, in the batch, and in the batch of the same query with `WHERE FALSE`, the observed value, and NULL in the second row; in a `#[cfg(feature = "native")]` block, it runs the same query over `common::get_test_connection_with_transport("native")` and asserts equal data types and equal arrays. `test_ws_interval_year_to_month_result_equals_native` also runs the `BIG` query over WebSocket and asserts an error that names `BIG`
- [ ] 2.5 In `tests/websocket_integration_tests.rs`, add `test_ws_type_parameters_in_field_metadata` (the `IV` query and the `G1`/`G0` query, asserting the WebSocket half of the INTERVAL fraction and GEOMETRY SRID scenarios on the result set metadata schema and the batch schema); change `test_ws_hashtype_field_metadata_carries_the_declared_byte_size` (l.2479) to return one row without `WHERE FALSE`, assert the batch schema field as well, remove the #107 comment, and add the `/// Scenario:` line

### Group C: Documentation and release

- [ ] 3.1 In `docs/type-mapping.md`, set the two INTERVAL rows of the type table (l.23-24) to `Interval(MonthDayNano)` and the GEOMETRY and HASHTYPE rows (l.26-27) to `Binary`, with notes that the value holds the WKT or hash text bytes; rewrite the INTERVAL, GEOMETRY, and HASHTYPE sections (l.59-77) with the observed text forms, the month, day, and nanosecond rule with its sign, the 178,956,970-year limit, and the effect of `HASHTYPE_FORMAT`; in the Field Metadata limits (l.104-110), state that only WebSocket GEOMETRY fields carry `exasol:srid` and delete the native INTERVAL precision 3 line; delete the native Utf8 note (l.112); in the WebSocket Transport Differences table, replace the INTERVAL and GEOMETRY rows with INTEGER (native `Int64`, WebSocket `Decimal128(18, 0)`) and SMALLINT (native `Int32`, WebSocket `Decimal128(9, 0)`)
- [ ] 3.2 In `Cargo.toml`, bump the version from 0.27.0 to 0.28.0
- [ ] 3.3 In `CHANGELOG.md`, add a `## 0.28.0` section above `## 0.27.0` with a Fix entry for #107, a Fix entry for `exasol:srid` on WebSocket and the native INTERVAL fraction, and one Breaking entry with its migration note (the Migration table) for each Breaking line of Impact

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Shared INTERVAL parser and native values | 1.1-1.5 | none | spec deltas `native-client/type-conversion`, `type-mapping/exasol-to-arrow` (native half), `type-mapping/exasol-to-arrow-dates`, `type-mapping/exasol-to-arrow-field-metadata` (native half); architecture delta (types, native transport); `src/types/conversion.rs`, `src/arrow_conversion/builders.rs`, `src/transport/native/result_parser.rs`, `src/transport/native/arrow_builder.rs`, `tests/native_protocol_tests.rs`, `tests/integration_tests.rs` |
| B: WebSocket values and the SRID | 2.1-2.5 | A (2.1 calls `parse_interval`; 2.4 compares with native values) | spec deltas `type-mapping/exasol-to-arrow` (WebSocket half and the equality scenario), `type-mapping/exasol-to-arrow-field-metadata` (WebSocket half); architecture delta (Data Flow, Interfaces); `src/query/results.rs`, `src/transport/messages.rs`, `src/arrow_conversion/converter.rs`, `tests/websocket_integration_tests.rs`, `tests/common/mod.rs` |
| C: Documentation and release | 3.1-3.3 | B | all four spec deltas; `docs/type-mapping.md`, `Cargo.toml`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/arrow_conversion/builders.rs`: `parse_interval_year_to_month`, `parse_interval_day_to_second`, `parse_interval_time_to_nanos`, `parse_interval_seconds_to_nanos` | Replaced by `types::conversion::parse_interval` in 1.1 |
| Code | `src/query/results.rs` l.557-558: the `_` arm of `json_column_to_array` that builds Utf8 text from `Value::to_string` | Cannot match a non-Utf8 field; replaced by an error in 2.1 |
| Test | `tests/integration_tests.rs`: `test_native_interval_field_keeps_utf8_and_carries_exasol_metadata` | Tests the removed scenario; replaced in 1.5 |
| Test | `src/transport/native/result_parser.rs`: the GEOMETRY, HASHTYPE, and INTERVAL entries of `single_pass_string_like_types_all_decode_as_utf8` | These types no longer decode as Utf8 (1.2) |
| Comment | `tests/websocket_integration_tests.rs` l.2477: the #107 note on `test_ws_hashtype_field_metadata_carries_the_declared_byte_size` | The WebSocket HASHTYPE row conversion works after 2.1 |
| Doc | `docs/type-mapping.md` l.112: the native Utf8 note | Native fields are Interval(MonthDayNano) and Binary after 1.2 |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| GEOMETRY type handling | Integration | `tests/native_protocol_tests.rs`, `tests/websocket_integration_tests.rs` | `test_native_geometry_and_hashtype_results_are_binary`, `test_ws_geometry_result_equals_native` |
| INTERVAL type mapping | Integration, Unit | `tests/native_protocol_tests.rs`, `tests/websocket_integration_tests.rs`, `src/types/conversion.rs` | `test_native_interval_results_are_month_day_nano`, `test_ws_interval_year_to_month_result_equals_native`, `test_ws_interval_day_to_second_result_equals_native`, the 1.1 unit tests |
| INTERVAL YEAR TO MONTH value outside the Arrow month range | Integration, Unit | `tests/native_protocol_tests.rs`, `tests/websocket_integration_tests.rs`, `src/types/conversion.rs`, `src/query/results.rs` | `test_native_interval_results_are_month_day_nano`, `test_ws_interval_year_to_month_result_equals_native`, the 1.1 and 2.1 unit tests |
| HASHTYPE type mapping | Integration | `tests/native_protocol_tests.rs`, `tests/websocket_integration_tests.rs` | `test_native_geometry_and_hashtype_results_are_binary`, `test_ws_hashtype_result_equals_native` |
| WebSocket INTERVAL, GEOMETRY, and HASHTYPE results equal native results | Integration | `tests/websocket_integration_tests.rs` | `test_ws_interval_year_to_month_result_equals_native`, `test_ws_interval_day_to_second_result_equals_native`, `test_ws_geometry_result_equals_native`, `test_ws_hashtype_result_equals_native` |
| Date and time types mapping | Integration | `tests/native_protocol_tests.rs`, `tests/websocket_integration_tests.rs`, `src/export/parquet.rs` | `test_native_interval_results_are_month_day_nano`, `test_ws_interval_year_to_month_result_equals_native`, the existing Parquet export tests (unchanged) |
| Direct binary to Arrow conversion for interval types | Integration, Unit | `tests/native_protocol_tests.rs`, `src/transport/native/result_parser.rs` | `test_native_interval_results_are_month_day_nano`, the 1.2 unit tests |
| Direct binary to Arrow conversion for binary and geometry types | Integration, Unit | `tests/native_protocol_tests.rs`, `src/transport/native/result_parser.rs` | `test_native_geometry_and_hashtype_results_are_binary`, the 1.2 unit tests |
| HASHTYPE field metadata carries the byte size of the column | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_hashtype_field_metadata_carries_the_declared_byte_size` (unchanged), `test_ws_hashtype_field_metadata_carries_the_declared_byte_size` |
| INTERVAL DAY TO SECOND field metadata carries the fractional second precision | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_native_type_parameters_in_field_metadata`, `test_ws_type_parameters_in_field_metadata` |
| GEOMETRY field metadata carries the SRID that Exasol reports | Integration | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_native_type_parameters_in_field_metadata`, `test_ws_type_parameters_in_field_metadata` |

Each test carries one `/// Scenario: <title>` line per scenario it implements, with the title quoted verbatim, per `AGENTS.md`. The native halves of the four `test_ws_*_result_equals_native` tests sit in `#[cfg(feature = "native")]` blocks, because `tests/websocket_integration_tests.rs` also builds without the `native` feature. The 1.1, 1.2, and 2.1 unit tests cover pure conversion of Exasol text with no I/O. Test commands and the Exasol test environment are in `specs/mission.md` and `AGENTS.md` § Testing.

### Manual Testing

Start Exasol per `AGENTS.md` § Testing, then build the driver with `cargo build --release --features 'ffi websocket'`. The commands use Python with `adbc-driver-manager` and `pyarrow`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Exasol to Arrow, Native Result Set Type Conversion, Exasol to Arrow Field Metadata (native) | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri':'exasol://sys:exasol@localhost:8563?validateservercertificate=0'}); cur=c.cursor(); cur.execute(\"SELECT CAST('1-2' AS INTERVAL YEAR(2) TO MONTH) I, CAST('-3 04:05:06.789' AS INTERVAL DAY(2) TO SECOND(6)) J, CAST('POINT(1 2)' AS GEOMETRY(4326)) G, CAST('550e8400-e29b-11d4-a716-446655440000' AS HASHTYPE(16 BYTE)) H\"); t=cur.fetch_arrow_table(); print(t.schema); print(t.to_pylist())"` | `I` and `J` are `month_day_nano_interval` fields and `G` and `H` are `binary` fields; `J` shows `"precision":6` in `exasol:type`, `G` shows no `exasol:srid`, and `H` shows `"byte_size":16`; the row holds months 14, days -3 with nanoseconds -14706789000000, `b'POINT (1 2)'`, and `b'550e8400e29b11d4a716446655440000'` |
| Exasol to Arrow, Exasol to Arrow Field Metadata (WebSocket) | The same command with `&transport=websocket` appended to the URI | No error; the schema shows the same Arrow types as the native run, `J` shows `"precision":6`, and `G` shows `exasol:srid: '4326'`; the row equals the native row |
| Exasol to Arrow, Native Result Set Type Conversion (month limit) | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri':'exasol://sys:exasol@localhost:8563?validateservercertificate=0'}); cur=c.cursor(); cur.execute(\"SELECT CAST('999999999-11' AS INTERVAL YEAR(9) TO MONTH) BIG\"); print(cur.fetch_arrow_table())"`, then the same command with `&transport=websocket` appended to the URI | Both runs raise an error whose message names the column `BIG` |
| Exasol to Arrow DATE and TIMESTAMP | The native and the WebSocket run of the first command | `I` and `J` are `month_day_nano_interval` on both transports |

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
