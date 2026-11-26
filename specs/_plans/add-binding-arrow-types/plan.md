# Plan: add-binding-arrow-types

## Summary

ADBC parameter binding accepts every Arrow type that ingest accepts, plus the Arrow `Null` type, by taking each value's text from the import CSV conversion. The native transport encodes DECIMAL parameters without loss, so decimal and large integer values store exactly on both transports.

## Context

- Issue #110: binding returns `NotImplemented` for Int8, UInt8 to UInt64, Date64, Decimal256, Utf8View, BinaryView, FixedSizeBinary, Dictionary, Null, and Interval.
- `arrow_value_to_parameter` in `src/adbc_ffi.rs` keeps its own list of types, separate from the import CSV conversion in `src/import/arrow.rs`, which already converts all of these types except Null.
- The native transport writes a DECIMAL parameter as a clamped i64 and reads decimal text as 0 (`scale_decimal_value` in `src/transport/native/mod.rs`). No test binds a decimal into a DECIMAL column, so Decimal128 binding over native is unverified today.
- Nothing tests INTERVAL parameters over the native transport.
- Time32, Time64, Duration, and List have no Exasol type and stay unsupported.
- The architecture delta (`architecture.md` in this directory) adds binding to the import CSV conversion's users and the native DECIMAL encoding rule.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Arrow to Exasol | CHANGED | `type-mapping/arrow-to-exasol/spec.md` |
| Native Prepared Statement Protocol | CHANGED | `native-client/prepared-statement-protocol/spec.md` |

## Impact

- ADBC users can bind Int8, UInt8, UInt16, UInt32, UInt64, Date64, Decimal256, Utf8View, BinaryView, FixedSizeBinary, string Dictionary, Null, and Interval columns.
- Decimal128 values bound to a DECIMAL parameter over the native transport now store exactly. Before, they were likely stored as 0.
- A bound column of an unsupported Arrow type now fails with `NotImplemented` even when it holds only nulls. Before, it bound NULL.
- Over the native transport, an integer or decimal text value bound to a DECIMAL parameter with more digits than the parameter's precision and scale allow fails before execution instead of being clamped or sent as 0. A float value bound to a DECIMAL parameter is still rounded to the scale.
- No breaking API change.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Match arms | `arrow_value_to_parameter` in `src/adbc_ffi.rs` (Utf8, LargeUtf8, Binary, LargeBinary, Date32, Timestamp, Decimal128) | Replaced by the import CSV conversion's value text |
| Function | `scale_decimal_value` in `src/transport/native/mod.rs` | Replaced by lossless DECIMAL encoding from text |
| Tests | Unit tests of the removed arms and of `scale_decimal_value` | Replaced by tests of the new paths |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Bound integer, Date64, and decimal values are stored on both transports | Integration | `tests/driver_manager_tests.rs` | `test_bind_integer_date64_decimal_types_native`, `test_bind_integer_date64_decimal_types_websocket` |
| Bound string view, string dictionary, binary, and Null values are stored on both transports | Integration | `tests/driver_manager_tests.rs` | `test_bind_string_binary_null_types_native`, `test_bind_string_binary_null_types_websocket` |
| Bound Interval values are stored on both transports | Integration | `tests/driver_manager_tests.rs` | `test_bind_interval_types_native`, `test_bind_interval_types_websocket` |
| A bound value that Exasol cannot store without loss fails before execution | Integration | `tests/driver_manager_tests.rs` | `test_bind_lossy_value_fails_before_execution` |
| A bound Arrow type without an Exasol type fails with NotImplemented | Integration | `tests/driver_manager_tests.rs` | `test_bind_unsupported_arrow_type_is_not_implemented` |
| DECIMAL parameter values are encoded without loss | Integration | `tests/native_protocol_tests.rs` | `test_native_decimal_parameters_round_trip` |
| A DECIMAL parameter value that does not fit is rejected before execution | Unit | `src/transport/native/mod.rs` | `test_decimal_parameter_that_does_not_fit_is_rejected` |
| DECIMAL parameter values are encoded without loss (float rounding step) | Unit | `src/transport/native/mod.rs` | `test_float_decimal_parameter_rounds_half_away_from_zero` |

Unit tests in `src/adbc_ffi.rs` cover one newly supported Arrow type each, checking the bound parameter value, as issue #110 asks.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Arrow to Exasol | `cargo build --release --features ffi`, then in Python with `adbc-driver-manager`: `cursor.execute("SELECT CAST(? AS DECIMAL(20,0))", parameters=pa.table({"v": pa.array([18446744073709551615], pa.uint64())}))` and `cursor.fetchall()` | `[(Decimal('18446744073709551615'),)]` |
| Native Prepared Statement Protocol | Same driver, `cursor.execute("SELECT CAST(? AS DECIMAL(36,2))", parameters=pa.table({"v": pa.array([Decimal("-1234567890123456789012345678901234.56")], pa.decimal128(36, 2))}))` | `[(Decimal('-1234567890123456789012345678901234.56'),)]` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release --features 'ffi websocket'` | Exit 0 |
| Unit tests | `cargo test --lib --features parquet` and `cargo test --lib --features ffi` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests --test native_protocol_tests -- --test-threads=1` | 0 failures |
| Import regression | `REQUIRE_EXASOL=1 cargo test --features 'ffi parquet' --test import_export_tests -- --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings`, with `--features ffi`, and with `--no-default-features --features websocket` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --features parquet --lcov --output-path lcov-unit.info`, then `scripts/strip_test_coverage.py strip` and `check` | Floors met |
