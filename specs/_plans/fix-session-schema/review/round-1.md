# Plan Review Findings: fix-session-schema (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 10 (Blockers: 1, Advisory: 9)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

1. The PR pipeline bumps the version a second time after task 1.10 has set 0.21.0. CI tags and publishes 0.21.1 to crates.io with empty release notes, because `CHANGELOG.md` says `## 0.21.0`. Routed to `[HIDDEN_DEPENDENCY]` (BLOCKER).
2. On the native transport, `current_schema()` keeps a schema after `CLOSE SCHEMA`, or the receive-path hook clears the value on every ordinary response, because `receive_into_buf` cannot tell a get-attributes response from the others. Code that qualifies names with `current_schema()` then targets the wrong schema. Routed to `[UNSTATED_ASSUMPTION]` and `[COMPLETENESS_GAP]` (ADVISORY).
3. The WebSocket transport reuses the `HashMap<String, String>` attribute type for the new top-level `attributes` member. The `getAttributes` response carries booleans and numbers, so it fails to deserialize and every `set_schema` fails on WebSocket. Alternatively, a `serde(flatten)` wrapper buffers every large fetch response. Routed to `[EFFORT_MISESTIMATION]` (ADVISORY).

Evidence the reviewer gathered against the running `exasol-test` container over WebSocket (pyexasol 2.2.1):
- A missing schema at login and a wrong password both fail with SQL state `08004` ("Connection exception - schema ZZ_REVIEW_NO_SUCH_SCHEMA not found" and "Connection exception - authentication failed."). `setAttributes` with a missing schema fails with `42000`. This matches plan.md § Context.
- The `execute` responses to `OPEN SCHEMA`, `CREATE SCHEMA`, `DROP SCHEMA` of the current schema, and `CLOSE SCHEMA` carry a top-level `attributes.currentSchema` (`""` after `DROP` and `CLOSE`). `SELECT 1` carries none. `getAttributes` always carries `currentSchema`, `""` when no schema is open, next to boolean and numeric attributes. The probe schema was dropped.

## Intent Fidelity
[no objection — axis checked: #77 rows 1 to 4 map to tasks 1.2 to 1.6 and to the schema-activation scenarios; every schema path (URI, `set_schema`, ADBC `adbc.connection.db_schema`) goes through ATTR_CURRENT_SCHEMA (native attribute 22, WebSocket `currentSchema`), so "a schema set through ATTR_CURRENT_SCHEMA must work" holds; the set-attributes-after-login deviation keeps the attribute, and the reviewer reproduced its evidence (08004 for both a missing schema and a wrong password at login, 42000 for a rejected set); #77's "matching on error text should go away" is met by failing every rejection; #72 is fixed at its source by deleting the five `set_state(SessionState::Executing)` calls (src/adbc/connection.rs lines 360, 513, 565, 621, 675), which covers the issue's points 1 to 4, and decision [6] gives the reason for replacing the issue's missing-URI-schema tests; #72 runs after #77 (group B depends on A); out-of-scope items stay untouched (task 1.6 limits src/adbc_ffi.rs to `FfiConnection` options); the 0.21.0 bump is stated and justified as a breaking 0.x change; the release-order claim holds: the CI `release` job runs only on push to `main` and tags the `Cargo.toml` version only when no tag exists, PR #90 is open against `main`, and its 0.20.0 commit b172b31 is not on `origin/main`]

## Feasibility

Checked without objection: `[NFR_IGNORED]`. The plan states the security gain (no SQL text built from the schema name), the extra round trip per connect, the schema step outside the connection-timeout deadline, the breaking Rust API changes, and the dbt release-ordering risk. FFI results are collected into `VecRecordBatchReader`, so the new `get_option_string` read does not wait on a lock held by an open reader. All four `TransportProtocol` implementations (native, WebSocket, `MockTransport`, `StalledQueryTransport`) are covered by tasks 1.1 to 1.3.

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: plan.md § Implementation Tasks 1.10; decision-log.md § [8] Release as 0.21.0
- Issue: Task 1.10 sets `version = "0.21.0"` and a `## 0.21.0` header inside the implementation. The PR pipeline bumps the version again after implementation. `/speq:implement-pr` step A3 says "bump the workspace version per the plan's `workspace/version` spec delta if it specifies one. Otherwise apply the conventional next version per Conventional Commits semantics (`feat` → minor bump; a purely `fix` plan → patch)", and `~/.claude/commands/speq-implement-pr.md` step 3 says the same. This plan has no `workspace/version` delta, so the fallback applies on top of 0.21.0 and gives 0.21.1, or 0.20.1 if the step computes from the base. The CI `release` job then tags and publishes that version to crates.io, and its release notes are empty, because it extracts them by an exact `## X.Y.Z` header match. A crates.io publish cannot be undone. The previous plan in this repository, fix-ffi-bound-execute, left the bump to the pipeline (commit b172b31) and so had no collision.
- Fix: In plan.md § Dependencies, add the bullet "Task 1.10 is the only version bump of this PR. The version-bump step of the PR pipeline (`/speq:implement-pr` A3, `speq-implement-pr` step 3) MUST leave `Cargo.toml` at 0.21.0 and the `## 0.21.0` header of `CHANGELOG.md` unchanged, because this plan has no `workspace/version` delta and that step's Conventional Commits fallback would bump this `fix` plan a second time." Add the same constraint as one sentence at the end of task 1.10 and as one bullet in decision-log.md § [8] Consequences.
- Escalation: MECHANICAL (the plan's own text and the pipeline's skill text settle it; no requester judgment is needed)

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Implementation Tasks 1.2; native-client/protocol/spec.md § Scenario: Track the current schema attribute from responses
- Issue: Task 1.2 records attribute 22 "in one place on the receive path (`receive_into_buf`)" and also requires that "an absent attribute in a `CMD_GET_ATTRIBUTES` response means none". `receive_into_buf` (src/transport/native/mod.rs:213) reads only the header and the payload, and nothing tells it which command the response answers. The login sends its `CMD_GET_ATTRIBUTES` through `send_message` and `receive_message`, outside `send_and_fill_buf`. Applying "absent means none" inside `receive_into_buf` would clear the value on every ordinary response. Ignoring it there would never clear the value after a get-attributes response.
- Fix: In task 1.2, state how the rule learns the command. Either record only present values in `receive_into_buf` and apply "absent means none" in `refresh_current_schema` and in the login's get-attributes step, or pass the sent command to the pure helper. Name the chosen option in decision-log.md § [2] Consequences.

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md § Implementation Tasks 1.3
- Issue: `send_receive<T, R>` (src/transport/websocket.rs:66) deserializes straight into a generic `R`. No response struct in src/transport/messages.rs has a top-level `attributes` field. The only one, `ExecuteResponseData.attributes`, sits inside `responseData` and is typed `HashMap<String, String>`. Reading `attributes.currentSchema` "within the one JSON parse" therefore needs a new field on about ten response types and a trait bound on `R`. A `serde(flatten)` wrapper would buffer the whole fetch response, which the task forbids. The reviewer's probe shows that the top-level `attributes` of a `getAttributes` response mixes value types (`autocommit: true`, `queryTimeout: 0`, `currentSchema: ""`), so the `HashMap<String, String>` pattern fails to deserialize it. Task 1.3 is untagged, while the smaller native task 1.2 is tagged `[expert]`.
- Fix: In task 1.3, name the mechanism: a top-level `attributes` field on each response type, typed to accept mixed value types, and a trait that `send_receive` reads. State the mixed-type fact, and tag the task `[expert]`.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Verification › Checklist, rows "Integration tests", "WebSocket integration tests", "Native protocol tests", and "Driver manager tests"
- Issue: AGENTS.md says "Tests must fail, not skip, when Exasol is unavailable". `skip_if_no_exasol!` (tests/common/mod.rs:303) skips unless `REQUIRE_EXASOL` is set, and CI sets `REQUIRE_EXASOL: "1"` for these suites (.github/workflows/ci.yml lines 255 to 289). The checklist sets it only on the import and export row. A local run without the container reports "0 failures" with every new test skipped, and the verification report would claim green without evidence.
- Fix: Prefix the commands of the four rows with `REQUIRE_EXASOL=1`, as the import and export row already does.

## Requirement Quality

Checked without objection: `[IMPLEMENTATION_LEAKAGE]`. Every fact in the schema-activation Background (no quoting, the two-step case rule, the kept previous schema, where the server reports the value, empty means none, the URI schema must exist) is used by a scenario step, and the new session-and-lifecycle Background sentences back the four new state scenarios. `[REQUIREMENT_CONFLICT]`: `speq plan validate` passes. The deltas agree with each other and with the recorded `adbc-driver/transactions`, `connection-management/connection-timeout` (which bounds only the TCP connect, TLS, WebSocket upgrade, and login), `websocket-client/async-lifecycle`, and both recorded protocol features. `[AMBIGUOUS_REQUIREMENT]`: each THEN step can be tested as written.

#### [COMPLETENESS_GAP] ADVISORY
- Location: connection-management/schema-activation/spec.md § Scenario: Current schema follows schema changes made in SQL; plan.md § Context (native spike bullet)
- Issue: The scenario lists `OPEN SCHEMA`, `CREATE SCHEMA`, and `DROP SCHEMA`, but not `CLOSE SCHEMA`, whose only effect is to clear the current schema. plan.md § Context says that on native "other responses carry no attribute 22". planning.md lists `CLOSE SCHEMA` after `DROP SCHEMA` in the spike, and Context reports no result for it, so the native case with a schema open is not shown to be tested. The reviewer's WebSocket probe shows that `CLOSE SCHEMA` with `SYS` open returns `attributes.currentSchema: ""`. If the native response lacks attribute 22, `current_schema()` stays stale after `CLOSE SCHEMA` on the default transport.
- Fix: Add `CLOSE SCHEMA` to the WHEN list of the scenario, and to `test_current_schema_follows_schema_changes_in_sql` and `test_websocket_current_schema_follows_schema_changes_in_sql` with a schema open before the statement. State the native result for `CLOSE SCHEMA` with a schema open in plan.md § Context.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Impact ("When both `myschema` and `MYSCHEMA` exist, `/myschema` now opens `myschema`"); connection-management/schema-activation/spec.md § Scenario: URI schema name follows the server's case rule
- Issue: The Impact lists this user-visible behavior change, and task 1.9 documents it, but no scenario step and no test covers it. The GIVEN of the case-rule scenario holds no two schemas that differ only in case.
- Fix: Add `"zz_both"` and `ZZ_BOTH` to the GIVEN of the case-rule scenario, add a THEN step that the URI schema `zz_both` opens `zz_both`, and add the same case to `test_uri_schema_name_follows_the_server_case_rule`.

## Task Breakdown

Checked without objection: `[TASK_GRANULARITY]` and `[CLUSTER_INCOHERENCE]`. Group A shares the schema deltas and the transport, connection, and FFI modules. Group B runs after A because both edit `src/adbc/connection.rs`, `src/connection/session.rs`, the integration test files, and `CHANGELOG.md`. Every spec delta has an implementing task, and every Scenario Coverage row names an existing or planned test.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Implementation Tasks 1.7, 1.8, and 2.3; § Verification › Scenario Coverage
- Issue: The brief names the AGENTS.md rule "one `/// Scenario: <title>` line per scenario" as a constraint, but no task or verification step schedules it. No test in `src` or `tests` carries a tag for a schema-activation scenario today. Reused tests get no instruction. `test_uri_schema_is_opened_on_connect` maps to two scenarios. It and `test_ffi_uri_schema_is_opened_on_connect` keep doc comments that describe the removed behavior, such as "`Database::connect()` MUST issue `OPEN SCHEMA <name>`" and "auto-OPEN-SCHEMA". AGENTS.md also requires updating comments when behavior changes.
- Fix: Add to plan.md § Verification the rule that every test in Scenario Coverage carries one `/// Scenario: <title>` line per mapped scenario, with the delta titles quoted verbatim. Add to tasks 1.7 and 1.8 the rewrite of the `OPEN SCHEMA` doc comments on `test_uri_schema_is_opened_on_connect` and `test_ffi_uri_schema_is_opened_on_connect`.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Verification › Scenario Coverage, rows for "schema-activation: Set the current schema at runtime", "native-client/protocol: Set the current schema attribute", and "websocket-client/protocol: Set the current schema attribute"
- Issue: The step "`current_schema()` MUST keep reporting the previous current schema" after a rejection is implemented inside each transport, which records a value only after an accepted set. The only test that asserts it is `set_schema_keeps_the_previous_schema_when_the_server_rejects_it` in src/adbc/connection.rs. That test runs against `MockTransport`, so it asserts the mock's behavior. `set_current_schema_returns_the_servers_rejection` and `native_set_current_schema_reports_a_rejected_schema` are named for the error only. `test_ffi_db_schema_option_rejects_a_missing_schema` reads through get-attributes, not through the recorded value.
- Fix: State in Scenario Coverage that `set_current_schema_returns_the_servers_rejection` (WebSocket) and `native_set_current_schema_reports_a_rejected_schema` (native) assert that `current_schema()` keeps the earlier value after the rejection. Limit the mock-based unit test to asserting that `set_schema` returns the transport's error.

## Design Depth

Checked without objection: `[ADR_CONFLICT]`. Decision [1] supersedes ADR `uri-specified-schema-is-best-effort-default` with `Supersedes:` and `Promotes to ADR: yes`. ADR `transport-owns-setup-deadline` bounds only the TCP connect, TLS, WebSocket upgrade, and login, so the schema step after the login does not conflict with it. ADRs `server-enforced-query-timeout` and `client-give-up-terminates-connection` are not contradicted. `[ADR_OVERPROMOTION]`: decision [1] is the only `yes` entry. It names rule-2 criteria 4 and 2, states the `speq decision-log show` result, is not on the never-an-ADR list, and holds no signatures or paths. `[ARCHITECTURE_DRIFT]`: the BASE comment equals `git hash-object specs/architecture.md` (ad26045e…). The three CHANGED blocks copy the full base sections and change only the schema facts (diffed). Decisions [1] and [2] name sections that the delta contains. `[SHALLOW_DESIGN]`, `[INFORMATION_LEAKAGE]`, and `[BOUNDARY_VIOLATION]`: the transport is the one owner of the server-reported value, three trait methods hide two wire formats, and the dependency direction stays adbc to transport.

#### [TACTICAL_SHORTCUT] ADVISORY
- Location: decision-log.md § [9] Scope boundaries; § [4] Consequences
- Issue: Two known defects stay open with no scheduled follow-up. First, docs/setup-and-connect.md § Session Attributes lists `currentSchema` as a URI query parameter that the driver forwards to the server, but `ConnectionParams::attributes` is never sent (decision [9]). Task 1.9 rewrites the schema guidance in the same file, so after this PR the file names four ways to set the schema, and one of them does nothing and reports no error. Second, a dropped execution leaves an unread response on the transport (decision [4]), while the new scenario "An abandoned execution leaves the session state unchanged" reads as if the connection stays usable.
- Fix: In task 1.9, delete the `currentSchema` row from the Session Attributes table, which stays within #77's documentation scope. Otherwise add a plan.md § Dependencies bullet that names a follow-up issue for the forwarding claim. Add a second bullet that names a follow-up issue for the transport desync after a dropped execution.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: plan.md § Implementation Tasks 1.10 (one 74-word sentence) and § Summary (first sentence); decision-log.md § [4] Alternatives (one 66-word sentence) and § [1] Alternatives
- Issue: Each of these sentences carries three or more claims, against writing-guardrails rule 2 (one idea per sentence). Task 1.10 puts the version, the lockfile, the header position, and eight CHANGELOG entries into one sentence.
- Fix: Split each sentence into one sentence per claim. In task 1.10, list the CHANGELOG entries as sub-bullets.
