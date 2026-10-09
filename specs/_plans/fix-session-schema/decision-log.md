# Decision Log: fix-session-schema

## Interview

The run is headless. The orchestrator's brief stands in for the interview; each pair below paraphrases one part of it.

**Q:** What is in scope?
**A:** One bug-fix PR for GitHub issues #77 and #72. #77 comes first in the task order and #72 after it. Both touch `src/adbc/connection.rs` and the session state handling. #72 stays a real bug after #77.

**Q:** What does #77 propose?
**A:** Send the URI schema as a login attribute, route `adbc.connection.db_schema` set and get through `setAttributes` and `getAttributes`, refresh the cached schema from every response that carries the attribute, drop `OPEN SCHEMA`, revert the #39 "ignore schema not found" behavior so that a missing schema fails the connection with a clear error and no error-text matching, update `docs/setup-and-connect.md` (the URI schema must exist; the server's case rule), and add a CHANGELOG entry. The issue's case-handling table is the server contract to specify.

**Q:** What does #72 propose?
**A:** Restore the session state on every exit path (an RAII guard or one shared wrapper, including dropped futures), make `validate_ready` reject `Executing` through `can_execute`, report accurate invalid-state errors (`ConnectionClosed` only for `Closing` and `Closed`), remove the unused `Error` and `Idle` states or give them transitions, add regression tests, and specify the rules in `specs/connection-management/session-and-lifecycle`.

**Q:** What is out of scope?
**A:** #73 and #75 (parameters), #76 and #57 (deadlines), #84 (dates, already fixed on this branch), the dbt adapter change (external repository; record it only as a release-ordering dependency and risk), and any change to bound execute in `src/adbc_ffi.rs`. The `CurrentSchema` option handling of `FfiConnection` is in scope.

**Q:** What is the base, and how is the PR versioned?
**A:** The base is `main`. PR #90 is merged, `Cargo.toml` on `main` is at 0.20.1, and the top header of `CHANGELOG.md` is `## 0.20.1`. This PR targets `main`. The version moves from 0.20.1 to 0.21.0, with a new `## 0.21.0` header above `## 0.20.1`. Task 1.10 is the only version bump, and the PR pipeline does not bump again.

**Q:** Which reproductions did the orchestrator run on this branch?
**A:** All four #77 rows reproduce: `/ZZ_MixedCase` connects with `CURRENT_SCHEMA` NULL, `/zz-hyphen` fails at `OPEN SCHEMA` with a syntax error, `/ZZ_TYPO` connects with no warning, and setting `CurrentSchema` leaves the server's schema NULL. #72 reproduces through a missing URI schema with `AutoCommit=false`, and through a failing `SELECT` on a healthy connection followed by `AutoCommit=false`. The second path remains after #77.

**Q:** How should the transports carry the current schema?
**A:** Check both transports for sending the attribute at login and through `setAttributes` and `getAttributes`, research the protocol in `src/transport/**`, and decide per transport.

**Q:** Is the dbt impact of reverting #39 an escalation?
**A:** No. Record it as a release-ordering risk in the plan unless the intent conflicts. The issue calls the revert required and says the dbt fix must ship before or with the release.

## Design Decisions

### [1] A URI schema must exist and is set with set-attributes after the login

- **Decision:** On both transports, the driver sets a schema from the connection URI or `ConnectionParams` with the protocol's set-attributes command right after the login. Any rejection closes the session and fails the connect. The driver never sends `OPEN SCHEMA` for it and never inspects the server's error text.
- **Alternatives:**
  - Keep the best-effort default: rejected. A mistyped schema connects silently. The dbt case belongs in the dbt adapter.
  - Send the schema as a login attribute, as #77 proposes: rejected. The server rejects a missing schema at login with SQL state `08004`. A wrong password fails with the same SQL state. The driver could then report a missing schema only as an authentication failure, or tell the two apart by matching the error text.
  - Quote the name in `OPEN SCHEMA "name"`: rejected. Quoting drops the server's upper-case fallback, so `/myschema` would stop opening `MYSCHEMA`.
- **Rationale:** `/speq:adr-rules` criterion 4: the decision rejects plausible alternatives, and a later reader needs the reason, because the login attribute saves a round trip and invites a later change back. Criterion 2: the rule binds both transports and every connect path. `speq decision-log show` lists ADR `uri-specified-schema-is-best-effort-default`, which this decision replaces. No other ADR covers schema activation. The protocol spike in `plan.md` Context is the evidence.
- **Consequences:**
  - The connect error is `ConnectionError::ConnectionFailed`, and its message names the schema and contains the server's message.
  - A connect with a URI schema takes two round trips after the login (set-attributes, then get-attributes to learn the opened name), where `OPEN SCHEMA` took one.
  - The schema step stays outside the connection-timeout deadline, as `OPEN SCHEMA` was.
  - A dbt run whose target schema does not exist yet fails at connect until the dbt adapter connects without the schema; `plan.md` Dependencies records the release ordering.
- **Supersedes:** uri-specified-schema-is-best-effort-default
- **Architecture:** § Data Flow, § Interfaces
- **Promotes to ADR:** yes

### [2] The transport owns the server-reported current schema

- **Decision:** The native and WebSocket transports record the current schema from every response that carries the attribute and from every get-attributes response. `TransportProtocol` exposes `set_current_schema`, `refresh_current_schema`, and `current_schema`. `Session` no longer stores a current schema, and `Connection::current_schema()` reads the transport's value without a server request.
- **Alternatives:** Keep the value in `Session` and copy it out of each transport result in `Connection`: rejected, because `ResultSet` fetches and import and export SQL reach the transport outside `Connection`'s execute path, and the attribute format would leak into `Connection`, giving one fact two owners. Send get-attributes on every `current_schema()` call: rejected, because `current_schema()` is infallible and local today, and a round trip per read changes that contract.
- **Rationale:** The transport is the only module that sees every response, so it is the one owner of the rule "which response field is the current schema". Three small methods hide two wire formats and the native snapshot rule. The dependency direction stays adbc to transport.
- **Consequences:**
  - The native transport records a present attribute 22 in `receive_into_buf`, the one receive path for all responses. `receive_into_buf` does not know which command a response answers, so there an absent attribute 22 leaves the value unchanged.
  - A `CMD_GET_ATTRIBUTES` response lists every attribute, so an absent attribute 22 there means "no current schema". The two native call sites that send `CMD_GET_ATTRIBUTES` apply this rule after the receive: `refresh_current_schema` and the login's existing `CMD_GET_ATTRIBUTES` step, which seeds the value. Passing the sent command into the receive path was rejected, because only these two call sites need it.
  - The WebSocket transport reads the top-level `attributes.currentSchema` member once per response in `send_receive`, through an `attributes` field on each response type and a trait that `send_receive` reads. That field declares only `currentSchema`, because a `getAttributes` response mixes strings, booleans, and numbers. The transport sends no extra `getAttributes` at login, because a fresh session has no current schema.
  - Neither set-attributes response echoes the attribute (spike), so `set_current_schema` ends with a get-attributes request to learn the name the server opened.
- **Architecture:** § Components, § Data Flow
- **Promotes to ADR:** no

### [3] The ADBC db_schema option sets through the session and reads from the server

- **Decision:** Setting `adbc.connection.db_schema` establishes the session if it does not exist and calls `Connection::set_schema`. Reading it on an established session sends get-attributes and returns the server's value, or `NOT_FOUND` when there is none. Before the session exists, the read returns the URI schema, or `NOT_FOUND`, without dialing.
- **Alternatives:** Read the transport's recorded value: rejected, because #77 asks for the server's value, and a get-attributes read also covers a change that no response of this session reported, such as another session dropping the schema. Store a set made before the session exists and apply it at connect: rejected, because a typo would surface at the first statement instead of at the option call. Dial on a read before the session exists: rejected, because `get_option_string` takes `&self`, and filling the lazy connection slot from it would change `FfiConnection`'s connection handling, which the out-of-scope bound execute path shares. Report `InvalidState` before the session exists, as GetObjects does: rejected, because a driver manager may read the option right after connect, and the URI schema is the value the login applies.
- **Rationale:** Setting the option follows the existing `AutoCommit` pattern, which also establishes the session. The Rust `current_schema()` keeps its contract.
- **Consequences:** Before the session exists, the read returns the URI text as given, not the server-resolved name: `/myschema` reads `myschema` until the session exists, then `MYSCHEMA`. Python DBAPI `connect()` with its default `autocommit=False` establishes the session at connect, so it reads the server's value.
- **Promotes to ADR:** no

### [4] Execution no longer changes the session state

- **Decision:** No execution path sets a session state. `SessionState` keeps `Ready`, `InTransaction`, `Closing`, and `Closed`. `Initializing`, `Executing`, `Idle`, `Error`, and `SessionState::is_active` are removed, and `validate_ready` and `begin_transaction` use `can_execute`.
- **Alternatives:**
  - An RAII guard that sets `Executing` and restores the state on drop, as #72 proposes: rejected for two reasons. First, `Session.state` is a `tokio::sync::RwLock`, and `Drop` cannot await it. Second, the `Executing` state carries no information. Every operation takes `&mut self`, and the FFI serializes access through a `Mutex`. `Executing` at the start of an operation can therefore only be a leak, as the issue's own analysis states.
  - One shared wrapper around the execution paths: rejected. It restores the state on `?` returns. It does not restore the state on a dropped future unless it also holds a guard.
  - A transition into `Error` after transport failures: rejected. The transport already reports a terminated or closed connection through `ConnectionState` and `is_closed()`.
- **Rationale:** Removing the state removes the leak instead of guarding against it. Every exit path, including a dropped future, leaves the state the operation started with. The issue's points 2 to 4 then reduce to `can_execute` in both checks, `ConnectionClosed` only for `Closing` and `Closed`, and no unused states.
- **Consequences:**
  - The issue's test "`validate_ready()` rejects `Executing`" has no subject. The closed-session and second-transaction tests replace it.
  - A caller that drops an execution mid-request can still leave an unread response on the transport. That desync predates this plan and is not a session-state question, so it stays out of scope. `plan.md` § Dependencies schedules a follow-up issue for it. ADR `client-give-up-terminates-connection` constrains only a give-up that the driver decides.
  - Removing public enum variants breaks Rust code that matches on `SessionState`.
- **Promotes to ADR:** no

### [5] begin_transaction checks the session before it changes autocommit

- **Decision:** `Connection::begin_transaction` checks the session state and the active-transaction flag before it sends `set_autocommit(false)`.
- **Alternatives:** Keep the current order: rejected, because a rejected start would already have switched the server's autocommit off.
- **Rationale:** The scenarios "A closed session rejects operations as closed" and "Starting a second transaction reports the active transaction" forbid a server request for a rejected start.
- **Promotes to ADR:** no

### [6] The #72 regression tests use a failing statement

- **Decision:** The #72 regression tests fail a statement on a healthy connection and then start a transaction, in unit, integration, and driver manager tests.
- **Alternatives:** The issue's tests "a missing URI schema with `autocommit=false` connects and can commit" and "connecting with a missing URI schema leaves the session `Ready`": rejected, because after #77 a missing URI schema fails the connect, so these tests contradict the schema-activation delta.
- **Rationale:** The brief names the failing-statement path as the one that remains after #77.
- **Promotes to ADR:** no

### [7] Spec placement

- **Decision:** All current-schema behavior, including the ADBC option, lives in `connection-management/schema-activation`. The wire rules for each transport live in `native-client/protocol` and `websocket-client/protocol`. The session state rules live in `connection-management/session-and-lifecycle`.
- **Alternatives:** Specify the ADBC option in `adbc-driver/driver-interface`: rejected, because it would split one behavior (set and read the current schema) across two features.
- **Rationale:** `schema-activation` already covers the ADBC FFI path of the URI schema. The absence check (`speq search query` for "ATTR_CURRENT_SCHEMA", "OPEN SCHEMA", "adbc.connection.db_schema option", "CurrentSchema connection option get set", "set session attributes setAttributes", "session state invalid error", "begin transaction after failed statement"; `speq feature get` of the named features; `speq decision-log show`) found no scenario for current-schema tracking, the ADBC option, or session state rules.
- **Promotes to ADR:** no

### [8] Release as 0.21.0

- **Decision:** This PR targets `main` and moves the version from 0.20.1 to 0.21.0. It adds a `## 0.21.0` CHANGELOG section above `## 0.20.1`.
- **Alternatives:**
  - Entries under `## [Unreleased]` without a bump: rejected. The change is breaking for URI users and for the Rust API. An unreleased entry would ship in a later, unplanned release.
  - 0.20.2: rejected. A breaking change in a 0.x crate takes a minor bump under SemVer.
- **Rationale:** AGENTS.md and the Releasing procedure in CONTRIBUTING.md tie the CHANGELOG header to the `Cargo.toml` version.
- **Consequences:**
  - Merging the PR publishes 0.21.0 to crates.io.
  - The dbt adapter change must ship before or with 0.21.0. The issue and the brief both call the revert required, so this is a documented release-ordering risk, not an escalation.
  - Task 1.10 is the only version bump of this PR. The version-bump step of the PR pipeline (`/speq:implement-pr` A3) leaves 0.21.0 and the `## 0.21.0` header unchanged, because the plan has no `workspace/version` delta and that step's Conventional Commits fallback would otherwise bump the version a second time.
- **Promotes to ADR:** no

### [9] Scope boundaries

- **Decision:** The plan leaves bound execute in `src/adbc_ffi.rs`, the connection-timeout deadline, parameters, and date conversion untouched. Task 1.9 deletes the `currentSchema` row from the `docs/setup-and-connect.md` § Session Attributes table. The driver never sends `ConnectionParams::attributes` to the server, so `?currentSchema=` does not set the schema. The plan leaves the general claim of that section, that unrecognized URI query parameters are forwarded as session attributes, to issue #73.
- **Alternatives:** Fix the whole attribute forwarding claim in the same PR: rejected. Issue #73 covers it, and the brief puts #73 out of scope.
- **Rationale:** The brief limits the PR to #77 and #72. The `currentSchema` row is in #77's documentation scope, because it names a way to set the schema that does nothing and reports no error.
- **Promotes to ADR:** no

## Review Findings

### [10] [plan-review] The PR pipeline would bump the version a second time

- **Finding:** Task 1.10 sets the version to 0.21.0 inside the implementation. The PR pipeline's version-bump step (`/speq:implement-pr` A3, `speq-implement-pr` step 3) falls back to a Conventional Commits bump when a plan has no `workspace/version` delta. On top of task 1.10, that step would produce 0.21.1 (or 0.20.2 from the 0.20.1 base on `main`). The CI `release` job would then publish that version to crates.io with empty release notes, because it matches the `## X.Y.Z` CHANGELOG header exactly.
- **Direction change:** `plan.md` § Dependencies, task 1.10, and entry [8] Consequences now state that task 1.10 is the only version bump of this PR and that the pipeline's version-bump step leaves `Cargo.toml` at 0.21.0 and the `## 0.21.0` header unchanged.
- **Promotes to ADR:** no

### [11] [plan-review] The base is main, and the version moves from 0.20.1

- **Finding:** The orchestrator reported changed facts with the round-1 review. PR #90 is merged. The plan branch is merged with `origin/main`, where `Cargo.toml` is at 0.20.1 and the top `CHANGELOG.md` header is `## 0.20.1`. PR #91 targets `main`. The plan still stacked on PR #90 and required #90 to merge first and this PR to be retargeted.
- **Direction change:** `plan.md` § Context states that the base is `main` at 0.20.1. § Dependencies drops the #90 merge-order bullet. Task 1.10, the Version checklist row, the Interview, and entry [8] place the `## 0.21.0` header above `## 0.20.1`, and entry [8] rejects 0.20.2 as the patch alternative. The rule of entry [10] stays: task 1.10 is the only version bump, and the pipeline does not bump again. The spec deltas and the architecture delta named no base branch, and the BASE hash of the architecture delta still matches `specs/architecture.md`.
- **Promotes to ADR:** no

### [12] [plan-review] The native receive path cannot tell a get-attributes response apart

- **Finding:** Task 1.2 recorded attribute 22 in `receive_into_buf` and also required that an absent attribute 22 in a `CMD_GET_ATTRIBUTES` response means none. `receive_into_buf` does not know which command a response answers.
- **Direction change:** Task 1.2 and entry [2] Consequences split the rule. `receive_into_buf` records only a present attribute 22, and an absent one leaves the value unchanged. `refresh_current_schema` and the login's `CMD_GET_ATTRIBUTES` step apply "absent means none" through a second pure helper.
- **Promotes to ADR:** no

### [13] [plan-review] The WebSocket response attributes need a typed field on every response

- **Finding:** `send_receive` deserializes straight into a generic `R`, and no response type has a top-level `attributes` field. The `getAttributes` response mixes value types, so the `HashMap<String, String>` pattern fails to deserialize it. A `serde(flatten)` envelope would buffer large fetch responses. Task 1.3 was untagged.
- **Direction change:** Task 1.3 names the mechanism: a response-attributes type that declares only `currentSchema`, an optional top-level `attributes` field on each of the ten response types, and a trait that bounds `R` in `send_receive`. Task 1.3 states the mixed-type fact, rejects `serde(flatten)`, and carries `[expert]`. Entry [2] Consequences names the mechanism.
- **Promotes to ADR:** no

### [14] [plan-review] Exasol-backed checklist rows could pass by skipping

- **Finding:** `skip_if_no_exasol!` skips unless `REQUIRE_EXASOL` is set. Four checklist rows ran without it, so a run without the container reported 0 failures with every new test skipped.
- **Direction change:** The Integration, WebSocket integration, Native protocol, and Driver manager rows of the `plan.md` checklist set `REQUIRE_EXASOL=1`.
- **Promotes to ADR:** no

### [15] [plan-review] CLOSE SCHEMA was missing from the SQL schema-change scenario

- **Finding:** The scenario "Current schema follows schema changes made in SQL" listed `OPEN SCHEMA`, `CREATE SCHEMA`, and `DROP SCHEMA`, but not `CLOSE SCHEMA`. The native spike result for `CLOSE SCHEMA` with a schema open was not shown.
- **Direction change:** The scenario's WHEN step and the schema-activation Background name `CLOSE SCHEMA`, and the THEN step covers a closed schema. Task 1.7 runs `CLOSE SCHEMA` with a schema open in both `..._follows_schema_changes_in_sql` tests. `plan.md` § Context states that the native result with a schema open is unverified and that the native test verifies it. No new spike ran.
- **Promotes to ADR:** no

### [16] [plan-review] No scenario covered two schemas that differ only in case

- **Finding:** The Impact lists that `/myschema` opens `myschema` when both `myschema` and `MYSCHEMA` exist, but no scenario step or test covered it.
- **Direction change:** The case-rule scenario's GIVEN holds `"zz_both"` and `ZZ_BOTH`, and a THEN step requires that the URI schema `zz_both` opens `zz_both`. Task 1.7 adds the case to both case-rule tests.
- **Promotes to ADR:** no

### [17] [plan-review] No task scheduled the Scenario doc-comment rule

- **Finding:** AGENTS.md requires one `/// Scenario: <title>` line per scenario on each test, but no task or verification step scheduled it. Two reused tests kept doc comments that describe `OPEN SCHEMA` on connect.
- **Direction change:** `plan.md` § Verification requires one verbatim `/// Scenario:` line per mapped scenario on every test in Scenario Coverage. Tasks 1.7 and 1.8 rewrite the `OPEN SCHEMA` comments on `test_uri_schema_is_opened_on_connect` and `test_ffi_uri_schema_is_opened_on_connect`.
- **Promotes to ADR:** no

### [18] [plan-review] Only a mock-based test asserted the kept schema after a rejection

- **Finding:** Each transport keeps the recorded current schema after a rejected set, but only a unit test against `MockTransport` asserted it, so it tested the mock.
- **Direction change:** Scenario Coverage states that `set_current_schema_returns_the_servers_rejection` (WebSocket) and `native_set_current_schema_reports_a_rejected_schema` (native) assert the kept value. The mock-based unit test is renamed `set_schema_returns_the_transports_error` and asserts only the returned error. Both transport "Set the current schema attribute" scenarios state that a rejection leaves the recorded current schema unchanged.
- **Promotes to ADR:** no

### [19] [plan-review] Two known defects had no scheduled follow-up

- **Finding:** `docs/setup-and-connect.md` § Session Attributes lists `currentSchema` as a URI query parameter that the driver never sends. A dropped execution can leave an unread response on the transport. Neither had a scheduled follow-up.
- **Direction change:** Task 1.9 deletes the `currentSchema` row, and entry [9] records the deletion. `plan.md` § Dependencies names issue #73 for the remaining forwarding claim. It also schedules a new follow-up issue for the transport desync, because no issue tracks it today.
- **Promotes to ADR:** no

### [20] [plan-review] Several sentences carried more than one claim

- **Finding:** Task 1.10, the first sentence of `plan.md` § Summary, and the Alternatives of entries [1] and [4] each put three or more claims into one sentence.
- **Direction change:** Each of those sentences is split into one sentence per claim. Task 1.10 lists the CHANGELOG entries as sub-bullets, and entries [1], [4], and [8] list their alternatives as sub-bullets.
- **Promotes to ADR:** no
