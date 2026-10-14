# Plan Review Findings: fix-connection-parameters (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 12 (Blockers: 1, Advisory: 11)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now this plan failed. Three failure stories:

1. A user puts the password `P@ss?word=1` unencoded in the URI. The parser splits at the first `?`, reads host `ss` from `alice:P@ss`, and the new unknown-key error prints `Invalid connection parameter 'word'` into a dbt or CI log. The spec library and the architecture still promise that a URI parse error never repeats a value from the URI, so nobody treats that log line as sensitive. Routed to `[REQUIREMENT_CONFLICT]` (BLOCKER) and `[UNSTATED_ASSUMPTION]` (ADVISORY).
2. A later release removes `idle_timeout`, as issue #75 proposes. The dbt Exasol adapter still writes a profile's `idle_timeout` into the URI and starts failing on connect. Its users never saw the deprecation warning, because a `log` record from the loaded cdylib reaches no logger of the host process, and no tracked follow-up announced when the removal would land. Routed to `[INTENT_DRIFT]` and `[TACTICAL_SHORTCUT]` (ADVISORY).
3. A contributor adds a key to the `match` in `apply_query_params` but not to the accepted-keys constant. The error message then omits a key the parser accepts, or advertises a key the parser rejects. Routed to `[INFORMATION_LEAKAGE]` (ADVISORY).

## Intent Fidelity

#### [INTENT_DRIFT] ADVISORY
- Location: decision-log.md § Design Decisions [2]; plan.md § Impact ("Deprecated" bullet); plan.md task 3.2 (`Deprecated:` entry)
- Issue: The user asked to "keep accepting `idle_timeout` with a deprecation warning". Issue #75 names the dbt Exasol adapter as the caller that sends `idle_timeout`. That adapter builds an ADBC database (`dbt_adbc` in `dbt-labs/dbt` `crates/dbt-auth/src/exasol/mod.rs`, which appends `idle_timeout={timeout}` to the URI). When the driver runs as the loaded cdylib, its `log::warn!` record reaches no logger of the host process, so the population the warning targets never sees it. Decision [2] Consequences and the session-and-lifecycle manual test ("no warning appears") record this, but plan.md § Impact and the changelog entry do not tell ADBC callers that they get no runtime signal.
- Fix: In plan.md § Impact "Deprecated" bullet and in the task 3.2 `Deprecated:` changelog entry, add one sentence: "Callers that load the driver through an ADBC driver manager, such as dbt and Python, see no runtime warning; the docs and this changelog are their notice." Keep decision [2] unchanged.

#### [SCOPE_REDUCTION] ADVISORY
- Location: plan.md § Implementation Tasks 3.5
- Issue: The user asked that "tasks must state ... cargo llvm-cov --lib + scripts/strip_test_coverage.py check (80.0 total / 50.0 per file)". Task 3.5 states the thresholds but names the commands only as "the coverage commands from the Checklist".
- Fix: In task 3.5, replace "the coverage commands from the Checklist" with the three commands: `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json`, and `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json`.

#### [SCOPE_REDUCTION] ADVISORY
- Location: plan.md § Context, last bullet ("Follow-up plans")
- Issue: The bullet says the session-attribute allow-list (issue #73, point 3) is "tracked as a separate issue". No such issue exists: searching exasol-labs/exarrow-rs issues (all states) for "session attribute" returns only #73 and #77, and for "writable" only #73. The deferred point has no tracker entry.
- Fix: Reword the bullet to "not filed yet; the PR description asks the maintainer to file it and link it from #73", or cite the issue number once someone files it before recording.

## Feasibility

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: decision-log.md § Design Decisions [4] Consequences; plan.md § Impact
- Issue: The plan accepts that the unknown-key error can print a fragment of a password (parse path in the Requirement Quality BLOCKER below). The user fixed "an error that names the key" but did not weigh this case. The plan records the exposure only in a decision-log Consequences line and a docs note, while it sends the error text into application, dbt, and CI logs.
- Fix: Add a bullet to plan.md § Impact so the PR reviewer sees and accepts the trade-off: "An unknown-key error names the key as written. When a password holds an unencoded `?`, part of the password can become that key and appear in the error."

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md task 1.4; plan.md § Dead Code Removal, "Test" row
- Issue: Task 1.4 removes "the builder test that calls `.attribute("custom", "value")` (around line 643)". That test is `test_builder_full` (`src/connection/params.rs:632-657`), which also asserts host, port, username, password, schema, both timeouts, TLS, and client name, so deleting it drops those checks. The task also omits two reads of the removed field that stop compiling: `test_builder_idle_timeout` asserts `params.idle_timeout` (line 878) after its call at line 874 goes, and `test_builder_default_values` asserts the 600-second default (line 916).
- Fix: In task 1.4, replace "the builder test that calls `.attribute("custom", "value")` (around line 643)" with "the `.attribute("custom", "value")` call (line 643) and its assertion (line 656) in `test_builder_full`". Replace "the `.idle_timeout(..)` call in the builder test around line 874" with "the test `test_builder_idle_timeout` (lines 869-879)". Add "the `idle_timeout` assertion in `test_builder_default_values` (line 916)". Make the Dead Code Removal "Test" row match.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Context ("`Cargo.lock` already holds `log` 0.4.32 as a transitive dependency"); decision-log.md [2] Rationale
- Issue: `cargo tree -i log --offline` prints nothing for the default features. `log` enters the dependency graph only through `tokio-tungstenite` and `tungstenite`, which the optional `websocket` feature pulls in. No new crate enters `Cargo.lock`, so the cargo-deny claim holds, but the default `native` build and the `ffi` cdylib compile `log` for the first time. "Already holds" invites the opposite reading.
- Fix: In the Context bullet and in decision [2] Rationale, state that `Cargo.lock` holds `log` 0.4.32 through the optional `websocket` feature, so cargo-deny sees no new crate, and that the default and `ffi` builds gain `log` as a new compiled dependency.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: architecture.md § Constraints (credentials bullet); decision-log.md [4] Consequences; recorded `connection-management/credential-sources` feature description
- Issue: The delta's Constraint says "Credentials are never logged or exposed, ... and a connection URI parse error names the field, the query key, or the query parameter position and never repeats a value from the URI". The recorded credential-sources description says "A password that is not percent-encoded can shift into the host, port, or query part of the URI, so an error from parsing the URI names the field at fault and never repeats a value from the URI." Decision [4] Consequences says the opposite: "An unencoded `&` or `?` in a password can split off a fragment that becomes a query key, and the error then names that fragment." The code confirms decision [4]. `parse_with_credentials` (`src/connection/params.rs:93-147`) splits at the first `?` before it searches for `@`. So `exasol://alice:P@ss?word=1@db.example.com:8563` parses user `alice`, host `ss`, and query key `word`, and the new catch-all names `word`, a fragment of the password `P@ss?word=1`. After recording, the spec library and the architecture promise a guarantee that the code breaks. The "`&` or `?`" wording is also wrong: without a `?`, an `&` stays inside the password, because the split at `?` happens first.
- Fix: (1) In `specs/_plans/fix-connection-parameters/connection-management/credential-sources/spec.md`, add a `<!-- DELTA:CHANGED -->` block anchored on `# Feature: Credential Sources`. Copy the current description in full and replace its last sentence with: "A password that is not percent-encoded can shift into the host, port, or query part of the URI. An error from parsing the URI names the field at fault, the query key, or the position of a query parameter without `=`, and never repeats the userinfo or a query value. The error names an unknown query key as written, so a password with an unencoded `?` can show a fragment in that error." (2) In architecture.md § Constraints, rewrite the credentials bullet to match: a URI parse error never repeats the userinfo or a query value, and names an unknown query key as written. (3) In decision-log.md [4] Consequences and in plan.md task 3.1 "URL Encoding", replace "an unencoded `&` or `?`" with "an unencoded `?`, and any `&` after it,".
- Escalation: MECHANICAL: the user fixed that the error names the key, and decision [4] already accepts the consequence; the defect is spec and architecture text that contradicts the plan's own decision, which the artifacts and the code settle.

#### [COMPLETENESS_GAP] ADVISORY
- Location: connection-management/auth-and-security/spec.md, Scenario: Unknown connection-string parameter is rejected; plan.md task 1.3
- Issue: The scenario covers one unknown key per URI. `parse_query_params` collects the pairs into a `HashMap`, and `apply_query_params` iterates it in arbitrary order. A URI with two unknown keys, or with an unknown key and an invalid `timeout` value, reports a different error from run to run, and a caller who fixes the named key meets the next one on the following attempt.
- Fix: In task 1.3, check every key against the accepted-keys constant before applying any value, and name all unknown keys in sorted order in one error, with `parameter` set to the first. Add a step to the scenario: "*AND* the URI `exasol://alice:Secret1@db.example.com:8563?autocommit=Qx7&queryTimeout=Qx7` fails with an error that names both `autocommit` and `queryTimeout`".

## Task Breakdown

No objection: axis checked. Every scenario in the three deltas maps to a test in § Scenario Coverage (9 rows), and every task traces to a delta, the docs fix, or the release. Groups A, B, and C run in sequence, and the plan names the file overlap behind each dependency (`src/connection/params.rs`, `src/adbc/connection.rs`). The cited code locations match this branch: `connect_with_transport` lines 187 and 211, test lines 2833 and 2842, `build_login_packet` test calls at lines 342-416, native `login_exchange` at line 932, `ConnectionBuilder::build` default at line 399. The URI keys used in `src/`, `tests/` (including `tests/python`), `examples/`, `benches/`, `docs/`, and the dbt adapter (`tls`, `validateservercertificate`, `certificatefingerprint`, `timeout`, `query_timeout`, `idle_timeout`) are all accepted keys, so the strict check breaks no existing test or the dbt URI.

## Design Depth

ADR and architecture check: decision [4] (`Promotes to ADR: yes`) names rule-2 criteria 2 and 4, states the ADR digest search, holds no paths or signatures, and carries `Architecture: § Interfaces, § Constraints`, both present in the delta. No accepted ADR conflicts: `server-enforced-query-timeout` covers only how `query_timeout` reaches the server. The delta follows the template: H1, BASE `e1893b3dd103cde5ce4f309c02ee775e556ad51e` equal to `git hash-object specs/architecture.md`, four `DELTA:CHANGED` canonical blocks with bullets only. Decision [1] (identity on transport `Credentials`) keeps the `TransportProtocol` signatures unchanged and the mock untouched, as claimed.

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md task 1.3
- Issue: The accepted key set lives in two places that must agree: the `match` arms in `apply_query_params` (plus the four credential keys that `parse_with_credentials` removes first) and the new constant that feeds the error message. Nothing enforces agreement. A key added to one place and not the other makes the error advertise a rejected key or omit an accepted one.
- Fix: In task 1.2, add a unit test that parses `exasol://alice@db.example.com:8563?<key>=<valid sample value>` for every entry of the constant except the credential keys and asserts success, so a constant entry without a `match` arm fails the tests.

#### [TACTICAL_SHORTCUT] ADVISORY
- Location: plan.md § Context, "Follow-up plans" bullet
- Issue: The deprecated `idle_timeout` query key and the `#[deprecated]` `ConnectionBuilder::idle_timeout` are a transitional state. Issue #75 proposes to "remove it in a later release", but the plan schedules no removal: the Follow-up plans bullet lists only the session-attribute allow-list.
- Fix: Add a second Follow-up plans entry: remove the `idle_timeout` query key and `ConnectionBuilder::idle_timeout` in a later breaking release, after the dbt adapter stops sending the key (issue #75, point 4).

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Context, bullet on `fix-execute-schema`
- Issue: "The parallel unit `fix-execute-schema` edits `src/adbc_ffi.rs`. This plan does not edit that file. Its edits in `src/adbc/connection.rs` are two lines ..." The word "Its" can refer to the parallel unit or to this plan.
- Fix: Replace "Its edits" with "This plan's edits".

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md task 3.1, "Parameters" and "Remove the `idle_timeout` row" bullets
- Issue: The docs will say that "a key missing from the table fails the parse", and the same task removes the `idle_timeout` row while the parser still accepts `idle_timeout`. Only the note below the table resolves the contradiction.
- Fix: In task 3.1, change the Parameters text to "a key that is neither in the table nor the deprecated `idle_timeout` fails the parse with an error that names the key and lists the accepted keys".
