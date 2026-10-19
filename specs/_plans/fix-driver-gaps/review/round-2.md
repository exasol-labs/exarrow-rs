# Plan Review Findings: fix-driver-gaps (round 2)

## Summary
- Axes checked: 6/6
- Total findings: 6 (Blockers: 0, Advisory: 6)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Round-1 Blocker Recheck
- Resolved: [SCOPE_CREEP] The two new scenarios go beyond "deltas only for scenarios that exist on main". Evidence: decision-log.md § Review Findings [1] records the requester's approval of both DELTA:NEW scenarios on 2026-10-19, and the orchestrator's User Intent confirms it ("The requester approved the two new scenarios"). Both blocks stay, and the threshold scenario covers `Session` only, as decision [3] states.

## Premortem

Six months from now this plan failed. These are the likely reasons:

1. The record step merged the `recordbatch-assembly` delta into a branch that still starts at b6f9395. Main had already merged PR 101, which deletes the scenarios around "Conversion error reporting", so the PR showed a conflict in that spec file. plan.md still called PR 101 open. Route: Feasibility, `[HIDDEN_DEPENDENCY]`.
2. A WebSocket user got NULL for the value `"yes"` in a BOOLEAN column and filed a bug that cites the new "Conversion error reporting" scenario, which requires an error whenever a JSON value fails to convert. Route: Requirement Quality, `[AMBIGUOUS_REQUIREMENT]`.
3. The implementer compared `protocol_version as u32` with `MIN_PROTOCOL_VERSION`, and a broken server that reported -1 passed the check. Route: Requirement Quality, `[COMPLETENESS_GAP]`.
4. A user of the public `build_array` passed a `Field` from another schema. The error then named `Decimal128(10, 2)`, but the parser had enforced DECIMAL(5,2). Route: Design Depth, `[INFORMATION_LEAKAGE]`.

## Intent Fidelity
[no objection — axis checked: each of the four asks has tasks. (1) Tasks 1.1-1.4 and the DELTA:NEW handshake scenario cover the rejection below 14. (2) Tasks 2.1-2.2 cover `Session::supports_at_least` returning `false` for an unparsable version. (3) Tasks 3.1-3.4 cover conversion errors that name the column. (4) Task 2.4 removes the keepalive fields and writes no delta. Decision [3] records the Session-only choice: no `Connection` method, and the free function stays unchanged. plan.md § Context bullet 21 excludes issues 102 and 103. The Context bullet on native decode errors states the narrower reading of "conversion errors", so the requester can see it.]

## Feasibility

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Context bullets 18 and 19 ("Open PR 101 removes ...", "Whichever of PR 101 and this plan's record step lands second rebases on the other")
- Issue: PR 101 is no longer open. `ghbrk gh pr view 101` reports `MERGED` at 2026-10-19T16:50:49Z as commit 57e68e9. Branch `feat/fix-driver-gaps` is at b6f9395, which is one commit behind main. This plan therefore lands second and must rebase, but plan.md still describes the order as undecided. PR 101 also deletes "Large batch handling" from `specs/arrow-conversion/recordbatch-assembly/spec.md`, and bullet 19 does not name that deletion. If the plan records on the old base, the edit to "Conversion error reporting" sits next to lines that main deleted, and the PR shows a conflict.
- Fix: In plan.md § Context, rewrite bullets 18 and 19 as follows. PR 101 is merged on main as 57e68e9. It removed "Heartbeat and keepalive" from `websocket-client/async-lifecycle` and removed "Large batch handling" and "Partial conversion handling" from `arrow-conversion/recordbatch-assembly`. It left "Conversion error reporting" and "Decimal overflow detection" unchanged. `feat/fix-driver-gaps` rebases onto main before implementation, so the record step merges the deltas into main's spec text.

## Requirement Quality

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: `specs/_plans/fix-driver-gaps/arrow-conversion/recordbatch-assembly/spec.md` DELTA:CHANGED "Conversion error reporting", WHEN step; plan.md § Features paragraph
- Issue: The WHEN step reads "converting a JSON value to the Arrow type of its column fails". On the WebSocket result path, `ResultSet::json_column_to_array` (`src/query/results.rs`) turns any non-DECIMAL value that does not fit the column type into NULL and raises no error. Its doc comment says: "A value that does not fit the target Arrow type becomes NULL instead of failing the batch". No spec and no plan line says that this NULL rule limits the scenario. A verifier can therefore send `"yes"` in a BOOLEAN column over WebSocket, receive NULL, and report that the scenario fails. Test 3.4 covers only a DECIMAL string.
- Fix: Add a plan.md § Context bullet: "On the WebSocket result path, a non-DECIMAL value that does not fit its column's Arrow type becomes NULL and raises no error, and this plan keeps that behavior. On that path, only a DECIMAL string that fails to convert raises an error under 'Conversion error reporting'."

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md task 1.4; tasks 1.2 and 1.3 (tested versions)
- Issue: Task 1.4 says "compare the negotiated version with `MIN_PROTOCOL_VERSION`" without naming the integer type. `attribute_integer` returns `i64`. `login_exchange` truncates the value with `version as i32`, and `MIN_PROTOCOL_VERSION` is a `u32` (`src/transport/native/constants.rs:8`). If the comparison casts the version to `u32`, a negative reported version passes as a large number. The tests check only 13 at the low end, so they would not catch that mistake. The DELTA:NEW scenario requires the rejection of every version below 14.
- Fix: In task 1.4, state that the check compares the attribute's `i64` value with `i64::from(MIN_PROTOCOL_VERSION)` before the `as i32` truncation. In task 1.2, also run the rejection check for a reported version of `-1`.

## Task Breakdown
[no objection — axis checked: each delta has implementing tasks: handshake 1.2-1.4, version-capability 2.1-2.3, and recordbatch-assembly 3.2-3.4. Task 2.4 removes the keepalive fields and has no delta, as requested. The source files of groups A, B, and C do not overlap. A uses `native/mod.rs`, `handshake.rs`, and `test_support.rs`. B uses `session.rs` and the `adbc/connection.rs` test module. C uses `error.rs`, `builders.rs`, `converter.rs`, and `results.rs`. Test 2.3 reuses `recording_transport`, which already takes `transport_session_info()`, so group B does not edit `test_support.rs`. The test call count in task 3.3 matches the code: 46 `build_*_array` calls, 85 `parse_*` calls, and 9 `validate_decimal_precision` calls make 140, plus 12 `build_array` calls and 2 variant matches. `SessionConfig` is built only in `src/adbc/connection.rs:211`, through `..Default::default()`, so task 2.4 breaks no other code. Group D runs after A, B, and C.]

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md task 3.3 ("Give `build_array` the column's Arrow `Field`"); § Impact bullet 4
- Issue: The public `arrow_conversion::build_array` already takes `exasol_type: &ExasolType`, and the plan adds the column's Arrow `Field`. The column type then arrives twice, as the Exasol type that drives parsing and as `field.data_type()`, which the error reports as `target_type`. Nothing makes the two agree. A caller outside `ArrowConverter` can pass a `Field` from another schema, and the error then names a type that the parser did not use. Inside the crate, `ArrowConverter` builds both from the same `ColumnInfo`, so only external callers of this public function hit the problem.
- Fix: In task 3.3 and § Impact, replace the `Field` parameter with the column name (`&str`). Derive the target Arrow type inside `build_array` from `exasol_type` with `TypeMapper::exasol_to_arrow`, which `ResultSet::exasol_datatype_to_arrow` already uses.

[ADR and architecture checks: no objection — axis checked. No decision-log entry has `Promotes to ADR: yes`. The seven accepted ADRs cover query timeout, client give-up, zero-row schema, the placeholder lexer, the Parquet export schema, batch arity, and the export timer. None covers protocol version, version capability, keepalive, or conversion errors. Task 3.4 leaves the zero-row branch of `column_major_to_record_batch` unchanged. plan.md § Context states "Architecture: no delta" and gives a reason. `specs/architecture.md` § Components line 36 still holds: the connection component checks the server version for features and still answers native Parquet support. Line 46 still calls `arrow_conversion` a utility with no caller inside the crate. The crate-private error type lives in `error.rs`, which both `arrow_conversion` and `query` already depend on, so no dependency edge changes.]

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Impact bullet 1 ("`Connection::connect` returns the failure as `ConnectionError::AuthenticationFailed`")
- Issue: `adbc::Connection` has no `connect` method. `Connection::from_params` (`src/adbc/connection.rs:131`) maps the login error. The public callers are `Database::connect` (`src/adbc/database.rs:89`) and `ConnectionBuilder::connect` (`src/adbc/connection.rs:2306`).
- Fix: In plan.md § Impact bullet 1, replace "`Connection::connect`" with "`Connection::from_params`, which `Database::connect` and `ConnectionBuilder::connect` call,".

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md task 1.2 ("`authenticate` fails with a message that starts with `unsupported server`")
- Issue: `TransportError::ProtocolError` displays as "Protocol error: {0}" (`src/error.rs:158-160`). The error's `to_string()` therefore starts with "Protocol error: ", not with "unsupported server". A test that reads the line literally and asserts `to_string().starts_with("unsupported server")` fails even when task 1.4 is correct.
- Fix: In task 1.2, replace the clause with: "`authenticate` fails with `TransportError::ProtocolError` whose inner message starts with `unsupported server` and contains `13` and `14`".
