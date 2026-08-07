# Decisions: fix-prepared-statement-result-columns

## ADR: Replace the `total_rows`-as-sentinel encoding with a `NativeResponse::PreparedStatement` variant

**ID:** native-response-prepared-statement-variant
**Plan:** fix-prepared-statement-result-columns
**Status:** Accepted

### Context

Issue #60 reports that `create_prepared_statement` discards result-set column metadata the server already sends. `parse_handle_only_at` decoded both sub-results of a prepared-statement reply's `R_HANDLE` part into locals, then collapsed them with `parameter_description.or(result_columns)` because the flat `NativeResponse::ResultSet` variant it returned could carry only one column list. The surviving sub-result's handle was also smuggled out through that variant's `total_rows` field, which `create_prepared_statement` read back and compared against `PARAMETER_DESCRIPTION`. `result_parser.rs` and `native/mod.rs` therefore shared an unenforced convention — when a `ResultSet` came from an `R_HANDLE` part, `total_rows` was not a row count — with nothing enforcing agreement between the two modules.

### Decision

Add `NativeResponse::PreparedStatement { handle: i32, parameters: Vec<NativeColumnMeta>, result_columns: Vec<NativeColumnMeta> }` and return it unconditionally from `parse_handle_only_at`, replacing both the `.or()` collapse and the `total_rows` sentinel.

### Options Considered

| Option | Verdict |
|--------|---------|
| New `NativeResponse::PreparedStatement` variant carrying both column lists | ✓ Chosen — removes the shared convention outright; every other `match` on `NativeResponse` already has a catch-all arm that correctly rejects a prepare reply where a query result was expected |
| Add a `result_columns` field to the existing `ResultSet` variant | ✗ Rejected — keeps `total_rows` meaning two different things depending on provenance and adds an always-empty field to every ordinary result set |
| Add an explicit sub-result-kind field alongside the sentinel | ✗ Rejected — names the convention but still leaves it spread across two modules |

### Consequences

Sub-result classification has exactly one owner, `parse_handle_only_at`, and `total_rows` means one thing again on the `ResultSet` variant. The change is breaking: `native::result_parser` is `pub mod`, so external exhaustive `match` on `NativeResponse` no longer compiles.

## ADR: Put the native result-columns integration tests in `integration_tests.rs`, not `native_transport_smoke_test.rs`

**ID:** native-integration-tests-in-integration-tests-rs
**Plan:** fix-prepared-statement-result-columns
**Status:** Accepted

### Context

`native_transport_smoke_test.rs` is the natural home by subject matter for a native-transport test and already contains the direct-transport pattern this test copies. CI, however, runs only `integration_tests`, `websocket_integration_tests`, and `driver_manager_tests`.

### Decision

Add the native result-columns tests to `tests/integration_tests.rs`, opening a dedicated `NativeTcpTransport` inside them, gated with `#[cfg(feature = "native")]` because that file carries no file-level feature gate and CI's *Check websocket-only test build* step compiles it with `native` off.

### Options Considered

| Option | Verdict |
|--------|---------|
| `tests/integration_tests.rs` | ✓ Chosen — CI runs this suite on every PR |
| `tests/native_transport_smoke_test.rs` | ✗ Rejected — a test placed there would pass locally and never run in CI, which is worse than no test because it looks like coverage |

### Consequences

Adding a CI step for `native_transport_smoke_test.rs` was the alternative fix and stayed out of scope for this plan.

## ADR: Give the `resultType` discriminator one owner in `src/transport/messages.rs`

**ID:** result-type-discriminator-single-owner
**Plan:** fix-prepared-statement-result-columns
**Status:** Accepted

### Context

An early revision of this plan's WebSocket task added a second `"resultSet"` string-matching decision inside `src/transport/websocket.rs`, alongside the existing dispatch at `src/transport/websocket.rs:174`, even though the field the decision reads belongs to `ResultSetInfo` in `src/transport/messages.rs`. Plan review flagged this as information leakage: the `"resultSet"`/`"rowCount"` strings would have had two decision sites instead of one.

### Decision

Add `pub enum ResultEntryKind { ResultSet, RowCount, Unknown }` and `pub fn kind(&self) -> ResultEntryKind` on `ResultSetInfo` in `src/transport/messages.rs`, plus `pub fn result_set_columns(&self) -> Vec<ColumnInfo>` on `PreparedStatementResponseData`, built on `kind()`. Rewrite the existing dispatch in `websocket.rs` to match on `kind()` instead of the raw strings, so `"resultSet"` and `"rowCount"` appear in exactly one `match`.

### Options Considered

| Option | Verdict |
|--------|---------|
| One owner (`ResultSetInfo::kind` in `messages.rs`), `pub` visibility | ✓ Chosen — one edit changes the decision; `pub` because the native path never calls these methods, so `pub(crate)` would trip `dead_code` in a default-features `cargo build` |
| Leave the second decision site in `websocket.rs` | ✗ Rejected — the field's owning module and its decision site would diverge, and the strings would need to change in two places |
| Gate the methods behind `#[cfg(any(feature = "websocket", test))]` to justify `pub(crate)` | ✗ Rejected — adds a cfg for no benefit over plain `pub` |

### Consequences

`src/transport/messages.rs`'s `mod tests` is ungated, so the discriminator's unit tests run under default features in the `unit-tests` job — unlike anything placed in `websocket.rs`, whose unit tests CI never executes (AGENTS.md § Coverage Measurement).
