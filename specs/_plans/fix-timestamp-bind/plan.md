# Plan: fix-timestamp-bind

## Summary

Make a prepared statement with a bound TIMESTAMP parameter work over the native protocol by writing the fractional-seconds precision field that Exasol expects in a TIMESTAMP parameter column header, and make the native encoder reject DATE and TIMESTAMP text it cannot read instead of storing a substitute value. Integration tests pin the behavior on both transports, and the fix for #38 ships as 0.20.2.

## Context

- Reproduced on `main` (b7897d1, 0.20.1) against `exasol/docker-db:latest` through the driver manager with `CREATE TABLE T (C TIMESTAMP)`, `INSERT INTO T VALUES (?)`, one bound `TimestampMicrosecondArray` `[1_704_096_000_000_000]`, and `execute_update`. Over the native transport the call fails with `Query execution failed: Failed to receive message: peer closed connection without sending TLS close_notify: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof`.
- The same native failure occurs for every Timestamp unit (Second, Millisecond, Microsecond, Nanosecond), with no time zone, `UTC`, or `Europe/Berlin`, for `TIMESTAMP`, `TIMESTAMP(0)`, `TIMESTAMP(3)`, `TIMESTAMP(6)`, `TIMESTAMP(9)`, and `TIMESTAMP WITH LOCAL TIME ZONE` columns, for a Utf8 value bound to a TIMESTAMP column, and through Python `adbc_driver_manager.dbapi` with a `datetime` parameter. A Date32 value bound to a DATE column succeeds.
- Over the WebSocket transport every bound Arrow Timestamp case above succeeds and reads back unchanged.
- Root cause: `parse_column_meta` (`src/transport/native/result_parser.rs`) reads a 4-byte precision after the type ID of a TIMESTAMP column description (protocol version 19 and later), but `write_column_metadata` (`src/transport/native/mod.rs`) writes no metadata for a TIMESTAMP parameter column. Exasol then reads the row data at the wrong offset and closes the connection.
- A temporary patch that writes a 4-byte precision for the three TIMESTAMP wire types made every native case above succeed. Precision 3 and precision 9 stored identical values in `TIMESTAMP(0)`, `TIMESTAMP(3)`, `TIMESTAMP(6)`, and `TIMESTAMP(9)` columns. Exasol reports precisions 0, 3, 6, and 9 in the parameter description, and describes a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as wire type 21.
- `arrow_value_to_parameter` (`src/adbc_ffi.rs`) formats every Timestamp unit as `YYYY-MM-DD HH:MM:SS.ffffff` in UTC, with a space separator, and floors nanoseconds to microseconds. It has used a space separator since v0.7.0, including 0.12.0, the version in #38. The WebSocket transport reproduces the #38 error text only for a caller-supplied Utf8 value: `[22018] invalid character value for cast, SPACE expected; Value: '2024-01-01T08:00:00' Format: 'YYYY-MM-DD HH24:MI:SS.FF6'`.
- With the precision field in place, the native encoder's text parsing becomes reachable for TIMESTAMP values. It stores `2000-01-01 00:00:00` for the text `garbage` and `2024-01-01 00:00:00` for `2024-1-1 8:0:0`, while the WebSocket transport rejects `garbage` and stores `2024-01-01 08:00:00` for `2024-1-1 8:0:0`. On `main` the native encoder already stores `2000-01-01` for the DATE text `2024/01/02`.
- Over the WebSocket transport, Exasol's text cast gave these results for prepared inserts into `TIMESTAMP(6)` and `DATE` columns on `exasol-test` (`exasol/docker-db:latest`, session time zone `EUROPE/BERLIN`):
  - Empty text stores NULL. Text of only spaces fails with SQL state 22018.
  - Leading and trailing spaces are ignored, and so are extra spaces between the date and the time. A tab fails with SQL state 22018.
  - `2024-01-01 08` and `2024-01-01 08:00:00.` both store `2024-01-01 08:00:00`.
  - A DATE column stores only the date of date-and-time text such as `2024-01-02 08:30`. It rejects an hour above 23 and a minute or second above 60 with SQL state 22018, and it accepts minute 60 and second 60.
  - The cast also accepts forms outside the native grammar of `decision-log.md` entry [3], such as the trailing colon in `2024-01-01 08:00:`.
- Exasol validates the field values of a binary DATE or TIMESTAMP: month 13 and February 30 fail with SQL state 22008, hour 25 fails with SQL state 22009, and the connection stays open.
- CI builds the driver manager cdylib with `--features ffi` only, so `tests/driver_manager_tests.rs` reaches only the native transport. `tests/integration_tests.rs` runs under `cargo llvm-cov` with the `ffi` feature, where the FFI runtime deadlocks, and `tests/websocket_integration_tests.rs` runs with `--features 'ffi websocket'` outside coverage.
- The plan changes no architecture. See `decision-log.md` entry [9].

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Native Prepared Statement Protocol | CHANGED | `native-client/prepared-statement-protocol/spec.md` |
| Parameter Binding | CHANGED | `prepared-statements/parameter-binding/spec.md` |
| Arrow to Exasol | CHANGED | `type-mapping/arrow-to-exasol/spec.md` |

## Impact

- Over the native transport, a prepared statement with a bound TIMESTAMP parameter no longer breaks the connection. This covers ADBC `execute_update` and `execute` with an Arrow Timestamp column, Python `cursor.execute` with a `datetime` parameter, and `Connection::execute_prepared_update` or `execute_batch_update` with timestamp text.
- Changed behavior over the native transport, for DATE and TIMESTAMP parameters:
  - Text outside the accepted forms, and a value that is not text, fail with an error before execution. The accepted forms are `Y-M-D`, optionally followed by one or more spaces and `h`, `h:m`, `h:m:s`, or `h:m:s.f`, with leading and trailing spaces ignored. Before, a DATE parameter stored `2000-01-01` or a partly wrong date in place of text it could not read.
  - Empty text stores NULL, and a DATE parameter stores only the date of date-and-time text, as over the WebSocket transport.
  - `2024-01-01T08:00:00`, `2024/01/02`, `garbage`, and text of only spaces fail on both transports.
  - Divergence: the accepted forms are a subset of the text that Exasol's cast accepts over the WebSocket transport. Some forms, such as the trailing colon in `2024-01-01 08:00:`, fail only over the native transport.
- No change to the WebSocket transport, the FFI Arrow-to-parameter conversion, time zone handling, or the batch splitting of #90.
- No breaking API change. Release 0.20.2.

## Dependencies

- Draft PR #91 (plan fix-session-schema) touches `src/adbc/connection.rs` and session state. This plan changes neither. If #91 merges first with its own version bump, this plan's version and changelog header move to the next patch version.

## Implementation Tasks

1. Temporal parameter binding (group A)
   - [ ] 1.1 Add failing unit tests in `src/transport/native/mod.rs` for the TIMESTAMP header: a `build_execute_prepared_payload` payload for a parameter described as `TIMESTAMP` (wire type 21) or `TIMESTAMP WITH LOCAL TIME ZONE` (wire type 125) carries the type ID followed by the literal bytes of precision 9 and an 11-byte value; `write_column_metadata` writes the same precision for wire type 124, which no parameter description maps to today; and `split_parameter_rows` plus `build_range_payload` repeat that header in every range.
   - [ ] 1.2 Add failing unit tests in `src/transport/native/mod.rs` for temporal values:
     - The accepted forms encode field by field: `2024-03-09 14:25:36.123456`, `2024-1-1 8:0:0`, `2024-03-09` as midnight, `2024-03-09 14` as 14:00:00, `2024-03-09 14:25`, `2024-03-09 14:25:36.` with 0 fraction digits, 1 and 9 fraction digits, `2024-03-09 14:25` with one leading and one trailing space, `2024-03-09   14:25` with three spaces before the time, and `1500-02-29` kept as written.
     - Empty text in a `T_DATE` column and in a timestamp column is written as the null marker 0 with no value bytes.
     - A `T_DATE` value with a time part encodes only its date: `2024-01-02 08:30`, `2024-01-02 23:59:59`, and `2024-01-02 08:60:60` each encode 2024-01-02.
     - Out-of-range fields are sent as written: `2024-13-01`, `2024-02-30`, and `2024-01-01 25:00:00` in a timestamp column.
     - The rejected forms return an error whose message contains the value and the accepted form: `2024-01-01T08:00:00`, `2024/01/02`, `garbage`, text of only spaces, a tab before the date, the trailing colon in `2024-01-01 08:00:`, 10 fraction digits, a 5-digit year, `2024-01-02 24:00`, `2024-01-02 08:61`, and `2024-01-02 08:30:61` in a `T_DATE` column, a JSON number, and a JSON boolean.
     - `build_execute_prepared_payload` and `split_parameter_rows` fail for a parameter set that contains a rejected value.
   - [ ] 1.3 In `write_column_metadata`, write a 4-byte little-endian precision of 9 for `T_TIMESTAMP`, `T_TIMESTAMP_LOCAL_TZ`, and `T_TIMESTAMP_UTC`, taken from a named constant in `src/transport/native/constants.rs` whose comment states that the encoder sends a nanosecond field.
   - [ ] 1.4 Replace the bodies of `parse_date_to_packed` and `write_timestamp_bytes` with fallible field-by-field parsers for the grammar of the scenario "DATE and TIMESTAMP parameter text is encoded field by field". Both share one parser for the date and the optional time part, do no calendar conversion, and check ranges only for the time part of a `T_DATE` value. `write_param_value` writes the null marker 0 for empty text in a `T_DATE` or timestamp column, as it does for a JSON null, and returns `TransportError::SerializationError` for rejected text and for non-text `T_DATE` or timestamp values. Delete the unit tests `packed_dates_fall_back_field_by_field` and `unparsable_timestamp_fields_fall_back_to_their_defaults`, and adapt `timestamps_encode_their_fractional_seconds_as_nanoseconds` and `timestamps_without_a_time_part_default_to_midnight` to the fallible signature.
   - [ ] 1.5 Add a fixture to `tests/common/mod.rs` that builds the RecordBatch of the scenario "Bound Arrow Timestamp and Date32 values are stored on both transports", the matching `CREATE TABLE` column list, and the expected microsecond and day values per column, so that the native and WebSocket tests share one definition. Nanosecond values are whole microseconds (see `decision-log.md` entry [5]).
   - [ ] 1.6 Add `test_prepared_temporal_text_parameters` to `tests/integration_tests.rs` with the cases of the scenario "DATE and TIMESTAMP text parameters are stored or rejected on both transports", emptying the table before each case: the valid text round trip; `2024-01-01 08:00:00` with one leading and one trailing space bound to `TS` and `2024-01-02 08:30` bound to `D`, stored as `2024-01-01 08:00:00` and `2024-01-02`; empty text bound to `TS` and `D`, stored as NULL; then each rejected value fails, stores no row, and a later `SELECT` on the same connection succeeds.
   - [ ] 1.7 Add `test_ws_prepared_temporal_text_parameters` to `tests/websocket_integration_tests.rs` with the same cases and steps over `transport=websocket`.
   - [ ] 1.8 Add `test_bind_arrow_temporal_values_round_trip` to `tests/driver_manager_tests.rs`: bind the fixture batch through the driver manager, call `execute_update`, compare every column read back in the same session with the fixture's expected values, then run a later statement. Assert microsecond values only, not the time zone of the read-back Arrow type. The test's doc comment states that it compares date and time values, not instants, because Exasol reads a value bound to the `TIMESTAMP(6) WITH LOCAL TIME ZONE` column in the session time zone.
   - [ ] 1.9 Add `test_ws_ffi_bind_arrow_temporal_values_round_trip` to `tests/websocket_integration_tests.rs` under `#[cfg(feature = "ffi")]` as a plain `#[test]`: the same steps as 1.8 through an in-process `exarrow_rs::FfiDriver` with `transport=websocket` in the URI.
2. Documentation and release (group B)
   - [ ] 2.1 In `docs/prepared-statements.md`, replace the `chrono::NaiveDate` and `chrono::NaiveDateTime` rows of "Supported Parameter Types", which name `From` conversions that do not exist, with rows for DATE and TIMESTAMP text in the accepted forms. In `docs/type-mapping.md`, add a parameter binding note: an Arrow Timestamp of any unit binds as its UTC date and time at microsecond precision with nanoseconds rounded down, and Date32 binds as DATE.
   - [ ] 2.2 Set `version = "0.20.2"` in `Cargo.toml` and run `cargo build` so that `Cargo.lock` records it.
   - [ ] 2.3 Add a `## 0.20.2` section to `CHANGELOG.md` above `## 0.20.1`, with a Fix entry for the native TIMESTAMP parameter failure that ends with `Fixes #38.` and a Changed entry for DATE and TIMESTAMP parameter text over the native transport: text outside the accepted forms fails before execution, and empty text stores NULL.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: temporal parameter binding | 1.1-1.9 | none | spec deltas `native-client/prepared-statement-protocol`, `prepared-statements/parameter-binding`, `type-mapping/arrow-to-exasol`; `src/transport/native/mod.rs`, `src/transport/native/constants.rs`, `src/transport/native/result_parser.rs` (read only), `src/adbc_ffi.rs` (read only), `tests/common/mod.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `tests/driver_manager_tests.rs` |
| B: documentation and release | 2.1-2.3 | A (documents A's accepted forms) | `docs/prepared-statements.md`, `docs/type-mapping.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `CONTRIBUTING.md` (Releasing) |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Test | `src/transport/native/mod.rs` `packed_dates_fall_back_field_by_field` | Pins the default-value fallback that task 1.4 removes |
| Test | `src/transport/native/mod.rs` `unparsable_timestamp_fields_fall_back_to_their_defaults` | Pins the default-value fallback that task 1.4 removes |
| Function body | `src/transport/native/mod.rs` `parse_date_to_packed`, `write_timestamp_bytes` | Lenient fixed-offset parsing replaced by fallible field-by-field parsing |
| Docs rows | `docs/prepared-statements.md` "Supported Parameter Types" | `chrono::NaiveDate` and `chrono::NaiveDateTime` have no `From` conversion into `Parameter` |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Outbound precision on a TIMESTAMP parameter column header | Unit (header and value bytes for wire types 21, 124, and 125) | `src/transport/native/mod.rs` | `prepared_payload_writes_timestamp_precision_for_each_timestamp_wire_type`, `prepared_payload_ranges_repeat_the_timestamp_precision` |
| Outbound precision on a TIMESTAMP parameter column header | Integration (Exasol accepts the data and keeps the connection open, wire type 21 only) | `tests/integration_tests.rs` | `test_prepared_temporal_text_parameters` |
| DATE and TIMESTAMP parameter text is encoded field by field | Unit | `src/transport/native/mod.rs` | `temporal_text_in_the_accepted_forms_encodes_field_by_field`, `out_of_range_temporal_fields_are_sent_as_written` |
| DATE and TIMESTAMP parameter text is encoded field by field | Integration | `tests/integration_tests.rs` | `test_prepared_temporal_text_parameters` |
| DATE and TIMESTAMP parameter values in another form are rejected before execution | Unit | `src/transport/native/mod.rs` | `temporal_values_in_other_forms_are_rejected`, `prepared_payload_with_a_rejected_temporal_value_fails` |
| DATE and TIMESTAMP parameter values in another form are rejected before execution | Integration | `tests/integration_tests.rs` | `test_prepared_temporal_text_parameters` |
| DATE and TIMESTAMP text parameters are stored or rejected on both transports | Integration (native) | `tests/integration_tests.rs` | `test_prepared_temporal_text_parameters` |
| DATE and TIMESTAMP text parameters are stored or rejected on both transports | Integration (WebSocket) | `tests/websocket_integration_tests.rs` | `test_ws_prepared_temporal_text_parameters` |
| Bound Arrow Timestamp and Date32 values are stored on both transports | Integration (native) | `tests/driver_manager_tests.rs` | `test_bind_arrow_temporal_values_round_trip` |
| Bound Arrow Timestamp and Date32 values are stored on both transports | Integration (WebSocket) | `tests/websocket_integration_tests.rs` | `test_ws_ffi_bind_arrow_temporal_values_round_trip` |

- Each test that implements a scenario carries one `/// Scenario: <title>` line per scenario, quoting the title verbatim.
- The native integration tests fail on `main`. The WebSocket integration tests pass on `main` and pin the WebSocket results for the same cases.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Arrow to Exasol | `cargo build --release --features ffi`, then the Python script below | `ROWS [(datetime.datetime(2024, 1, 1, 8, 0, 0, 123456),)]` and `cleaned`. On `main` it prints `INSERT failed: INTERNAL: Query execution failed: Failed to receive message: peer closed connection without sending TLS close_notify: ...` |
| Native Prepared Statement Protocol | `REQUIRE_EXASOL=1 cargo test --test integration_tests test_prepared_temporal_text_parameters -- --nocapture` | `test result: ok. 1 passed` |
| Parameter Binding | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests temporal -- --test-threads=1` | `test result: ok. 2 passed` |

```bash
uv run --quiet --with adbc-driver-manager --with pyarrow python3 - <<'EOF'
import datetime as dt, adbc_driver_manager.dbapi as d
uri = {"uri": "exasol://sys:exasol@localhost:8563?tls=true&validateservercertificate=0"}
c = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="ExarrowDriverInit", db_kwargs=uri, autocommit=True)
cur = c.cursor()
cur.execute("CREATE SCHEMA MANUAL_ISSUE_38")
cur.execute("CREATE TABLE MANUAL_ISSUE_38.T (TS TIMESTAMP(6))")
try:
    cur.execute("INSERT INTO MANUAL_ISSUE_38.T VALUES (?)", (dt.datetime(2024, 1, 1, 8, 0, 0, 123456),))
    cur.execute("SELECT TS FROM MANUAL_ISSUE_38.T"); print("ROWS", cur.fetchall())
except Exception as e:
    print("INSERT failed:", str(e).splitlines()[-1][:200])
c2 = d.connect(driver="target/release/libexarrow_rs.so", entrypoint="ExarrowDriverInit", db_kwargs=uri, autocommit=True)
c2.cursor().execute("DROP SCHEMA MANUAL_ISSUE_38 CASCADE"); print("cleaned")
EOF
```

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build FFI | `cargo build --release --features ffi` | Exit 0 |
| Build WebSocket-only tests | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Format | `cargo fmt --all -- --check` | No changes |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all -D warnings` | 0 warnings |
| Unit tests | `cargo test --lib`, `cargo test --lib --features websocket`, `cargo test --lib --features ffi` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| WebSocket integration tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Native protocol tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `cargo build --release --features ffi && REQUIRE_EXASOL=1 cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Import/export tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Check passes |
