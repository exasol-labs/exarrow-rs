# Verification Report: fix-prepared-statement-result-columns

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Both transports now decode and surface result-set column metadata from `createPreparedStatement` replies (fixes #60); the native vcFlag VARCHAR/CHAR mask is corrected; all checklist and scenario-coverage items pass against a live Exasol container; version bumped to 0.17.0 |
| Code review | 7 findings — standard: 6 fixed, expert: 1 fixed |

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
| Unit (production-only, stripped) | 86.12% (floor 80.0%) |
| Per-file floor | min file 57.94% (`src/import/csv.rs`), floor 50.0% — no exemptions needed |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (default features) | `cargo test --lib` | 1571 | 0 |
| Unit (websocket-only) | `cargo test --no-default-features --features websocket --lib` | 1388 | 0 |
| Integration | `cargo test --test integration_tests -- --test-threads=1` | 69 | 4 (pre-existing) |
| Native protocol | `cargo test --test native_protocol_tests -- --test-threads=1` | 12 | 0 |
| WebSocket parity | `cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 44 | 0 |
| Driver manager | `cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 42 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `cargo test --test integration_tests test_prepared_result_columns_native -- --nocapture --test-threads=1` (3 tests) | ✓ — `ID`/`DECIMAL` and `NAME`/`VARCHAR` for parameterized SELECT; 0 result columns for INSERT/DELETE |
| `cargo test --lib varchar_flag_bit_discriminates_varchar_from_char -- --nocapture` | ✓ — `0x11` → `VARCHAR`, `0x10` → `CHAR` |
| `cargo test --lib prepare_response -- --nocapture` (5 tests) | ✓ — `rowCount` case reports 0 columns without error |
| `cargo test --features 'ffi websocket' --test websocket_integration_tests test_prepared_result_columns_websocket_matches_native -- --nocapture --test-threads=1` | ✓ — WebSocket column names/types equal native |
| `cargo test --test integration_tests test_prepared_non_ascii_varchar_parameter_round_trip -- --nocapture --test-threads=1` | ✓ — non-ASCII string returns byte-identical with outbound vcFlag `0x11` |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -W clippy::all
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.77s
(0 warnings)
```

### Formatter

```
cargo fmt --all -- --check
(exit 0; only pre-existing nightly-feature notices, no diff)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| native-client | protocol | Prepared statement lifecycle | `tests/integration_tests.rs` | `test_prepared_result_columns_native_parameterized_select` | Pass |
| native-client | protocol | Sub-result classification in a prepared statement reply | `src/transport/native/result_parser.rs` | `handle_with_both_sub_results_keeps_parameters_and_result_columns` | Pass |
| native-client | protocol | Prepared statement reply for a row-count-producing statement | `tests/integration_tests.rs` | `test_prepared_result_columns_native_row_count_statements` | Pass |
| native-client | protocol | Outbound vcFlag on a CHAR parameter column header | `src/transport/native/mod.rs` | `prepared_payload_interleaves_parameter_values_row_by_row` | Pass |
| native-client | result-sets | Column metadata parsing | `src/transport/native/result_parser.rs` | `varchar_flag_bit_discriminates_varchar_from_char` | Pass |
| native-client | result-sets | VARCHAR distinguished from CHAR by vcFlag bit (bit values) | `src/transport/native/result_parser.rs` | `varchar_flag_bit_discriminates_varchar_from_char` | Pass |
| native-client | result-sets | VARCHAR distinguished from CHAR by vcFlag bit (transport parity) | `tests/websocket_integration_tests.rs` | `test_prepared_result_columns_websocket_matches_native` | Pass |
| native-client | result-sets | Direct binary to Arrow conversion for string types | `src/transport/native/result_parser.rs` | `single_pass_string_like_types_all_decode_as_utf8` | Pass |
| native-client | result-sets | Prepared-statement sub-results carry both descriptions | `src/transport/native/result_parser.rs` | `handle_with_both_sub_results_keeps_parameters_and_result_columns`, `legacy_handle_without_sub_result_reports_no_columns`, `handle_sub_result_that_is_not_a_result_set_leaves_no_columns` | Pass |
| native-client | result-sets | Exception sub-result rejected, not swallowed *(added by code review, expert fix)* | `src/transport/native/result_parser.rs` | `handle_sub_result_carrying_an_exception_is_rejected` | Pass |
| native-client | result-sets | Warning sub-result propagated, not discarded *(added by code review, expert fix)* | `src/transport/native/result_parser.rs` | `handle_sub_result_warning_is_propagated` | Pass |
| websocket-client | protocol | Create prepared statement response carries result-set column metadata | `src/transport/messages.rs` | `prepare_response_result_set_columns_are_read`, `prepare_response_with_two_result_sets_takes_the_first` | Pass |
| websocket-client | protocol | Create prepared statement response without a result set | `src/transport/messages.rs` | `prepare_response_without_a_result_set_yields_no_columns`, `prepare_response_with_absent_results_yields_no_columns`, `prepare_response_result_set_without_columns_yields_no_columns` | Pass |
| prepared-statements | binding-and-execution | Prepared statement creation | `tests/integration_tests.rs` | `test_prepared_result_columns_native_parameterized_select` | Pass |
| prepared-statements | binding-and-execution | Result-set column metadata for a parameterized SELECT (native) | `tests/integration_tests.rs` | `test_prepared_result_columns_native_parameterized_select` | Pass |
| prepared-statements | binding-and-execution | Result-set column metadata for a parameterized SELECT (transport parity) | `tests/websocket_integration_tests.rs` | `test_prepared_result_columns_websocket_matches_native` | Pass |
| prepared-statements | binding-and-execution | Result-set column metadata for a derived select list | `tests/integration_tests.rs` | `test_prepared_result_columns_native_derived_select_list` | Pass |
| prepared-statements | binding-and-execution | Row-count-producing statements report no result-set columns | `tests/integration_tests.rs` | `test_prepared_result_columns_native_row_count_statements` | Pass |
| prepared-statements | binding-and-execution | Non-ASCII VARCHAR parameter round-trip | `tests/integration_tests.rs` | `test_prepared_non_ascii_varchar_parameter_round_trip` | Pass |

## Notes

- **No fallback triggered.** Task 1's outbound-vcFlag contingency (reverting to `0x01` alone if Exasol rejected `0x11`) was tested against a live server by mutating the byte across `0x00`/`0x01`/`0x10`/`0x11` — only `0x00` was rejected (`too large character string type`, SQLSTATE 40001). `0x11` ships as planned; the `native-client/protocol` delta scenario needed no rewrite.
- **Plan assumption corrected.** Plan § Dependencies flagged "whether bit `0x10` changes how the server reads `maxLen`/`octetLen`" as unverified; it is now verified to have no effect — `0x01`, `0x10`, and `0x11` all round-trip identically. The old `0x81` worked by accident (`0x80 | 0x01` happened to set the real varchar bit).
- **Native derived-column shape confirmed, no spec weakening needed.** The plan anticipated the native transport might report an empty name for a derived select-list column (as `PARAMETER_DESCRIPTION` sub-results do) and pre-authorized weakening the delta step if so. Verified against a live server: native reports the same derived name WebSocket does (`UPPER(T.NAME)`), so the delta step stands as written. One correction from the plan's own worked example: `ID*2` widens `DECIMAL(18,0)` to **precision 19**, not 18 — both the derived-select test and this report use 19.
- **Code review surfaced and fixed one genuine defect beyond the plan's original scope.** The expert finding identified that `parse_handle_only_at`'s pre-existing (and, after task 3's rewrite, newly-explicit) handling of non-result-set sub-results would silently swallow a server exception carried in a prepared-statement reply sub-result, reporting a successful preparation with empty parameter/result-column metadata instead of surfacing the error. Fixed: an `Exception` sub-result now returns `Err(ProtocolError)` naming the server message and SQL state; sub-result warnings are now propagated instead of discarded. Two new unit tests pin this; two steps were added to the `native-client/result-sets` spec delta's *Prepared-statement sub-results carry both descriptions* scenario to make the boundary explicit for the recorder.
- **Two error messages hardened during review.** The catch-all match arms in `native_result_to_query_result` and `fetch_results` (both now reachable from the new `NativeResponse::PreparedStatement` variant when a query result was expected instead) previously returned bare four-word strings with no indication of what actually arrived. Both now interpolate the received variant via `{other:?}`.
- **Test duplication and gaps closed during review.** The three native result-column integration tests were de-duplicated behind two shared helpers (`connect_native_with_test_table`/`drop_native_schema`); the derived-select test now matches the plan's zero-parameter statement shape exactly and asserts `num_params == 0`; the non-ASCII round-trip test is now pinned to the native transport explicitly (`#[cfg(feature = "native")]` + `get_test_connection_with_transport("native")`) rather than relying on native being the ambient default feature, and no longer discards the `CREATE SCHEMA` result.
- **Breaking changes recorded.** `Cargo.toml` bumped to `0.17.0`; `CHANGELOG.md` documents all three breaking changes (`PreparedStatementHandle` new field, `NativeResponse` new variant, `IS_VARCHAR`/`IS_UTF8` value changes) and both fixes (VARCHAR/CHAR type-name correction, result-column metadata now surfaced).
- **Deliberately out of scope**, per plan § Non-Goals: no new public accessor on `PreparedStatement`, no ADBC-facing API, no export-path wiring (issue #58's concern), and `pub mod result_parser`/`native::constants` visibility narrowing was left as a follow-up issue rather than folded into this plan.
