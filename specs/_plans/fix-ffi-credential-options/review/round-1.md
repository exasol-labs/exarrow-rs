# Plan Review Findings: fix-ffi-credential-options (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 3, Advisory: 6)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now this plan failed. Three ways it could happen:

1. The new FFI unit tests never ran green anywhere. CI runs only the `arrow_value_to_parameter` FFI unit tests. The plan's local step, `cargo test --lib --features ffi`, fails on two existing tests that dial a server. The implementer either reports a red checklist or edits unrelated tests to get green. A later change to `build_connection_params` breaks a status mapping, and only the slower Exasol-bound suite can notice. Routed to `[UNSTATED_ASSUMPTION]` (BLOCKER) and `[TACTICAL_SHORTCUT]`.
2. The recorded spec says "the client name SHALL be `dbt@ci`". A maintainer working on #75 reads it as delivered behavior. The server session still reports `exarrow-rs`, because neither transport sends `client_name`. The spec library and the system disagree. Routed to `[AMBIGUOUS_REQUIREMENT]` (BLOCKER).
3. A dbt user puts `?user=bob` in the URI and passes only the `password` option. This plan makes that log in as `bob`, where the current code logs in as `sys`. No scenario and no test pins the result. A later refactor of the precedence code changes it silently. Routed to `[IMPLEMENTATION_LEAKAGE]` (BLOCKER).

## Intent Fidelity

The plan covers the ask. Symptom 1 maps to task 3.5, symptoms 2 and 3 to task 3.4, and symptom 4 to tasks 3.6 and 2.5. The three proposal items are tasks 1.2 and 2.1 to 2.3 and decision [3]. Docs (tasks 4.1, 4.2) and the changelog (task 4.3) are present, and `Cargo.toml` stays at 0.18.0. No task changes code owned by #67, #72, #73, #75, #77, or #78. The deviations from the issue's spike stay within #74:

- Statements drop the URI (decision [5]). Verified: `FfiStatement::with_connection` has one caller, `FfiConnection::new_statement` (`src/adbc_ffi.rs:1497`), which passes `Some(conn)` after `ensure_connected`. `FfiStatement::new` is `#[cfg(test)]`. The standalone branches of `query_sql` and `update_sql` therefore never run in production, and removing them changes no production behavior.
- Configuration errors move to `AdbcConnectionInit` (decision [4]). Verified: adbc_ffi 0.23 `connection_init` calls `new_connection_with_opts`, and adbc_driver_manager 0.23 `ManagedDatabase::new_connection_with_opts` calls `ConnectionInit`. The move follows from the issue's item 2, which keeps `ConnectionParams` in the FFI layer.
- `ConnectionBuilder` Debug redaction (decision [6]) traces to the brief's requirement that passwords stay out of Debug output on the new path. `parse_with_credentials` puts the option password into a builder.
- Per-field precedence (decision [2]) matches the issue: options override the URI, and the username check runs after the merge. One part goes beyond the issue, as the finding below states.

#### [SCOPE_CREEP] ADVISORY
- Location: decision-log.md § [2] Consequences; plan.md § Impact ("Fix (Rust API and driver manager): `exasol://alice@host?password=pw` logs in with `pw`"); plan.md § Implementation Tasks 4.3, third changelog entry
- Issue: The brief scopes the plan strictly to #74, which concerns ADBC option credentials. Decision [2] also changes the Rust URI path: a `password` or `pass` query key now applies when the userinfo names only the user. Today `exasol://alice@host?password=pw` logs in with an empty password and keeps `pw` as an attribute. The security requirement needs only the key removed from `attributes`, so that Debug output omits it. Using the key as the password is a separate change to `ConnectionParams::from_str`, which every Rust API caller uses. The risk is low: the current result is an empty password, which cannot log in as a password user, so no working setup changes result. The plan states the change in § Impact and in the changelog. It does not say that the change goes beyond #74 or what the narrower option is.
- Fix: Keep decision [2]. Add one bullet to plan.md § Impact stating that this is the only Rust API behavior change outside the four symptoms of #74. State that the narrower option removes all four credential query keys but uses them only when the URI has no userinfo, as `from_str` does today. State that the narrower option changes task 1.2, the scenario "Query credentials fill only what the userinfo omits", the test `test_parse_query_credentials_fill_only_what_userinfo_omits`, and the third changelog entry.

## Feasibility

Checked without objection:

- The temporary-user driver manager tests (decision [8]) are feasible. `ALTER USER <name> IDENTIFIED BY "Ab?cd1234"` reaches the server through `FfiStatement::update_sql`, `Connection::execute_update`, and `Statement::build_sql`. The placeholder scanner in `build_sql` skips a `?` inside a double-quoted identifier (ADR-001, test `build_sql_does_not_substitute_question_mark_in_double_quoted_identifier`). Otherwise the statement would fail with `Not enough parameters bound`. The existing helper `execute_ddl` already sends DDL through `execute_update`. `generate_unique_test_name("EXARROW_CRED")` yields an upper-case regular identifier, so `SELECT CURRENT_USER` returns the name unchanged. CI runs the suite as `sys` with `--test-threads=1`. The reviewer did not run `CREATE USER` against Exasol. The claim that the test passwords are valid delimited identifiers rests on the Exasol references cited in [8].
- On the current code, tasks 3.4 (the `?` and `%41` cases), 3.5, and 3.6 fail, and tasks 3.2, 3.3, and 3.7 pass and act as regression locks.
- No production code logs connection parameters. `FfiDatabase` and `FfiConnection` do not implement `Debug`. The parser never parses option values, so no parse error can echo them.
- `ConnectionParams.attributes` has no reader outside `src/connection/params.rs`, so removing the credential keys changes nothing on the wire.

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Verification › Checklist, row "Unit test (FFI)" (`cargo test --lib --features ffi`, expected "0 failures"); plan.md § Scenario Coverage, last bullet ("The `ffi` unit test step of the Checklist runs them locally"); decision-log.md § [8] Consequences
- Issue: The plan assumes that the FFI unit suite passes. It fails today. The reviewer ran `cargo test --lib --features ffi adbc_ffi::tests` on HEAD 3601116 with the local Exasol container running: 96 passed and 2 failed. `test_autocommit_set_false` and `test_ffi_connection_options` call `set_option(AutoCommit, "false")` on an unconnected `FfiConnection`. That call opens a transaction, so `ensure_connected` dials `localhost:8563` with a URI that lacks `validateservercertificate=0`, and the TLS handshake fails (`invalid peer certificate: Other(OtherError(CaUsedAsEndEntity))`). Without a server, the dial fails as well (inferred, not run). Both tests still dial after this plan. The Checklist row therefore cannot reach "0 failures". It is also the only place where the plan runs the new FFI unit tests of task 2.5.
- Fix: In plan.md § Verification › Checklist, change the "Unit test (FFI)" command to `cargo test --lib --features ffi -- --skip test_autocommit_set_false --skip test_ffi_connection_options`. Add a § Context bullet stating that these two existing tests dial the server from `set_option(AutoCommit, "false")`, fail with or without Exasol, and stay out of scope. Name the new command in the last bullet of § Scenario Coverage and in decision-log.md § [8] Consequences.
- Escalation: MECHANICAL. The reviewer reproduced the failure from the repository alone, and the fix is a command change.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Verification › Checklist, row "Integration test"
- Issue: Task 1.2 rewrites `ConnectionParams::from_str`, which parses the URI of every connection in every test target. The Checklist runs `integration_tests` without `--features ffi`, unlike CI. It also omits the other CI targets that open connections: `websocket_integration_tests`, `native_protocol_tests`, `native_transport_smoke_test`, and `tests/python/test_driver_integration.py`. CI runs all of them on the PR, so a parser regression on those paths appears only after the push.
- Fix: In plan.md § Verification › Checklist, add `--features ffi` to the "Integration test" row. Add rows for `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1`, `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1`, `REQUIRE_EXASOL=1 cargo test --features ffi --test native_transport_smoke_test -- --test-threads=1`, and `pytest tests/python/test_driver_integration.py -v`. The last row runs after the FFI build and needs `pip install adbc-driver-manager pyarrow pytest polars`.

#### [NFR_IGNORED] ADVISORY
- Location: decision-log.md § [10] ("parse errors of the URI path that echo a malformed query pair, which can contain part of an unencoded password")
- Issue: The plan names a credential leak and leaves it open: `exasol://u:pa?ss@host` reports `Invalid query parameter format: ss@host`. The recorded scenario `connection-management/auth-and-security` › "No credential logging" states that the driver "SHALL redact credentials from error messages". This plan closes the leak for option credentials, and the brief supports leaving the URI path out of scope. The defect is that § [10] records no follow-up, so a known gap against a recorded security scenario has no owner. With this plan, the same error also surfaces earlier, at `AdbcConnectionInit`.
- Fix: In decision-log.md § [10], add the number of a follow-up issue for the URI-path echo, or state that the PR description asks the maintainer to file one.

## Requirement Quality

#### [AMBIGUOUS_REQUIREMENT] BLOCKER
- Location: specs/_plans/fix-ffi-credential-options/connection-management/credential-sources/spec.md § Scenario "An at sign in a query parameter value does not change the host", step "*AND* the client name SHALL be `dbt@ci`"
- Issue: The step does not say where the client name is observed, and the two readings give opposite results. The scenario's WHEN step is "a connection is created from the database", so the natural reading is the client name of the server session. That reading is false. `ConnectionParams.client_name` has no reader outside `src/connection/params.rs`. Both transports send the hard-coded name `exarrow-rs` at login (`src/transport/native/handshake.rs:34`, `src/transport/websocket.rs:513`). That gap is issue #75, which the brief excludes. The planned tests check only the parsed field (`test_parse_with_credentials_at_sign_in_query_value_keeps_host`, `test_ffi_database_at_sign_in_query_value_keeps_host`), and task 3.5 does not check the client name. After recording, the spec library would claim the behavior that #75 reports as broken.
- Fix: In that scenario, replace "*AND* the client name SHALL be `dbt@ci`" with "*AND* the connection parameters SHALL hold the `client_name` value `dbt@ci`". Add one sentence to decision-log.md § [10] stating that sending `client_name` to the server stays with #75.
- Escalation: MECHANICAL. The code settles the fact, and the fix is a wording change that does not affect behavior.

#### [IMPLEMENTATION_LEAKAGE] BLOCKER
- Location: specs/_plans/fix-ffi-credential-options/connection-management/credential-sources/spec.md § Background ("then the URI query parameters (`user` before `username`, `password` before `pass`)"); decision-log.md § [2] Consequences ("On the driver manager path, a `user` query parameter now counts when only the `password` option is set; the current FFI code ignores it and logs in as `sys`.")
- Issue: The Background states that the `user` query key takes precedence over `username`. No scenario step depends on that order. The only scenario with both keys, "Query credentials fill only what the userinfo omits", takes the username from the userinfo (`alice`), so the order of `user` and `username` never decides a result. The scenario does cover `password` before `pass`. The gap also leaves a consequence of decision [2] untested: a URI with `?user=bob` and only the `password` option logs in as `sys` today and as `bob` after this plan. That case is a variant of symptom 4, a login as a user the caller did not name.
- Fix: Add a scenario to the credential-sources spec.md: "Scenario: Query user applies when only the password option is set". GIVEN an ADBC database whose `uri` option is `exasol://db.example.com:8563?user=bob&username=carol`. AND its `password` option is `Secret1` and its `username` option is not set. WHEN a connection is created from the database. THEN the driver SHALL log in as `bob` with the password `Secret1`. AND the driver MUST NOT log in as `sys`. Add `test_parse_with_credentials_query_user_with_password_option` to task 1.4: `parse_with_credentials` with `None, Some("Secret1")` gives `bob` and `Secret1`. Add `test_ffi_database_query_user_with_password_option` to task 2.5: `build_connection_params()` returns username `bob`. Add both rows to § Scenario Coverage. The feature then holds 9 scenarios. If the planner does not add the scenario, delete "`user` before `username`, " from the Background instead, and record in decision-log.md § [2] that the driver manager consequence has no test.
- Escalation: MECHANICAL. The Background rule and the scenario text settle the defect.

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: specs/connection-management/auth-and-security/spec.md § Scenario "Connection string parsing" ("a connection string is provided in the format `exasol://host:port`"); specs/adbc-driver/driver-interface/spec.md § Background and § Scenario "Driver initialization" ("connection strings in the format `exasol://host:port`"); credential-sources/spec.md § Scenario "Missing username is rejected"
- Issue: Three recorded steps present `exasol://host:port` as a valid connection string. The new scenario "Missing username is rejected" states that a URI with no user and no `username` option fails with `Username is required`, which is also the current result of `from_str`. The recorded steps read as shorthand, because the scenario "Parameter validation" in the same feature rejects missing credentials. Task 4.1 corrects the same shorthand in `docs/setup-and-connect.md` but leaves the specs, so the spec library after recording holds both statements. Decision [7] says the existing scenarios stay accurate, but it names only "Parameter validation" and "No credential logging".
- Fix: Add one item to decision-log.md § [10] naming these three `exasol://host:port` steps as shorthand that this plan does not correct, next to the stale WebSocket statement already listed there. Alternatively, add DELTA:CHANGED blocks that change them to `exasol://user@host:port`. Both features hold 13 scenarios, so the recorder's threshold check may ask the user about them.

## Task Breakdown

No objection, axis checked: every scenario in the delta maps to at least one test in § Scenario Coverage, and every task traces to the spec delta, the architecture delta, or decisions [4] to [6]. One group fits, because tasks 2.x and 3.x call the function that task 1.2 adds, and all tasks share `src/connection/params.rs` and `src/adbc_ffi.rs`. Task 2.4 lists every constructor site the reviewer found: four `FfiConnection::new` calls including `unconnected_connection`, three `FfiConnection` struct literals, and the `FfiStatement::new` calls. The Dead Code Removal table matches the code: the standalone branches at `src/adbc_ffi.rs` lines 2020 to 2034 and 2052 to 2062 are the only readers of `FfiStatement::uri`.

## Design Depth

Checked without objection:

- `parse_with_credentials` is a deep interface. One call hides the URI grammar, the percent-decoding, and the precedence rule. The FFI layer stops knowing the URI format, which removes the back-door leakage behind #74, where two modules split the same string. `pub(crate)` is enough, because `adbc_ffi` is in the same crate.
- No decision-log entry sets `Promotes to ADR: yes`, so `[ADR_OVERPROMOTION]` and architecture-drift trigger (1) do not apply. `speq decision-log show` lists ten accepted ADRs. None covers credentials, URI parsing, or FFI error status. ADR-001 (placeholder lexer) supports decision [8], and the plan does not touch ADR-006 (best-effort URI schema).
- The architecture delta follows the template: the H1, a BASE comment equal to `git hash-object specs/architecture.md` (`3e25a7ed`), and four CHANGED blocks on canonical sections. A line diff against `specs/architecture.md` shows only the intended edits: two Components bullets, one added Data Flow bullet, two Interfaces bullets, and one Constraints bullet. No line is removed. `plan.md` states no component, interface, data flow, or constraint that the delta lacks.

#### [TACTICAL_SHORTCUT] ADVISORY
- Location: decision-log.md § [8] Consequences ("The CI filter stays unchanged."); decision-log.md § [10] ("the CI filter on FFI unit tests")
- Issue: The orchestrator asked whether the plan should extend the CI filter. Verdict: it should, because the change is one line, but leaving it out does not block the plan. CI builds the cdylib and runs the driver manager tests with `REQUIRE_EXASOL=1`, so tasks 3.2 to 3.7 give CI evidence for every FFI-path scenario. CI does not run the FFI unit tests that need no server: the five `test_ffi_database_*` tests and `test_ffi_statement_without_connection_does_not_dial`. The last one is the only test of decision [5]. A regression there surfaces only in the Exasol-bound suite, or not at all for decision [5], and the plan records no follow-up. libtest accepts several name filters: the reviewer confirmed that `cargo test --lib --features ffi -- arrow_value_to_parameter test_ffi_database --list` selects both groups.
- Fix: Add a task that changes the `.github/workflows/ci.yml` step "Run FFI parameter binding unit tests" to run `cargo test --lib --features ffi -- arrow_value_to_parameter test_ffi_database test_ffi_statement_without_connection_does_not_dial`, and renames the step to "Run FFI unit tests that need no server". None of the matched tests dials a server. Remove the CI-filter items from decision-log.md § [8] Consequences and § [10].

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Implementation Tasks 4.3, first changelog entry ("A password with `?`, `@`, `#`, or a `%XX` sequence no longer fails to log in.")
- Issue: The entry says that passwords with `@` or `#` failed before this fix. They did not. The reviewer traced the current `build_connection_uri` and `from_str`: `exasol://u:Ab@:/#cd1234@host:8563?tls=true` splits at the first `?`, then at the last `@`, then at the first `:`, and yields the password `Ab@:/#cd1234`. Only `?` (symptom 2) and a valid `%XX` sequence (symptom 3) fail. A changelog reader would look for a defect in passwords with `@` or `#` that never existed.
- Fix: In plan.md task 4.3, change the sentence to "A password with `?` or a `%XX` sequence no longer fails to log in."
