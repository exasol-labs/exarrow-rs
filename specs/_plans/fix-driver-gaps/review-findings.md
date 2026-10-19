# Code Review Findings: fix-driver-gaps

## Summary
- Files reviewed: 13
- Total findings: 8 (standard: 6, expert: 2)
- Evidence: `cargo clippy --all-targets --all-features -- -D warnings` reports no issues. The new and changed unit tests (`cargo test --lib -- login_ supports_at_least capability_checks conversion_error decimal_overflow json_decimal column_context parse_string_to_decimal`) pass: 52 passed, 0 failed.
- No findings: `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `specs/_plans/fix-driver-gaps/tasks.md`, `src/arrow_conversion/converter.rs`, `src/transport/native/handshake.rs`.

## Standard fixes

### src/arrow_conversion/builders.rs

#### [SWALLOWED_ERROR] `build_array` discards the value failure when the type mapping fails
- Location: lines 51-59, `build_array`
- Issue: The `map_err` closure calls `TypeMapper::exasol_to_arrow` after `build_typed_array` has failed. Its `Err(unsupported) => unsupported` arm returns the mapping error and drops the `ColumnConversionError` that carries the row and the reason. `exasol_to_arrow` returns `Ok` for every `ExasolType` today, so the arm is also unreachable.
- Fix: In src/arrow_conversion/builders.rs `build_array`, compute `let target_type = TypeMapper::exasol_to_arrow(exasol_type, true)?;` before the call to `build_typed_array`, and move the comment `// Same mapping SchemaBuilder uses, so the error names the type that drove parsing.` above that line. Replace the `map_err` closure body with `error.into_conversion_error(column, column_name, values, &target_type)` and delete the `match` with its `Err(unsupported) => unsupported` arm.

### src/query/results.rs

#### [SHRINKABLE] Flag-to-constructor round trip in `json_to_decimal_array`
- Location: lines 587-593, `ResultSet::json_to_decimal_array`
- Issue: The closure reads `failure.overflow` only to choose between `ColumnConversionError::overflow` and `ColumnConversionError::invalid_value`, which store the same flag back into `ColumnConversionError::Value`. Seven lines express a field copy.
- Fix: In src/query/results.rs `json_to_decimal_array`, replace the `if failure.overflow { ... } else { ... }` closure body with `ColumnConversionError::Value { row, reason: failure.reason, overflow: failure.overflow }`.

### src/connection/session.rs

#### [MISSING_DOC_COMMENT] Test for an existing scenario lacks its `/// Scenario:` line
- Location: line 554, `supports_at_least_is_false_for_an_unparsable_release_version`
- Issue: The plan's Scenario Coverage table maps this test to the existing scenario "Reject an unparsable release_version" of `connection-management/version-capability`. AGENTS.md requires one `/// Scenario: <title>` line per implemented scenario. No test in `src/` or `tests/` carries this title.
- Fix: In src/connection/session.rs, insert the line `/// Scenario: Reject an unparsable release_version` directly above the `#[test]` attribute of `supports_at_least_is_false_for_an_unparsable_release_version`.

### src/adbc/connection.rs

#### [MISSING_DOC_COMMENT] Test for an existing scenario lacks its `/// Scenario:` line
- Location: line 2470, `capability_checks_issue_no_sql`
- Issue: The plan's Scenario Coverage table maps this test to the existing scenario "Capability check is in-process and round-trip-free" of `connection-management/version-capability`. AGENTS.md requires one `/// Scenario: <title>` line per implemented scenario. No test in `src/` or `tests/` carries this title.
- Fix: In src/adbc/connection.rs, insert the line `/// Scenario: Capability check is in-process and round-trip-free` directly above the `#[tokio::test]` attribute of `capability_checks_issue_no_sql`.

### src/transport/test_support.rs

#### [OUTDATED_COMMENT] Module doc does not list the new native login fake
- Location: lines 10-21, module doc comment
- Issue: The module doc enumerates every double in the file. It does not name `NativeLoginServer`, `LoginExchange`, or the `raw_public_key` and `test_phrase` fixtures that this change added.
- Fix: In src/transport/test_support.rs, extend the "Doubles here:" paragraph of the module doc, after the `SilentServer` clause, with: "`NativeLoginServer` (`native` feature) for a peer that completes TLS, answers one native login packet with a chosen protocol version, and records in `LoginExchange` what the client sends after it, with `raw_public_key` and `test_phrase` as its login fixtures,".

#### [MIXED_ABSTRACTION_LEVEL] `reporting_protocol_version` mixes the exchange script with byte framing
- Location: lines 689-746, `NativeLoginServer::reporting_protocol_version`
- Issue: The spawned task interleaves the exchange steps (accept, TLS, read, answer, record) with byte-level work: the bare prefix length `8`, the length field at bytes 4 to 8, and the attribute and header assembly of the login response. The other fakes in this module keep such steps in small helpers such as `read_magic_packet` and `read_response_head`.
- Fix: In src/transport/test_support.rs, add `#[cfg(feature = "native")] const LOGIN_PACKET_PREFIX_LEN: usize = 8;` with the doc comment `/// Magic and length fields that precede the length-counted body of a native login packet.`. Extract lines 705-716 into `#[cfg(feature = "native")] async fn read_login_packet(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8>`, which uses `LOGIN_PACKET_PREFIX_LEN` in place of the literal `8` and reads the length field as `u32::from_le_bytes(prefix[4..LOGIN_PACKET_PREFIX_LEN].try_into().expect("four length bytes"))`. Extract lines 718-732 into `#[cfg(feature = "native")] fn login_response(protocol_version: Option<i32>) -> Vec<u8>` and move the three `use` statements of `reporting_protocol_version` into it. The spawned task then accepts the connection and the TLS handshake, calls `read_login_packet`, writes and flushes `login_response(protocol_version)`, and calls `read_until_end_of_stream`.

## Expert fixes

### src/transport/native/mod.rs

#### [MISSING_BOUNDARY_TEST] A protocol version above the i32 range passes the minimum check and truncates below 14
- Location: lines 961-970, `login_exchange`
- Issue: `attribute_integer` returns an `i64` and accepts an `Int64` attribute. The check compares that `i64` with 14, and `version as i32` truncates it afterwards. A malformed login response with `ATTR_PROTOCOL_VERSION` = `Int64((1 << 32) + 13)` passes the check, the driver sends the password, and `SessionInfo.protocol_version` records 13. No test covers a reported version outside the `i32` range, because `NativeLoginServer` only sends `Int32`.
- Fix: In src/transport/native/mod.rs, add a private free function `fn negotiated_protocol_version(reported: Option<i64>) -> Result<i32, TransportError>` next to `attribute_integer`. It returns `Ok(PROTOCOL_VERSION as i32)` for `None` and keeps the comment about the missing attribute. It returns the existing `unsupported server: native protocol version {version} is below the minimum supported version {MIN_PROTOCOL_VERSION}` `ProtocolError` for a value below `i64::from(MIN_PROTOCOL_VERSION)`. It returns `TransportError::ProtocolError(format!("malformed login response: native protocol version {version} does not fit in 32 bits"))` when `i32::try_from(version)` fails, and the converted value otherwise. In `login_exchange`, replace lines 961-970 with `let protocol_version = negotiated_protocol_version(attribute_integer(&server_attrs, ATTR_PROTOCOL_VERSION))?;` at the same position, before `build_auth_message`. Write the failing unit test `negotiated_protocol_version_rejects_a_value_outside_the_i32_range` first, with inputs `Some((1_i64 << 32) + 13)` and `Some(i64::MAX)`, each expecting a `ProtocolError` that contains `does not fit in 32 bits`. Add a unit test `negotiated_protocol_version_keeps_values_from_14_and_the_fallback` for `Some(14)`, `Some(22)`, and `None` (expecting 14, 22, and `PROTOCOL_VERSION as i32`). Keep the two loopback tests unchanged.

### src/error.rs

#### [CONTEXTLESS_ERROR] `NumericOverflow` drops the overflow reason that the per-value step computed
- Location: lines 138-147 (`ConversionError::NumericOverflow`), lines 203-222 (`ColumnConversionError::into_conversion_error`); producers in src/arrow_conversion/builders.rs lines 179-192 and 235, and src/query/results.rs lines 639-646, 651, and 668
- Issue: Every overflow producer builds a reason, such as `"8 significant digits exceed precision 5"` in `validate_decimal_precision`. `into_conversion_error` discards `reason` when `overflow` is set, because `NumericOverflow` has no field for it. No production code reads an overflow reason, and no test asserts one. The error therefore gives the declared precision but not the digit count that broke it, although the scenario "Decimal overflow detection" requires the error to explain the precision/scale mismatch. The integer-part overflow in `parse_string_to_decimal` also labels itself `Invalid integer part`, which misnames an overflow.
- Fix: In src/error.rs, add `message: String` with the doc comment `/// Why the value does not fit the target type` to `ConversionError::NumericOverflow`, change its `#[error]` text to `"Numeric overflow at row {row}, column {column} ({column_name}) for {target_type}: {message}; value: {value}"`, and set `message: reason` in the `overflow` branch of `ColumnConversionError::into_conversion_error`. In src/query/results.rs `parse_string_to_decimal`, give the `PosOverflow | NegOverflow` arm the reason `format!("integer part {integer_part} exceeds the 128-bit range")` and keep `Invalid integer part: {integer_part}` for the other arm. Update the `NumericOverflow` row of the display-table test in src/error.rs (add `message: "6 significant digits exceed precision 5".to_string()` and the matching expected text), make `column_context_turns_an_overflow_into_numeric_overflow` assert `message == "too many digits"`, and in src/arrow_conversion/converter.rs bind `message` in `decimal_overflow_reports_the_column_row_value_and_precision` and assert `message.contains("exceed precision 5")`. In CHANGELOG.md, change the 0.25.0 entry for `ConversionError` so that it states that `NumericOverflow` also gains the field `message`.
