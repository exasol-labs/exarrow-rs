# Plan: fix-ffi-credential-options

## Summary

ADBC `username` and `password` database options reach the server exactly as set, an `@` in a URI query value no longer changes the host, and a connection without a username fails with `Username is required` instead of logging in as `sys`. The connection module parses the URI once and merges the option credentials, and the FFI layer keeps the parsed `ConnectionParams` instead of a rebuilt URI string.

## Context

- Issue exasol-labs/exarrow-rs#74 (labels: bug, security) reports four failures when credentials are set as ADBC database options and not in the URI. The ADBC driver manager and the dbt Exasol adapter use this path.
- Failure 1: with `?client_name=dbt@ci` in the URI, the connection and the credentials go to host `ci` (`Failed to connect to ci:8563`).
- Failure 2: the password `Ab?cd1234` fails with `Username is required`.
- Failure 3: the password `Ab%41cd1234` fails with `authentication failed`.
- Failure 4: with no username anywhere, the driver logs in as `sys`. The Rust API requires a username, and the issue states that JDBC, pyexasol, and the Go driver do too.
- `FfiDatabase::build_connection_uri` (`src/adbc_ffi.rs`) rebuilds the URI as `exasol://{user}:{pass}@{rest}`. It searches the whole URI, query string included, for the last `@`, inserts the password without percent-encoding, and falls back to the username `sys`.
- `ConnectionParams::from_str` (`src/connection/params.rs`) splits off the query string at the first `?` before it looks for `@`, and percent-decodes the userinfo. Credentials inside the URI, percent-encoded, work today.
- `FfiConnection` stores the rebuilt URI string and parses it at the first statement. `FfiStatement` keeps a copy of the string and parses it only in a fallback for a statement without a parent connection. `FfiConnection::new_statement` always passes the parent connection, so production never reaches that fallback.
- `ConnectionBuilder` derives `Debug`, which prints the password. `ConnectionParams` `Debug` output prints `attributes`, where the parser keeps a credential query key that the userinfo makes redundant.
- The features `connection-management/auth-and-security` and `adbc-driver/driver-interface` hold 13 scenarios each, above the limit of 10.
- Two existing FFI unit tests, `test_autocommit_set_false` and `test_ffi_connection_options`, call `set_option(AutoCommit, "false")` on an unconnected `FfiConnection`. That call opens a transaction, so `ensure_connected` dials `localhost:8563` with a URI that lacks `validateservercertificate=0`. Both tests fail with or without Exasol: certificate validation rejects the self-signed certificate of the local container, and without a server the dial fails. The two tests stay out of scope, and the Checklist skips them.
- The component, data flow, interface, and constraint changes are in the architecture delta, `architecture.md` in this directory.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Credential Sources | NEW | `connection-management/credential-sources/spec.md` |

## Impact

- Fix: a `username` or `password` database option reaches the server byte for byte. Passwords with `?`, `@`, `:`, `/`, `#`, or a `%XX` sequence log in.
- Fix: an `@` in a URI query value no longer changes the host of a driver manager connection.
- Breaking: a driver manager connection with no username in the URI userinfo, the `user` or `username` query parameter, or the `username` option fails with `Username is required`. It used to log in as `sys`.
- Changed: on the driver manager path, a URI parse error or a missing username fails connection creation (`AdbcConnectionInit`) with status `InvalidArguments`. It used to fail the first statement with status `Internal`.
- Fix (Rust API and driver manager): `exasol://alice@host?password=pw` logs in with `pw`. It used to log in with an empty password. A credential query parameter is never kept as a connection attribute.
- `ConnectionBuilder` `Debug` output shows `<redacted>` in place of the password.
- No public function signature changes. `Cargo.toml` keeps version 0.18.0, and the changelog entries go under `## [Unreleased]`.

## Dependencies

None. The fix uses existing crates only.

## Implementation Tasks

1. Connection module: one parse with option credentials (`src/connection/params.rs`)

- [ ] 1.1 Change `parse_auth` to return `(String, Option<String>)`. The password is `Some` only when the userinfo contains `:`, so `exasol://alice:@host` gives `Some("")` and `exasol://alice@host` gives `None`. Keep its decode error messages, which name the field and never the value.
- [ ] 1.2 Add `pub(crate) fn parse_with_credentials(uri: &str, username: Option<&str>, password: Option<&str>) -> Result<ConnectionParams, ConnectionError>` to `impl ConnectionParams` and move the body of `from_str` into it. `from_str` returns `Self::parse_with_credentials(s, None, None)`. After `parse_query_params` and the userinfo split, remove `user`, `username`, `password`, and `pass` from the query map in every case, then resolve each field per `decision-log.md` entry [2]: username from the option, the userinfo, `user`, then `username`; password from the option, the userinfo, `password`, `pass`, then empty. A missing username returns `ConnectionError::ParseError("Username is required".to_string())` (entry [3]). Pass option values to the builder unchanged, never percent-decoded. The doc comment states the precedence and that option values are used verbatim. [expert]
- [ ] 1.3 Replace `#[derive(Debug, Clone)]` on `ConnectionBuilder` with `#[derive(Clone)]` and a manual `fmt::Debug` that prints every field, with `Some("<redacted>")` in place of a set password (entry [6]).
- [ ] 1.4 Add unit tests to `mod tests` in `src/connection/params.rs`. Each test that names a scenario carries its `/// Scenario:` line.
  - `test_parse_with_credentials_options_override_userinfo`: `exasol://nobody:Wrong1@db.example.com:8563` with `Some("alice"), Some("Secret1")` gives username `alice` and `password()` `Secret1` (Scenario: Option credentials take precedence over URI credentials).
  - `test_parse_with_credentials_password_option_keeps_userinfo_username`: `exasol://alice:Wrong1@db.example.com:8563` with `None, Some("Secret1")` gives `alice` and `Secret1` (Scenario: A single credential option replaces only its own field).
  - `test_parse_with_credentials_keeps_option_password_verbatim`: `exasol://db.example.com:8563` with username `alice` and each of `Ab?cd1234`, `Ab%41cd1234`, and `Ab@:/#cd1234` gives a `password()` equal to the input and host `db.example.com` (Scenario: Option password reaches the server verbatim).
  - `test_parse_with_credentials_at_sign_in_query_value_keeps_host`: `exasol://db.example.com:8563?client_name=dbt@ci` with both options gives host `db.example.com`, port 8563, and `client_name` `dbt@ci` (Scenario: An at sign in a query parameter value does not change the host).
  - `test_parse_with_credentials_requires_username`: `exasol://db.example.com:8563` with `None, Some("Secret1")` fails, and the error's `to_string()` contains `Username is required` and not `Secret1` (Scenario: Missing username is rejected).
  - `test_parse_with_credentials_empty_username_option_is_rejected`: `exasol://alice:Pw1@db.example.com:8563` with `Some(""), None` fails with `Username cannot be empty` (entry [2]; no scenario line).
  - `test_parse_with_credentials_decodes_userinfo_without_options`: `exasol://alice:Ab%3Fcd1234@db.example.com:8563` with `None, None` gives `alice` and `Ab?cd1234` (Scenario: URI credentials apply when no credential option is set).
  - `test_parse_query_credentials_fill_only_what_userinfo_omits`: `exasol://alice@db.example.com:8563?user=bob&username=carol&password=Secret1&pass=Other1` through `from_str` gives `alice` and `Secret1`, and `attributes` holds none of the keys `user`, `username`, `password`, and `pass` (Scenario: Query credentials fill only what the userinfo omits).
  - `test_parse_with_credentials_query_user_with_password_option`: `exasol://db.example.com:8563?user=bob&username=carol` with `None, Some("Secret1")` gives `bob` and `Secret1` (Scenario: Query user applies when only the password option is set).
  - `test_debug_redacts_password_from_every_source`: `{:?}` of params whose password came from the option, from the userinfo, and from `?password=`, and of `ConnectionBuilder::new().password("Secret1")`, contains no `Secret1` (Scenario: Password stays out of Debug output).
  - The existing parse tests, among them `test_parse_missing_username`, `test_parse_url_encoded`, `test_parse_auth_without_password`, `test_parse_username_from_query_user`, `test_parse_password_from_query_pass`, and `test_debug_no_password_leak`, pass unchanged.

2. FFI layer: keep the parsed parameters (`src/adbc_ffi.rs`)

- [ ] 2.1 Replace `FfiDatabase::build_connection_uri` with `build_connection_params(&self) -> AdbcResult<ConnectionParams>`. It keeps the `InvalidState` error for an unset `uri` option, calls `ConnectionParams::parse_with_credentials(uri, self.username.as_deref(), self.password.as_deref())`, and maps a `ConnectionError` to `AdbcError::with_message_and_status(err.to_string(), AdbcStatus::InvalidArguments)` (entry [4]). Update the doc comments of `FfiDatabase` and its `username` and `password` fields: a set option replaces the URI value verbatim.
- [ ] 2.2 Replace `FfiConnection::uri` with `params: ConnectionParams`. `FfiConnection::new` takes `ConnectionParams`. `new_connection` and `new_connection_with_opts` pass the result of `build_connection_params()`. `ensure_connected` calls `ExaConnection::from_params(self.params.clone())` and no longer parses a string.
- [ ] 2.3 Remove `FfiStatement::uri` (entry [5]). `with_connection` takes `Arc<Mutex<ExaConnection>>` and stores `Some(conn)`. The test-only `new()` takes no argument. `query_sql` and `update_sql` take the connection from `self.require_connection()?` and no longer open a connection of their own. Update their doc comments and the struct doc comment.
- [ ] 2.4 Update the existing unit tests to the new constructors. Add one test helper that parses `exasol://user@localhost:8563` into `ConnectionParams`, and use it in `unconnected_connection`, the `FfiConnection::new` calls, and the `FfiConnection` struct literals of `test_get_objects_invalid_catalog`, `test_get_objects_no_connection`, and `test_get_table_schema_no_connection`. Drop the argument from every `FfiStatement::new` call. Replace `test_ffi_database_build_uri_with_overrides` with `test_ffi_database_build_params_with_overrides`, which asserts username `admin`, password `secret`, host `localhost`, port 8563, and schema `schema`.
- [ ] 2.5 Add unit tests. Each test that names a scenario carries its `/// Scenario:` line.
  - `test_ffi_database_without_username_rejects_connection`: `uri` option `exasol://localhost:8563`, `password` option `Secret1`, no `username` option. `new_connection()` fails with status `InvalidArguments` and a message that contains `Username is required` and not `Secret1` (Scenario: Missing username is rejected).
  - `test_ffi_database_option_password_is_verbatim`: `uri` option `exasol://localhost:8563`, `username` option `alice`, and each of the three passwords of task 1.4. `build_connection_params()` returns that password unchanged (Scenario: Option password reaches the server verbatim).
  - `test_ffi_database_at_sign_in_query_value_keeps_host`: `uri` option `exasol://localhost:8563?client_name=dbt@ci` with both credential options. `build_connection_params()` returns host `localhost` and `client_name` `dbt@ci` (Scenario: An at sign in a query parameter value does not change the host).
  - `test_ffi_database_query_user_with_password_option`: `uri` option `exasol://localhost:8563?user=bob&username=carol`, `password` option `Secret1`, no `username` option. `build_connection_params()` returns username `bob` and password `Secret1` (Scenario: Query user applies when only the password option is set).
  - `test_ffi_statement_without_connection_does_not_dial`: a standalone `FfiStatement` with SQL `SELECT 1` fails `execute()` and `execute_update()` with status `InvalidState` and `No connection available`.

3. Driver manager tests (`tests/driver_manager_tests.rs`)

- [ ] 3.1 Add a `// Credential Option Tests` section with these helpers: `get_test_uri_without_credentials()`, which returns `exasol://{host}:{port}?tls=true&validateservercertificate=0`; `open_database(driver, uri, username: Option<&str>, password: Option<&str>)`, which passes `OptionDatabase::Uri` and each set `OptionDatabase::Username` and `OptionDatabase::Password` to `new_database_with_opts`; `current_user(conn)`, which runs `SELECT CURRENT_USER` and returns the value; `create_temporary_user(admin_conn, password)`, which creates a user named by `generate_unique_test_name("EXARROW_CRED")` with `CREATE USER <name> IDENTIFIED BY "<password>"`, grants `CREATE SESSION`, and returns the name; and `drop_temporary_user(admin_conn, name)`, which runs `DROP USER <name> CASCADE` and ignores a failure, like `drop_test_schema` (entry [8]).
- [ ] 3.2 Add `test_driver_manager_option_credentials_override_uri_credentials`. The `uri` option is `exasol://NO_SUCH_USER:Wrong1@{host}:{port}?tls=true&validateservercertificate=0`, and the options carry `get_user()` and `get_password()`. `current_user` equals `get_user()` in upper case (Scenario: Option credentials take precedence over URI credentials).
- [ ] 3.3 Add `test_driver_manager_password_option_keeps_uri_username`. The `uri` option carries `get_user()` and the password `Wrong1` in the userinfo, and only the `password` option is set, to `get_password()`. `current_user` equals `get_user()` in upper case (Scenario: A single credential option replaces only its own field).
- [ ] 3.4 Add `test_driver_manager_option_password_reaches_server_verbatim`. Create one temporary user. For each of `Ab?cd1234`, `Ab%41cd1234`, and `Ab@:/#cd1234`, set the password with `ALTER USER <name> IDENTIFIED BY "<password>"`, open a database with `get_test_uri_without_credentials()` and the user's name and password as options, and assert that `current_user` equals the name. Drop the user at the end (Scenario: Option password reaches the server verbatim).
- [ ] 3.5 Add `test_driver_manager_at_sign_in_query_value_keeps_host`. The `uri` option is `get_test_uri_without_credentials()` plus `&client_name=dbt@ci`, and the options carry `get_user()` and `get_password()`. `current_user` equals `get_user()` in upper case. Before the fix, this test fails with `Failed to connect to ci:8563` (Scenario: An at sign in a query parameter value does not change the host).
- [ ] 3.6 Add `test_driver_manager_missing_username_is_rejected`, which needs the library but not Exasol. The `uri` option is `get_test_uri_without_credentials()`, the `password` option is `Secret1`, and no `username` option is set. `new_connection()` fails with status `InvalidArguments` and a message that contains `Username is required` and not `Secret1` (Scenario: Missing username is rejected).
- [ ] 3.7 Add `test_driver_manager_uri_credentials_apply_without_options`. Create a temporary user with the password `Ab?cd1234`. The `uri` option is `exasol://{name}:Ab%3Fcd1234@{host}:{port}?tls=true&validateservercertificate=0`, and no credential option is set. `current_user` equals the name. Drop the user at the end (Scenario: URI credentials apply when no credential option is set).
- [ ] 3.8 Every test of tasks 3.2 to 3.7 starts with `skip_if_no_library!()`. Each test except 3.6 also calls `skip_if_no_exasol!()`. Each test carries its `/// Scenario:` line.

4. Documentation and changelog

- [ ] 4.1 In `docs/setup-and-connect.md`, replace the example `exasol://localhost:8563` with `exasol://user@localhost:8563`. Add a `### Credentials` subsection under `## Connection String Format`. It states that a username is required and that the driver has no default user. It names the three sources of the username: the URI userinfo, the `user` or `username` query parameter, and the ADBC `username` option. It names the matching sources of the password. It states the per-field order: ADBC option, then userinfo, then query parameter. It states that URI values are percent-decoded and that option values are used verbatim. Change the `user` / `username` and `password` / `pass` rows of the Parameters table to say that they apply when the userinfo omits the field.
- [ ] 4.2 In `docs/driver-manager.md`, add a `### Credentials as database options` subsection under `## Connection URI`. It shows `adbc_driver_manager.dbapi.connect` with `db_kwargs={"uri": "exasol://localhost:8563?tls=true&validateservercertificate=0", "username": "user", "password": "p@ss?word"}`. It states that option values need no URL-encoding, that a set option replaces the matching URI credential, and that a connection without a username fails with `Username is required`.
- [ ] 4.3 In `CHANGELOG.md`, add a `## [Unreleased]` section above `## 0.18.0`, and leave `## 0.18.0` and `Cargo.toml` unchanged (entry [9]). Entries:
  - `Fix:` ADBC `username` and `password` database options reach the server exactly as set. A password with `?`, `@`, `#`, or a `%XX` sequence no longer fails to log in. Fixes #74.
  - `Fix:` an `@` in a URI query parameter value, such as `client_name=dbt@ci`, no longer sends a driver manager connection and its option credentials to another host.
  - `Fix:` a `password` or `pass` query parameter applies when the URI userinfo names only the user. Credential query parameters no longer appear in the `Debug` output of `ConnectionParams`, and the `Debug` output of `ConnectionBuilder` no longer shows the password.
  - `Breaking:` a driver manager connection with no username in the URI or in the `username` option fails with `Username is required`. It used to log in as `sys`.
  - `Changed:` the driver manager path checks the URI and the credentials when the connection is created and reports a problem with status `InvalidArguments`. It used to report it at the first statement with status `Internal`.

5. Verification

- [ ] 5.1 Start Exasol if it is not running (`docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`) and wait until `exapump sql 'select 1'` returns `1`.
- [ ] 5.2 Run every Checklist step. Build the release cdylib before the driver manager tests.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Credential sources | 1.1-5.2 | none | spec delta `connection-management/credential-sources`; `architecture.md` in this directory; `src/connection/params.rs`, `src/adbc_ffi.rs`, `tests/driver_manager_tests.rs`, `tests/common/mod.rs`, `docs/setup-and-connect.md`, `docs/driver-manager.md`, `CHANGELOG.md` |

- One group, because every task implements or documents the one spec delta, and the FFI tasks call the function that task 1.2 adds.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/adbc_ffi.rs` `FfiDatabase::build_connection_uri` | Replaced by `build_connection_params` |
| Field | `src/adbc_ffi.rs` `FfiConnection::uri` | Replaced by `params: ConnectionParams` |
| Field | `src/adbc_ffi.rs` `FfiStatement::uri` | Fed only the standalone-connection fallback |
| Code branch | `src/adbc_ffi.rs` `FfiStatement::query_sql` and `FfiStatement::update_sql`, the branches that open a connection of their own | Unreachable in production, and the second place that parsed the URI string |
| Test | `src/adbc_ffi.rs` `test_ffi_database_build_uri_with_overrides` | Replaced by `test_ffi_database_build_params_with_overrides` |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Option credentials take precedence over URI credentials | Integration | `tests/driver_manager_tests.rs` | `test_driver_manager_option_credentials_override_uri_credentials` |
| Option credentials take precedence over URI credentials | Unit | `src/connection/params.rs` | `test_parse_with_credentials_options_override_userinfo` |
| A single credential option replaces only its own field | Integration | `tests/driver_manager_tests.rs` | `test_driver_manager_password_option_keeps_uri_username` |
| A single credential option replaces only its own field | Unit | `src/connection/params.rs` | `test_parse_with_credentials_password_option_keeps_userinfo_username` |
| Option password reaches the server verbatim | Integration | `tests/driver_manager_tests.rs` | `test_driver_manager_option_password_reaches_server_verbatim` |
| Option password reaches the server verbatim | Unit | `src/connection/params.rs` | `test_parse_with_credentials_keeps_option_password_verbatim` |
| Option password reaches the server verbatim | Unit | `src/adbc_ffi.rs` | `test_ffi_database_option_password_is_verbatim` |
| An at sign in a query parameter value does not change the host | Integration | `tests/driver_manager_tests.rs` | `test_driver_manager_at_sign_in_query_value_keeps_host` |
| An at sign in a query parameter value does not change the host | Unit | `src/connection/params.rs` | `test_parse_with_credentials_at_sign_in_query_value_keeps_host` |
| An at sign in a query parameter value does not change the host | Unit | `src/adbc_ffi.rs` | `test_ffi_database_at_sign_in_query_value_keeps_host` |
| Missing username is rejected | Integration | `tests/driver_manager_tests.rs` | `test_driver_manager_missing_username_is_rejected` |
| Missing username is rejected | Unit | `src/connection/params.rs` | `test_parse_with_credentials_requires_username` |
| Missing username is rejected | Unit | `src/adbc_ffi.rs` | `test_ffi_database_without_username_rejects_connection` |
| URI credentials apply when no credential option is set | Integration | `tests/driver_manager_tests.rs` | `test_driver_manager_uri_credentials_apply_without_options` |
| URI credentials apply when no credential option is set | Unit | `src/connection/params.rs` | `test_parse_with_credentials_decodes_userinfo_without_options` |
| Query credentials fill only what the userinfo omits | Unit | `src/connection/params.rs` | `test_parse_query_credentials_fill_only_what_userinfo_omits` |
| Query user applies when only the password option is set | Unit | `src/connection/params.rs` | `test_parse_with_credentials_query_user_with_password_option` |
| Query user applies when only the password option is set | Unit | `src/adbc_ffi.rs` | `test_ffi_database_query_user_with_password_option` |
| Password stays out of Debug output | Unit | `src/connection/params.rs` | `test_debug_redacts_password_from_every_source` |

- The scenarios "Query credentials fill only what the userinfo omits" and "Password stays out of Debug output" are pure parsing and formatting with no I/O, so unit tests cover them.
- In the scenario "Query user applies when only the password option is set", parsing decides the login user before any I/O. `test_ffi_database_query_user_with_password_option` checks the `ConnectionParams` value that `ensure_connected` passes to `Connection::from_params`, so unit tests cover it.
- CI runs the `src/adbc_ffi.rs` unit tests only when they match `arrow_value_to_parameter`. The `ffi` unit test step of the Checklist, `cargo test --lib --features ffi -- --skip test_autocommit_set_false --skip test_ffi_connection_options`, runs them locally, and the driver manager tests carry the CI evidence for the FFI path (`decision-log.md` entry [8]).

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Credential Sources | `cargo build --release --features ffi && python3 -c "import adbc_driver_manager.dbapi as d; c = d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://localhost:8563?validateservercertificate=0&client_name=dbt@ci', 'username': 'sys', 'password': 'exasol'}); cur = c.cursor(); cur.execute('SELECT CURRENT_USER'); print(cur.fetchone())"` | Prints `('SYS',)`. Needs `pip install adbc-driver-manager pyarrow` and a running Exasol |
| Credential Sources | `python3 -c "import adbc_driver_manager.dbapi as d; d.connect(driver='target/release/libexarrow_rs.so', entrypoint='ExarrowDriverInit', db_kwargs={'uri': 'exasol://localhost:8563?validateservercertificate=0', 'password': 'Secret1'})"` | Raises an exception whose message contains `Username is required` and does not contain `Secret1` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Format | `cargo fmt --all -- --check` | No changes |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Build | `cargo build` | Exit 0 |
| Build (FFI) | `cargo build --release --features ffi` | Exit 0 |
| Build (WebSocket only) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit test | `cargo test --lib` | 0 failures |
| Unit test (FFI) | `cargo test --lib --features ffi -- --skip test_autocommit_set_false --skip test_ffi_connection_options` | 0 failures |
| Unit test (WebSocket) | `cargo test --lib --features websocket` | 0 failures |
| Integration test | `REQUIRE_EXASOL=1 cargo test --test integration_tests -- --test-threads=1` | 0 failures, no skips |
| Driver manager test | `REQUIRE_EXASOL=1 cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1` | 0 failures, no skips, run after the FFI build |
| Import/export test | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info && python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json && python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Check passes: total production coverage at least 80%, every file at least 50% |
| Changelog | `git diff main -- CHANGELOG.md Cargo.toml` | New `## [Unreleased]` section above `## 0.18.0`, no change to `## 0.18.0` or to the `Cargo.toml` version |
