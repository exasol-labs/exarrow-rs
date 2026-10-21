# Verification Report: fix-result-set-schema-metadata

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Every result set metadata schema and batch schema on both transports carries `exasol:*` field metadata. All checks and manual commands pass. |
| Code review | 11 findings: 11 fixed (1 expert, 10 standard) |

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
| Unit (production lines) | 91.10 |
| Integration | not measured |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test --lib`) | 1773 | 1773 | 0 |
| Unit (`--features websocket`) | 1818 | 1818 | 0 |
| Unit (`--features ffi`) | 1894 | 1894 | 0 |
| Native integration (`integration_tests`, Exasol 2025.2.1) | 97 | 96 | 1 |
| WebSocket integration (`websocket_integration_tests`) | 63 | 63 | 0 |
| Driver manager (`driver_manager_tests`, `--include-ignored`) | 75 | 75 | 0 |

### Manual Tests

Run through the ADBC driver manager (Python `adbc-driver-manager`) against `target/release/libexarrow_rs.so` and Exasol 2025.2.1 in Docker.

| Test | Result |
|------|--------|
| Type mapping, native: `AMOUNT` DECIMAL precision 10 scale 2, `CODE` CHAR size 3, `TSL` reported as `TIMESTAMP`, every field has `exasol:type` | ✓ |
| Type mapping, WebSocket: same metadata, `TSL` reported as `TIMESTAMP WITH LOCAL TIME ZONE` | ✓ |
| HASHTYPE(32 BYTE), native query: `exasol:type` holds `byte_size` 32 | ✓ |
| HASHTYPE(32 BYTE), WebSocket `adbc_execute_schema` (no rows fetched): `byte_size` 32 | ✓ |
| 300,000-row query, 2 batches on each transport: every batch reports `exasol:type_name` `DECIMAL` | ✓ |
| Reader schema: `AMOUNT` DECIMAL 10,2 and `NAME` VARCHAR size 20 | ✓ |
| `adbc_execute_schema`: `AMOUNT` is `decimal128(10, 2)` with precision 10 and scale 2 | ✓ |

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
| type-mapping | exasol-to-arrow | Column metadata (native) | `tests/integration_tests.rs` | `test_result_set_fields_carry_exasol_column_metadata` | Pass |
| type-mapping | exasol-to-arrow | Column metadata (WebSocket) | `tests/websocket_integration_tests.rs` | `test_ws_result_set_fields_carry_exasol_column_metadata` | Pass |
| type-mapping | exasol-to-arrow | Schema documentation, nullability (both transports) | same tests, shared table `M` | | Pass |
| type-mapping | exasol-to-arrow | Field metadata on the native transport keeps the decoded Arrow data type | `tests/integration_tests.rs` | `test_native_interval_field_keeps_utf8_and_carries_exasol_metadata` | Pass |
| type-mapping | exasol-to-arrow | HASHTYPE field metadata carries the byte size of the column | `tests/integration_tests.rs` | `test_hashtype_field_metadata_carries_the_declared_byte_size` | Pass |
| query-execution | result-sets | Zero-row result set preserves schema | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_zero_row_result_batch_carries_field_metadata`, `test_ws_zero_row_result_batch_carries_field_metadata` | Pass |
| query-execution | result-sets | Every batch carries the field metadata | `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` | `test_every_fetched_batch_carries_the_result_set_field_metadata`, `test_ws_every_fetched_batch_carries_the_result_set_field_metadata` | Pass |
| adbc-driver | statement-and-results | RecordBatchReader schema carries Exasol field metadata | `tests/driver_manager_tests.rs` | `test_reader_schema_carries_exasol_field_metadata` | Pass |
| adbc-driver | get-parameter-schema | Execute schema returns the result columns of a query without executing it | `tests/driver_manager_tests.rs`, `src/adbc_ffi.rs` | `test_execute_schema_reports_columns_without_running_the_query`, `execute_schema_maps_result_columns_without_executing` | Pass |

## Notes

- Native TIMESTAMP WITH LOCAL TIME ZONE is reported as `TIMESTAMP`, because the server sends the same wire type for both. The spec states this per transport. A probe showed the Arrow type also differs (native: no zone, WebSocket: "UTC"), with identical values. This plan changes no Arrow type.
- HASHTYPE byte size is half the reported size, which is correct for the default HEX format. The server reports hex characters on both transports. A UUID or BASE64 session format gives a wrong size. `docs/type-mapping.md` says so.
- WebSocket results with rows in INTERVAL, GEOMETRY, or HASHTYPE columns fail before this change and after it. The issue is #107, out of scope.
- The manual script left a cursor open at exit, so Python printed a close error at teardown. The driver is not involved.
