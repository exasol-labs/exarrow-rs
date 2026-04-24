# Plan: add-native-protocol

## Summary

Implement Exasol's native binary TCP protocol as the default transport for exarrow-rs, delivering ~16x throughput improvement over the WebSocket JSON transport by eliminating JSON serialization and converting column-major binary wire data directly to Arrow RecordBatches. The native protocol is behind a `native` feature flag (enabled by default), with WebSocket available as an opt-in fallback via the `websocket` feature flag.

## Design

### Context

The current WebSocket transport serializes all data through JSON (`serde_json::Value`), then converts to Arrow arrays — a double-copy path that dominates query latency for analytical workloads. Exasol's native binary TCP protocol sends result sets in column-major format with typed binary encoding, which maps directly to Arrow's columnar memory layout. Re-implementing this protocol in Rust enables a binary-to-Arrow zero-copy path.

The native protocol is closed-source (C++ CLI SDK), so this is a clean-room Rust implementation based on protocol analysis of the C++ driver and server-side connection handling code.

- **Goals** — binary TCP transport with direct Arrow conversion; ChaCha20 + TLS encryption; feature-flagged transport selection; protocol v14+ support; full functional test parity between transports
- **Non-Goals** — connection pooling; parallel worker connections (`CMD_ENTER_PARALLEL`); Kerberos authentication; protocol versions below 14; replacing the HTTP tunneling mechanism for IMPORT/EXPORT (remains unchanged)

### Decision

#### Architecture

```
┌──────────────┐     ┌──────────────────┐     ┌───────────────────────────────┐
│  ADBC Layer  │────▶│  Connection      │────▶│  Transport (feature-gated)    │
│  Driver /    │     │  (owns transport │     │  ┌─────────────────────────┐  │
│  Database /  │     │   exclusively)   │     │  │ NativeTcpTransport     │  │
│  Statement   │     │                  │     │  │ (default, binary→Arrow)│  │
│              │     │  Works with      │     │  └─────────────────────────┘  │
│              │     │  RecordBatch     │     │  ┌─────────────────────────┐  │
│              │     │                  │     │  │ WebSocketTransport     │  │
│              │     │                  │     │  │ (opt-in, JSON→Arrow)   │  │
└──────────────┘     └──────────────────┘     │  └─────────────────────────┘  │
                                              └───────────────────────────────┘

Native TCP Data Path (zero-copy):
  Wire bytes (column-major) ──▶ Arrow ArrayBuilders ──▶ RecordBatch

WebSocket Data Path (legacy):
  JSON text ──▶ serde_json::Value ──▶ ArrowConverter ──▶ RecordBatch
```

#### Test Architecture

```
tests/
├── common/mod.rs                   # Shared helpers, transport-agnostic connection factory
├── integration_tests.rs            # Functional tests — run against default transport (native)
├── native_protocol_tests.rs        # Native-specific: handshake, framing, encryption
├── websocket_integration_tests.rs  # Same functional tests, forced WebSocket transport
├── driver_manager_tests.rs         # FFI driver manager tests (unchanged)
└── import_export_tests.rs          # HTTP tunneling tests (unchanged, transport-independent)
```

The `common` module provides a `get_test_connection()` that uses the default transport and a `get_test_connection_with_transport(transport: &str)` to force a specific transport. `integration_tests.rs` uses the default (native). `websocket_integration_tests.rs` forces `transport=websocket`. Both files exercise the same functional scenarios, ensuring feature parity.

#### Wire Protocol Structure

```
LOGIN PACKET (client → server):
┌───────────┬─────────────┬──────────────────┬─────────────┬────────────┐
│ Magic (4) │ MsgLen (4)  │ ProtocolVer (4)  │ ChgDate (4) │ Attrs (var)│
│ 0x01121201│ big-endian  │ big-endian       │ big-endian  │ binary     │
└───────────┴─────────────┴──────────────────┴─────────────┴────────────┘

MESSAGE HEADER (21 bytes, all fields big-endian):
┌────────────┬──────────┬────────────┬──────────────┬──────────────┬───────────────┐
│ Length (4) │ Cmd (1)  │ Serial (4) │ NumAttrs (4) │ AttrLen (4)  │ NumParts (4)  │
└────────────┴──────────┴────────────┴──────────────┴──────────────┴───────────────┘

RESULT SET (column-major):
┌──────────┬──────────┬──────────┬───────────┬────────────┬──────────┬────────────┐
│ Type (1) │ Hndl (4) │ Cols (4) │ Total (8) │ Recvd (8)  │ ColMeta  │ ColData    │
│ R_Result │          │          │           │            │ (var)    │ (col-major)│
└──────────┴──────────┴──────────┴───────────┴────────────┴──────────┴────────────┘
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Strategy (transport selection) | Connection layer | Feature flags select NativeTcp or WebSocket at build time |
| State machine | NativeTcpTransport | Disconnected → Connected → Authenticated → Encrypted → Closed |
| Builder | Arrow array construction | Column-major binary data streams directly into Arrow ArrayBuilders |
| Layered encryption | TLS + ChaCha20 | TLS is the outer socket layer; ChaCha20 encrypts individual message payloads |
| Shared test harness | tests/common | Transport-agnostic helpers enable identical functional tests across both transports |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Direct binary→Arrow (skip JSON) | Convert binary→JSON→Arrow; Generic trait with enum return | Eliminating the JSON intermediary is the primary source of the 16x speedup; the whole point of the native protocol |
| `native` feature default, `websocket` opt-in | Both default; native opt-in | Native is the performance path; WebSocket becomes the compatibility fallback |
| Protocol v14+ minimum | v12+ (RC4+ChaCha20); v6+ (no encryption) | v14 requires ChaCha20 only — no RC4 legacy code; compatible with Exasol 7.1+ |
| TLS + ChaCha20 dual encryption | TLS only; ChaCha20 only | Matches C++ driver behavior; ChaCha20 encrypts at message level independent of transport |
| Modified TransportProtocol trait | New separate trait; adapter pattern | Single trait keeps Connection layer simple; WebSocket adapter converts JSON→RecordBatch internally |
| Transport-agnostic test harness | Duplicate test files; test macros | Shared helpers with `get_test_connection_with_transport()` avoid duplication while guaranteeing identical coverage |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Native TCP protocol | NEW | `native-client/protocol/spec.md` |
| ChaCha20 session encryption | NEW | `native-client/encryption/spec.md` |
| Binary result set to Arrow | NEW | `native-client/result-sets/spec.md` |
| Zero-copy fetch optimization | NEW | `native-client/zero-copy-fetch/spec.md` |
| Transport selection | NEW | `connection-management/transport-selection/spec.md` |
| Auth and security (native) | CHANGED | `connection-management/auth-and-security/spec.md` |

## Dependencies

| Dependency | Purpose | Crate |
|------------|---------|-------|
| chacha20 | ChaCha20 stream cipher for session encryption | `chacha20` (RustCrypto) |
| tokio (existing) | Async TCP I/O via `TcpStream` | `tokio` with `net` feature |
| rustls (existing) | TLS layer for native TCP connections | `rustls` |
| aws-lc-rs (existing) | RSA encryption for password and key exchange | `aws-lc-rs` |
| arrow (existing) | RecordBatch and array builders for direct conversion | `arrow` |

## Implementation Tasks

1. Define native protocol constants module (`src/transport/native/constants.rs`): command codes, result types, attribute IDs, magic values, data type IDs — derived from Exasol's `protocolconstants.h`, `protocoltypedecl.h`, and `protocolattributedecl.h`
2. Implement binary attribute serialization/deserialization (`src/transport/native/attributes.rs`): encode and decode key-value attribute pairs in Exasol's binary wire format
3. Implement message framing (`src/transport/native/framing.rs`): 21-byte header construction/parsing, big-endian encoding, serial number tracking
4. Implement ChaCha20 encryption module (`src/transport/native/encryption.rs`): key generation, RSA-encrypted key exchange, stream encrypt/decrypt for message payloads
5. Implement login handshake (`src/transport/native/handshake.rs`): LOGIN_MAGIC packet, protocol version negotiation, attribute exchange, RSA password encryption, ChaCha20 key setup
6. Implement result set parser (`src/transport/native/result_parser.rs`): parse R_ResultSet, R_RowCount, R_Empty, R_Exception; extract column metadata and column-major binary data
7. Implement binary-to-Arrow converter (`src/transport/native/arrow_builder.rs`): direct conversion from column-major binary wire data to Arrow RecordBatches using Arrow ArrayBuilders for each Exasol type
8. Implement `NativeTcpTransport` struct (`src/transport/native/mod.rs`): full `TransportProtocol` implementation with connect, authenticate, execute, fetch, prepared statements, disconnect
9. Modify `TransportProtocol` trait to return `RecordBatch` from `execute_query` and `fetch_results` instead of JSON-based `ResultData`
10. Adapt `WebSocketTransport` to produce `RecordBatch` (wrap existing JSON→Arrow conversion inside the transport)
11. Add `native` and `websocket` feature flags to `Cargo.toml`; gate transport modules behind features; make `native` default
12. Update `Connection` layer to select transport based on feature flags and connection string `transport=` parameter
13. Add `transport=websocket|native` parameter to connection string parser
14. Refactor `tests/common/mod.rs` to support transport-agnostic connection factory with `get_test_connection_with_transport(transport: &str)` helper
15. Write native-specific protocol tests (`tests/native_protocol_tests.rs`): handshake, framing, encryption, binary result parsing — protocol-level behaviors unique to native transport
16. Create `tests/websocket_integration_tests.rs` that mirrors all functional tests from `integration_tests.rs` but forces `transport=websocket` — ensures WebSocket feature parity under the new trait
17. Verify existing `integration_tests.rs` passes with native transport as default — these become the native functional test suite
18. Update mission.md tech stack and architecture sections
19. Bump version in `Cargo.toml` and update `CHANGELOG.md`

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A: Protocol foundations | 1 (constants), 2 (attributes), 3 (framing) |
| Group B: Security | 4 (ChaCha20 encryption) |
| Group C: Core transport | 5 (handshake), 6 (result parser), 7 (arrow builder) |
| Group D: Integration | 8 (NativeTcpTransport), 9 (trait change), 10 (WS adapter) |
| Group E: Feature flags & wiring | 11 (Cargo.toml), 12 (Connection dispatch), 13 (conn string) |
| Group F: Test infrastructure | 14 (common helpers), 15 (native protocol tests), 16 (WS parity tests) |
| Group G: Validation & release | 17 (verify integration_tests), 18 (docs), 19 (version bump) |

Sequential dependencies:
- Group A → Group C (handshake/parser depend on constants, attributes, framing)
- Group B → Group C (handshake depends on ChaCha20)
- Group C → Group D (transport struct depends on handshake, parser, builder)
- Group D → Group E (feature flags wire up the transports)
- Group E → Group F (tests validate the wired-up system)
- Group F → Group G (version bump after all tests pass)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None yet | — | WebSocket transport is preserved behind `websocket` feature flag; no code is removed in this plan |

## Verification

### Scenario Coverage

#### Native-specific protocol tests (`tests/native_protocol_tests.rs`)

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Native TCP connection establishment | Integration | `tests/native_protocol_tests.rs` | `test_native_tcp_connect` |
| Protocol version negotiation | Integration | `tests/native_protocol_tests.rs` | `test_native_protocol_version_negotiation` |
| Login handshake | Integration | `tests/native_protocol_tests.rs` | `test_native_login_handshake` |
| Binary message framing | Unit | `src/transport/native/framing.rs` | `test_header_roundtrip` |
| Disconnect | Integration | `tests/native_protocol_tests.rs` | `test_native_disconnect` |
| Error response handling | Integration | `tests/native_protocol_tests.rs` | `test_native_error_response` |
| Still-executing handling | Integration | `tests/native_protocol_tests.rs` | `test_native_still_executing` |
| ChaCha20 key generation | Unit | `src/transport/native/encryption.rs` | `test_chacha20_key_generation` |
| RSA-encrypted key exchange | Integration | `tests/native_protocol_tests.rs` | `test_native_chacha20_key_exchange` |
| Encrypted message sending | Integration | `tests/native_protocol_tests.rs` | `test_native_encrypted_execute` |
| Encrypted message receiving | Integration | `tests/native_protocol_tests.rs` | `test_native_encrypted_fetch` |
| TLS and ChaCha20 layering | Integration | `tests/native_protocol_tests.rs` | `test_native_tls_plus_chacha20` |
| Column metadata parsing | Integration | `tests/native_protocol_tests.rs` | `test_native_column_metadata` |
| Binary to Arrow numeric types | Integration | `tests/native_protocol_tests.rs` | `test_native_arrow_numeric_types` |
| Binary to Arrow string types | Integration | `tests/native_protocol_tests.rs` | `test_native_arrow_string_types` |
| Binary to Arrow temporal types | Integration | `tests/native_protocol_tests.rs` | `test_native_arrow_temporal_types` |
| Binary to Arrow interval types | Integration | `tests/native_protocol_tests.rs` | `test_native_arrow_interval_types` |
| Binary to Arrow binary/geometry types | Integration | `tests/native_protocol_tests.rs` | `test_native_arrow_binary_types` |
| Boolean type conversion | Integration | `tests/native_protocol_tests.rs` | `test_native_arrow_boolean` |
| NULL handling in column data | Integration | `tests/native_protocol_tests.rs` | `test_native_null_handling` |
| Small result set (complete in one message) | Integration | `tests/native_protocol_tests.rs` | `test_native_small_result_set` |
| Large result set (multi-fetch) | Integration | `tests/native_protocol_tests.rs` | `test_native_large_result_set` |
| Row count result | Integration | `tests/native_protocol_tests.rs` | `test_native_row_count` |
| Empty result | Integration | `tests/native_protocol_tests.rs` | `test_native_empty_result` |
| Native protocol password encryption | Integration | `tests/native_protocol_tests.rs` | `test_native_password_encryption` |
| Binary attribute serialization | Unit | `src/transport/native/attributes.rs` | `test_attribute_roundtrip` |

#### Functional tests — native transport (`tests/integration_tests.rs`, default feature = `native`)

These are the existing integration tests that now run against the native transport by default:

| Scenario | Test Location | Test Name |
|----------|---------------|-----------|
| Connection succeeds with valid credentials | `tests/integration_tests.rs` | `test_connection_succeeds_with_valid_credentials` |
| Connection fails with invalid credentials | `tests/integration_tests.rs` | `test_connection_fails_with_invalid_credentials` |
| Connection closure and cleanup | `tests/integration_tests.rs` | `test_connection_closure_and_cleanup` |
| Connection health check | `tests/integration_tests.rs` | `test_connection_health_check` |
| SELECT from DUAL | `tests/integration_tests.rs` | `test_select_from_dual` |
| Arrow RecordBatch schema and data | `tests/integration_tests.rs` | `test_arrow_recordbatch_schema_and_data` |
| Arithmetic expressions | `tests/integration_tests.rs` | `test_arithmetic_expressions` |
| String data and UTF-8 | `tests/integration_tests.rs` | `test_string_data_and_utf8` |
| CREATE SCHEMA | `tests/integration_tests.rs` | `test_create_schema` |
| CREATE TABLE various types | `tests/integration_tests.rs` | `test_create_table_various_types` |
| DROP TABLE | `tests/integration_tests.rs` | `test_drop_table` |
| DROP SCHEMA | `tests/integration_tests.rs` | `test_drop_schema` |
| INSERT single row | `tests/integration_tests.rs` | `test_insert_single_row` |
| INSERT multiple rows | `tests/integration_tests.rs` | `test_insert_multiple_rows` |
| SELECT inserted data | `tests/integration_tests.rs` | `test_select_inserted_data` |
| UPDATE row | `tests/integration_tests.rs` | `test_update_row` |
| DELETE row | `tests/integration_tests.rs` | `test_delete_row` |
| Transaction begin | `tests/integration_tests.rs` | `test_transaction_begin` |
| Transaction commit | `tests/integration_tests.rs` | `test_transaction_commit` |
| Transaction rollback | `tests/integration_tests.rs` | `test_transaction_rollback` |
| Auto-commit behavior | `tests/integration_tests.rs` | `test_auto_commit_behavior` |
| INTEGER to Arrow Int64 | `tests/integration_tests.rs` | `test_integer_to_arrow_int64` |
| VARCHAR to Arrow Utf8 | `tests/integration_tests.rs` | `test_varchar_to_arrow_utf8` |
| DECIMAL type conversion | `tests/integration_tests.rs` | `test_decimal_type_conversion` |
| NULL value handling | `tests/integration_tests.rs` | `test_null_value_handling` |
| DATE/TIMESTAMP conversion | `tests/integration_tests.rs` | `test_date_timestamp_conversion` |
| BOOLEAN type conversion | `tests/integration_tests.rs` | `test_boolean_type_conversion` |
| DOUBLE type conversion | `tests/integration_tests.rs` | `test_double_type_conversion` |
| Large result set (1000 rows) | `tests/integration_tests.rs` | `test_large_result_set` |
| Empty result set | `tests/integration_tests.rs` | `test_empty_result_set` |
| Prepared statement lifecycle | `tests/integration_tests.rs` | `test_prepared_statement_lifecycle` |
| Prepared statement with parameters | `tests/integration_tests.rs` | `test_prepared_statement_with_parameters` |
| Prepared SELECT with parameters | `tests/integration_tests.rs` | `test_prepared_select_with_parameters` |
| Prepared statement parameter types | `tests/integration_tests.rs` | `test_prepared_statement_parameter_types` |
| Large result set exceeds frame limit | `tests/integration_tests.rs` | `test_large_result_set_exceeds_default_frame_limit` |
| Connect with wrong fingerprint fails | `tests/integration_tests.rs` | `test_connect_with_wrong_fingerprint_fails` |
| Connect with certificate fingerprint | `tests/integration_tests.rs` | `test_connect_with_certificate_fingerprint` |

#### Functional tests — WebSocket transport (`tests/websocket_integration_tests.rs`, forces `transport=websocket`)

Mirrors all functional tests above with explicit `transport=websocket`:

| Scenario | Test Location | Test Name |
|----------|---------------|-----------|
| Connection succeeds with valid credentials | `tests/websocket_integration_tests.rs` | `test_ws_connection_succeeds_with_valid_credentials` |
| Connection fails with invalid credentials | `tests/websocket_integration_tests.rs` | `test_ws_connection_fails_with_invalid_credentials` |
| Connection closure and cleanup | `tests/websocket_integration_tests.rs` | `test_ws_connection_closure_and_cleanup` |
| Connection health check | `tests/websocket_integration_tests.rs` | `test_ws_connection_health_check` |
| SELECT from DUAL | `tests/websocket_integration_tests.rs` | `test_ws_select_from_dual` |
| Arrow RecordBatch schema and data | `tests/websocket_integration_tests.rs` | `test_ws_arrow_recordbatch_schema_and_data` |
| Arithmetic expressions | `tests/websocket_integration_tests.rs` | `test_ws_arithmetic_expressions` |
| String data and UTF-8 | `tests/websocket_integration_tests.rs` | `test_ws_string_data_and_utf8` |
| CREATE SCHEMA | `tests/websocket_integration_tests.rs` | `test_ws_create_schema` |
| CREATE TABLE various types | `tests/websocket_integration_tests.rs` | `test_ws_create_table_various_types` |
| DROP TABLE | `tests/websocket_integration_tests.rs` | `test_ws_drop_table` |
| DROP SCHEMA | `tests/websocket_integration_tests.rs` | `test_ws_drop_schema` |
| INSERT single row | `tests/websocket_integration_tests.rs` | `test_ws_insert_single_row` |
| INSERT multiple rows | `tests/websocket_integration_tests.rs` | `test_ws_insert_multiple_rows` |
| SELECT inserted data | `tests/websocket_integration_tests.rs` | `test_ws_select_inserted_data` |
| UPDATE row | `tests/websocket_integration_tests.rs` | `test_ws_update_row` |
| DELETE row | `tests/websocket_integration_tests.rs` | `test_ws_delete_row` |
| Transaction begin | `tests/websocket_integration_tests.rs` | `test_ws_transaction_begin` |
| Transaction commit | `tests/websocket_integration_tests.rs` | `test_ws_transaction_commit` |
| Transaction rollback | `tests/websocket_integration_tests.rs` | `test_ws_transaction_rollback` |
| Auto-commit behavior | `tests/websocket_integration_tests.rs` | `test_ws_auto_commit_behavior` |
| INTEGER to Arrow Int64 | `tests/websocket_integration_tests.rs` | `test_ws_integer_to_arrow_int64` |
| VARCHAR to Arrow Utf8 | `tests/websocket_integration_tests.rs` | `test_ws_varchar_to_arrow_utf8` |
| DECIMAL type conversion | `tests/websocket_integration_tests.rs` | `test_ws_decimal_type_conversion` |
| NULL value handling | `tests/websocket_integration_tests.rs` | `test_ws_null_value_handling` |
| DATE/TIMESTAMP conversion | `tests/websocket_integration_tests.rs` | `test_ws_date_timestamp_conversion` |
| BOOLEAN type conversion | `tests/websocket_integration_tests.rs` | `test_ws_boolean_type_conversion` |
| DOUBLE type conversion | `tests/websocket_integration_tests.rs` | `test_ws_double_type_conversion` |
| Large result set (1000 rows) | `tests/websocket_integration_tests.rs` | `test_ws_large_result_set` |
| Empty result set | `tests/websocket_integration_tests.rs` | `test_ws_empty_result_set` |
| Prepared statement lifecycle | `tests/websocket_integration_tests.rs` | `test_ws_prepared_statement_lifecycle` |
| Prepared statement with parameters | `tests/websocket_integration_tests.rs` | `test_ws_prepared_statement_with_parameters` |
| Prepared SELECT with parameters | `tests/websocket_integration_tests.rs` | `test_ws_prepared_select_with_parameters` |
| Prepared statement parameter types | `tests/websocket_integration_tests.rs` | `test_ws_prepared_statement_parameter_types` |
| Large result set exceeds frame limit | `tests/websocket_integration_tests.rs` | `test_ws_large_result_set_exceeds_default_frame_limit` |
| Connect with wrong fingerprint fails | `tests/websocket_integration_tests.rs` | `test_ws_connect_with_wrong_fingerprint_fails` |
| Connect with certificate fingerprint | `tests/websocket_integration_tests.rs` | `test_ws_connect_with_certificate_fingerprint` |

#### Transport selection tests

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Default native transport | Integration | `tests/native_protocol_tests.rs` | `test_default_native_transport` |
| WebSocket transport via feature flag | Integration | `tests/websocket_integration_tests.rs` | `test_ws_transport_selected` |
| Connection string transport override | Integration | `tests/native_protocol_tests.rs` | `test_transport_override_websocket` |
| No transport feature enabled | Unit | `src/transport/mod.rs` | compile-time check (`compile_error!` macro) |
| ChaCha20 key exchange during login | Integration | `tests/native_protocol_tests.rs` | `test_native_chacha20_key_exchange` |
| Native protocol password encryption | Integration | `tests/native_protocol_tests.rs` | `test_native_password_encryption` |
| Set session attributes | Integration | `tests/native_protocol_tests.rs` | `test_native_set_attributes` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Native TCP protocol | `cargo run --example basic_usage` | Connects via native TCP, executes `SELECT 1`, prints Arrow RecordBatch |
| ChaCha20 encryption | `cargo run --example basic_usage` (with trace logging) | Log shows "ChaCha20 encryption enabled" after handshake |
| Binary result to Arrow | `cargo run --example basic_usage` with a query returning multiple types | RecordBatch contains correct Arrow types for DECIMAL, VARCHAR, TIMESTAMP, etc. |
| Transport selection (WS) | `cargo run --example basic_usage --no-default-features --features websocket` | Connects via WebSocket, same output |
| Transport override | `cargo run --example basic_usage` with `transport=websocket` in connection string | Connects via WebSocket when both features enabled |
| Native protocol auth | `cargo run --example basic_usage` with wrong password | Returns authentication error, connection closed |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Build (WebSocket only) | `cargo build --no-default-features --features websocket` | Exit 0 |
| Build (all features) | `cargo build --all-features` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Integration tests (native, default) | `cargo test --test integration_tests` | 0 failures |
| Native protocol tests | `cargo test --test native_protocol_tests` | 0 failures |
| WebSocket integration tests | `cargo test --test websocket_integration_tests --features websocket` | 0 failures |
| Driver manager tests | `cargo test --test driver_manager_tests` | 0 failures |
| Import/export tests | `cargo test --test import_export_tests -- --ignored` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 errors/warnings |
| Format | `cargo fmt --all -- --check` | No changes |
