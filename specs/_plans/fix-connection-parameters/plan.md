# Plan: fix-connection-parameters

## Summary

The connection string rejects query parameters the driver does not implement, `client_name` and `client_version` reach the Exasol login on the native and the WebSocket transport, and `idle_timeout` becomes a deprecated parameter with no effect. The change fixes issues #73 and #75 and ships as its own release, expected 0.22.0.

## Context

- Issue #73: `apply_query_params` in `src/connection/params.rs` stores an unknown query key in `ConnectionParams.attributes`, and no code reads that map, so the driver drops the key silently. `docs/setup-and-connect.md` ("Session Attributes") claims the driver forwards such a key to Exasol as a session attribute.
- The URI keys `autocommit` and `queryTimeout` are unknown keys today. `query_timeout` is the working key, and autocommit works through the ADBC `AutoCommit` option and `Connection::begin_transaction()`.
- Issue #75: `client_name` and `client_version` parse into `ConnectionParams`, but the native login packet (`build_login_packet` in `src/transport/native/handshake.rs`) and the WebSocket login request (`login_exchange` in `src/transport/websocket.rs`) send the fixed name `exarrow-rs` and the crate version. `EXA_DBA_SESSIONS.CLIENT` therefore never shows the caller's values.
- Issue #75: `idle_timeout` parses into `ConnectionParams.idle_timeout` and is copied to `SessionConfig.idle_timeout`, and no code enforces it. Idle session cleanup was removed in #43.
- `connection::auth::AuthRequest` has no caller and sends nothing to the server. This plan leaves it unchanged.
- The crate has no logging facade. `Cargo.lock` holds `log` 0.4.32 through optional dependencies, such as `tungstenite` in the `websocket` feature, so cargo-deny sees no new crate. The default and `ffi` builds compile `log` as a new dependency.
- This branch is stacked on PR #93 (`fix-tls-import-handshake`, expected release 0.21.2). The architecture delta of PR #93 changes the same four sections of `specs/architecture.md` from the same BASE, so task 3.4 re-bases this plan's delta before recording.
- The parallel unit `fix-execute-schema` edits `src/adbc_ffi.rs`. This plan does not edit that file. This plan's edits in `src/adbc/connection.rs` are two lines in `connect_with_transport` and two lines in one unit test.
- Architecture changes (connection component, `Credentials`, login data flow, connection-string interface, constraints): see `architecture.md` in this plan directory.
- Follow-up plans:
  - An allow-list of writable session attributes in the connection string (issue #73, point 3). Not filed yet; the PR description asks the maintainer to file it and link it from #73.
  - Remove the `idle_timeout` query key and `ConnectionBuilder::idle_timeout` in a later breaking release, after the dbt adapter stops sending the key (issue #75, point 4).

## Features

| Feature | Status | Spec |
|---------|--------|------|
| auth-and-security | CHANGED | `connection-management/auth-and-security/spec.md` |
| credential-sources | CHANGED | `connection-management/credential-sources/spec.md` |
| session-and-lifecycle | CHANGED | `connection-management/session-and-lifecycle/spec.md` |

## Impact

- Breaking: a connection string with a query key the driver does not implement fails to parse. The parse fails with `ConnectionError::InvalidParameter` that names the key, so `Driver::open` fails, and an ADBC driver manager reports status `InvalidArguments` when it creates the connection. Keys are case-sensitive, so `queryTimeout`, `autocommit`, and spellings such as `validateServerCertificate` now fail. Before, the driver ignored them.
- Breaking (Rust API): `ConnectionParams.attributes`, `ConnectionBuilder::attribute`, `ConnectionParams.idle_timeout`, and `SessionConfig.idle_timeout` are removed. `transport::Credentials` gains the public fields `client_name` and `client_version`, and `transport::native::handshake::build_login_packet` takes `&Credentials`.
- An unknown-key error names the key as written. When a password holds an unencoded `?`, part of the password can become that key and appear in the error.
- Deprecated: the `idle_timeout` query key parses as before, has no effect, and produces a `log` warning. `ConnectionBuilder::idle_timeout` compiles with a deprecation warning and has no effect. Callers that load the driver through an ADBC driver manager, such as dbt and Python, see no runtime warning; the docs and the changelog are their notice.
- Fixed: Exasol shows `<client_name> <client_version>` in `EXA_DBA_SESSIONS.CLIENT` on both transports. The dbt Exasol adapter can identify its sessions with `client_name=dbt`.
- Release: a minor release of its own, expected 0.22.0.

## Requirements

| Requirement | Details |
|-------------|---------|
| Version | The next minor version above the `Cargo.toml` version on the base branch at implement time. That is 0.22.0 when PR #93 has released 0.21.2. The version changes if PR #93 or another release PR merges first. |
| Test database | Integration tests run against the Docker Exasol container `exasol-test` (one container system-wide, reuse a running one): `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`, ready when `exapump sql 'select 1'` prints `1`. Connection strings set `validateservercertificate=0`. |
| No skipping | New integration tests do not call `skip_if_no_exasol!`, so they fail when Exasol is unreachable. |
| Scenario lines | Every test that implements a scenario carries one `/// Scenario: <title>` line per scenario, with the title quoted verbatim. |
| Comments | Code comments state only a non-obvious reason, in 1 or 2 lines. |
| Quality gates | `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -W clippy::all` with zero warnings, unit coverage at least 80.0% total and 50.0% per file. Coverage runs with default features only, never with the `ffi` feature. |

## Dependencies

- `log` 0.4 as a direct dependency (decision-log entry [2]). No new crate enters `Cargo.lock`.
- PR #93 merges first. Its release decides the version of this plan (task 3.3), and its recorded architecture delta decides the base of this plan's delta (task 3.4).
- Manual testing needs the Python package `adbc_driver_manager`, which `tests/python/test_driver_integration.py` already uses.

## Migration

| Current | New |
|---------|-----|
| `?autocommit=false` | ADBC connection option `AutoCommit` set to `false`, or `Connection::begin_transaction()` |
| `?queryTimeout=30` | `?query_timeout=30` |
| `?idle_timeout=600` | Remove the key. It never closed idle connections. |
| `ConnectionBuilder::attribute(key, value)` | Remove the call. The value never reached the server. |
| Reading `ConnectionParams.idle_timeout` or `SessionConfig.idle_timeout` | Remove the read. |
| `transport::Credentials { username, password }` struct literal | `Credentials::new(username, password)`, optionally `.with_client(name, version)` |
| `build_login_packet(username)` | `build_login_packet(&credentials)` |

## Implementation Tasks

### 1. Connection-string parameter validation

- [ ] 1.1 Add `log = "0.4"` to `[dependencies]` in `Cargo.toml` and run `cargo check`. Confirm that `Cargo.lock` gains only the `log` entry in the dependency list of `exarrow-rs` and no new package.
- [ ] 1.2 Write the failing unit tests in the test module of `src/connection/params.rs`, each with its `/// Scenario:` line:
  - `unknown_query_parameters_are_rejected_by_name` replaces `test_parse_custom_attribute_param` (Scenario: Unknown connection-string parameter is rejected). It parses both URIs of the scenario and asserts `ConnectionError::InvalidParameter` whose `parameter` is the key, a message that contains `query_timeout` and not `idle_timeout`, and a message without `Qx7` or `Secret1`.
  - `idle_timeout_is_accepted_with_a_deprecation_warning` replaces `test_parse_idle_timeout` (Scenario: Idle timeout). A test `log::Log` implementation records `Warn` records. The test installs it once per test binary with `std::sync::OnceLock`, sets the maximum level to at least `Warn`, and inspects only records that name `idle_timeout`, because unit tests run in parallel threads of one process. The test asserts that the parse succeeds and that a record names `idle_timeout`, says that it is deprecated and has no effect, and does not contain `120`.
  - `accepted_query_keys_all_parse`: for every entry of the accepted-keys constant from 1.3 except `user`, `username`, `password`, and `pass`, parse `exasol://alice@db.example.com:8563?<key>=<valid sample value>` and assert success, so a constant entry without a `match` arm fails the tests.
  - `test_parse_invalid_idle_timeout_value` gets the line for Scenario: Idle timeout with an invalid value, uses the value `Qx7`, and asserts that the message does not contain `Qx7`.
  - `test_parse_query_credentials_fill_only_what_userinfo_omits` drops its `params.attributes` assertions and asserts that the parse succeeds with username `alice` and password `Secret1` (Scenario: Query credentials fill only what the userinfo omits). It passes before and after the change and guards the accepted credential keys.
  - Run `cargo test --lib connection::params`. Confirm that the first two tests fail, the first because the unknown key parses and the second because no warning is emitted.
- [ ] 1.3 In `apply_query_params`, replace the catch-all arm with a `ConnectionError::InvalidParameter` error whose `parameter` is the key and whose message lists the accepted keys, without the value (decision-log entry [4]). Keep the accepted key names, including `user`, `username`, `password`, and `pass` and excluding `idle_timeout`, in one constant next to the `match`. The `idle_timeout` arm keeps the `parse_timeout` validation, discards the value, and calls `log::warn!` with a message that names the key and never the value.
- [ ] 1.4 Remove the API that decision-log entry [3] names. In `src/connection/params.rs`: `ConnectionParams.attributes`, `ConnectionBuilder::attribute` and the builder's map, both `Debug` fields, and the `.attribute("custom", "value")` call (line 643) and its assertion (line 656) in `test_builder_full`; `ConnectionParams.idle_timeout`, the builder field, the 600-second default, both `Debug` fields, the test `test_builder_idle_timeout` (lines 869-879), and the `idle_timeout` assertion in `test_builder_default_values` (line 916). Mark `ConnectionBuilder::idle_timeout` `#[deprecated(note = "...")]` with a note that it has no effect, and make it ignore its argument. In `src/connection/session.rs`: `SessionConfig.idle_timeout` and its default. In `src/adbc/connection.rs`: only the `idle_timeout:` line in `connect_with_transport` (around line 211) and the `.idle_timeout(..)` and `idle_timeout` assertion lines in `connect_seeds_the_session_from_the_authentication_response` (around lines 2833 and 2842).
- [ ] 1.5 Run `cargo test --lib` and `cargo test --no-default-features --features websocket --tests --no-run`. Both pass.

### 2. Client identification at login

- [ ] 2.1 Start the test database. If `docker ps --filter name=exasol-test` shows no running container, run `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`, or `docker start exasol-test` when the container exists but is stopped. Poll `exapump sql 'select 1'` until it prints `1`.
- [ ] 2.2 Write the integration tests. They do not call `skip_if_no_exasol!`, and each runs `SELECT CLIENT FROM EXA_DBA_SESSIONS WHERE SESSION_ID = CURRENT_SESSION` on a connection that `Driver::new().open(..)` and `connect()` open:
  - `tests/integration_tests.rs`: `test_client_name_and_version_identify_the_session_native` (Scenario: Client name and version identify the session) opens `get_test_connection_string()` plus `&client_name=dbt&client_version=9.9` and asserts `dbt 9.9`.
  - `tests/integration_tests.rs`: `test_default_client_identification` (Scenario: Default client identification) opens `get_test_connection_string()` and asserts `format!("exarrow-rs {}", env!("CARGO_PKG_VERSION"))`.
  - `tests/websocket_integration_tests.rs`: `test_client_name_and_version_identify_the_session_websocket` (Scenario: Client name and version identify the session) opens `ws_connection_string()` plus the two parameters and asserts `dbt 9.9`.
  - Run `cargo test --test integration_tests -- client_name_and_version default_client_identification` and `cargo test --features websocket --test websocket_integration_tests -- client_name_and_version`. Confirm that both `dbt 9.9` tests fail because `CLIENT` shows `exarrow-rs` and the crate version. The default test passes before the change and guards the default.
  - The expected `dbt 9.9` comes from issue #75 and assumes that Exasol joins the client name and the client version with one space on both protocols. If the column shows another form, stop and report it instead of changing the scenario.
- [ ] 2.3 Write the failing unit tests for the login message (Scenario: Client name and version identify the session):
  - `src/transport/native/handshake.rs`: `login_packet_identifies_the_user_and_the_driver` builds the packet from `Credentials` with client name `dbt` and client version `9.9`, and asserts `ATTR_CLIENTNAME` `dbt`, `ATTR_CLIENTVERSION` `9.9`, `ATTR_DRIVERNAME` `exarrow-rs`, the username, and the OS. The other `build_login_packet` calls in that test module (around lines 342 to 416) pass `Credentials` too.
  - `src/transport/websocket.rs`: `auth_request_reports_the_client_and_the_driver` tests a new private function that builds the `AuthRequest` from `Credentials` and the encrypted password, and asserts the serialized `clientName` `dbt`, `clientVersion` `9.9`, and `driverName` `exarrow-rs`.
  - `src/transport/protocol.rs`: `Credentials::new` sets the client name `exarrow-rs` and the crate version, and `with_client` replaces both.
- [ ] 2.4 Implement the login identity (decision-log entry [1]):
  - `src/lib.rs`: `pub(crate) const DRIVER_NAME: &str = "exarrow-rs";`, used by the native login packet, the WebSocket auth request, `Credentials::new`, and the client-name default in `ConnectionBuilder::build` (`src/connection/params.rs`, around line 399).
  - `src/transport/protocol.rs`: `Credentials` gains public `client_name` and `client_version` fields, which `new` sets to `DRIVER_NAME` and `env!("CARGO_PKG_VERSION")`, and a `with_client(name, version)` method. Its doc comment describes the login identity: the user, the password, and the client the server reports.
  - `src/transport/native/handshake.rs`: `build_login_packet(&Credentials)` writes the client name and client version from the credentials and the driver name `DRIVER_NAME`. `login_exchange` in `src/transport/native/mod.rs` (around line 932) passes `credentials`.
  - `src/transport/websocket.rs`: `login_exchange` builds its `AuthRequest` through the function from 2.3, as `AuthRequest::new(username, encrypted_password, client_name)` with `with_driver_name(DRIVER_NAME)` and `with_client_version(client_version)`.
  - `src/adbc/connection.rs`: chain `.with_client(params.client_name.clone(), params.client_version.clone())` onto `TransportCredentials::new` in `connect_with_transport` (around line 187). Group 2 makes no other edit in this file.
- [ ] 2.5 Run `cargo test --lib`, `cargo test --features websocket --lib`, `cargo test --test integration_tests`, `cargo test --test native_protocol_tests`, and `cargo test --features websocket --test websocket_integration_tests`. Then run `cargo build --release --features ffi` and `cargo test --test driver_manager_tests`, which still passes the `client_name=dbt@ci` test, and `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1`, because every import and export logs in through the changed login code. All pass.

### 3. Documentation and release

- [ ] 3.1 Update `docs/setup-and-connect.md`:
  - "Parameters": state below the first sentence that a key that is neither in the table nor the deprecated `idle_timeout` fails the parse with an error that names the key and lists the accepted keys, and that keys are case-sensitive.
  - Remove the `idle_timeout` row. Add a note below the table: `idle_timeout` is deprecated and has no effect, the driver accepts it with a warning through the `log` crate, an invalid value still fails, and `ConnectionBuilder::idle_timeout` is deprecated.
  - `client_name` and `client_version` rows: Exasol shows `<client_name> <client_version>` in the `CLIENT` column of `EXA_DBA_SESSIONS`.
  - "URL Encoding": add that an unencoded `?`, and any `&` after it, in a password can split off a fragment as a query key, and that the unknown-key error then names that fragment.
  - "Session Attributes": keep the heading, so the `#session-attributes` link in the `query_timeout` row still resolves. Replace the body: the driver sets `queryTimeout` from `query_timeout`, autocommit from the ADBC `AutoCommit` option and `Connection::begin_transaction()`, `commit()`, and `rollback()`, and the current schema from the URI schema and `set_schema()`. The connection string sets no other session attribute. Remove the attribute table and the read-only list.
  - Search `README.md`, `docs/`, and `examples/` for `session attribute`, `unrecognized`, `idle_timeout`, and `.attribute(` and correct any other claim. None exists at planning time.
- [ ] 3.2 Add a `## <version>` section to `CHANGELOG.md` above the newest section, with the version from 3.3, and fold any `## [Unreleased]` entries into it. Entries:
  - `Breaking:` a connection-string query key that the driver does not implement fails with `ConnectionError::InvalidParameter`, which names the key and lists the accepted keys, and an ADBC driver manager reports `InvalidArguments`. Before, the driver ignored the key, and the docs wrongly said it reached the server as a session attribute. Keys are case-sensitive. `autocommit=` and `queryTimeout=` now fail: use `query_timeout`, and the ADBC `AutoCommit` option or `begin_transaction()`. Fixes #73.
  - `Breaking:` `ConnectionParams.attributes`, `ConnectionBuilder::attribute`, `ConnectionParams.idle_timeout`, and `SessionConfig.idle_timeout` are removed. `transport::Credentials` gains `client_name` and `client_version`, and `build_login_packet` takes `&Credentials`.
  - `Deprecated:` the `idle_timeout` query key and `ConnectionBuilder::idle_timeout` have no effect. The driver never closed idle connections. The query key logs a warning through the `log` crate. Callers that load the driver through an ADBC driver manager, such as dbt and Python, see no runtime warning; the docs and this changelog are their notice.
  - `Fix:` `client_name` and `client_version` reach Exasol on the native and the WebSocket transport, so `EXA_DBA_SESSIONS.CLIENT` shows `dbt 9.9` for `client_name=dbt&client_version=9.9`. The driver name stays `exarrow-rs`. Fixes #75.
- [ ] 3.3 Set `version` in `Cargo.toml` to the next minor version above the `Cargo.toml` version on the base branch at implement time: 0.22.0 when PR #93 has released 0.21.2. Recompute it after every rebase and before merge, because PR #93 or another release PR can merge first. Run `cargo check` so `Cargo.lock` takes the version, and check `README.md` for version strings (none at planning time).
- [ ] 3.4 Before `/speq:record`, compare `git hash-object specs/architecture.md` with the BASE in `specs/_plans/fix-connection-parameters/architecture.md`. If they differ, copy Components, Data Flow, Interfaces, and Constraints again from the current `specs/architecture.md`, apply this plan's edits to them again, and set BASE to the new hash. Run `speq plan validate fix-connection-parameters`.
- [ ] 3.5 Run the quality gates: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -W clippy::all` with zero warnings, and, with default features only, `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json`, and `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json`. Total production line coverage is at least 80.0% and every file is at least 50.0%.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: connection-string parameters | 1.1-1.5 | none | spec deltas `connection-management/auth-and-security` (Unknown connection-string parameter is rejected), `connection-management/credential-sources`, `connection-management/session-and-lifecycle`; `Cargo.toml`, `src/connection/params.rs`, `src/connection/session.rs`, the `idle_timeout` lines of `src/adbc/connection.rs` |
| B: login identity | 2.1-2.5 | A (both edit `src/connection/params.rs` and `src/adbc/connection.rs`) | spec delta `connection-management/auth-and-security` (Client name and version identify the session, Default client identification); `src/lib.rs`, `src/transport/protocol.rs`, `src/transport/native/handshake.rs`, `src/transport/native/mod.rs`, `src/transport/websocket.rs`, the credentials line of `src/adbc/connection.rs`, `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs` |
| C: docs and release | 3.1-3.5 | A, B (the docs and changelog describe their result) | `docs/setup-and-connect.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `specs/_plans/fix-connection-parameters/architecture.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Field and method | `ConnectionParams.attributes`, `ConnectionBuilder::attribute` (`src/connection/params.rs`) | The parser rejects unknown keys, and no code read the map |
| Field | `ConnectionParams.idle_timeout`, the `ConnectionBuilder` field (`src/connection/params.rs`) | No code enforced it |
| Field | `SessionConfig.idle_timeout` (`src/connection/session.rs`) | No code enforced it |
| Statement | `idle_timeout:` in `connect_with_transport` (`src/adbc/connection.rs`) | Copies a removed field |
| Test | the `.attribute("custom", "value")` call and its assertion in `test_builder_full`, `test_builder_idle_timeout`, the `idle_timeout` assertion in `test_builder_default_values`, `test_parse_custom_attribute_param`, `test_parse_idle_timeout` (`src/connection/params.rs`) | Tested removed behavior; the last two are replaced in 1.2 |
| Literals | fixed `exarrow-rs` client name and crate version in `build_login_packet` and `login_exchange` (WebSocket) | Replaced by the `Credentials` fields and `DRIVER_NAME` |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Unknown connection-string parameter is rejected | Unit | `src/connection/params.rs` | `unknown_query_parameters_are_rejected_by_name` |
| Client name and version identify the session | Integration | `tests/integration_tests.rs` | `test_client_name_and_version_identify_the_session_native` |
| Client name and version identify the session | Integration | `tests/websocket_integration_tests.rs` | `test_client_name_and_version_identify_the_session_websocket` |
| Client name and version identify the session (login message fields) | Unit | `src/transport/native/handshake.rs` | `login_packet_identifies_the_user_and_the_driver` |
| Client name and version identify the session (login message fields) | Unit | `src/transport/websocket.rs` | `auth_request_reports_the_client_and_the_driver` |
| Default client identification | Integration | `tests/integration_tests.rs` | `test_default_client_identification` |
| Query credentials fill only what the userinfo omits | Unit | `src/connection/params.rs` | `test_parse_query_credentials_fill_only_what_userinfo_omits` |
| Idle timeout | Unit | `src/connection/params.rs` | `idle_timeout_is_accepted_with_a_deprecation_warning` |
| Idle timeout with an invalid value | Unit | `src/connection/params.rs` | `test_parse_invalid_idle_timeout_value` |

The parsing scenarios are pure computation with no I/O, so unit tests cover them.

### Manual Testing

Run from the repository root after `cargo build --release --features ffi`, with `exasol-test` running.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| auth-and-security (unknown key) | `python3 -c "import adbc_driver_manager.dbapi as d; d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://sys:exasol@localhost:8563?validateservercertificate=0&autocommit=false'}).cursor().execute('SELECT 1')"` | An exception whose message contains `Invalid connection parameter 'autocommit'` and `query_timeout`, and does not contain `false` |
| auth-and-security (client identification) | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://sys:exasol@localhost:8563?validateservercertificate=0&client_name=dbt&client_version=9.9'}); cur=c.cursor(); cur.execute('SELECT CLIENT FROM EXA_DBA_SESSIONS WHERE SESSION_ID = CURRENT_SESSION'); print(cur.fetchone())"` | `('dbt 9.9',)` |
| credential-sources | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://localhost:8563?validateservercertificate=0&user=sys&password=exasol'}); cur=c.cursor(); cur.execute('SELECT 1'); print(cur.fetchone())"` | `(1,)` |
| session-and-lifecycle | `python3 -c "import adbc_driver_manager.dbapi as d; c=d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://sys:exasol@localhost:8563?validateservercertificate=0&idle_timeout=120'}); cur=c.cursor(); cur.execute('SELECT 1'); print(cur.fetchone())"` | `(1,)`; no warning appears, because the Python process installs no Rust logger |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release --features ffi` | Exit 0 |
| WebSocket build | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit tests | `cargo test --lib` and `cargo test --features websocket --lib` | 0 failures |
| Integration tests | `cargo test --test integration_tests` | 0 failures |
| Native protocol tests | `cargo test --test native_protocol_tests` | 0 failures |
| WebSocket integration tests | `cargo test --features websocket --test websocket_integration_tests` | 0 failures |
| Driver manager tests | `cargo test --test driver_manager_tests` (after the release build) | 0 failures |
| Import and export tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json`, then `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | `check` passes: at least 80.0% total, at least 50.0% per file |
| Plan | `speq plan validate fix-connection-parameters` | Pass |
