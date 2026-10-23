# Plan: fix-native-timestamp-ltz-type

## Summary

The native login sets the session attribute `ATTR_TSUTC_ENABLED` (51) to true, so that a `TIMESTAMP WITH LOCAL TIME ZONE` column has the same Exasol type, the Arrow type `Timestamp(Microsecond, "UTC")`, and the same `exasol:` field metadata on the native and the WebSocket transport. Native values become UTC instants, which equal the WebSocket values only in a `UTC` session, and the plan documents that difference; bound parameters of this type keep the wire type `T_TIMESTAMP`, so Exasol still reads them in the session time zone.

## Context

- On the native transport, a `TIMESTAMP WITH LOCAL TIME ZONE` column reports `exasol:type_name` `TIMESTAMP` and has the Arrow type `Timestamp(Microsecond, None)`. Over WebSocket, the same column reports `TIMESTAMP WITH LOCAL TIME ZONE` and has the Arrow type `Timestamp(Microsecond, "UTC")`. The spec `type-mapping/exasol-to-arrow-field-metadata` and `docs/type-mapping.md:22,162` record this difference.
- A finding about native versus WebSocket behavior: Exasol sends such a column with the wire type `T_TIMESTAMP` (21) unless the login sets `ATTR_TSUTC_ENABLED` (51) to true.
- `build_login_packet` (`src/transport/native/handshake.rs:32`) does not send this attribute. `attribute_type` (`src/transport/native/attributes.rs:227`) already encodes attribute 51 as a 1-byte boolean.
- The native decoder names the wire types `T_TIMESTAMP_LOCAL_TZ` (124) and `T_TIMESTAMP_UTC` (125) `TIMESTAMP WITH LOCAL TIME ZONE` (`src/transport/native/arrow_builder.rs:74-92`). `fill_timestamp_column` (`src/transport/native/result_parser.rs:866`) gives only 125 the Arrow time zone `UTC`.
- The WebSocket path parses the text that Exasol sends in the session time zone and labels the result `UTC` (`src/query/results.rs:554-565`). Exasol stores these values normalized to UTC and shows them in the session time zone.
- `data_type_to_wire_type` (`src/transport/native/mod.rs:615`) encodes a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as `T_TIMESTAMP_UTC` (125).
- Measured on Exasol 2026.1.0 (`exasol/docker-db:latest`), whose default session time zone is `EUROPE/BERLIN`, with the attribute in the login packet: Exasol accepts the attribute there and sends a `TIMESTAMP WITH LOCAL TIME ZONE` result column with the wire type 125. It never sends 124.
- Measured on the same server, for `CAST(TIMESTAMP '2024-01-02 03:04:05.123456' AS TIMESTAMP(6) WITH LOCAL TIME ZONE)` with the attribute: in a `UTC` session, the native and the WebSocket value are both `1_704_164_645_123_456` µs. In a `EUROPE/BERLIN` session, the native value is the UTC instant `1_704_161_045_123_456` µs, and the WebSocket value stays `1_704_164_645_123_456` µs. Without the attribute, both transports return `1_704_164_645_123_456` µs in both sessions.
- Measured on the same server with the attribute: the prepare reply reports a `TIMESTAMP WITH LOCAL TIME ZONE` parameter with the wire type 125. The bound text `2024-01-01 08:00:00` sent as 125 reads back as `2024-01-01 09:00:00` in a `EUROPE/BERLIN` session. Sent as `T_TIMESTAMP` (21), it reads back as `2024-01-01 08:00:00` in a `UTC` and in a `EUROPE/BERLIN` session.
- CI runs the integration tests against `exasol/docker-db:2025.2.1` (`.github/workflows/ci.yml:248`). Exasol 2026.1.0 is the oldest release known to accept the attribute in the login packet. Whether 2025.2.1 or older releases, such as the 7.x and 8.x deployments that `docs/import-export.md` names, accept it is unverified. Task 4.5 checks 2025.2.1.
- The architecture delta (`architecture.md` in this plan directory) changes § Data Flow.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Native TCP Handshake | CHANGED | `native-client/handshake/spec.md` |
| Native Result Set Type Conversion | CHANGED | `native-client/type-conversion/spec.md` |
| Native Prepared Statement Protocol | CHANGED | `native-client/prepared-statement-protocol/spec.md` |
| Exasol to Arrow Field Metadata | CHANGED | `type-mapping/exasol-to-arrow-field-metadata/spec.md` |
| Exasol to Arrow DATE and TIMESTAMP | CHANGED | `type-mapping/exasol-to-arrow-dates/spec.md` |
| Arrow to Exasol | CHANGED | `type-mapping/arrow-to-exasol/spec.md` |
| Parameter Binding | CHANGED | `prepared-statements/parameter-binding/spec.md` |

## Impact

- Breaking: on the native transport, a `TIMESTAMP WITH LOCAL TIME ZONE` column reports `exasol:type_name` `TIMESTAMP WITH LOCAL TIME ZONE` and has the Arrow type `Timestamp(Microsecond, "UTC")`, and `TypeMapper::from_field_metadata` returns `with_local_time_zone: true`. Before, the column reported `TIMESTAMP` and had the Arrow type `Timestamp(Microsecond, None)`. A caller that matches the Arrow type of such a column must accept the time zone `UTC`.
- Breaking: on the native transport, the values of such a column are UTC instants. In a `UTC` session, they equal the earlier native values and the WebSocket values. In another session time zone, they differ from both by the session's UTC offset. For example, in a `EUROPE/BERLIN` session, `2024-01-02 03:04:05.123456` is `1_704_161_045_123_456` µs over native and `1_704_164_645_123_456` µs over WebSocket. The default session time zone of the Exasol Docker image is `EUROPE/BERLIN`.
- Not unified: the WebSocket transport keeps the date and time that the session time zone shows, under the Arrow time zone `UTC` (decision [3]). The two transports return equal values only in a `UTC` session.
- Breaking: on the native transport, ADBC `get_parameter_schema` reports a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as `Timestamp(Microsecond, "UTC")`, as the WebSocket transport does. Before, it reported `Timestamp(Microsecond, None)`.
- ADBC driver manager users, such as Python or Polars, see such a native column as `timestamp[us, tz=UTC]`, as over WebSocket.
- Bound `TIMESTAMP WITH LOCAL TIME ZONE` parameters are stored as before on both transports: Exasol reads the bound date and time in the session time zone (decision [4]). On the native transport in a non-UTC session, an Arrow `UTC` timestamp bound to such a column therefore reads back shifted by the session's UTC offset.
- WebSocket results do not change. Parquet export keeps writing such a column as `Timestamp(Microsecond, None)`.
- Risk: a server that rejects the attribute in the login packet fails every native connect. Exasol 2026.1.0 accepts it, task 4.5 checks 2025.2.1, and older releases are untested.

## Implementation Tasks

1. Native login turns on the attribute
   - [ ] 1.1 In `src/transport/native/handshake.rs`, write the unit test `login_packet_turns_on_local_time_zone_timestamps`. It parses the attributes of `build_login_packet` and asserts that `ATTR_TSUTC_ENABLED` is `AttributeValue::Bool(true)`. It carries `/// Scenario: Login makes Exasol send TIMESTAMP WITH LOCAL TIME ZONE columns with their own wire type`. Raise the attribute count that `login_packet_identifies_the_user_and_the_driver` parses from 5 to 6.
   - [ ] 1.2 In `build_login_packet`, add `ATTR_TSUTC_ENABLED` with `AttributeValue::Bool(true)` (decision [1]). A one-line comment states why: without it, Exasol sends a `TIMESTAMP WITH LOCAL TIME ZONE` column with the wire type `T_TIMESTAMP`. Update the doc comment of `build_login_packet`, which lists what the packet sends.
2. Local time zone results decode to `Timestamp(Microsecond, "UTC")`, and local time zone parameters encode as `T_TIMESTAMP`
   - [ ] 2.1 In `src/transport/native/result_parser.rs`, replace the unit test `single_pass_timestamp_with_local_time_zone_has_no_arrow_timezone` with `single_pass_local_time_zone_timestamps_have_the_utc_arrow_timezone`. It asserts that the wire types 124 and 125 decode to `Timestamp(Microsecond, "UTC")`, that the wire type 21 decodes to `Timestamp(Microsecond, None)`, and that all three hold the same microsecond value. It carries `/// Scenario: Direct binary to Arrow conversion for temporal types`.
   - [ ] 2.2 In `fill_timestamp_column`, decode `T_TIMESTAMP_LOCAL_TZ` the same way as `T_TIMESTAMP_UTC` (decision [2]).
   - [ ] 2.3 In `src/transport/native/mod.rs`, first change the unit tests: `declared_parameter_types_map_to_their_wire_types` expects `T_TIMESTAMP` for `TIMESTAMP WITH LOCAL TIME ZONE` and gains `/// Scenario: Outbound precision on a TIMESTAMP parameter column header`. Rename `prepared_payload_writes_timestamp_precision_for_each_timestamp_wire_type` to `prepared_payload_writes_timestamp_precision_for_both_timestamp_types`, expect the type ID `T_TIMESTAMP` for both type names, and delete its direct `write_column_metadata` call with `T_TIMESTAMP_LOCAL_TZ`. Then change `data_type_to_wire_type` to map `TIMESTAMP WITH LOCAL TIME ZONE` to `T_TIMESTAMP` (decision [4]), with a one-line comment: Exasol reads a `T_TIMESTAMP_UTC` parameter as UTC, not in the session time zone. Delete the `T_TIMESTAMP_LOCAL_TZ` and `T_TIMESTAMP_UTC` arms of `write_column_metadata` and `write_param_value`, which no parameter reaches after this change.
   - [ ] 2.4 Rewrite the doc comment of `ResultSet::payload_to_record_batch` (`src/query/results.rs:405-413`). Its example, a 124 column that decodes to `Timestamp(Microsecond, None)`, is no longer true. Keep the rule that the field metadata goes onto the batch's own fields.
3. Existing tests that expect `TIMESTAMP` for a native local time zone column
   - [ ] 3.1 In `tests/common/mod.rs`, delete `default_transport_local_time_zone_timestamp`. Remove the `local_time_zone_timestamp` parameter of `assert_column_metadata_table_fields`, so that it always expects `TSL` with the type name `TIMESTAMP WITH LOCAL TIME ZONE` and `ExasolType::Timestamp { with_local_time_zone: true }`. Delete the doc comment text about the wire type 21.
   - [ ] 3.2 Update the callers. In `tests/integration_tests.rs`, these are the import list (line 83), `test_result_set_fields_carry_exasol_column_metadata`, and the zero-row test near line 4203. In `tests/websocket_integration_tests.rs`, delete `WS_TSL_TYPE` and update `test_ws_result_set_fields_carry_exasol_column_metadata` and the zero-row test near line 2371. Add `/// Scenario: Login makes Exasol send TIMESTAMP WITH LOCAL TIME ZONE columns with their own wire type` to `test_result_set_fields_carry_exasol_column_metadata`, which runs over the native transport.
4. Integration tests against Exasol
   - [ ] 4.1 Start the Exasol Docker container as AGENTS.md § Testing describes, and run `cargo build --release --features 'ffi websocket'`.
   - [ ] 4.2 In `tests/websocket_integration_tests.rs`, write `test_ws_local_time_zone_timestamp_result_matches_native`. It carries `/// Scenario: TIMESTAMP WITH LOCAL TIME ZONE results have the same type on both transports`, `/// Scenario: TIMESTAMP WITH LOCAL TIME ZONE values differ between the transports in a session outside UTC`, and `/// Scenario: Direct binary to Arrow conversion for temporal types`. For each session time zone, `UTC` and `EUROPE/BERLIN`, the test runs `ALTER SESSION SET TIME_ZONE` on a WebSocket connection and, under `#[cfg(feature = "native")]` as `test_ws_integer_result_equals_native` does, on a native connection. On each connection, it runs `assert_two_row_result` for a `TwoRowCase` of the literal with the expected type `Timestamp(Microsecond, "UTC")`, which checks the metadata schema, the batch, the zero-row batch, and the NULL second row. With `metadata_schema_and_batch`, it asserts that the field `V` of the metadata schema and of the batch carries `exasol:type_name` `TIMESTAMP WITH LOCAL TIME ZONE` on each transport. It asserts the first-row values: `1_704_164_645_123_456` on both transports in the `UTC` session; in the `EUROPE/BERLIN` session, `1_704_164_645_123_456` over WebSocket and `1_704_161_045_123_456` over native. `assert_equals_native` compares values in every session, so a new helper next to it takes the session time zone and the expected native value.
   - [ ] 4.3 Write `test_local_time_zone_timestamp_text_parameter_reads_in_session_time_zone` in `tests/integration_tests.rs` and `test_ws_local_time_zone_timestamp_text_parameter_reads_in_session_time_zone` in `tests/websocket_integration_tests.rs`. Each test carries `/// Scenario: TIMESTAMP WITH LOCAL TIME ZONE text parameters are stored the same on both transports`, and the native test also carries `/// Scenario: Outbound precision on a TIMESTAMP parameter column header`. Follow `test_prepared_temporal_text_parameters` (`conn.prepare`, `prepared.bind`, `conn.execute_prepared_update`). Run the insert and the `TO_CHAR` read-back once with the session time zone `UTC` and once with `EUROPE/BERLIN`, each into an empty table, and assert `2024-01-01 08:00:00` both times.
   - [ ] 4.4 In `tests/driver_manager_tests.rs`, change `assert_bound_temporal_values_are_stored` for the changed scenario "Bound Arrow Timestamp and Date32 values are stored on both transports". It sets the session time zone to `EUROPE/BERLIN` after it connects, so that the result does not depend on the server's default time zone. It takes the expected `TS_LOCAL` values as an argument: `test_bind_arrow_temporal_values_round_trip` (native) passes `[Some(-3_600_999_999), Some(1_704_092_400_000_000), None]`, and `test_bind_arrow_temporal_values_round_trip_websocket` passes the current `[Some(-999_999), Some(1_704_096_000_000_000), None]`. The `TO_CHAR` assertion stays `2024-01-01 08:00:00 2024-01-01 08:00:00`. Rewrite the doc comment of the native test: Exasol reads the bound value in the session time zone, and the native read returns the UTC instant. The native values follow from the measured one-hour offset of `EUROPE/BERLIN` and are not yet measured.
   - [ ] 4.5 Rebuild the release cdylib, then run the tests of tasks 3, 4.2, 4.3, and 4.4 and the regression tests in § Verification against the `latest` image (Exasol 2026.1.0). Then stop and remove that container, start `exasol/docker-db:2025.2.1`, the image that CI runs, with the same `docker run` arguments, and run the same tests again. Exasol 2026.1.0 accepts the attribute in the login packet, so the phase-2 fallback applies only to 2025.2.1. If 2025.2.1 rejects the login or the native `TSL` field still reports `TIMESTAMP` there, move the attribute to the phase-2 `CMD_SET_ATTRIBUTES` message (decision [1]), rebuild, and run the tests on both images again. If a test still fails on 2025.2.1, stop and report to the user the server release, the wire type that Exasol sent, the native and WebSocket values, and the failing assertion. [expert]
5. Documentation, changelog, and version
   - [ ] 5.1 In `docs/type-mapping.md`, map `TIMESTAMP WITH LOCAL TIME ZONE` to `Timestamp(Microsecond, "UTC")` in the Exasol to Arrow table, with a note that links to the section "WebSocket Transport Differences". Delete the field metadata bullet that says the native transport reports `TIMESTAMP`. Rewrite that section: both transports map the type to `Timestamp(Microsecond, "UTC")`; the native value is the UTC instant, and the WebSocket value is the date and time that the session time zone shows, under the time zone `UTC`; the values are equal only in a `UTC` session; give the `EUROPE/BERLIN` example from § Impact. Add that both transports store a bound value of this type in the session time zone, so a native Arrow `UTC` timestamp bound in a non-UTC session reads back shifted by the session's UTC offset. Rewrite line 7 to state that the type mappings apply to both transports, and that the linked section lists the value differences.
   - [ ] 5.2 In `Cargo.toml`, set the version to 0.30.0. This is a minor bump because the Arrow type of a native result field changes.
   - [ ] 5.3 In `CHANGELOG.md`, add `## 0.30.0` with one `Breaking:` entry. It states the native type name, Arrow type, and `with_local_time_zone` change from § Impact, the native `get_parameter_schema` change, and that native values are now UTC instants. It states plainly that native and WebSocket values of this type differ in a non-UTC session: native returns the UTC instant, and WebSocket returns the session date and time under the time zone `UTC`.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Native local time zone column | 1.1-4.5 | none | spec deltas `native-client/handshake`, `native-client/type-conversion`, `native-client/prepared-statement-protocol`, `type-mapping/exasol-to-arrow-field-metadata`, `type-mapping/exasol-to-arrow-dates`, `type-mapping/arrow-to-exasol`, `prepared-statements/parameter-binding`; `src/transport/native/handshake.rs`, `src/transport/native/result_parser.rs`, `src/transport/native/mod.rs` (`data_type_to_wire_type`, `write_column_metadata`, `write_param_value`, and `login_exchange` only for the fallback of decision [1]), `src/query/results.rs`, `tests/common/mod.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `tests/driver_manager_tests.rs` |
| B: Documentation and release | 5.1-5.3 | A | § Impact of this plan, decisions [3] and [4]; `docs/type-mapping.md`, `Cargo.toml`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `tests/common/mod.rs` `default_transport_local_time_zone_timestamp` | Both transports report the same type for `TSL` |
| Constant | `tests/websocket_integration_tests.rs` `WS_TSL_TYPE` | Same reason |
| Parameter | `tests/common/mod.rs` `assert_column_metadata_table_fields(…, local_time_zone_timestamp)` | Same reason |
| Test | `src/transport/native/result_parser.rs` `single_pass_timestamp_with_local_time_zone_has_no_arrow_timezone` | Replaced by `single_pass_local_time_zone_timestamps_have_the_utc_arrow_timezone` |
| Match arms | `src/transport/native/mod.rs` `write_column_metadata`, `write_param_value`: `T_TIMESTAMP_LOCAL_TZ`, `T_TIMESTAMP_UTC` | The parameter encoder sends every timestamp parameter as `T_TIMESTAMP` |

## Follow-up Plans

- The WebSocket transport returns a `TIMESTAMP WITH LOCAL TIME ZONE` value as the date and time that the session time zone shows, under the Arrow time zone `UTC`. Arrow consumers read that value as an instant, which is off by the session's UTC offset. A follow-up plan decides whether the WebSocket transport converts these values to UTC instants (decision [3], option (b)), which would make both transports return equal values in every session.
- On the native transport in a non-UTC session, an Arrow `UTC` timestamp bound to a `TIMESTAMP WITH LOCAL TIME ZONE` column reads back shifted by the session's UTC offset (decision [4]). The same follow-up plan decides whether bound values with a time zone are stored as UTC instants.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Login makes Exasol send TIMESTAMP WITH LOCAL TIME ZONE columns with their own wire type | Unit | `src/transport/native/handshake.rs` | `login_packet_turns_on_local_time_zone_timestamps` |
| Login makes Exasol send TIMESTAMP WITH LOCAL TIME ZONE columns with their own wire type | Integration | `tests/integration_tests.rs` | `test_result_set_fields_carry_exasol_column_metadata` |
| Direct binary to Arrow conversion for temporal types | Unit | `src/transport/native/result_parser.rs` | `single_pass_local_time_zone_timestamps_have_the_utc_arrow_timezone` |
| Direct binary to Arrow conversion for temporal types | Integration | `tests/websocket_integration_tests.rs` | `test_ws_local_time_zone_timestamp_result_matches_native` |
| Outbound precision on a TIMESTAMP parameter column header | Unit | `src/transport/native/mod.rs` | `declared_parameter_types_map_to_their_wire_types` (changed), `prepared_payload_writes_timestamp_precision_for_both_timestamp_types` (renamed), `prepared_payload_ranges_repeat_the_timestamp_precision` (existing) |
| Outbound precision on a TIMESTAMP parameter column header | Integration | `tests/integration_tests.rs` | `test_local_time_zone_timestamp_text_parameter_reads_in_session_time_zone` |
| Column metadata | Integration | `tests/integration_tests.rs` | `test_result_set_fields_carry_exasol_column_metadata` |
| Column metadata | Integration | `tests/websocket_integration_tests.rs` | `test_ws_result_set_fields_carry_exasol_column_metadata` |
| Schema documentation | Integration | `tests/integration_tests.rs` | `test_result_set_fields_carry_exasol_column_metadata` |
| Schema documentation | Integration | `tests/websocket_integration_tests.rs` | `test_ws_result_set_fields_carry_exasol_column_metadata` |
| TIMESTAMP WITH LOCAL TIME ZONE results have the same type on both transports | Integration | `tests/websocket_integration_tests.rs` | `test_ws_local_time_zone_timestamp_result_matches_native` |
| TIMESTAMP WITH LOCAL TIME ZONE values differ between the transports in a session outside UTC | Integration | `tests/websocket_integration_tests.rs` | `test_ws_local_time_zone_timestamp_result_matches_native` |
| TIMESTAMP WITH LOCAL TIME ZONE text parameters are stored the same on both transports | Integration | `tests/integration_tests.rs` | `test_local_time_zone_timestamp_text_parameter_reads_in_session_time_zone` |
| TIMESTAMP WITH LOCAL TIME ZONE text parameters are stored the same on both transports | Integration | `tests/websocket_integration_tests.rs` | `test_ws_local_time_zone_timestamp_text_parameter_reads_in_session_time_zone` |
| Bound Arrow Timestamp and Date32 values are stored on both transports | Integration | `tests/driver_manager_tests.rs` | `test_bind_arrow_temporal_values_round_trip`, `test_bind_arrow_temporal_values_round_trip_websocket` (changed) |

Regression tests that must pass unchanged, because the attribute can change how Exasol reads a bound value or reports column types:

| Scenario | Test Location | Test Name |
|----------|---------------|-----------|
| DATE and TIMESTAMP text parameters are stored or rejected on both transports | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_prepared_temporal_text_parameters`, `test_ws_prepared_temporal_text_parameters` |
| Export preserves schema (session time zone `EUROPE/BERLIN`, column `TS_LTZ`) | `tests/import_export_tests.rs` | `test_parquet_export_preserves_schema` |

### Manual Testing

Prerequisites: the Exasol Docker container from AGENTS.md § Testing, `cargo build --release --features 'ffi websocket'`, and the Python packages `adbc-driver-manager` and `pyarrow`. `URI` stands for `exasol://sys:exasol@localhost:8563?validateservercertificate=0` for the native transport, and the same URI with `&transport=websocket` for the WebSocket transport.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Native TCP Handshake, Native Result Set Type Conversion, Exasol to Arrow Field Metadata, Exasol to Arrow DATE and TIMESTAMP | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri':'URI'}); u=c.cursor(); u.execute(\"ALTER SESSION SET TIME_ZONE = 'EUROPE/BERLIN'\"); u.execute(\"SELECT CAST(TIMESTAMP '2024-01-02 03:04:05.123456' AS TIMESTAMP(6) WITH LOCAL TIME ZONE) AS V\"); t=u.fetch_arrow_table(); f=t.schema.field('V'); print(f.type, f.metadata[b'exasol:type_name'], t.column('V')[0])"`, once with each URI, then again with `'UTC'` in place of `'EUROPE/BERLIN'` | In `EUROPE/BERLIN`, the native run prints `timestamp[us, tz=UTC] b'TIMESTAMP WITH LOCAL TIME ZONE' 2024-01-02 02:04:05.123456+00:00`, and the WebSocket run prints `timestamp[us, tz=UTC] b'TIMESTAMP WITH LOCAL TIME ZONE' 2024-01-02 03:04:05.123456+00:00`. In `UTC`, both runs print the WebSocket line |
| Parameter Binding, Native Prepared Statement Protocol | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri':'URI'}); u=c.cursor(); u.execute(\"ALTER SESSION SET TIME_ZONE = 'EUROPE/BERLIN'\"); u.execute('CREATE SCHEMA LTZ_MANUAL'); u.execute('CREATE TABLE LTZ_MANUAL.T (TSL TIMESTAMP(6) WITH LOCAL TIME ZONE)'); u.execute('INSERT INTO LTZ_MANUAL.T VALUES (?)', ('2024-01-01 08:00:00',)); u.execute(\"SELECT TO_CHAR(TSL, 'YYYY-MM-DD HH24:MI:SS') FROM LTZ_MANUAL.T\"); print(u.fetchall()); u.execute('DROP SCHEMA LTZ_MANUAL CASCADE')"`, once with each URI | Both runs print `[('2024-01-01 08:00:00',)]` |

### Checklist

Commands come from `specs/mission.md` § Commands and AGENTS.md.

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` and `cargo build --release --features 'ffi websocket'` | Exit 0 |
| Unit tests | `cargo test --lib`, `cargo test --lib --features websocket`, `cargo test --lib --features ffi` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| WebSocket integration tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Native protocol tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1` | 0 failures |
| Import and export tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json` and `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Production coverage at least 80 percent, every file at least 50 percent |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
