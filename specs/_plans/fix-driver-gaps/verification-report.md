# Verification Report: fix-driver-gaps

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All four fixes are implemented. Every checklist command exits 0, and every scenario in the coverage table has a passing test. |
| Code review | 8 findings, 8 fixed (6 standard, 2 expert) |

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
| Unit (production lines only) | 90.83 |
| Integration | not measured (the `ffi` feature deadlocks `cargo-llvm-cov`) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (default features) | 1753 | 1753 | 0 |
| Unit (`websocket` feature) | 1798 | 1798 | 0 |
| Integration (`integration_tests`) | 91 | 90 | 1 |
| Native protocol | 16 | 16 | 0 |
| WebSocket integration | 58 | 58 | 0 |
| Driver manager | 71 | 71 | 0 |
| Import and export | 79 | 78 | 1 |

### Manual Tests

| Test | Result |
|------|--------|
| `cargo run --example health_check` against the Exasol Docker database prints `OK` | ✓ |
| `grep -rn keepalive src tests examples docs` prints nothing | ✓ |
| Version agreement: `Cargo.toml` reads `0.25.0` and the first `## ` line of `CHANGELOG.md` is `## 0.25.0` | ✓ |
| Targeted unit tests for the handshake, `supports_at_least`, and conversion errors | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings: exit 0, no warnings
```

### Formatter

```
cargo fmt --all -- --check: exit 0, no changes
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| native-client | handshake | Server protocol version below 14 is rejected before the password is sent | `src/transport/native/mod.rs` | `login_rejects_a_server_protocol_version_below_14_before_sending_the_password` | Pass |
| native-client | handshake | Protocol version negotiation | `src/transport/native/mod.rs` | `login_accepts_server_protocol_versions_from_14_to_the_client_maximum` | Pass |
| connection-management | version-capability | Compare the server version against a caller-supplied threshold | `src/connection/session.rs` | `supports_at_least_compares_major_minor_and_patch` | Pass |
| connection-management | version-capability | Reject an unparsable release_version | `src/connection/session.rs` | `supports_at_least_is_false_for_an_unparsable_release_version` | Pass |
| connection-management | version-capability | Capability check is in-process and round-trip-free | `src/adbc/connection.rs` | `capability_checks_issue_no_sql` | Pass |
| arrow-conversion | recordbatch-assembly | Conversion error reporting | `src/arrow_conversion/converter.rs` | `conversion_errors_name_the_column_row_value_and_target_type` | Pass |
| arrow-conversion | recordbatch-assembly | Conversion error reporting (WebSocket result path) | `src/query/results.rs` | `json_decimal_conversion_error_names_the_column` | Pass |
| arrow-conversion | type-converters | Decimal overflow detection | `src/arrow_conversion/converter.rs` | `decimal_overflow_reports_the_column_row_value_and_precision` | Pass |

## Notes

- The two new scenarios ("Server protocol version below 14 is rejected before the password is sent" and "Compare the server version against a caller-supplied threshold") exist only in the plan deltas until `/speq:record` merges them. The tag checker reports their two test tags as unmatched until then.
- The code review fixes added a 32-bit range check for the reported protocol version, and added a `message` field to `ConversionError::NumericOverflow`. The changelog entry names `message`.
- The Docker image cannot report a protocol version below 14, so a loopback TLS fake server covers the rejection. No real server below version 14 was tested.
- WebSocket path: only a DECIMAL string that fails to convert raises an error. A non-DECIMAL value that does not fit its column still becomes NULL without an error, as before.
- Pre-existing and not part of this plan: `cargo clippy --no-default-features --features websocket` reports dead-code warnings in `src/types/conversion.rs`. That command is not in the CI checklist.
