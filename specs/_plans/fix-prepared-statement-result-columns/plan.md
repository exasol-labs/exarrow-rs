# Plan: fix-prepared-statement-result-columns

## Summary

Both transports receive result-set column metadata in the `createPreparedStatement` response, then discard it; this plan stores it on `PreparedStatementHandle`. It also corrects the native vcFlag bitmasks, which report every native `VARCHAR` as `CHAR`.

## Design

### Context

Issue #60 reports that `create_prepared_statement` discards result-set column metadata. A spike verified against `exasol/docker-db` (Exasol 8, cos-8.66.1) that the server sends that metadata in the same response as the parameter metadata, on both transports. No extra round-trip, protocol change, catalog lookup, or probe query is needed. The metadata is decoded correctly and then dropped.

Each transport drops it for a different reason.

**Native.** `parse_handle_only_at` decodes both sub-results of the `R_HANDLE` part into locals, then collapses them with `parameter_description.or(result_columns)` because the flat `NativeResponse::ResultSet` variant it must return can carry only one column list. It also smuggles the surviving sub-result's handle out through the `total_rows` field, which `create_prepared_statement` reads back and compares against `PARAMETER_DESCRIPTION`. Two modules therefore share an unenforced convention: *when this `ResultSet` came from an `R_HANDLE` part, `total_rows` is not a row count.* That is back-door leakage, and it is also the structural cause of the discard.

Observed native wire shape for `SELECT ID, NAME FROM T WHERE ID = ? AND NAME = ?`:

```
02 09 00 00 00                 R_HANDLE, stmt_handle = 9
01 fb ff ff ff 02 00 00 00 ..  sub-result handle = -5 (PARAMETER_DESCRIPTION), 2 cols, names ""
01 fd ff ff ff 02 00 00 00 ..  sub-result handle = -3 (SMALL_RESULTSET),       2 cols, "ID"/"NAME"
```

**WebSocket.** `responseData.results[]` already deserializes into `Vec<ResultSetInfo>` and is simply never read. `create_prepared_statement` reads `parameter_data` only.

**Second defect, same code path.** `IS_VARCHAR` is `0x80`. Exasol sends vcFlag `0x11` for `VARCHAR(20)` and `0x10` for `CHAR(10)`, so the varchar bit is `0x01` and `0x10` marks UTF-8. The mask never matches, `parse_column_meta` sets `is_varchar = false` on every string column, and `native_meta_to_data_type` maps every native `VARCHAR` to `CHAR`. This affects all native result-set metadata, not only prepared statements; the executed-query path shows the identical defect. WebSocket is unaffected because it reads the type name from JSON, and that divergence is what exposed the bug.

`IS_UTF8` is `0x01` — the same value the corrected `IS_VARCHAR` needs. The two constants are also OR-ed together and written to the wire at `src/transport/native/mod.rs:420` as the vcFlag of every outbound `T_CHAR` parameter column header. Fixing `IS_VARCHAR` alone would leave two differently-named constants holding one value and turn that OR into a silent no-op.

- **Goals** — decode result-set column metadata on both transports and store it on `PreparedStatementHandle`; give the native prepare reply a response shape that cannot drop either description; correct both vcFlag bitmasks; pin all of it with tests, including against a real Exasol.
- **Non-Goals**
  - No new public accessor on `PreparedStatement` and no ADBC-facing API.
  - No export-path wiring. Issue #58 consumes this metadata later.
  - No attempt to obtain result-set columns for `EXPORT`, `IMPORT`, or DML. Exasol computes none (see Consequences).
  - No narrowing of `pub mod result_parser`, the deeper fix for the leak this plan works around (see Dead Code Removal).

### Decision

Replace the sentinel encoding with a dedicated `NativeResponse` variant. Reach the WebSocket result columns through pure helpers on the message types in `src/transport/messages.rs`, which are testable without a socket and without the `websocket` feature.

#### Architecture

```
NATIVE                                         WEBSOCKET
 R_HANDLE part                                  createPreparedStatement JSON
      │                                              │
      ▼                                              ▼
 parse_handle_only_at                           PreparedStatementResponseData
   ├─ sub-result handle == -5 → parameters        ├─ parameter_data ──────┐
   └─ any other result-set   → result_columns     └─ results[] ──┐        │
      │                                                          ▼        │
      ▼                                             result_set_columns()  │
 NativeResponse::PreparedStatement                  in messages.rs, over  │
   { handle, parameters, result_columns }           ResultSetInfo::kind   │
      │                                                         │         │
      ▼                                                         ▼         ▼
 create_prepared_statement                            create_prepared_statement
   parameters     → native_meta_to_data_type              │
   result_columns → to_column_info                        │
      │                                                    │
      └──────────────► PreparedStatementHandle ◄───────────┘
                         .with_result_columns(Vec<ColumnInfo>)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Distinct enum variant per response shape | `NativeResponse::PreparedStatement` | A prepare reply carries two column lists and no row data; the existing `ResultSet` variant can represent only one, which is why metadata was discarded |
| Additive builder | `PreparedStatementHandle::with_result_columns` | Keeps `new()`'s signature, so none of the 4 production and 20-plus test call sites change |
| Single owner for the `resultType` discriminator | `ResultSetInfo::kind` and `PreparedStatementResponseData::result_set_columns` in `src/transport/messages.rs` | The `"resultSet"`/`"rowCount"` strings otherwise get a second decision site next to the existing dispatch at `src/transport/websocket.rs:174`. One owner means one edit to change the decision. The helpers are also pure, so their tests live in an ungated `mod tests` and run under default features — unlike anything placed in `websocket.rs`, which CI never executes |
| Parser speaks its own vocabulary | variant carries `Vec<NativeColumnMeta>` | Conversion to the shared `ColumnInfo` stays in `native/mod.rs`, so the parser keeps depending inward |

The new variant passes the design Quick Diagnostic on the two questions the current shape fails. *Would changing how a module works internally force an edit outside it?* Today yes — changing the sentinel encoding forces an edit in `native/mod.rs`. After, no. *Is there exactly one module that owns each significant design decision?* After, `parse_handle_only_at` alone owns sub-result classification.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| New `NativeResponse::PreparedStatement { handle, parameters, result_columns }` | Add a `result_columns` field to `ResultSet`; add an explicit sub-result-kind field alongside the sentinel | Both alternatives keep `total_rows` meaning two things by provenance and add an always-empty field to every result set. The variant removes the shared convention outright. Blast radius is small: every other `match` site on `NativeResponse` already has a catch-all arm that correctly rejects a prepare reply where a query result was expected |
| Fix `IS_VARCHAR` to `0x01` **and** `IS_UTF8` to `0x10`, so the outbound vcFlag becomes `0x11` | Fix `IS_VARCHAR` only, leaving `IS_UTF8` at `0x01`; fix both constants but write `IS_VARCHAR` alone (`0x01`) outbound | Fixing one constant leaves two constants with one value and makes `IS_VARCHAR \| IS_UTF8` at `native/mod.rs:420` a no-op OR. `0x11` is the byte Exasol itself sends for UTF-8 `VARCHAR`. Writing `0x01` alone rests on the same unverified "`0x80` is ignored" premise as `0x11` and gives up the UTF-8 declaration, so it is the fallback rather than the primary choice. Guarded by the non-ASCII parameter integration test in the same task |
| Reject an `R_HANDLE` part from any command other than `CMD_CREATE_PREPARED` | Keep today's behavior — yield an empty `QueryResult::ResultSet` | `parse_handle_only_at` is reachable from `parse_response` and `parse_legacy_response_body`, so *any* command's reply could in principle carry an `R_HANDLE` part. Assumption: none other than `CMD_CREATE_PREPARED` does. Task 3 must confirm that before converting today's silent empty result set into a protocol error |
| Store result columns as `Vec<ColumnInfo>` on `PreparedStatementHandle` | Expose a `result_columns()` accessor on `PreparedStatement` | `PreparedStatementHandle` is already publicly re-exported with public fields, so integration tests reach the metadata through the public `TransportProtocol` trait without any new accessor. #58 adds the consuming API |
| Report zero result columns for non-`SELECT` statements | Synthesize columns, or return an error | Exasol classifies a statement at compile time as result-set-producing or row-count-producing, and only `SELECT` and `DESCRIBE` are the former. For the latter the second part of the prepare reply is a row count, so no result table is built and no names or types exist server-side. Verified: preparing `EXPORT (SELECT ID FROM T WHERE ID = ?) INTO CSV AT …` returns 1 parameter and 0 result columns. #58 must therefore prepare the export *source* SELECT, not the `EXPORT` statement. That `EXPORT` observation is spike evidence for #58, not a requirement this plan states or tests — the delta obliges only the `INSERT`/`DELETE` statements task 6 covers |
| No delta to `type-mapping/exasol-to-arrow` | Add a VARCHAR/CHAR delta there | Both `VARCHAR` and `CHAR` already map to Arrow `Utf8`, so the mask bug changes no Arrow type. It corrupts only the reported Exasol type name, which is `native-client/result-sets` territory |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| native-client/protocol | CHANGED | `specs/_plans/fix-prepared-statement-result-columns/native-client/protocol/spec.md` |
| native-client/result-sets | CHANGED | `specs/_plans/fix-prepared-statement-result-columns/native-client/result-sets/spec.md` |
| websocket-client/protocol | CHANGED | `specs/_plans/fix-prepared-statement-result-columns/websocket-client/protocol/spec.md` |
| prepared-statements/binding-and-execution | CHANGED | `specs/_plans/fix-prepared-statement-result-columns/prepared-statements/binding-and-execution/spec.md` |

## Impact

Native-transport callers see corrected Exasol type names: a `VARCHAR(n)` column now reports `VARCHAR(n)` where it previously reported `CHAR(n)`. Code that branched on the reported type name over the native transport changes behavior, and the two transports now agree where they previously diverged. Arrow types are unaffected — both map to `Utf8`.

Three breaking changes to public surface; none removes an existing item:

- `PreparedStatementHandle` gains a public field, so struct-literal construction outside the crate no longer compiles. `new()` is unchanged.
- `NativeResponse` gains a variant, so external exhaustive `match` no longer compiles.
- `IS_VARCHAR` and `IS_UTF8` change value.

Two additive public methods appear on the wire-message types in `src/transport/messages.rs`: `ResultSetInfo::kind` and `PreparedStatementResponseData::result_set_columns`.

The outbound vcFlag on `T_CHAR` parameter column headers changes from `0x81` to `0x11`. Bit `0x80` carries no known server-side meaning, so `0x81` and `0x01` are expected to mean the same thing to the server. `0x11` additionally sets the UTF-8 bit; the driver has always encoded parameter payloads as UTF-8.

Contingency: if task 1's non-ASCII round-trip test shows Exasol rejecting `0x11`, the outbound byte becomes `0x01` instead. Task 1 then rewrites the `native-client/protocol` delta scenario *Outbound vcFlag on a CHAR parameter column header* to require the varchar bit alone.

No new method appears on `PreparedStatement`, `Connection`, or the ADBC surface.

## Dependencies

None. Integration tests need a running Exasol container:

```bash
docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest
```

Task 6's three tests and their `NativeTcpTransport` import MUST carry `#[cfg(feature = "native")]`, because `tests/integration_tests.rs` has no file-level feature gate and CI compiles it with `native` off in the *Check websocket-only test build* step.

Task 1's outbound wire-byte change rests on two unverified server-side claims. First, that bit `0x80` of the vcFlag is ignored. Second, that setting bit `0x10` does not change how the server reads the `maxLen`/`octetLen` that follow. The spike verified the inbound direction only. Their single guard is the non-ASCII round-trip integration test inside task 1, which is why that test is not deferred to a later group.

## Implementation Tasks

1. **Correct the native vcFlag bitmasks and guard the outbound byte.** [expert] Set `IS_VARCHAR = 0x01` and `IS_UTF8 = 0x10` in `src/transport/native/constants.rs`; update both doc comments to state the bit each mask selects. Add a unit test `varchar_flag_bit_discriminates_varchar_from_char` in `result_parser.rs` driving `parse_column_meta` from literal vcFlag bytes and asserting through `native_meta_to_data_type`: `0x11` → `VARCHAR`, `0x10` → `CHAR`, `0x00` → not `VARCHAR`. Replace `expected.push(IS_VARCHAR | IS_UTF8);` at `src/transport/native/mod.rs:1941` with `expected.push(0x11u8);`, so `prepared_payload_interleaves_parameter_values_row_by_row` pins the outbound byte independently of the two constants. Nothing pins `0x80` today, so no other existing test should fail: `varchar_meta_bytes` and the three `vc_extra` builders in `result_parser.rs` write the byte from the constants and read it back through the same mask. Verify `cargo test --lib` is green.
   Then add the guard for the outbound change in the same task: `test_prepared_non_ascii_varchar_parameter_round_trip` in `tests/integration_tests.rs`, in the `prepared_*` section. Open with `skip_if_no_exasol!()`, take a connection from `get_test_connection()`, create a unique schema from `generate_test_schema_name()`, create a qualified table with `NAME VARCHAR(50)`, prepare a qualified `INSERT` over the default native transport, bind a non-ASCII UTF-8 string, execute, select it back, assert it returns unchanged byte for byte, and finish with `DROP SCHEMA … CASCADE`.
   *Expert because* the same two constants are OR-ed and written outbound at `src/transport/native/mod.rs:420`, so this changes a wire byte the driver sends (`0x81` → `0x11`), not only a byte it reads. The round-trip test is that change's only guard, which is why it lands here rather than in a later group.
   **Fallback if the round-trip test shows Exasol rejecting `0x11`:** keep `IS_VARCHAR = 0x01`, write `IS_VARCHAR` alone at line 420, pin `mod.rs:1941` to `0x01u8`, document why — do not restore two constants holding one value — and rewrite the `native-client/protocol` delta scenario *Outbound vcFlag on a CHAR parameter column header* to require the varchar bit `0x01` alone, deleting the UTF-8-bit step.
2. **Add `result_columns` to `PreparedStatementHandle`.** In `src/transport/protocol.rs` add `pub result_columns: Vec<ColumnInfo>` with a doc comment stating it is empty for row-count-producing statements, initialize it empty in `new()` (signature unchanged), and add `pub(crate) fn with_result_columns(mut self, result_columns: Vec<ColumnInfo>) -> Self`. `pub(crate)` is deliberate: the field is `pub`, so integration tests read the metadata without the builder, and both callers — `native/mod.rs` and `websocket.rs` — are in-crate. Unit-test with `prepared_statement_handle_result_columns_default_empty` and `prepared_statement_handle_with_result_columns_sets_them`.
3. **Replace the sentinel with `NativeResponse::PreparedStatement`, rewrite its tests, and wire the native call site.** [expert] All three land together: the variant change on its own leaves `cargo test --lib` and every prepared-statement integration test red, because the six pinned tests destructure `NativeResponse::ResultSet` and `create_prepared_statement` still matches it.
   1. Add the variant `PreparedStatement { handle: i32, parameters: Vec<NativeColumnMeta>, result_columns: Vec<NativeColumnMeta> }` to `NativeResponse` with a doc comment explaining why a prepare reply is a distinct shape. Rewrite `parse_handle_only_at` to return it unconditionally, keeping today's classification (`sub_handle == PARAMETER_DESCRIPTION` → parameters, any other result-set sub-result → result columns, last one wins) and deleting the `.or()` collapse and the `total_rows`-as-sentinel comment.
   2. Before converting today's empty-`ResultSet` outcome into a protocol error, confirm no `CMD_EXECUTE`, `CMD_EXECUTE_PREPARED`, or `CMD_FETCH2` reply can carry an `R_HANDLE` part. `parse_handle_only_at` is reachable from both `parse_response` and `parse_legacy_response_body`, so an `R_HANDLE` part from any other command would now be rejected where it previously yielded an empty result set. If such a path exists, stop and report it.
   3. Confirm the catch-all arms in `native_result_to_query_result`, `fetch_results`, and `convert_and_cache_result` reject the new variant with a protocol error rather than silently mishandling it.
   4. Rewrite the six pinned `result_parser.rs` unit tests against the new variant. `envelope_with_a_handle_part_returns_the_statement_handle` and `legacy_handle_without_sub_result_reports_no_columns` assert the variant with both lists empty; `handle_sub_result_with_parameter_description_handle_describes_parameters` asserts parameters populated and result columns empty; `handle_sub_result_with_small_resultset_handle_describes_result_columns` asserts the reverse; `handle_sub_result_that_is_not_a_result_set_leaves_no_columns` asserts both empty. Rename `handle_with_both_sub_results_prefers_the_parameter_description` to `handle_with_both_sub_results_keeps_parameters_and_result_columns`, assert both descriptions survive with the right names, and give it a third sub-result so it also pins last-wins for two non-`PARAMETER_DESCRIPTION` result-set sub-results. Drop the `total_rows` assertions — the field no longer exists on this shape.
   5. In `src/transport/native/mod.rs`, match `NativeResponse::PreparedStatement`, build `param_types`/`param_names` from `parameters` exactly as today, and attach `Self::to_column_info(&result_columns)` via `with_result_columns()`. Delete the `sub_handle_indicator` branch and the discarding `else`. Keep the `NativeResponse::Empty` arm as-is.
   6. Verify `cargo test --lib` is green.
   *Expert because* it removes a convention shared across two modules, every `match` on a public enum must still behave correctly, and the variant, its tests, and its only production consumer have to change in one step to keep the suite green.
4. **Give the `resultType` discriminator one owner in `src/transport/messages.rs`.** Add `pub enum ResultEntryKind { ResultSet, RowCount, Unknown }` and `pub fn kind(&self) -> ResultEntryKind` on `ResultSetInfo`, so the strings `"resultSet"` and `"rowCount"` appear in exactly one `match`. Add `pub fn result_set_columns(&self) -> Vec<ColumnInfo>` on `PreparedStatementResponseData`, returning the `resultSet.columns` of the **first** entry whose kind is `ResultSet` in column order and ignoring any later entry, because a `createPreparedStatement` reply describes exactly one result set. Return an empty vector for an absent `results`, a `RowCount` entry, an absent `result_set`, or absent `columns`. Both methods are `pub`, not `pub(crate)`: nothing on the native path calls them, so `pub(crate)` would trip `dead_code` in a default-features `cargo build`.
   Unit-test in `messages.rs`'s existing ungated `mod tests`, driving `serde_json::from_str::<CreatePreparedStatementResponse>` from raw JSON in the style of `test_create_prepared_statement_response_deserialization`, for five cases: `prepare_response_result_set_columns_are_read` (a `"resultSet"` entry with two columns), `prepare_response_without_a_result_set_yields_no_columns` (a `"rowCount"` entry), `prepare_response_with_absent_results_yields_no_columns`, `prepare_response_result_set_without_columns_yields_no_columns`, and `prepare_response_with_two_result_sets_takes_the_first`. `ColumnInfo` and `DataType` do not derive `PartialEq` — assert `name`, `data_type.type_name`, and `data_type.size`/`precision`/`scale` field by field. These tests run under default features; anything placed in `websocket.rs` would not (see AGENTS.md § Coverage Measurement).
5. **Wire the WebSocket transport to that owner.** In `src/transport/websocket.rs`, have `create_prepared_statement` attach `response_data.result_set_columns()` via `with_result_columns()`. Rewrite the `match result.result_type.as_str()` dispatch in the query-response path (`src/transport/websocket.rs:174`) to `match result.kind()`, mapping `ResultEntryKind::ResultSet`, `RowCount`, and `Unknown` onto today's three arms, with the `Unknown` arm reading `result.result_type` for its error message. Behavior is unchanged; the two string literals leave `websocket.rs`.
6. **Add the native result-columns integration tests** to `tests/integration_tests.rs`, in the `prepared_*` section: `test_prepared_result_columns_native_parameterized_select`, `test_prepared_result_columns_native_derived_select_list`, and `test_prepared_result_columns_native_row_count_statements`. Each opens with `skip_if_no_exasol!()`, builds its `NativeTcpTransport` from `common::get_host()`, `get_port()`, `get_user()`, and `get_password()` rather than literals — `ConnectionParams::new(host, port).with_tls(true).with_validate_server_certificate(false)` — creates a unique schema from `generate_test_schema_name()`, qualifies every table name with that schema, and finishes with `DROP SCHEMA … CASCADE`. Gate the three tests and their `NativeTcpTransport` import with `#[cfg(feature = "native")]` — `tests/integration_tests.rs` carries no file-level feature gate, and CI compiles it with `native` off in the *Check websocket-only test build* step. The integration job runs `--features ffi`, which implies `native`, so the tests still execute there. Do not copy `tests/native_transport_smoke_test.rs` verbatim: it hardcodes `localhost`/`8563`/`sys`/`exasol`, omits `skip_if_no_exasol!()`, and creates no schema, so a local run without Exasol would hard-fail and a bare `CREATE TABLE T` would have no current schema.
   Create the table with `ID DECIMAL(18,0)` and `NAME VARCHAR(50)`, then assert `handle.result_columns` for a parameterized `SELECT` (two columns, `ID`/`DECIMAL` 18,0 and `NAME`/`VARCHAR` 50), for a derived select list (`SELECT ID*2 AS DOUBLED, UPPER(NAME) FROM T` → `DOUBLED` plus the derived column), and for `INSERT`/`DELETE` (zero result columns, parameters still reported). Call `close_prepared_statement` on every handle — each `create` allocates a server-side statement that otherwise leaks for the session.
   Before asserting the derived column, observe and report what the native transport actually names it. The spike saw a derived name (`UPPER(T.NAME)`) only on the WebSocket JSON, and the native `PARAMETER_DESCRIPTION` sub-result arrives with empty names, so an empty native name is a shape this parser is known to produce. If native reports an empty name, first weaken the `prepared-statements/binding-and-execution` delta step to "SHALL carry the name the transport reports, without substituting an empty name or a positional placeholder", then assert that.
   Place these tests in `integration_tests.rs`, not `native_transport_smoke_test.rs`, because CI runs only `integration_tests`, `websocket_integration_tests`, and `driver_manager_tests`.
7. **Add the WebSocket parity integration test** `test_prepared_result_columns_websocket_matches_native` to `tests/websocket_integration_tests.rs`. Prepare the same parameterized `SELECT` task 6 prepares — identical apart from the generated schema qualifier, against an identically defined table — over `WebSocketTransport`, and assert hard-coded expected column names and Exasol type names identical to the ones task 6 asserts — `ID`/`DECIMAL` 18,0 and `NAME`/`VARCHAR` 50. The test MUST NOT name any `native`-gated type: the file opens `#![cfg(feature = "websocket")]`, `websocket` does not imply `native` (`Cargo.toml § [features]`), and CI compiles this target with `native` off in the "Check websocket-only test build" step. If a native reference proves unavoidable, wrap it in `#[cfg(feature = "native")]`. Apply the same hygiene as task 6: `skip_if_no_exasol!()`, `common::get_host()`/`get_port()`/`get_user()`/`get_password()`, a unique schema from `generate_test_schema_name()` with qualified table names, and `DROP SCHEMA … CASCADE`. CI runs this suite with `--features 'ffi websocket'`.
8. **Bump the version and record it.** Set `version = "0.17.0"` in `Cargo.toml` and add a `## 0.17.0` section at the top of `CHANGELOG.md` with the three `Breaking:` entries and the two `Fix:` entries described in Impact, in the existing entry style.

## Parallelization

| Parallel Group | Tasks | Files touched |
|----------------|-------|---------------|
| Group A | 1, 2, 4 | 1: `native/constants.rs`, `native/result_parser.rs`, `native/mod.rs`, `tests/integration_tests.rs` · 2: `transport/protocol.rs` · 4: `transport/messages.rs` |
| Group B | 3, 5 | 3: `native/result_parser.rs`, `native/mod.rs` · 5: `transport/websocket.rs` |
| Group C | 6, 7 | 6: `tests/integration_tests.rs` · 7: `tests/websocket_integration_tests.rs` |
| Group D | 8 | `Cargo.toml`, `CHANGELOG.md` |

No two tasks in the same group edit the same file.

Sequential dependencies:

- Group A → Group B — task 3 edits `result_parser.rs` and `native/mod.rs`, which task 1 also touches; task 3 needs task 2's builder; task 5 needs task 2's builder and task 4's helper.
- Group B → Group C — the integration tests exercise the wired transports.
- Group C → Group D — the changelog states verified behavior.

Task 1 carries its own guard, so no group has a back-dependency on an earlier one.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Import | `PARAMETER_DESCRIPTION` in the `constants` use list at `src/transport/native/mod.rs:32` | The sentinel comparison that needed it is deleted by task 3. `SMALL_RESULTSET` in the same list stays — still used at lines 338, 699, 1587 |
| Code branch | the `else` arm of `create_prepared_statement`, `src/transport/native/mod.rs:1144-1147` | This is the discard reported by issue #60 |
| Comment | the `total_rows`-as-sentinel comments at `src/transport/native/mod.rs:1120-1122` and `src/transport/native/result_parser.rs:327-329` | They document an encoding that no longer exists |
| Code | `parameter_description.or(result_columns)` collapse in `parse_handle_only_at`, `src/transport/native/result_parser.rs:326` | Replaced by the variant carrying both |
| String literals | `"resultSet"` and `"rowCount"` in the dispatch at `src/transport/websocket.rs:174` | Task 5 moves the discriminator to `ResultSetInfo::kind`, so the strings live in one `match` in `src/transport/messages.rs` |

Not removed, deliberately: `pub mod result_parser` in `src/transport/native/mod.rs:7` publishes the parser's vocabulary — `NativeResponse`, `NativeColumnMeta` — outside the crate, which is why adding a variant is a breaking change at all. Narrowing it to `pub(crate)` is the correct fix and is a larger break than this plan should carry. **Follow-up:** open an issue to narrow `native::result_parser` and `native::constants` visibility before 1.0.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| native-client/protocol — Prepared statement lifecycle | Integration | `tests/integration_tests.rs` | `test_prepared_result_columns_native_parameterized_select` |
| native-client/protocol — Sub-result classification in a prepared statement reply | Unit | `src/transport/native/result_parser.rs` | `handle_with_both_sub_results_keeps_parameters_and_result_columns` |
| native-client/protocol — Prepared statement reply for a row-count-producing statement | Integration | `tests/integration_tests.rs` | `test_prepared_result_columns_native_row_count_statements` |
| native-client/protocol — Outbound vcFlag on a CHAR parameter column header | Unit | `src/transport/native/mod.rs` | `prepared_payload_interleaves_parameter_values_row_by_row` (asserts the literal byte `0x11`, per task 1) |
| native-client/result-sets — Column metadata parsing | Unit | `src/transport/native/result_parser.rs` | `varchar_flag_bit_discriminates_varchar_from_char` |
| native-client/result-sets — VARCHAR is distinguished from CHAR by the vcFlag varchar bit (bit values) | Unit | `src/transport/native/result_parser.rs` | `varchar_flag_bit_discriminates_varchar_from_char` |
| native-client/result-sets — VARCHAR is distinguished from CHAR by the vcFlag varchar bit (transport parity) | Integration | `tests/websocket_integration_tests.rs` | `test_prepared_result_columns_websocket_matches_native` |
| native-client/result-sets — Direct binary to Arrow conversion for string types | Unit | `src/transport/native/result_parser.rs` | `single_pass_string_like_types_all_decode_as_utf8` |
| native-client/result-sets — Prepared-statement sub-results carry both descriptions | Unit | `src/transport/native/result_parser.rs` | `handle_with_both_sub_results_keeps_parameters_and_result_columns`, `legacy_handle_without_sub_result_reports_no_columns`, `handle_sub_result_that_is_not_a_result_set_leaves_no_columns` |
| websocket-client/protocol — Create prepared statement response carries result-set column metadata | Unit | `src/transport/messages.rs` | `prepare_response_result_set_columns_are_read`, `prepare_response_with_two_result_sets_takes_the_first` |
| websocket-client/protocol — Create prepared statement response without a result set | Unit | `src/transport/messages.rs` | `prepare_response_without_a_result_set_yields_no_columns`, `prepare_response_with_absent_results_yields_no_columns`, `prepare_response_result_set_without_columns_yields_no_columns` |
| prepared-statements/binding-and-execution — Prepared statement creation | Integration | `tests/integration_tests.rs` | `test_prepared_result_columns_native_parameterized_select` |
| prepared-statements/binding-and-execution — Result-set column metadata for a parameterized SELECT (native) | Integration | `tests/integration_tests.rs` | `test_prepared_result_columns_native_parameterized_select` |
| prepared-statements/binding-and-execution — Result-set column metadata for a parameterized SELECT (transport parity) | Integration | `tests/websocket_integration_tests.rs` | `test_prepared_result_columns_websocket_matches_native` |
| prepared-statements/binding-and-execution — Result-set column metadata for a derived select list | Integration | `tests/integration_tests.rs` | `test_prepared_result_columns_native_derived_select_list` |
| prepared-statements/binding-and-execution — Row-count-producing statements report no result-set columns | Integration | `tests/integration_tests.rs` | `test_prepared_result_columns_native_row_count_statements` |
| prepared-statements/binding-and-execution — Non-ASCII VARCHAR parameter round-trip | Integration | `tests/integration_tests.rs` | `test_prepared_non_ascii_varchar_parameter_round_trip` |

Unit tests are used only where the behavior is pure decoding of a byte buffer or a JSON string with no I/O. Every scenario that names a live server is covered by an integration test. Two scenarios carry both a unit and an integration row because a single AND step in each — the vcFlag bit values and the native/WebSocket parity claim — is verifiable only against a live server.

Every unit test named above runs in CI. The two `websocket-client/protocol` rows sit in `src/transport/messages.rs`, whose `mod tests` is ungated, so the `unit-tests` job executes them under default features. `src/transport/websocket.rs` is not an acceptable location. CI never runs its unit tests (AGENTS.md § Coverage Measurement), so a test there would look like coverage and assert nothing.

The `Outbound vcFlag` row's assertion is the literal byte `0x11`, not `IS_VARCHAR | IS_UTF8`. Building the expectation from the same constants the production write uses would pass for any pair of values. It could not fail if the requirement were violated.

`PreparedStatementHandle::with_result_columns` (task 2) is covered by `prepared_statement_handle_result_columns_default_empty` and `prepared_statement_handle_with_result_columns_sets_them` in `src/transport/protocol.rs`. It carries no scenario of its own: it is the storage the scenarios above assert through.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| native-client/protocol | `cargo test --test integration_tests test_prepared_result_columns_native -- --nocapture --test-threads=1` | 3 tests pass; output shows `ID`/`DECIMAL` and `NAME`/`VARCHAR` for the parameterized SELECT and 0 result columns for INSERT and DELETE |
| native-client/result-sets | `cargo test --lib varchar_flag_bit_discriminates_varchar_from_char -- --nocapture` | 1 test passes; `0x11` reports `VARCHAR`, `0x10` reports `CHAR` |
| websocket-client/protocol | `cargo test --lib prepare_response -- --nocapture` | 5 tests pass under default features; the `rowCount` case reports 0 columns without error |
| prepared-statements/binding-and-execution | `cargo test --test websocket_integration_tests --features 'ffi websocket' test_prepared_result_columns_websocket_matches_native -- --nocapture --test-threads=1` | 1 test passes; WebSocket column names and Exasol type names equal the native ones |
| prepared-statements/binding-and-execution | `cargo test --test integration_tests test_prepared_non_ascii_varchar_parameter_round_trip -- --nocapture --test-threads=1` | 1 test passes; the non-ASCII string returns byte-identical |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build (websocket-only) | `cargo build --no-default-features --features websocket` | Exit 0 |
| Test build (websocket-only) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Build (FFI release) | `cargo build --release --features ffi` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Unit tests (websocket) | `cargo test --no-default-features --features websocket --lib` | 0 failures |
| Integration tests | `cargo test --test integration_tests -- --test-threads=1` | 0 failures |
| Native protocol tests | `cargo test --test native_protocol_tests -- --test-threads=1` | 0 failures |
| WebSocket parity tests | `cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `cargo build --release --features ffi && cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage floors | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Exit 0; total production coverage at or above 80.0%, no file below 50.0% |
