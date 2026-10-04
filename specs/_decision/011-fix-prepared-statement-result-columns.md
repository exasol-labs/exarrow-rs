# Decisions: fix-prepared-statement-result-columns

## ADR: Replace the `total_rows`-as-sentinel encoding with a `NativeResponse::PreparedStatement` variant

**ID:** native-response-prepared-statement-variant
**Plan:** fix-prepared-statement-result-columns
**Status:** Accepted

### Context

A prepared-statement reply carries two sub-results in its `R_HANDLE` part: parameter description and result columns. `parse_handle_only_at` returned the flat `NativeResponse::ResultSet`, which holds one column list. It kept one with `parameter_description.or(result_columns)` and dropped the result-column metadata (issue #60). It also passed the handle through `total_rows`, which `create_prepared_statement` compared against `PARAMETER_DESCRIPTION`. `result_parser.rs` and `native/mod.rs` shared this unenforced convention.

### Decision

`NativeResponse::PreparedStatement { handle: i32, parameters: Vec<NativeColumnMeta>, result_columns: Vec<NativeColumnMeta> }` is the unconditional return of `parse_handle_only_at`. It replaces both the `.or()` collapse and the `total_rows` sentinel.

### Options Considered

| Option | Verdict |
|--------|---------|
| New `PreparedStatement` variant with both column lists | ✓ Chosen. It removes the shared convention, and existing catch-all arms already reject it where a query result is expected |
| `result_columns` field on `ResultSet` | ✗ Rejected. `total_rows` keeps two meanings and every result set gains an empty field |
| Explicit sub-result-kind field beside the sentinel | ✗ Rejected. The convention stays spread across two modules |

### Consequences

`parse_handle_only_at` alone classifies sub-results, and `total_rows` always means a row count. The change is breaking: `native::result_parser` is `pub mod`, so external exhaustive matches on `NativeResponse` stop compiling.
