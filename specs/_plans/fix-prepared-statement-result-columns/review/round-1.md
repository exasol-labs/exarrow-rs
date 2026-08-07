# Plan Review Findings: fix-prepared-statement-result-columns (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 19 (Blockers: 6, Advisory: 13)
- Intent Fidelity blockers: 0

## Premortem

Three ways this plan fails six months out:

1. **The WebSocket half ships unverified.** Task 7's four unit tests live in `src/transport/websocket.rs`, which CI never executes — the `unit-tests` job runs `cargo llvm-cov --lib` with default features (`native` only) and the integration job only compiles the websocket target with `--no-run`. Both `websocket-client/protocol` deltas are recorded as verified while nothing in CI ever asserts them. Routed to Requirement Quality `[TRACEABILITY_GAP]`.
2. **The outbound wire byte breaks parameter binding for someone else's server build.** Task 1 changes the vcFlag the driver *sends* from `0x81` to `0x11` on the strength of an unverified claim ("bit `0x80` is unused by the server"), and the only test mapped to that normative requirement computes its expected byte from the same two constants production code uses — so it cannot fail. Routed to Feasibility `[UNSTATED_ASSUMPTION]` and Requirement Quality `[TRACEABILITY_GAP]`.
3. **Implementation stalls mid-plan with a red suite.** Task 3 flips `parse_handle_only_at` to the new variant while the six tests that pin the old shape (task 4) and the native call site (task 5) are a later parallel group, so `cargo test --lib` and every prepared-statement integration test are red at the Group B → C boundary with no task able to close them. Routed to Task Breakdown `[TASK_GRANULARITY]`.

## Intent Fidelity

Verified: the "reachable from `Connection`/`PreparedStatement` internals" half of the interview answer is already satisfied by existing code — `src/query/prepared.rs:20` stores the whole `PreparedStatementHandle` and exposes it via `pub(crate) fn handle_ref()` at `:54` — so decision 4's "no accessor needed" holds. "Plumbing only, no export-path wiring" is honored (plan.md § Design, Non-Goals). Both transports are covered (tasks 5 and 6). No artifact references `SPIKE_60.md`; `grep -rn "SPIKE" specs/_plans/fix-prepared-statement-result-columns/` returns nothing, so the committed plan has no dependency on the untracked spike file.

#### [SCOPE_CREEP] ADVISORY
- Location: plan.md § Implementation Tasks task 1; decision-log.md § Design Decisions [2]
- Issue: the interview authorized the `IS_VARCHAR` **decode** fix. Task 1 additionally changes a byte the driver **sends**: `src/transport/native/mod.rs:420` writes `IS_VARCHAR | IS_UTF8` on every outbound `T_CHAR` parameter column header, so the plan takes it from `0x81` to `0x11` — dropping bit `0x80` *and* setting bit `0x10`, a bit the driver has never sent. Decision [2]'s alternatives list only "fix `IS_VARCHAR` only" and "delete `IS_UTF8`". It omits the zero-wire-risk option: set `IS_VARCHAR = 0x01` and `IS_UTF8 = 0x10` (the same constant cleanup), but write `IS_VARCHAR` alone at line 420. By the plan's own premise that bit `0x80` is ignored, that byte (`0x01`) is semantically identical to what the server receives today, so it delivers the constant fix with no change in server-visible behavior.
- Fix: Add that third alternative to decision-log.md § Design Decisions [2] § Alternatives and state why it is rejected, or adopt it as the decision and rewrite task 1, the `native-client/protocol` delta scenario "Outbound vcFlag on a CHAR parameter column header", and plan.md § Impact to specify `0x01`.

#### [SCOPE_CREEP] ADVISORY
- Location: plan.md § Implementation Tasks task 2
- Issue: task 2 adds `pub fn with_result_columns(...) -> Self`. The interview constrained the plan to "no new public method/accessor beyond what's needed for internal reachability and testability". Public visibility is not needed: the field is `pub`, `PreparedStatementHandle` has no `#[non_exhaustive]` (`src/transport/protocol.rs:102-112`), so any external `TransportProtocol` implementor can populate it by struct literal or functional update, and every in-crate caller is inside the crate.
- Fix: Change task 2 in plan.md to specify `pub(crate) fn with_result_columns`, and note in decision-log.md [4] that the public field alone satisfies the interview's testability carve-out.

## Feasibility

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: plan.md § Implementation Tasks task 9; plan.md § Verification § Checklist
- Issue: `tests/websocket_integration_tests.rs` opens with `#![cfg(feature = "websocket")]` and today names no native type. `pub mod native` is `#[cfg(feature = "native")]` (`src/transport/mod.rs:45-46`), and `native` is not implied by `websocket` (`Cargo.toml § [features]`). CI runs `cargo test --no-default-features --features websocket --tests --no-run` (`.github/workflows/ci.yml`, step "Check websocket-only test build"), which compiles that file with `native` off. Task 9 says only "asserting the same result columns for the same SQL text over the WebSocket transport", and plan.md § Manual Testing says "WebSocket column names and Exasol type names equal the native ones" — so whether the test opens a `NativeTcpTransport` for comparison is undetermined. If it does, that CI step fails to compile. The plan's Checklist cannot catch it: it lists `cargo build --no-default-features --features websocket`, which builds the library only, not the test targets.
- Fix: Rewrite task 9 in plan.md to state that the test asserts hard-coded expected column names and Exasol type names identical to the ones task 8 asserts, and references no `native`-gated type; if any native reference is kept, require it be wrapped in `#[cfg(feature = "native")]`. Add the row `| Test build (websocket-only) | cargo test --no-default-features --features websocket --tests --no-run | Exit 0 |` to plan.md § Verification § Checklist.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Impact ("Bit `0x80` is unused by the server, so `0x81` and `0x01` mean the same thing to it"); plan.md § Implementation Tasks tasks 1 and 10; plan.md § Parallelization
- Issue: the spike verified only the inbound direction (Exasol *sends* `0x11` for UTF-8 `VARCHAR`, `0x10` for `CHAR`). Two server-side claims carry the outbound change and neither is verified: that bit `0x80` is ignored, and that setting bit `0x10` does not change how the server reads the `maxLen`/`octetLen` that follow. The guard is task 10, in **Group D** — three groups after task 1 in Group A. Task 10 ends "Report the result to task 1 before its fallback is or is not taken", a Group D → Group A dependency absent from § Parallelization § Sequential dependencies, which lists only A → B → C → D → E. Task 1 therefore reports complete while its guard has not run.
- Fix: Merge task 10 into task 1 in plan.md § Implementation Tasks so the wire-byte change and its non-ASCII round-trip guard land as one task in Group A, delete task 10 from Group D in § Parallelization, and add a line to § Dependencies stating that "bit `0x80` is ignored by the server" is an unverified assumption whose guard is that test.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Implementation Tasks task 3
- Issue: `parse_handle_only_at` is reachable from both `parse_response` (`src/transport/native/result_parser.rs:126`) and `parse_legacy_response_body` (`:206-208`), i.e. from *any* command's reply, not only `CMD_CREATE_PREPARED`. Today an `R_HANDLE` part with no sub-result yields `NativeResponse::ResultSet { columns: [], total_rows: 0 }`, which `native_result_to_query_result` turns into an empty `QueryResult::ResultSet` — reached by `execute_query` (`src/transport/native/mod.rs:979`) and `execute_prepared_statement` (`:1174`), both via `convert_and_cache_result`. After task 3 the same reply becomes `ProtocolError("Unexpected response type")`. The plan treats that rejection as desired without stating the assumption that no command other than `CMD_CREATE_PREPARED` ever replies with `R_HANDLE`.
- Fix: Add that assumption explicitly to plan.md § Design § Consequences as a row, and add a sentence to task 3 requiring the implementer to confirm no `CMD_EXECUTE`, `CMD_EXECUTE_PREPARED`, or `CMD_FETCH2` path can receive an `R_HANDLE` part before converting the empty-result behavior into an error.

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md § Implementation Tasks task 8
- Issue: task 8 hides four steps every other test in `tests/integration_tests.rs` performs and the cited pattern does not. `tests/native_transport_smoke_test.rs` hardcodes `"localhost"`/`8563`/`sys`/`exasol` and never creates a schema, so copying it gives a test that (a) ignores `EXASOL_HOST`/`EXASOL_PORT`/`EXASOL_USER`/`EXASOL_PASSWORD`, which CI sets in `env:`; (b) omits `skip_if_no_exasol!()` (`tests/common/mod.rs:296`), so a local `cargo test --test integration_tests` without Exasol hard-fails instead of skipping; (c) has no current schema, so a bare `CREATE TABLE T` fails — the test must issue `CREATE SCHEMA`/`OPEN SCHEMA` or fully qualify; (d) leaves the schema behind, where `test_prepared_select_with_parameters` (`tests/integration_tests.rs:1869-1934`) uses `generate_test_schema_name()` and `DROP SCHEMA … CASCADE`.
- Fix: Extend task 8 in plan.md to require `skip_if_no_exasol!()`, `common::get_host()`/`get_port()`/`get_user()`/`get_password()` instead of literals, a unique schema from `generate_test_schema_name()` with qualified table names, and `DROP SCHEMA … CASCADE` cleanup. Apply the same list to tasks 9 and 10.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: `specs/_plans/fix-prepared-statement-result-columns/prepared-statements/binding-and-execution/spec.md` § Scenario: Result-set column metadata for a derived select list
- Issue: "the unaliased derived column SHALL carry the name Exasol assigned it rather than an empty name or a positional placeholder" is normative for both transports, but the spike observed derived names (`UPPER(T.NAME)`) on the WebSocket JSON. Nothing in the plan shows a *native* `R_HANDLE` sub-result carrying a non-empty derived name — and the native wire trace quoted in plan.md § Design § Context shows the `PARAMETER_DESCRIPTION` sub-result arriving with names `""`, so empty names are demonstrably a shape this parser sees. If native returns `""` for `UPPER(NAME)`, this requirement is false and task 8's assertion fails.
- Fix: Add a step to task 8 in plan.md requiring the derived-name behavior be observed on the native transport first, and if it differs from WebSocket, weaken the delta step to "SHALL carry the name the transport reports, without substituting an empty name or a positional placeholder" and delete the cross-transport identity claim for derived columns.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: `specs/_plans/fix-prepared-statement-result-columns/native-client/protocol/spec.md` § Scenario: Outbound vcFlag on a CHAR parameter column header; plan.md § Implementation Tasks task 1
- Issue: the delta states "the system SHALL write a vcFlag byte of `0x11`" and "`0x11` SHALL be the varchar bit `0x01` combined with the UTF-8 bit `0x10`". Task 1's own fallback ships a different byte: "**Fallback if task 10 shows Exasol rejecting `0x11`:** keep `IS_VARCHAR = 0x01`, write `IS_VARCHAR` alone at line 420" — which writes `0x01`. The delta carries no contingency, and `/speq:record` would merge a normative requirement the shipped code violates. plan.md § Parallelization covers the changelog against this (Group D → E, "the changelog entry states verified behavior") but not the spec delta.
- Fix: Append to task 1's fallback in plan.md: "and rewrite the `native-client/protocol` delta scenario *Outbound vcFlag on a CHAR parameter column header* to require the varchar bit `0x01` alone, deleting the UTF-8-bit step." Add the same contingency sentence to plan.md § Impact next to "The outbound vcFlag on `T_CHAR` parameter column headers changes from `0x81` to `0x11`."

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Verification § Scenario Coverage, row "native-client/protocol — Outbound vcFlag on a CHAR parameter column header"; plan.md § Implementation Tasks task 1
- Issue: that scenario's normative content is the literal byte `0x11`, and its only mapped test is `prepared_payload_interleaves_parameter_values_row_by_row`. That test builds its expectation as `expected.push(IS_VARCHAR | IS_UTF8);` (`src/transport/native/mod.rs:1941`) from the same two constants the production write uses at `:420`, so it passes for **any** pair of constant values and cannot fail if the requirement is violated. Task 1 deliberately preserves the tautology: "the `varchar_meta_bytes` helper and the `mod.rs:1941` expectation both build their bytes from the constants and stay correct." By the plan's own standard in decision-log [5] — a test that "looks like coverage" is worse than none — this scenario is unverified.
- Fix: Add to task 1 in plan.md: "replace `expected.push(IS_VARCHAR | IS_UTF8);` at `src/transport/native/mod.rs:1941` with `expected.push(0x11u8);` so the outbound byte is pinned independently of the constants." State in § Verification that this row's assertion is the literal byte.

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Verification § Scenario Coverage, rows "websocket-client/protocol — Create prepared statement response carries result-set column metadata" and "… without a result set"; plan.md § Implementation Tasks task 7
- Issue: both `websocket-client/protocol` deltas are verified solely by unit tests placed in `src/transport/websocket.rs`, which CI never executes. The `unit-tests` job runs `cargo llvm-cov --lib --lcov …` with no `--features`, so default features apply and `websocket` is off (`Cargo.toml`: `default = ["native"]`); the integration job's only websocket-unit step is `cargo test --no-default-features --features websocket --tests --no-run`, which compiles without running. AGENTS.md states the consequence outright: "`src/transport/websocket.rs` has 26 unit tests that never run in CI today." Decision-log [5] rejects exactly this placement for the native test — "would pass locally and never run in CI, which is worse than no test because it looks like coverage" — yet task 7 adopts it without acknowledging the same defect. The "without a result set" scenario (`rowCount` entry, absent `results`, absent `columns`) then has zero CI-executed coverage, since task 9's parity test exercises a `SELECT` only.
- Fix: Rewrite tasks 6 and 7 in plan.md to put the extraction helper and its four unit tests in `src/transport/messages.rs`, where `ResultSetInfo` (`:303`) and `CreatePreparedStatementResponse` (`:601`) are defined, `mod tests` is ungated (`:950`), and an equivalent `CreatePreparedStatementResponse` JSON test already runs under default features (`:1239`). Have `WebSocketTransport::create_prepared_statement` call it. Update the two § Verification rows to name `src/transport/messages.rs` as the test location.

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: `specs/_plans/fix-prepared-statement-result-columns/prepared-statements/binding-and-execution/spec.md` § Scenario: Row-count-producing statements report no result-set columns, final AND
- Issue: three defects in one step. (a) The rationale is false and contradicts the plan: the delta says "Exasol classifies **every non-`SELECT`** statement as row-count-producing", while plan.md § Design § Consequences says "only `SELECT` and `DESCRIBE` are the former" — `DESCRIBE` is a non-`SELECT` counterexample. (b) The step is normative for `EXPORT` and `IMPORT`, but § Verification § Scenario Coverage maps the whole scenario only to `test_prepared_result_columns_native_row_count_statements`, which task 8 scopes to `INSERT`/`DELETE` — so the `EXPORT`/`IMPORT` obligation has no implementing test. (c) plan.md § Design § Non-Goals declares `EXPORT`/`IMPORT` out of scope, so the delta requires what the plan excludes.
- Fix: In that delta scenario, change "every non-`SELECT` statement" to "every statement other than `SELECT` and `DESCRIBE`", and either delete the `EXPORT`/`IMPORT` clause or extend task 8 in plan.md to prepare `EXPORT (SELECT ID FROM T WHERE ID = ?) INTO CSV AT …` and assert one parameter and zero result columns — then add the matching § Verification row.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: `specs/_plans/fix-prepared-statement-result-columns/native-client/result-sets/spec.md` § Scenario: VARCHAR is distinguished from CHAR by the vcFlag varchar bit
- Issue: the GIVEN scopes the scenario to "a column of type `T_char` (10)", but the third AND reasons about a different type: "a vcFlag byte of `0x00`, which Exasol sends for `HASHTYPE`". `T_HASHTYPE` is 126 (`src/transport/native/constants.rs:156`), not 10. No pass/fail test follows from a `HASHTYPE` premise under a `T_char` GIVEN, and task 1's unit test in fact drives `0x00` through a `T_char` column.
- Fix: In that delta scenario, replace "which Exasol sends for `HASHTYPE`" with "for a `T_char` column with the varchar bit clear" so the step matches its GIVEN and task 1's test.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Implementation Tasks task 6; `specs/_plans/fix-prepared-statement-result-columns/websocket-client/protocol/spec.md` § Scenario: Create prepared statement response carries result-set column metadata
- Issue: task 6 says "select entries whose `result_type == "resultSet"`, take `result_set.columns`" and the delta says "take column metadata from the `resultSet.columns` of entries whose `resultType` equals `"resultSet"`" — both plural, neither stating the behavior when `responseData.results` holds more than one such entry. Concatenate, take the first, or error? `PreparedStatementResponseData` also carries `num_results` (`src/transport/messages.rs:619`), which the plan never mentions. Not testable as written.
- Fix: State in task 6 and in that delta scenario that the helper takes the columns of the **first** `"resultSet"` entry and ignores any later entry, because a `createPreparedStatement` reply describes exactly one result set.

#### [COMPLETENESS_GAP] ADVISORY
- Location: `specs/_plans/fix-prepared-statement-result-columns/native-client/protocol/spec.md` § Scenario: Sub-result classification in a prepared statement reply
- Issue: the scenario says "the system SHALL classify every other result-set sub-result as the result-set column description", singular in effect but plural in wording, with no requirement for two or more non-`PARAMETER_DESCRIPTION` sub-results. Today's loop (`src/transport/native/result_parser.rs:309-324`) silently overwrites, keeping the last. Task 3 says only "keeping today's classification", so the outcome stays unspecified and unpinned.
- Fix: Add an AND to that delta scenario stating the retained description when more than one non-`PARAMETER_DESCRIPTION` result-set sub-result is present (last wins, matching today's loop), and add an assertion for it to the rewritten `handle_with_both_sub_results_keeps_parameters_and_result_columns` test in task 4.

## Task Breakdown

#### [TASK_GRANULARITY] BLOCKER
- Location: plan.md § Implementation Tasks tasks 3, 4, 5; plan.md § Parallelization (Group B = 3, 6; Group C = 4, 5, 7)
- Issue: task 3 cannot be verified as one unit. Once `parse_handle_only_at` returns `NativeResponse::PreparedStatement`, all six pinned tests still destructure `NativeResponse::ResultSet` with an `other => panic!` arm (`src/transport/native/result_parser.rs:1969`, `2094`, `2119`, `2143`, `2170`, `2251`), so they compile and fail; and `create_prepared_statement` still matches `NativeResponse::ResultSet` at `src/transport/native/mod.rs:1114`, so every prepare falls to the catch-all at `:1151` and returns `ProtocolError("Unexpected response from CREATE PREPARED")`. The rewrites are tasks 4 and 5, one group later. `cargo test --lib` and every prepared-statement integration test are therefore red for the whole Group B → C interval, with no task in Group B able to close them. Task 1 sets the standard the plan then abandons: "Verify `cargo test --lib` is green before moving on."
- Fix: Merge tasks 3, 4, and 5 into a single `[expert]` task in plan.md § Implementation Tasks — variant, six test rewrites, and native call site together — ending with "Verify `cargo test --lib` is green." Update § Parallelization so Group B is that merged task plus task 6, and Group C is task 7 alone.

#### [TASK_GRANULARITY] ADVISORY
- Location: plan.md § Parallelization, Group D
- Issue: Group D lists tasks 8, 9, 10 as parallel, but tasks 8 and 10 both add tests to `tests/integration_tests.rs`. Two agents editing one file concurrently is not independent work; the same table's own Sequential-dependencies list gives no ordering between them.
- Fix: Split Group D into "Group D — task 8, then task 10 (both edit `tests/integration_tests.rs`)" and "Group D′ — task 9", or note the shared file and sequence 8 → 10 in § Parallelization.

## Design Depth

The new `NativeResponse::PreparedStatement` variant is the right call and passes the Quick Diagnostic: the back-door leakage is real and verified — `src/transport/native/result_parser.rs:326-336` writes the convention "`total_rows` is the sub-result handle" and `src/transport/native/mod.rs:1117-1123` reads it back with nothing enforcing agreement — and the variant gives sub-result classification exactly one owner. The parser keeps its own vocabulary (`Vec<NativeColumnMeta>`), with conversion to `ColumnInfo` staying in `native/mod.rs`, so the dependency still points inward. No `[SHALLOW_DESIGN]` or `[BOUNDARY_VIOLATION]` objection: `PreparedStatementHandle` stays a data carrier and no business logic gains a transport dependency. No `[TACTICAL_SHORTCUT]` objection: the `pub mod result_parser` overexposure is named and scheduled, not silently deferred — plan.md § Dead Code Removal closes with "**Follow-up:** open an issue to narrow `native::result_parser` and `native::constants` visibility before 1.0", and decision-log [6] records the reasoning.

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md § Implementation Tasks task 6; plan.md § Design § Patterns, row "Pure extraction function"
- Issue: the `resultType` discriminator already has an owner-in-practice at `src/transport/websocket.rs:175-204`, where `match result.result_type.as_str()` dispatches on `"resultSet"` and `"rowCount"`. Task 6 adds a second site in the same file that re-derives the same decision from the same strings, while the field itself belongs to `ResultSetInfo` in `src/transport/messages.rs:303`. Two places now decide what a `"resultSet"` entry is; changing that decision means editing both.
- Fix: Rewrite task 6 in plan.md to add the discriminator as a method on `ResultSetInfo` in `src/transport/messages.rs` (for example `fn result_set_columns(&self) -> &[ColumnInfo]`), have both `create_prepared_statement` and the existing `websocket.rs:175` dispatch read the decision from that one place, and note the consolidation in § Design § Patterns. This is the same relocation finding `[TRACEABILITY_GAP]` on task 7 requires, so make both changes together.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: decision-log.md § Design Decisions [2] § Rationale; plan.md § Design, Non-Goals bullet; plan.md § Summary
- Issue: decision [2]'s Rationale is one ~200-word paragraph carrying five separate ideas (the constant collision, the outbound OR, the spike's `0x11` evidence, bit `0x80`'s inertness, the fallback), with several sentences past the 25-word cap — against "One idea per paragraph" and "Cap sentences at 25 words". The Non-Goals bullet is a single ~75-word semicolon chain holding four unrelated exclusions. § Summary's first sentence runs 31 words against the same cap.
- Fix: Split decision [2]'s Rationale into one paragraph per idea. Break plan.md § Design's Non-Goals bullet into four bullets, one exclusion each. Cut § Summary's first sentence to under 25 words.

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Impact
- Issue: two unclear phrases. "Three breaking changes to public surface, all additive in intent" pairs "breaking" with "additive" without saying what makes them additive. "`0x11` additionally declares UTF-8, which the driver's payloads have always been" strands the relative pronoun — "which" reads as referring to `0x11`, and a payload cannot "be" a declaration.
- Fix: In plan.md § Impact, replace the first with "Three breaking changes to public surface; none removes an existing item." Replace the second with "`0x11` additionally sets the UTF-8 bit; the driver has always encoded parameter payloads as UTF-8."
