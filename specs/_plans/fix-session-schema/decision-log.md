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

**Q:** How is the PR stacked and versioned?
**A:** The base is `feat/fix-ffi-bound-execute` until PR #90 merges, and #90 already bumps the version to 0.20.0. The plan decides whether this PR bumps to 0.21.0 and sets the CHANGELOG header accordingly.

**Q:** Which reproductions did the orchestrator run on this branch?
**A:** All four #77 rows reproduce: `/ZZ_MixedCase` connects with `CURRENT_SCHEMA` NULL, `/zz-hyphen` fails at `OPEN SCHEMA` with a syntax error, `/ZZ_TYPO` connects with no warning, and setting `CurrentSchema` leaves the server's schema NULL. #72 reproduces through a missing URI schema with `AutoCommit=false`, and through a failing `SELECT` on a healthy connection followed by `AutoCommit=false`. The second path remains after #77.

**Q:** How should the transports carry the current schema?
**A:** Check both transports for sending the attribute at login and through `setAttributes` and `getAttributes`, research the protocol in `src/transport/**`, and decide per transport.

**Q:** Is the dbt impact of reverting #39 an escalation?
**A:** No. Record it as a release-ordering risk in the plan unless the intent conflicts. The issue calls the revert required and says the dbt fix must ship before or with the release.

## Design Decisions

### [1] A URI schema must exist and is set with set-attributes after the login

- **Decision:** On both transports, the driver sets a schema from the connection URI or `ConnectionParams` with the protocol's set-attributes command right after the login. Any rejection closes the session and fails the connect. The driver never sends `OPEN SCHEMA` for it and never inspects the server's error text.
- **Alternatives:** Keep the best-effort default: rejected, because a mistyped schema connects silently, and the dbt case belongs in the dbt adapter. Send the schema as a login attribute, as #77 proposes: rejected, because the server rejects a missing schema at login with SQL state `08004`, the same state as a wrong password, so the driver could report it only as an authentication failure or tell the two apart by matching the error text. Quote the name in `OPEN SCHEMA "name"`: rejected, because quoting drops the server's upper-case fallback, so `/myschema` would stop opening `MYSCHEMA`.
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
  - The native transport records attribute 22 in `receive_into_buf`, the one receive path for all responses. An absent attribute 22 means "unchanged", except in a `CMD_GET_ATTRIBUTES` response, which lists every attribute, so there it means "no current schema". The login's existing `CMD_GET_ATTRIBUTES` seeds the value.
  - The WebSocket transport reads the top-level `attributes.currentSchema` member once per response in `send_receive`. It sends no extra `getAttributes` at login, because a fresh session has no current schema.
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
- **Alternatives:** An RAII guard that sets `Executing` and restores the state on drop, as #72 proposes: rejected, because `Session.state` is a `tokio::sync::RwLock` that `Drop` cannot await, and because the state carries no information: every operation takes `&mut self` and the FFI serializes access through a `Mutex`, so `Executing` at the start of an operation can only be a leak, as the issue's own analysis states. One shared wrapper around the execution paths: rejected, because it restores the state on `?` returns but not on a dropped future unless it also holds a guard. A transition into `Error` after transport failures: rejected, because the transport already reports a terminated or closed connection through `ConnectionState` and `is_closed()`.
- **Rationale:** Removing the state removes the leak instead of guarding against it. Every exit path, including a dropped future, leaves the state the operation started with. The issue's points 2 to 4 then reduce to `can_execute` in both checks, `ConnectionClosed` only for `Closing` and `Closed`, and no unused states.
- **Consequences:**
  - The issue's test "`validate_ready()` rejects `Executing`" has no subject. The closed-session and second-transaction tests replace it.
  - A caller that drops an execution mid-request can still leave an unread response on the transport. That desync predates this plan, is not a session-state question, and stays out of scope. ADR `client-give-up-terminates-connection` constrains only a give-up that the driver decides.
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

- **Decision:** This PR sets the version to 0.21.0 and adds a `## 0.21.0` CHANGELOG section above `## 0.20.0`. It merges after #90 reaches `main`.
- **Alternatives:** Entries under `## [Unreleased]` without a bump: rejected, because the change is breaking for URI users and for the Rust API, and an unreleased entry would ship in a later, unplanned release. 0.20.1: rejected, because a breaking change in a 0.x crate takes a minor bump under SemVer.
- **Rationale:** AGENTS.md and the Releasing procedure in CONTRIBUTING.md tie the CHANGELOG header to the `Cargo.toml` version.
- **Consequences:**
  - Merging the PR publishes 0.21.0 to crates.io.
  - The dbt adapter change must ship before or with 0.21.0. The issue and the brief both call the revert required, so this is a documented release-ordering risk, not an escalation.
  - Task 1.10 is the only version bump of this PR. The version-bump step of the PR pipeline (`/speq:implement-pr` A3) leaves 0.21.0 and the `## 0.21.0` header unchanged, because the plan has no `workspace/version` delta and that step's Conventional Commits fallback would otherwise bump the version a second time.
- **Promotes to ADR:** no

### [9] Scope boundaries

- **Decision:** The plan leaves bound execute in `src/adbc_ffi.rs`, the connection-timeout deadline, parameters, and date conversion untouched. It does not fix the claim in `docs/setup-and-connect.md` § Session Attributes that unrecognized URI query parameters are forwarded as session attributes; `ConnectionParams::attributes` is never sent to the server, so `?currentSchema=` is not a supported way to set the schema.
- **Alternatives:** Fix the attribute forwarding claim in the same PR: rejected, because it is a separate documentation bug outside #77 and #72.
- **Rationale:** The brief limits the PR to #77 and #72.
- **Promotes to ADR:** no

## Review Findings

### [10] [plan-review] The PR pipeline would bump the version a second time

- **Finding:** Task 1.10 sets the version to 0.21.0 inside the implementation. The PR pipeline's version-bump step (`/speq:implement-pr` A3, `speq-implement-pr` step 3) falls back to a Conventional Commits bump when a plan has no `workspace/version` delta. On top of task 1.10, that step would produce 0.21.1 (or 0.20.1 from the base). The CI `release` job would then publish that version to crates.io with empty release notes, because it matches the `## X.Y.Z` CHANGELOG header exactly.
- **Direction change:** `plan.md` § Dependencies, task 1.10, and entry [8] Consequences now state that task 1.10 is the only version bump of this PR and that the pipeline's version-bump step leaves `Cargo.toml` at 0.21.0 and the `## 0.21.0` header unchanged.
- **Promotes to ADR:** no
