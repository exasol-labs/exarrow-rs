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
