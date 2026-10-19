# Decision Log: fix-driver-gaps

## Design Decisions

### [1] Per-value steps return a crate-private error, and only the per-column step builds the public variant

- **Decision:** The per-value steps of both JSON-to-Arrow paths return a crate-private error that holds the row, the reason, and an overflow flag. The per-column step converts it into `ConversionError::ValueConversionFailed` or `ConversionError::NumericOverflow` through one crate-private method that adds the column index, the column name, the failing JSON value, and the column's Arrow data type. On the `ArrowConverter` path the per-column step is `build_array`. On the WebSocket result path it is `ResultSet::column_major_to_record_batch`. No other code builds either public variant.
- **Rationale:** Only the per-column step knows the column name, the Arrow data type, and the column's values. Neither transport calls `ArrowConverter`, so a fix limited to `src/arrow_conversion/builders.rs` would leave every query error on the WebSocket transport without the column. The native transport decodes binary result data and raises no `ConversionError`, so the cited scenario, whose GIVEN is JSON result data, does not apply to it. The cited scenario also requires the failing value and the attempted conversion: `NumericOverflow` carries neither, and several parse messages carry only part of the value, so the per-column step adds the full JSON value and the Arrow data type.
- **Alternatives:** Let the per-value steps return the public variants with empty context fields that the per-column step fills. Rejected: `error.rs`, `builders.rs`, and `results.rs` would all follow a two-step construction that the type system does not enforce. Pass the column name into every private builder and parse helper in `builders.rs`. Rejected: every helper would carry the column context, which one per-column step can attach. Change only the existing producers in `builders.rs`. Rejected: no conversion error that a driver user receives from a query would name the column.
- **Consequences:** The `column` parameter of the per-value helpers and the `build_*_array` functions in `builders.rs` becomes unused and is removed, so about 140 test calls lose one argument.
- **Promotes to ADR:** no

### [2] A login response without `ATTR_PROTOCOL_VERSION` keeps counting as the client's maximum version

- **Decision:** The native login rejects only a reported protocol version below 14. A login response without the attribute keeps the current fallback to the client's maximum version (21). The check runs before the driver builds the password message.
- **Rationale:** The request names versions below 14. Rejecting a missing attribute would fail the login to any server that omits it, and this plan has no evidence about which servers omit it. Checking before the password message keeps the encrypted password away from a server the driver refuses.
- **Alternatives:** Reject a missing attribute as malformed input, as the login does for a missing public key. Rejected for the reason above.
- **Consequences:** A reported version above the client's maximum stays accepted, as today, and the "Protocol version negotiation" delta states it. The deltas state no requirement for a missing attribute, because the recorded "Login handshake" scenario lists `ATTR_PROTOCOL_VERSION` in every login response. Task 1.4 keeps the fallback and test 1.3 guards it.
- **Promotes to ADR:** no

### [3] Only `Session` gains `supports_at_least`, and the free function stays

- **Decision:** `Session` memoizes the parsed release version and answers `supports_at_least(major, minor, patch)` and `supports_native_parquet_import()` from it. `Connection` gains no `supports_at_least` method. The free function `connection::version::supports_at_least(parsed, major, minor)` stays unchanged.
- **Rationale:** The requester chose the session method only (2026-10-19). A `Connection` method and the removal of the free function are out of scope by that choice.
- **Alternatives:** Add `Connection::supports_at_least`, delegating to the session as `Connection::supports_native_parquet_import` does. Out of scope by the requester's choice. Remove the free function, which compares only the major and minor components and has no caller. Out of scope by the requester's choice.
- **Consequences:** A driver user who holds an `adbc::Connection` cannot reach `Session::supports_at_least`, because `Connection` has no accessor for its `Session`. The free function keeps its two-component comparison.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] The two new scenarios go beyond "deltas only for scenarios that exist on main"

- **Finding:** Round 1 `[SCOPE_CREEP]`, escalated to a human: the plan adds the DELTA:NEW scenarios "Server protocol version below 14 is rejected before the password is sent" and "Compare the server version against a caller-supplied threshold", although the request limits spec deltas to scenarios that exist on main.
- **Direction change:** The requester approved both new scenarios on 2026-10-19 (human decision). Both DELTA:NEW blocks stay. The threshold scenario covers `Session` only, per entry [3].
- **Promotes to ADR:** no
