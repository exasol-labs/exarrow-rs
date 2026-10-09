# Plan Review Findings: fix-timestamp-bind (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 3, Advisory: 6)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

The root cause holds against the code. `parse_column_meta` (`src/transport/native/result_parser.rs:479`) reads a 4-byte precision for the three TIMESTAMP wire types, and `write_column_metadata` (`src/transport/native/mod.rs:504`) writes nothing for them. The three blockers are in the new requirements, not in the header fix. The native text grammar contradicts the plan's own parity policy for inputs that the WebSocket transport accepts. The TSLTZ step of the Arrow scenario states an instant round trip that Exasol does not perform. One protocol step is untestable for two of its three wire types.

Scope verdict on decisions [3] and [4], as the brief asked:
- The TIMESTAMP half of [3] belongs in this PR. The header fix makes `write_timestamp_bytes` reachable for the first time. That function stores `2000-01-01 00:00:00` for `garbage` and drops the time of `2024-1-1 8:0:0`. It also slices the text at byte offsets 10 and 11 (`&s[..10.min(s.len())]`, `&s[11..]`), which panics when a multi-byte character spans either offset. Without [3], the fix would replace a closed connection with wrong stored values.
- [4] belongs. It is the narrowest option and adds no feature. Its parity rationale is undermined by the first blocker below.
- The DATE half of [3] is scope creep. See the `[SCOPE_CREEP]` advisory.

Evidence gathered for this review: `exasol-test` (`exasol/docker-db:latest`, `SESSIONTIMEZONE` and `DBTIMEZONE` both `EUROPE/BERLIN`), with WebSocket `createPreparedStatement` and `executePreparedStatement` requests sent through pyexasol into `TS TIMESTAMP(6), D DATE` and `TS TIMESTAMP(6) WITH LOCAL TIME ZONE` tables, and `SELECT CAST(... AS TIMESTAMP | DATE)` probes. All probe schemas were dropped afterward.

## Premortem

Six months from now this plan failed. These are three ways it could happen:

1. Native callers bind text taken from CSV files, where blank cells are `''` and values are padded with spaces. Over the native transport these calls fail with a client-side serialization error. Over the WebSocket transport, and in Exasol's own cast, the same values store NULL or the trimmed value. Users report the 0.20.2 "Changed" entry as a regression. This goes under Requirement Quality as `[REQUIREMENT_CONFLICT]` (blocker 1).
2. A user binds UTC Arrow instants into a `TIMESTAMP WITH LOCAL TIME ZONE` column. A reader in a UTC session sees every value one or two hours early. The recorded spec says the driver "SHALL return the bound instant", and the planned test passes, so the bug is closed as working as specified. This goes under Requirement Quality as `[COMPLETENESS_GAP]` (blocker 2).
3. PR #91 merges first. It edits `src/transport/native/mod.rs`, `constants.rs`, and the same three test files, and it releases 0.21.0. This plan's rebase then conflicts, and the "next patch version" note in plan.md Dependencies gives the wrong header. This goes under Feasibility as `[HIDDEN_DEPENDENCY]` (advisory).

## Intent Fidelity

#### [SCOPE_CREEP] ADVISORY
- Location: decision-log.md § [3] Rationale; plan.md § Summary, § Impact bullet 2, tasks 1.4 and 2.3
- Issue: The brief says "Scope: bound timestamp parameters only" and "check the same for Date32 and Time-less variants". The check result for Date32 is that it already works: a bound Date32 value and well-formed DATE text succeed on main over both transports (plan.md Context bullet 10). Even so, [3] also replaces the DATE text parser, because "the brief asks to check DATE the same way, so one date-part parser serves both". The DATE defect (`2024/01/02` stores `2000-01-01`) exists on main and is unrelated to #38. Fixing it adds a user-visible DATE "Changed" entry to a patch release whose changelog says "Fixes #38". The TIMESTAMP change does not need the DATE change: `parse_date_to_packed` can stay as it is.
- Fix: In task 2.3, give the DATE change its own CHANGELOG `Fix:` entry that names DATE parameter text and does not cite #38. Also name the DATE change in plan.md § Summary as a second fix, so the PR reviewer can accept or remove it on its own. If the requester declines it, restore `parse_date_to_packed`, drop the DATE rows from tasks 1.2 and 1.4 and from the delta scenarios, and add the DATE text defect to the follow-up list in decision-log.md [5].

#### [SCOPE_REDUCTION] ADVISORY
- Location: plan.md task 2.3; decision-log.md § [4] Consequences
- Issue: #38 reports `SPACE expected; Value: '2024-01-01T08:00:00'` from a dbt-fusion seed. The plan shows that the driver never produced `T` text. A `git grep` for `T`-separator formatting finds nothing in `src/` at v0.12.0 or at HEAD, and both `format_timestamp_micros` and the v0.12.0 import formatter use a space. The `T` text therefore came from the caller. After this plan, the same caller input still fails over both transports ([4] Consequences: "Binding ISO 8601 text fails on both transports"). The "Fixes #38" entry closes the issue while the reporter's workflow still fails, and nothing tells the reporter why.
- Fix: In task 2.3, extend the Fix entry with one sentence: ISO 8601 text with a `T` separator, bound to a TIMESTAMP parameter, still fails on both transports, and a bound Arrow Timestamp column never produces such text. Add a task 2.4 that puts the same sentence in the PR description.

## Feasibility

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Dependencies; decision-log.md § [8] Consequences
- Issue: plan.md says PR #91 "touches `src/adbc/connection.rs` and session state. This plan changes neither." The plan.md on PR #91's branch (`plan/fix-session-schema`, Parallelization group A) also lists `src/transport/native/mod.rs`, `src/transport/native/constants.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, `tests/driver_manager_tests.rs`, `Cargo.toml`, and `CHANGELOG.md`, and this plan edits all of them. PR #91 bumps the version to 0.21.0 (its task 1.10). The [8] case "If PR #91 releases 0.20.2 first" therefore cannot happen. If #91 merges first, this plan becomes 0.21.1, not "the next patch version" of 0.20.
- Fix: In plan.md § Dependencies, list the files that both PRs edit. State the rule: if PR #91 (0.21.0) merges first, rebase, set `version = "0.21.1"`, and put this plan's CHANGELOG section above `## 0.21.0`. Change decision-log.md [8] Consequences to the same rule.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: decision-log.md § [3] Decision and § [4] Rationale; plan.md § Impact bullet 2 and task 1.2; `native-client/prepared-statement-protocol/spec.md` § "DATE and TIMESTAMP parameter text is encoded field by field" and § "DATE and TIMESTAMP parameter values in another form are rejected before execution"; `prepared-statements/parameter-binding/spec.md` § "DATE and TIMESTAMP text parameters behave the same on both transports"
- Issue: The plan's stated policy is parity between the two transports. [4] says "Equal behavior on both transports is the conventional default". Impact says "The WebSocket transport already rejected such text through Exasol". The parity scenario's title says the two transports "behave the same". The planned native grammar rejects text that the WebSocket transport accepts. These results were verified on `exasol-test` with WebSocket `executePreparedStatement` into `TS TIMESTAMP(6), D DATE`:
  - `''` bound to TS or D stores NULL, because Exasol treats the empty string as NULL. Task 1.2 rejects "empty text".
  - `'2024-01-01 08:00:00 '` and `' 2024-01-01 08:00:00'` bound to TS store `08:00:00`. Task 1.2 rejects "a trailing space".
  - `'2024-01-02 08:30'` and `'2024-01-02 00:00:00'` bound to D store `2024-01-02`. Task 1.2 and the rejection scenario reject "date-and-time text in a DATE column".
  - `'2024-01-01 08'` bound to TS stores `08:00:00`. The grammar requires at least `h:m`.
  - `SELECT CAST('2024-01-01 08:00:00.' AS TIMESTAMP)` succeeds. The grammar requires 1 to 9 digits after the dot.
  The empty string matters most. An Arrow Utf8 value `''` bound through the FFI to a TIMESTAMP column stores NULL over WebSocket and fails over native. These cases agree with the grammar: `2024-01-01T08:00:00`, `2024/01/02`, `garbage`, 10 fraction digits, a 5-digit year, `2024-1-1 8:0:0`, a date without a time, and 9 fraction digits.
- Fix: Align the native grammar with the verified WebSocket results, and pin each case. In decision-log.md [3] and in the scenario "DATE and TIMESTAMP parameter text is encoded field by field", specify the following:
  - Empty text in a `T_DATE` or timestamp column is encoded as NULL (null marker 0).
  - Leading and trailing spaces are ignored.
  - A `T_DATE` value accepts the full timestamp form and encodes only its date part.
  - The time part is `h`, `h:m`, `h:m:s`, or `h:m:s.f`, where `f` has 0 to 9 digits.

  Remove empty text, a trailing space, and date-and-time text in a DATE column from the rejected list in task 1.2 and from the examples in the rejection scenario. Add them to the accepted list in task 1.2 with their expected encodings. Add three cases to the parity scenario and to tasks 1.6 and 1.7: `''` bound to TS (stores NULL), `' 2024-01-01 08:00:00 '` bound to TS, and `2024-01-02 08:30` bound to D. If the DATE change is removed (see the `[SCOPE_CREEP]` advisory), apply only the TIMESTAMP parts. If any case stays native-only, list that divergence in plan.md § Impact and in decision-log.md [4], and rename the parity scenario so that it no longer claims identical behavior.
- Escalation: MECHANICAL. The plan's own decision [4] sets the parity policy, and `exasol-test` settles every case listed above.

#### [COMPLETENESS_GAP] BLOCKER
- Location: `type-mapping/arrow-to-exasol/spec.md` § "Bound Arrow Timestamp and Date32 values are stored on both transports"; plan.md tasks 1.5 and 1.8
- Issue: The scenario binds a `Timestamp(Microsecond, "UTC")` column to a `TIMESTAMP(6) WITH LOCAL TIME ZONE` column. It then states that "selecting each timestamp column back SHALL return the bound instant". Exasol interprets a value bound to a TIMESTAMP WITH LOCAL TIME ZONE parameter in the session time zone. On `exasol-test`, the session time zone is `EUROPE/BERLIN`. A WebSocket prepared insert of `2024-01-01 08:00:00.000000`, which is the text the FFI conversion produces for the UTC instant 08:00Z, reads back as `08:00:00` in the Berlin session. After `ALTER SESSION SET TIME_ZONE='UTC'`, it reads back as `07:00:00`. The stored instant is 07:00Z, not the bound instant. Task 1.8 compares microsecond values in the same session, so the test passes while the THEN step is false. The scenario does not mention the session time zone, which decides the result. Recording the step would pin a time zone claim that decision [5] puts out of scope.
- Fix: In the Arrow scenario, keep "the bound instant" only for the `TIMESTAMP(6)` columns. For the `TIMESTAMP(6) WITH LOCAL TIME ZONE` column, replace it with this step: selecting it back in the same session SHALL return the bound UTC date and time as written. Add to the GIVEN that the session uses the server's default time zone. Add an observation to the follow-up list in decision-log.md [5]: a bound Arrow Timestamp stored in a `TIMESTAMP WITH LOCAL TIME ZONE` column is read as session-local time, so the stored instant differs from the bound instant by the session offset (verified on `exasol-test` with `SESSIONTIMEZONE` `EUROPE/BERLIN`). Keep task 1.8's same-session comparison, and make its doc comment say that it compares date and time values, not instants.
- Escalation: MECHANICAL. The fix rewords the step to match the behavior the plan keeps. It does not change what the feature does.

#### [AMBIGUOUS_REQUIREMENT] BLOCKER
- Location: `native-client/prepared-statement-protocol/spec.md` § "Outbound precision on a TIMESTAMP parameter column header", final step; plan.md § Scenario Coverage, first two rows
- Issue: The scenario's GIVEN covers wire types 21, 124, and 125. Its final step says "Exasol SHALL accept the parameter data and SHALL keep the connection open". plan.md Context bullet 13 says Exasol describes a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as wire type 21. Task 1.1 says no parameter description maps to 124. `data_type_to_wire_type` (`src/transport/native/mod.rs:586`) produces 125 only for a parameter that Exasol describes as `TIMESTAMP WITH LOCAL TIME ZONE`, and according to Context that does not happen. No test can send a 124 or 125 parameter column to Exasol through a prepared statement. The planned integration coverage (`test_prepared_temporal_text_parameters`) sends only wire type 21. The temporary header patch in Context was never exercised with 124 or 125. The step is untestable for two of its three wire types.
- Fix: Split the scenario's THEN steps. Keep the precision-bytes step and the value-layout step for all three wire types. Limit "Exasol SHALL accept the parameter data and SHALL keep the connection open" to wire type `T_TIMESTAMP` (21), which is the type Exasol reports for TIMESTAMP and TIMESTAMP WITH LOCAL TIME ZONE parameters. Make the same split in § Scenario Coverage: a unit row for the bytes of 21, 124, and 125, and an integration row for 21 only.
- Escalation: MECHANICAL. The fix rests only on facts stated in the plan's own Context and in the code.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: `native-client/prepared-statement-protocol/spec.md` § "DATE and TIMESTAMP parameter values in another form are rejected before execution", first THEN step; plan.md task 1.2
- Issue: "an error whose message contains the value and the accepted form" names no text for the accepted form. The test in task 1.2 can only assert whatever string the implementer chooses.
- Fix: State the exact text in the scenario and in task 1.2, for example `YYYY-MM-DD` for a `T_DATE` column and `YYYY-MM-DD HH:MI:SS.FFFFFFFFF` for a timestamp column. Assert that text in `temporal_values_in_other_forms_are_rejected`.

## Task Breakdown

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md tasks 1.1 and 1.2; § Scenario Coverage
- Issue: Scenario Coverage names six unit tests: `prepared_payload_writes_timestamp_precision_for_each_timestamp_wire_type`, `prepared_payload_ranges_repeat_the_timestamp_precision`, `temporal_text_in_the_accepted_forms_encodes_field_by_field`, `out_of_range_temporal_fields_are_sent_as_written`, `temporal_values_in_other_forms_are_rejected`, and `prepared_payload_with_a_rejected_temporal_value_fails`. Tasks 1.1 and 1.2 name none of them. Each task packs its cases into a single sentence: 4 cases in task 1.1 and more than 20 in task 1.2. The implementer can group or name the tests differently, and the coverage table then cites tests that do not exist.
- Fix: Rewrite tasks 1.1 and 1.2 as sub-bullets with one test function per sub-bullet. Each sub-bullet gives the function name from Scenario Coverage and lists its cases.

Group A and group B are coherent. Group A shares the three deltas and `src/transport/native/mod.rs`, and group B depends on group A's accepted forms. Every delta scenario has an implementing task.

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md task 1.4; decision-log.md § [3]
- Issue: `format_date32` and `format_timestamp_micros` in `src/types/conversion.rs` produce the Exasol temporal text form. After task 1.4, a separate grammar inside `src/transport/native/mod.rs` (2430 lines) parses that form. Two modules then own one format with nothing to keep them in agreement. The FFI path depends on both. If the formatter's output changes, for example to 9 fraction digits for Nanosecond values, the new output can fall outside the native grammar.
- Fix: Put the fallible date and timestamp text parsers in `src/types/conversion.rs` next to the formatters. They return the field values, and the native encoder writes those fields. Add a unit test that parses the output of `format_date32` and `format_timestamp` for year 1, year 9999, and a value before 1970, and checks that the fields match.

Other Design Depth checks found no defect. All nine decision-log entries are `Promotes to ADR: no`, so there is no `[ADR_OVERPROMOTION]`. Entry [9] carries `Architecture: no change` with a reason, and plan.md names no new component, boundary, or dependency, so there is no `[ARCHITECTURE_DRIFT]`. Field-by-field encoding with no calendar conversion conforms to `date-timestamp-convert-by-calendar-label` and to the constraint in `specs/architecture.md` line 89. Rejecting a value before any I/O does not contradict `fail-fast-per-row-arity-check-in-batch-execution`. `split_parameter_rows` encodes every row before `execute_prepared_ranges` sends a request (`src/transport/native/mod.rs:1284`), so there is no `[ADR_CONFLICT]`.

## Prose Quality
[no objection — axis checked: plan.md, decision-log.md, and the three deltas lead with the decision or result. They name wire types consistently by constant and ID, and they use no em dashes in new prose. The only em dashes are in the recorded Background of `native-client/prepared-statement-protocol`, which the delta does not change. The single-sentence form of tasks 1.1 and 1.2 is covered by the `[TRACEABILITY_GAP]` advisory.]
