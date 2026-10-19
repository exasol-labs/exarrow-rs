# Plan Review Findings: fix-driver-gaps (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 10 (Blockers: 1, Advisory: 9)
- Intent Fidelity blockers: 1
- Human-escalation blockers: 1

## Premortem

Six months from now this plan failed. These are the likely reasons:

1. The requester rejected the PR because it adds two new spec scenarios after the request said "Write spec deltas only for scenarios that exist on main". The plan then went through a second planning cycle. Route: Intent Fidelity, `[SCOPE_CREEP]` BLOCKER.
2. A user on the default native transport hit a VARCHAR value that was not valid UTF-8. The error still did not name the column, because the plan treats "conversion errors" as the `ConversionError` type, and only the opt-in WebSocket path and the unused `ArrowConverter` utility changed. Route: Feasibility, `[UNSTATED_ASSUMPTION]`.
3. Task 4.1 set the version to 0.25.0, and the `implement-pr` version-bump step then bumped it again. `Cargo.toml` and the `## 0.25.0` changelog header disagreed, and the CI release job published the wrong version. Route: Feasibility, `[HIDDEN_DEPENDENCY]`.
4. A WebSocket query returned a DECIMAL(5,2) value of 12345.67 without an error, although the plan's coverage table says it implements "Decimal overflow detection". Route: Requirement Quality, `[COMPLETENESS_GAP]`.

## Intent Fidelity

#### [SCOPE_CREEP] BLOCKER
- Location: `specs/_plans/fix-driver-gaps/native-client/handshake/spec.md` § Scenarios (second block); `specs/_plans/fix-driver-gaps/connection-management/version-capability/spec.md` § Scenarios; plan.md § Features and § Scenario Coverage
- Issue: The request says "Write spec deltas only for scenarios that exist on main." The plan writes two `<!-- DELTA:NEW -->` blocks for scenarios that do not exist on main: "### Scenario: Server protocol version below 14 is rejected before the password is sent" and "### Scenario: Compare the server version against a caller-supplied threshold". Main already covers both asks. "Protocol version negotiation" says "the system SHALL reject protocol versions below 14 with a clear error". The version-capability Background names `supports_at_least(major, minor, patch)`, and "Reject an unparsable release_version" requires it to return `false`. decision-log.md has no entry that addresses the constraint. Escalation is HUMAN because this is an Intent Fidelity finding: only the requester can decide whether the two new scenarios are acceptable.
- Fix: In `native-client/handshake/spec.md`, delete the DELTA:NEW block. In the DELTA:CHANGED "Protocol version negotiation" block, keep a rejection step that names the server's version and the minimum version 14. Add "*AND* the system SHALL NOT send the password message to a server whose version is below 14". In `connection-management/version-capability/spec.md`, delete the DELTA:NEW block. Then do one of two things: delete the delta file and plan.md's Features row, because main already specifies `supports_at_least`, or move the threshold comparison and the six boundary steps into a DELTA:CHANGED block of an existing main scenario such as "Capability check is in-process and round-trip-free". In plan.md § Scenario Coverage, map tests 1.2, 2.1, and 2.3 to main scenario titles only. If the requester approves the new scenarios, keep them and record that approval as a decision-log.md entry.
- Escalation: HUMAN

#### [SCOPE_CREEP] ADVISORY
- Location: plan.md § Impact ("New API: ... `Connection::supports_at_least(major, minor, patch)`", "Breaking: the free function `connection::version::supports_at_least(parsed, major, minor)` is removed"); tasks 2.2 and 2.3; decision-log.md [3]
- Issue: The request is "Add Session::supports_at_least(major, minor, patch)" and "Keep it a small plan". The plan also adds a public `Connection` method and removes a public function. `exarrow_rs::connection::version::supports_at_least` is reachable through `pub mod version` (`src/connection/mod.rs:33`), so removing it is a breaking change that nobody requested. Decision [3] gives a sound reason: `Connection` has no session accessor. The requester has still not agreed to the extra API.
- Fix: Add a plan.md § Context bullet stating that `Connection::supports_at_least` and the removal of the free function go beyond the request, and cite decision [3]. The PR reviewer can then accept or reject them. If the requester rejects them, remove the `Connection` method and its test from task 2.3, keep the free function, and delete the two Impact bullets.

## Feasibility

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: decision-log.md [1] Rationale ("The native transport decodes binary result data and returns no `ConversionError`, so it needs no change."); plan.md § Context bullet 7
- Issue: The plan treats "Conversion errors name the column" as applying only to the `ConversionError` type. The default native transport reports an invalid UTF-8 VARCHAR value as `TransportError::ProtocolError("Invalid UTF-8 in string: ...")` (`src/transport/native/result_parser.rs:912`), and the message does not name the column. The column `name` is already a parameter of the enclosing `fill_string_builder` (`result_parser.rs:900`). `ConversionError::InvalidUtf8` exists for this case and has no producer. As planned, fix (3) changes nothing that a user of the default transport sees.
- Fix: Do one of two things. Add a task that puts the column name into the native "Invalid UTF-8 in string" message, with a test in `result_parser.rs`. Or add a plan.md § Context bullet stating that native decode errors stay unchanged, so that the requester sees and can accept this interpretation of "conversion errors".

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Impact bullet 1; task 1.4
- Issue: The request asks for "a clear error". An ADBC caller receives the rejection as `ConnectionError::AuthenticationFailed`, which displays as "Authentication failed: Protocol error: ..." (`src/error.rs:39`, `src/adbc/connection.rs:190-193`). That prefix suggests wrong credentials, not an unsupported server. No test checks the text that a `Connection` caller sees.
- Fix: In task 1.4, make the message start with the incompatibility, for example "unsupported server: native protocol version 13 is below the minimum supported version 14". In plan.md § Impact, state that the message arrives with the prefix "Authentication failed: ". Alternatively, add to task 1.4 a mapping of this rejection to `ConnectionError::ConnectionFailed`, with a test through `Connection::connect_with_transport`.

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md task 4.1; § Impact last bullet
- Issue: `/speq:implement-pr` step A3 bumps the version itself: "Otherwise apply the conventional next version per Conventional Commits semantics". If task 4.1 has already set 0.25.0, step A3 can bump again: a `fix` plan gets a patch bump to 0.25.1. That would leave the `## 0.25.0` changelog header out of step with `Cargo.toml`, and AGENTS.md § Changelog requires them to match. Also, merging a version bump triggers the CI `release` job and a crates.io publish. The request does not mention a release.
- Fix: Add to task 4.1: "The version-bump step of `/speq:implement-pr` keeps 0.25.0 and does not bump again (minor bump for breaking changes before 1.0)." Also add a plan.md § Impact line stating that merging this PR publishes 0.25.0.

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Context bullet 10 (PR 101)
- Issue: The plan names open PR 101 only for the keepalive scenario. PR 101 also edits `specs/arrow-conversion/recordbatch-assembly/spec.md`: it deletes "Partial conversion handling", the scenario directly after "Conversion error reporting", which this plan's DELTA:CHANGED rewrites. It also edits `specs/arrow-conversion/type-converters/spec.md`. Recording this plan and merging PR 101 change adjacent lines of the same file, so the change that lands second probably gets a git conflict. Evidence: `ghbrk gh pr diff 101`.
- Fix: Add a plan.md § Context bullet stating that PR 101 also edits `recordbatch-assembly/spec.md` and `type-converters/spec.md` and leaves "Conversion error reporting" and "Decimal overflow detection" unchanged. State that whichever change lands second rebases on the other.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Features paragraph ("... `arrow-conversion/type-converters` 'Decimal overflow detection'"); § Scenario Coverage row "type-converters: Decimal overflow detection"; task 3.4
- Issue: That scenario's GIVEN is "Exasol returns JSON result data with typed columns". The WebSocket path is the only transport path that converts Exasol JSON. On that path, `parse_string_to_decimal` drops extra fraction digits and checks no precision (`src/query/results.rs:606`). `scale_to_decimal` multiplies an i64 without a check: `integer * 10i64.pow(scale as u32)` (`results.rs:595`). For a large scale this panics in debug builds and wraps in release builds. Task 3.4 reports `NumericOverflow` only for an i128 overflow, so a DECIMAL(5,2) value `"12345.678"` still passes on that path. The plan's test covers only `ArrowConverter`, which no transport calls. Decision [1] rejected a fix limited to builders.rs for the same reason.
- Fix: Do one of two things. Reduce the claim in plan.md § Features and § Scenario Coverage: state that only the `ArrowConverter` path detects precision overflow, and that the WebSocket path detects only an i128 overflow. Or extend task 3.4: check the declared precision in `json_to_decimal_array`, use checked arithmetic in the integer branch of `scale_to_decimal`, and add a test for each.

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: `specs/_plans/fix-driver-gaps/native-client/handshake/spec.md` DELTA:CHANGED "Protocol version negotiation", last step; recorded scenario "Login handshake"
- Issue: The new step "the system SHALL treat a login response without `ATTR_PROTOCOL_VERSION` as negotiating the client's maximum version" conflicts with the main step "the system SHALL receive the server's response containing `ATTR_PUBLIC_KEY`, `ATTR_RANDOM_PHRASE`, and `ATTR_PROTOCOL_VERSION`", which says the attribute is always present. The CHANGED scenario also says nothing about a version above the client's maximum, which decision [2] keeps accepted. The fallback step is also a requirement that the request did not ask for.
- Fix: Delete the fallback step from the CHANGED block. Decision [2] and task 1.4 keep the behavior. If the planner keeps the step, add a DELTA:CHANGED "Login handshake" block that marks `ATTR_PROTOCOL_VERSION` as optional. In either case, add one step that states the outcome for a version above the client's maximum.

## Task Breakdown
[no objection — axis checked: every delta has an implementing task (handshake: 1.2-1.4; version-capability: 2.1-2.3; recordbatch-assembly: 3.2-3.4). Task 2.5 implements the requested keepalive removal, which has no delta by request. The files edited by groups A, B, and C do not overlap (A: `native/mod.rs`, `handshake.rs`, `test_support.rs`; B: `session.rs`, `version.rs`, `adbc/connection.rs`, `tests/integration_tests.rs`; C: `error.rs`, `builders.rs`, `converter.rs`, `results.rs`). A grep found no `ConversionError` variant match in `adbc/connection.rs`. Task 2.3 can create its `MockTransport` with release "8.27.1" in its own test without editing `test_support.rs`. Group D correctly runs after A, B, and C.]

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: decision-log.md [1] Consequences; plan.md tasks 3.1, 3.3, and 3.4
- Issue: Task 3.1 adds `column_name`, `value`, and `target_type` to the public variants `ValueConversionFailed` and `NumericOverflow`. These fields stay empty from the per-value step until the per-column step fills them. Three modules (`error.rs`, `builders.rs`, `results.rs`) must follow this two-step construction, and the type system does not enforce it. The variants are public and have public fields, so code outside the crate can also create the incomplete state. The plan also edits two separate decimal parsers that already disagree: `types::conversion::parse_decimal_to_i128` with `validate_decimal_precision` in builders.rs, and `ResultSet::parse_string_to_decimal`. No follow-up is scheduled for either issue.
- Fix: Make the per-value steps return a crate-private error type that holds the row, the reason, and an overflow flag. Convert it to the public variant only at the per-column step, through the one crate-private method from task 3.1. Record this as a rejected or adopted alternative in decision [1]. Add a plan.md § Context bullet that names the duplicate decimal parsers as a follow-up.

[ADR and architecture checks: no objection — axis checked: the ADR digest lists seven accepted ADRs (query timeout, client give-up, zero-row schema, placeholder lexer, Parquet export schema, batch arity, export timer). None covers protocol version, version capability, or conversion errors, and the plan touches no `empty_record_batch` behavior. No decision-log entry has `Promotes to ADR: yes`. plan.md § Context has "Architecture: no delta" with a reason. `specs/architecture.md` § Interfaces lists only the Connection type, not its methods. The connection component's responsibility, "checks the server version for features", still covers the cached parsed release version that replaces the cached Parquet flag.]

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Context bullet 5
- Issue: "`ConversionError::ValueConversionFailed` and `ConversionError::NumericOverflow` carry the row and column index only." This is wrong for `ValueConversionFailed`, which also carries `message` (`src/error.rs:116-120`).
- Fix: Replace the sentence with: "`ValueConversionFailed` carries the row, the column index, and a message. `NumericOverflow` carries the row and the column index only."
