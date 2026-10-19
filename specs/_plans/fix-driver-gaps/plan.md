# Plan: fix-driver-gaps

## Summary

This plan closes four gaps between the exarrow-rs specs and the code: the native login rejects servers with a protocol version below 14, `Session` gains a three-component `supports_at_least` check, JSON-to-Arrow conversion errors name the column, and the unused keepalive settings are removed. The four fixes are independent and touch disjoint files, except for the shared release notes.

## Context

- `MIN_PROTOCOL_VERSION` (14) is defined in `src/transport/native/constants.rs`, but `login_exchange` in `src/transport/native/mod.rs` never compares the server's version with it and sends the password message to any server.
- `login_exchange` falls back to the client's maximum version (`PROTOCOL_VERSION`, 21) when the login response has no `ATTR_PROTOCOL_VERSION`, and it accepts a version above 21.
- The spec defines `supports_at_least(major, minor, patch)`. The code has only the free function `connection::version::supports_at_least(parsed, major, minor)`, which ignores the patch component and has no caller. This plan adds the method to `Session` only and leaves the free function unchanged, by the requester's choice (decision [3]).
- `adbc::Connection` has no accessor for its `Session`, so a driver user who holds a `Connection` cannot call `Session::supports_at_least`.
- `ValueConversionFailed` carries the row, the column index, and a message. `NumericOverflow` carries the row and the column index only.
- Two code paths convert JSON result data to Arrow. The public `ArrowConverter` utility (`src/arrow_conversion/`) returns `ValueConversionFailed` and `NumericOverflow`, and neither transport calls it. The WebSocket result path in `src/query/results.rs` returns `ConversionError::InvalidFormat` for a DECIMAL value that fails to convert, with no column and no row.
- This plan reads "conversion errors" as `ConversionError` raised while converting JSON result data, which is the GIVEN of "Conversion error reporting". The native transport decodes binary result data and raises no `ConversionError`. Native decode errors stay unchanged: an invalid UTF-8 VARCHAR value on the native transport still returns `TransportError::ProtocolError("Invalid UTF-8 in string: ...")` without the column name (`src/transport/native/result_parser.rs`).
- On the WebSocket result path, a non-DECIMAL value that does not fit its column's Arrow type becomes NULL and raises no error, and this plan keeps that behavior. On that path, only a DECIMAL string that fails to convert raises an error under "Conversion error reporting".
- `ConversionError::InvalidUtf8` has no producer, and this plan leaves it unchanged.
- The Arrow-to-Exasol direction (parameter binding and import, `type-mapping/arrow-to-exasol`) reports overflows through other error types, not through `ValueConversionFailed` or `NumericOverflow`, and stays out of scope.
- `SessionConfig.enable_keepalive` and `SessionConfig.keepalive_interval` have no reader. This plan writes no spec delta for their removal, because PR 101 removed the scenario "Heartbeat and keepalive".
- PR 101 is merged on main as 57e68e9. It removed "Heartbeat and keepalive" from `websocket-client/async-lifecycle`, "Large batch handling" and "Partial conversion handling" from `arrow-conversion/recordbatch-assembly`, and "Large strings" from `arrow-conversion/type-converters`. It left "Conversion error reporting" and "Decimal overflow detection" unchanged. `feat/fix-driver-gaps` starts at b6f9395, one commit behind main. Whoever starts implementation first updates the branch to main, by a fast-forward or a merge, because `/speq:git-operations` forbids a rebase. The record step then merges the deltas into main's spec text.
- Follow-up, out of scope: the two decimal parsers disagree. The `ArrowConverter` parser in `src/arrow_conversion/builders.rs` checks the declared precision. `ResultSet::parse_string_to_decimal` in `src/query/results.rs` drops extra fraction digits and checks no declared precision, and `ResultSet::scale_to_decimal` multiplies an i64 JSON number without an overflow check. A later plan unifies the two parsers.
- Out of scope: result schema metadata (issue 102) and CMD_CONTINUE (issue 103).
- Architecture: no delta. The connection component in `specs/architecture.md` § Components already checks the server version for features, and no component, boundary, interface, data flow, or dependency changes.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| native-client/handshake | CHANGED | `specs/_plans/fix-driver-gaps/native-client/handshake/spec.md` |
| connection-management/version-capability | CHANGED | `specs/_plans/fix-driver-gaps/connection-management/version-capability/spec.md` |
| arrow-conversion/recordbatch-assembly | CHANGED | `specs/_plans/fix-driver-gaps/arrow-conversion/recordbatch-assembly/spec.md` |

The plan also implements three existing scenarios without changing their text: `connection-management/version-capability` "Reject an unparsable release_version" and "Capability check is in-process and round-trip-free" (their `supports_at_least` steps, on `Session`), and `arrow-conversion/type-converters` "Decimal overflow detection" on the `ArrowConverter` path only. Only the `ArrowConverter` path detects a value that exceeds the declared precision. The WebSocket result path detects only an i128 overflow of a decimal string.

## Impact

- Behavior change: a native login to a server that reports a protocol version below 14 fails, and the driver sends no password to that server. `Connection::from_params`, which `Database::connect` and `ConnectionBuilder::connect` call, returns the failure as `ConnectionError::AuthenticationFailed`, so the message arrives with the prefix `Authentication failed: Protocol error: `, for example `Authentication failed: Protocol error: unsupported server: native protocol version 13 is below the minimum supported version 14`.
- New API: `Session::supports_at_least(major, minor, patch)`. `Connection` gains no such method.
- Breaking: `SessionConfig.enable_keepalive` and `SessionConfig.keepalive_interval` are removed. Neither field had any effect.
- Breaking: `ConversionError::ValueConversionFailed` and `ConversionError::NumericOverflow` gain the fields `column_name`, `value`, and `target_type`, and their messages change. `arrow_conversion::build_array` takes the column name as a new `&str` parameter.
- Breaking: on the WebSocket transport, a DECIMAL value that fails to convert returns `ValueConversionFailed` or `NumericOverflow` instead of `InvalidFormat`.
- Target version: 0.25.0, a minor bump, because the plan has breaking changes before 1.0. Task 4.1 sets it, and the version-bump step of `/speq:implement-pr` keeps it.
- Release: merging this PR runs the CI `release` job, which tags, releases, and publishes 0.25.0 to crates.io.

## Implementation Tasks

### 1. Native protocol version check (group A)

- [ ] 1.1 Add a native login fake to `src/transport/test_support.rs` (native feature), built like `SilentServer::after_tls`. It completes the TLS handshake, reads one login packet (length from its header), answers with a login response framed by `MessageHeader::serialize` whose attributes (`AttributeSet::serialize`) carry a public key, a random phrase, and a test-chosen `ATTR_PROTOCOL_VERSION` or none, and then records every byte the client sends after the login packet until end of stream. Move the `raw_public_key` and `test_phrase` fixtures out of the `handshake.rs` test module so that the fake can use them.
- [ ] 1.2 Write the failing test `login_rejects_a_server_protocol_version_below_14_before_sending_the_password` in the `src/transport/native/mod.rs` test module: for a reported version of 13 and of -1, `authenticate` fails with `TransportError::ProtocolError` whose inner message starts with `unsupported server` and contains the reported version and `14`, the test drops the transport (no `close()`, which sends a disconnect message), and the fake records zero bytes after the login packet.
- [ ] 1.3 Write the test `login_accepts_server_protocol_versions_from_14_to_the_client_maximum` in the same module: for version 14, version 21, version 22 (above the client's maximum), and a response without the attribute (the fallback that decision [2] keeps), the received login packet starts with `LOGIN_MAGIC` and carries `PROTOCOL_VERSION`, the client sends a non-empty password message, and any later error does not name the minimum version. The fake never answers the password message, so these logins end at the 300 ms connection timeout of `silent_server_params`.
- [ ] 1.4 In `login_exchange`, compare the attribute's `i64` value with `i64::from(MIN_PROTOCOL_VERSION)` right after reading it, before the `as i32` truncation and before `build_auth_message`, so that a negative version cannot pass as a large unsigned number, and return `TransportError::ProtocolError` with a message that starts with the incompatibility: `unsupported server: native protocol version <v> is below the minimum supported version 14`. Keep the fallback to `PROTOCOL_VERSION` for a missing attribute.

### 2. Version capability and session settings (group B)

- [ ] 2.1 Write failing unit tests in `src/connection/session.rs`: `supports_at_least_compares_major_minor_and_patch` (release `"8.27.1"`, the six boundary cases of the new scenario) and `supports_at_least_is_false_for_an_unparsable_release_version` (releases `""`, `"unknown"`, `"v1"`, thresholds `(0,0,0)` and `(9,0,0)`).
- [ ] 2.2 In `Session`, replace `native_parquet_cache: OnceLock<bool>` with a memoized `Option<(u32, u32, u32)>` from `parse_release_version`. Add `supports_at_least(major, minor, patch)`, which compares the parsed tuple lexicographically and returns `false` when parsing failed. Derive `supports_native_parquet_import()` from the same memoized value. Leave the free function `supports_at_least` in `src/connection/version.rs` unchanged.
- [ ] 2.3 Write the test `capability_checks_issue_no_sql` in the `src/adbc/connection.rs` test module: a connection over `recording_transport` calls `supports_native_parquet_import()` and, through its private `session` field, `supports_at_least` several times, and the SQL log stays empty. `Connection` gains no `supports_at_least` method (decision [3]).
- [ ] 2.4 Remove `enable_keepalive` and `keepalive_interval` from `SessionConfig` and from its `Default` implementation in `src/connection/session.rs`.

### 3. Conversion error context (group C)

- [ ] 3.1 In `src/error.rs`, add `column_name: String`, `value: String`, and `target_type: String` to `ConversionError::ValueConversionFailed` and `ConversionError::NumericOverflow`. Each message names the column index and name, the row, the value, the target type, and, for `ValueConversionFailed`, the reason. Add a crate-private error type for the per-value steps that holds the row, the reason, and an overflow flag, and that also carries any other `ConversionError` a per-column builder raises, such as `ArrowError`, unchanged. Add one crate-private method that converts it, with the column index, the column name, the failing JSON value, and the Arrow data type, into `NumericOverflow` when the overflow flag is set and into `ValueConversionFailed` for any other value failure, and that returns a carried `ConversionError` unchanged. This method is the only production code that builds either variant (decision [1]). Update the `error.rs` tests that construct these variants.
- [ ] 3.2 Write the failing tests in `src/arrow_conversion/converter.rs`: `conversion_errors_name_the_column_row_value_and_target_type` (an `ArrowConverter` with a BOOLEAN column `FLAG` receives `"yes"` at row 1, and the `ValueConversionFailed` error names `FLAG`, column 0, row 1, the value, the Boolean Arrow data type, and the reason) and `decimal_overflow_reports_the_column_row_value_and_precision` (a DECIMAL(5,2) column `AMOUNT` receives `"12345.678"`, and the `NumericOverflow` error names `AMOUNT`, the row, the value, and the Decimal128 data type with precision 5 and scale 2). Assert the data type through the display text of Arrow's `DataType`, not a hard-coded spelling.
- [ ] 3.3 In `src/arrow_conversion/builders.rs`, make the per-value helpers and the `build_*_array` functions return the crate-private error from 3.1 instead of building `ValueConversionFailed` or `NumericOverflow`, and remove their `column` parameter, which becomes unused. Give `build_array` the column name (`&str`) and convert the crate-private error there with the method from 3.1, taking the failing JSON value from `values[row]` and the Arrow data type from `TypeMapper::exasol_to_arrow(exasol_type, true)`. That function is the mapping `SchemaBuilder` uses for the `ArrowConverter` schema, so the error always names the type that drove parsing. Pass `self.schema.field(col_idx).name()` from `ArrowConverter::build_record_batch`. Update the `builders.rs` tests: about 140 helper calls lose the `column` argument, the 12 `build_array` calls gain a column name, and the tests that inspect a helper's error, including the 2 matches on these variants, switch to the crate-private error.
- [ ] 3.4 Write the failing test `json_decimal_conversion_error_names_the_column` in `src/query/results.rs`: `column_major_to_record_batch` with a `Decimal128(10, 2)` field `AMOUNT` and the value `"1.2.3"` at row 1 returns `ValueConversionFailed` that names `AMOUNT`, column 0, row 1, the value, and the display text of `DataType::Decimal128(10, 2)`. Then make `parse_string_to_decimal` report a reason and an overflow flag (set for an i128 overflow) instead of `InvalidFormat`, make `json_to_decimal_array` turn that into the crate-private error from 3.1 with the row, let `json_column_to_array` pass it on, and convert it in `column_major_to_record_batch` with the method from 3.1. Update the tests that expect `InvalidFormat` from `parse_string_to_decimal`.

### 4. Release notes (group D)

- [ ] 4.1 Set the version in `Cargo.toml` to 0.25.0 (a minor bump for breaking changes before 1.0) and run `cargo build` so that `Cargo.lock` records it. Add a `## 0.25.0` section to `CHANGELOG.md` with one concise entry per item in Impact, marking the breaking changes, and fold any `## [Unreleased]` entries into it. The version-bump step of `/speq:implement-pr` keeps 0.25.0 and does not bump again. After that step, the Checklist row "Version agreement" passes.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Native protocol version check | 1.1-1.4 | none | spec delta `native-client/handshake`; `src/transport/native/mod.rs`, `src/transport/native/constants.rs`, `src/transport/native/handshake.rs` (test fixtures), `src/transport/test_support.rs` |
| B: Version capability and session settings | 2.1-2.4 | none | spec delta `connection-management/version-capability`; `src/connection/session.rs`, `src/connection/version.rs` (`parse_release_version`, read only), `src/adbc/connection.rs` (test module only) |
| C: Conversion error context | 3.1-3.4 | none | spec delta `arrow-conversion/recordbatch-assembly`; `specs/arrow-conversion/type-converters/spec.md` (Decimal overflow detection); `src/error.rs`, `src/arrow_conversion/builders.rs`, `src/arrow_conversion/converter.rs`, `src/query/results.rs` |
| D: Release notes | 4.1 | A, B, C (summarizes their changes) | `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Field | `src/connection/session.rs` `SessionConfig.enable_keepalive` | No code reads it |
| Field | `src/connection/session.rs` `SessionConfig.keepalive_interval` | No code reads it |
| Field | `src/connection/session.rs` `Session.native_parquet_cache` | Replaced by the memoized parsed release version |
| Parameter | `src/arrow_conversion/builders.rs` `column` of the per-value helpers and the `build_*_array` functions | `build_array` attaches the column index (decision [1]) |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| native-client/handshake: Server protocol version below 14 is rejected before the password is sent | Integration (loopback TLS fake server) | `src/transport/native/mod.rs` | `login_rejects_a_server_protocol_version_below_14_before_sending_the_password` |
| native-client/handshake: Protocol version negotiation | Integration (loopback TLS fake server) | `src/transport/native/mod.rs` | `login_accepts_server_protocol_versions_from_14_to_the_client_maximum` |
| version-capability: Compare the server version against a caller-supplied threshold | Unit | `src/connection/session.rs` | `supports_at_least_compares_major_minor_and_patch` |
| version-capability: Reject an unparsable release_version (existing scenario) | Unit | `src/connection/session.rs` | `supports_at_least_is_false_for_an_unparsable_release_version` |
| version-capability: Capability check is in-process and round-trip-free (existing scenario) | Unit (mock transport) | `src/adbc/connection.rs` | `capability_checks_issue_no_sql` |
| recordbatch-assembly: Conversion error reporting (ArrowConverter path) | Unit | `src/arrow_conversion/converter.rs` | `conversion_errors_name_the_column_row_value_and_target_type` |
| recordbatch-assembly: Conversion error reporting (WebSocket result path) | Unit | `src/query/results.rs` | `json_decimal_conversion_error_names_the_column` |
| type-converters: Decimal overflow detection (existing scenario, ArrowConverter path only) | Unit | `src/arrow_conversion/converter.rs` | `decimal_overflow_reports_the_column_row_value_and_precision` |

- Each test carries one `/// Scenario: <title>` line per scenario it implements, quoting the title verbatim.
- The version comparison and the JSON-to-Arrow conversion are pure computations, so unit tests cover them. The protocol version check needs a server that reports a version below 14, which the Exasol Docker image cannot provide, so a loopback TLS fake covers it.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| native-client/handshake | `cargo run --example health_check` (Exasol Docker running) | Prints `OK`: the server's protocol version passes the check |
| native-client/handshake | `cargo test --lib login_rejects_a_server_protocol_version_below_14 -- --nocapture` | 1 passed |
| connection-management/version-capability | `cargo test --lib supports_at_least -- --nocapture` | `test result: ok`, 0 failed, and both `session.rs` tests from task 2.1 listed |
| arrow-conversion/recordbatch-assembly | `cargo test --lib conversion_errors_name_the_column -- --nocapture` | 1 passed |
| Keepalive removal | `grep -rn keepalive src/ tests/ examples/ docs/` | No output |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build FFI | `cargo build --release --features ffi` | Exit 0 |
| Build WebSocket-only | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Unit tests (WebSocket) | `cargo test --lib --features websocket` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| Native protocol tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1` | 0 failures |
| WebSocket integration tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `cargo build --release --features ffi && cargo test --test driver_manager_tests` | 0 failures |
| Version agreement | `grep -m1 '^version' Cargo.toml` and `grep -m1 '^## ' CHANGELOG.md` | `version = "0.25.0"` and `## 0.25.0` |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json` and `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | `check` passes |
