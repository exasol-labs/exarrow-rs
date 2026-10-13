# Verification report: fix-timestamp-bind

Implemented directly in one session without the sub-agent loop.

## Done
- TIMESTAMP parameter column header carries precision 9 (wire types 21, 124, 125).
- `parse_date_text` and `parse_timestamp_text` in `src/types/conversion.rs`; the native encoder rejects text outside the accepted forms, and writes NULL for empty text.
- Unit tests for the header, ranges, accepted forms, out-of-range fields, rejected forms, and the formatter round trip (1650 lib tests pass).
- Integration: `test_prepared_temporal_text_parameters` (native, fails on the old encoder, passes now), `test_ws_prepared_temporal_text_parameters`, `test_bind_arrow_temporal_values_round_trip` (driver manager, native).
- Docs, CHANGELOG 0.20.2, version 0.20.2 (PR #91 still open).
- fmt clean, clippy `--all-targets --all-features -W clippy::all` zero warnings, websocket-only test build compiles.

## Not done
- Shared fixture in `tests/common/mod.rs` (task 1.5) and `test_ws_ffi_bind_arrow_temporal_values_round_trip` (task 1.9): the Arrow round trip is covered on the native transport only.
- Full integration suites run in CI, not locally.
