# Verification Report: change-integer-result-columns-to-decimal

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Native and WebSocket INTEGER and SMALLINT results are Decimal128(18, 0) and Decimal128(9, 0) with equal values, the specs and docs now state that, and the native decoder builds the same types for the `T_INTEGER` and `T_SMALLINT` wire types. |
| Code review | 4 findings, 4 fixed |

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
| Unit | 91.15 (production lines, floor 80) |
| Integration | not measured (CI measures unit coverage only) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit, default features | 1785 | 1785 | 0 |
| Unit, `websocket` | 1830 | 1830 | 0 |
| Unit, `ffi` | 1906 | 1906 | 0 |
| Integration, `native_protocol_tests` | 19 | 19 | 0 |
| Integration, `websocket_integration_tests` | 69 | 69 | 0 |
| Integration, `integration_tests` | 96 | 95 | 1 |
| Integration, `driver_manager_tests` | 75 | 75 | 0 |

The ignored test in `integration_tests` carried its `#[ignore]` before this plan. The driver manager tests ran against a release library built with `--features 'ffi websocket'`, because three WebSocket driver manager tests need the `websocket` feature in the library.

### Manual Tests

| Test | Result |
|------|--------|
| Native: `I` and `N` are `decimal128(18, 0)`, `S` is `decimal128(9, 0)`, `TSL` is `timestamp[us]`, row is `Decimal('-999999999999999999')`, `Decimal('999999999')`, `None` | ✓ |
| WebSocket: `I`, `S`, and `N` have the native types and values, `TSL` is `timestamp[us, tz=UTC]` | ✓ |

## Tool Evidence

### Linter

```
cargo +1.92 clippy --all-targets --all-features -- -D warnings: exit 0
```

### Formatter

```
cargo +1.92 fmt --all -- --check: exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| native-client | type-conversion | Direct binary to Arrow conversion for numeric types | `src/transport/native/result_parser.rs` | `single_pass_integer_column_decodes_to_decimal128` | Pass |
| type-mapping | exasol-to-arrow | Non-decimal numeric type mapping | `tests/integration_tests.rs` | `test_double_type_conversion` | Pass |
| type-mapping | exasol-to-arrow | INTEGER and SMALLINT type mapping | `tests/native_protocol_tests.rs` | `test_native_integer_and_smallint_results_are_decimal128` | Pass |
| type-mapping | exasol-to-arrow | WebSocket INTEGER and SMALLINT results equal native results | `tests/websocket_integration_tests.rs` | `test_ws_integer_result_equals_native` | Pass |

## Notes

- Native INTEGER and SMALLINT results were already Decimal128 before this plan, because Exasol 2025.2.1 sends these columns as `T_DECIMAL` and `T_SMALLDECIMAL`. The plan changes no result type on the tested server. It corrects the specs, `docs/type-mapping.md`, and `docs/queries.md`, and it aligns the decoding of the `T_INTEGER` and `T_SMALLINT` wire types, which no tested server sends for a result column.
- `test_native_integer_and_smallint_results_are_decimal128` is a regression test that passed before the decoder change. Its red step is the unit tests with synthetic `T_INTEGER` and `T_SMALLINT` bytes, which failed with Int64 and Int32 before the change.
- The change to the benchmark's COUNT(*) reader (task 5.1) has no test, because the benchmark is a binary that needs a populated table and a real server always returns Decimal128. It compiles under `--all-features` clippy.
- The same plan corrects the TIMESTAMP WITH LOCAL TIME ZONE rows of `docs/type-mapping.md`: native `Timestamp(Microsecond, None)`, WebSocket `Timestamp(Microsecond, "UTC")`. The manual test confirms both.
- The version stays 0.28.0 because PR #108 is unmerged and 0.28.0 is unreleased.
