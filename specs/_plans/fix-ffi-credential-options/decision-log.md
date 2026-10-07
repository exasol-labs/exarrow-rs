# Decision Log: fix-ffi-credential-options

## Interview

The plan ran in headless mode. The orchestrator passed the content of issue exasol-labs/exarrow-rs#74 and the user's constraints. Each pair below paraphrases one part of that brief.

**Q:** What is the defect?
**A:** Issue #74 (labels: bug, security). ADBC `username` and `password` database options are pasted into the URI string and parsed again by the FFI layer. This is the path of the ADBC driver manager and the dbt Exasol adapter. Four symptoms: an `@` in a query value (`?client_name=dbt@ci`) sends the connection and the credentials to the wrong host (`Failed to connect to ci:8563`). A password with `?` (`Ab?cd1234`) fails with `Username is required`. A password with a valid `%XX` sequence (`Ab%41cd1234`) fails with `authentication failed`. With no username anywhere, the driver logs in as `sys`; the expected result is `Username is required`, as in the Rust API, JDBC, pyexasol, and Go. Credentials inside the URI, percent-encoded, are unaffected.

**Q:** What causes it?
**A:** `FfiDatabase::build_connection_uri` builds `exasol://{user}:{pass}@{rest}`. It runs `rfind('@')` over the whole URI including the query string, inserts the password without percent-encoding, and defaults the username to `sys`.

**Q:** Which fix does the issue propose?
**A:** A spike exists and is a strong hint, not a prescription. 1: parse once and merge the options while parsing, through `ConnectionParams::parse_with_credentials(uri, username, password)`, with `from_str` calling it with `None, None`; options take precedence, and the username check runs after the merge. 2: replace `build_connection_uri()` with `build_connection_params()` and store `ConnectionParams` instead of the URI string. 3: remove the `sys` default. An error on conflicting URI and option credentials is optional and separate, and not in scope.

**Q:** What scope applies?
**A:** Strictly #74, as a `fix` plan. No work for #78, #67, #77, #72, #73, or #75. In particular, rejecting unknown parameters and sending `client_name` and `client_version` belong to #73 and #75.

**Q:** Which documentation and changelog changes are required?
**A:** `docs/setup-and-connect.md` shows `exasol://localhost:8563` as valid, but a username is always required. Document that the username can come from the URI userinfo, from `?user=`, or from the ADBC `username` option. Add `CHANGELOG.md` entries for all three proposal items under `## [Unreleased]`. Do not bump the `Cargo.toml` version, because the user picks the release number. Do not touch the released `## 0.18.0` section.

**Q:** Which security requirements must the scenarios and tests cover?
**A:** A password with `?`, `%41`, `@`, `:`, `/`, or `#` set as an option reaches the server verbatim. An `@` in a query value does not change the host. Passwords never appear in error messages, Debug output, or logs, including the new `ConnectionParams` path and the new `Username is required` error. Option credentials take precedence over URI credentials. No username anywhere gives `Username is required`.

**Q:** Which tests are expected?
**A:** Unit tests in `src/connection/params.rs` and `src/adbc_ffi.rs`, and driver manager tests in `tests/driver_manager_tests.rs` (Exasol and the release cdylib) that cover the four cases of the issue, following the repository's test conventions.

## Design Decisions

### [1] The connection module parses the URI once and merges the option credentials

- **Decision:** `ConnectionParams` gains `pub(crate) fn parse_with_credentials(uri, username: Option<&str>, password: Option<&str>)`. It holds the current `from_str` body and resolves the credentials after it has split the URI into userinfo, host part, and query parameters. `from_str` calls it with `None, None`. The FFI layer passes the option values to it and never reads or writes the URI string.
- **Alternatives:** Percent-encode the option credentials into the rebuilt URI: rejected, because the FFI layer would still re-implement the URI grammar (`@`, `?`, userinfo) next to the parser, and the `@`-in-query defect lives in that copy. Parse the URI first and override the credential fields afterwards: rejected, because the parser rejects `exasol://host:8563` without a user before an override can apply. Make the function `pub`: rejected, because no Rust API caller needs it today and Rust callers have `ConnectionBuilder`.
- **Rationale:** The URI format is a decision of the connection module. Two modules that each split the same string is back-door leakage, and the defect in #74 comes from that duplicate. With one parser, the FFI layer passes values and owns no URI knowledge. The function is deep: one call hides the grammar, the decoding, and the precedence rule.
- **Promotes to ADR:** no

### [2] Credentials resolve per field, and credential query keys are always consumed

- **Decision:** The username is the `username` option, else the userinfo user, else the `user` query key, else the `username` query key, else the parse fails. The password is the `password` option, else the userinfo password, else the `password` query key, else the `pass` query key, else empty. The userinfo password counts only when the userinfo contains `:`. The parser removes all four credential query keys in every case, so none of them becomes a connection attribute. A set option counts even when it is empty: an empty `username` option fails with `Username cannot be empty`.
- **Alternatives:** Options replace the URI credentials only when both are set: rejected, because the current FFI code already resolves each field on its own and a URI user with a password option is a common setup. Fail on conflicting URI and option credentials: rejected as out of scope; the issue lists it as a separate, optional change.
- **Rationale:** The current FFI code already resolves an option against the userinfo field by field, so callers that set both keep their result. `ConnectionParams` `Debug` output prints `attributes`, so a credential query key kept as an attribute shows its value. Today `exasol://alice@host?password=pw` logs in with an empty password and keeps `pw` in the attributes.
- **Consequences:** `exasol://alice@host?password=pw` now logs in with `pw`. In every other URI-only case the resolved credentials stay the same, and a credential query key that loses to the userinfo is dropped instead of stored as an attribute. On the driver manager path, a `user` query parameter now counts when only the `password` option is set; the current FFI code ignores it and logs in as `sys`. The scenario "Query user applies when only the password option is set" pins this result and the order of `user` before `username`. `parse_auth` returns the userinfo password as `Option<String>`.
- **Promotes to ADR:** no

### [3] The driver has no default username

- **Decision:** When no source supplies a username, `parse_with_credentials` fails with the existing `ConnectionError::ParseError("Username is required")`, the same error that `from_str` returns today for a URI without a user.
- **Alternatives:** Keep `sys` as the FFI default: rejected, because it logs in as the most privileged account without the caller naming it, and the Rust API, JDBC, pyexasol, and the Go driver require a username (stated in issue #74; not checked against those drivers' sources). Switch to `ConnectionError::InvalidParameter`: rejected, because it would change the error variant of the Rust URI path without a reason.
- **Rationale:** A missing username is a configuration error and must not turn into a login as `sys`.
- **Consequences:** A driver manager setup that relied on the implicit `sys` user now fails at connection creation. The changelog labels this entry `Breaking:`.
- **Promotes to ADR:** no

### [4] The FFI connection holds `ConnectionParams` and parses at connection creation

- **Decision:** `FfiDatabase::build_connection_uri` becomes `build_connection_params`, which calls `ConnectionParams::parse_with_credentials` and maps a `ConnectionError` to `AdbcStatus::InvalidArguments` with the error's `Display` text. It keeps the existing `InvalidState` error when the `uri` option is not set. `new_connection` and `new_connection_with_opts` call it. `FfiConnection` stores `params: ConnectionParams` instead of `uri: String`, and `ensure_connected` passes a clone to `Connection::from_params`.
- **Alternatives:** Keep the URI string and parse at the first statement: rejected, because the string would carry the merged credentials again and a configuration error would surface at the first statement with status `Internal`.
- **Rationale:** The ADBC driver manager calls `new_connection` from `AdbcConnectionInit`, so a configuration error now reports at connect time with a status that names the cause. The network connection stays lazy, as today.
- **Consequences:** A URI parse error or a missing username fails `AdbcConnectionInit` with `InvalidArguments`. It used to fail the first statement with `Internal`.
- **Promotes to ADR:** no

### [5] FFI statements drop the URI and the standalone-connection fallback

- **Decision:** `FfiStatement` loses its `uri` field. `query_sql` and `update_sql` call `require_connection()` instead of opening and closing a connection of their own when the statement has no parent connection. `with_connection` takes the shared connection, and the test-only `new()` takes no argument.
- **Alternatives:** Store `ConnectionParams` in every statement, as the issue proposes: rejected, because it copies the password into each statement for a code path that production never runs.
- **Rationale:** `FfiConnection::new_statement` is the only constructor outside tests, and it always passes the parent connection. The fallback in `query_sql` and `update_sql` is unreachable in production and is the second place that parses the URI string. Removing it also shortens the lifetime of credential copies in memory.
- **Consequences:** A test-only standalone statement that executes SQL fails with `No connection available` instead of dialing the server.
- **Promotes to ADR:** no

### [6] `ConnectionBuilder` `Debug` output redacts the password

- **Decision:** Replace the derived `Debug` of `ConnectionBuilder` with a manual implementation that prints `<redacted>` for a set password and keeps the other fields. `Clone` stays derived.
- **Alternatives:** Leave the derived implementation: rejected, because it prints the password, and `parse_with_credentials` puts the option password into a builder.
- **Rationale:** The orchestrator's security requirement covers Debug output on the new credential path. `ConnectionParams` already redacts its password, and the builder is the other type on that path that holds it.
- **Promotes to ADR:** no

### [7] A new feature spec, `connection-management/credential-sources`

- **Decision:** The scenarios go into a new feature in `connection-management`. No existing feature changes.
- **Alternatives:** Add the scenarios to `connection-management/auth-and-security` or `adbc-driver/driver-interface`: rejected, because both already hold 13 scenarios, above the limit of 10. Place the feature in `adbc-driver`: rejected, because the precedence rule and the URI parsing belong to the connection module, and two of the nine scenarios have no ADBC part.
- **Rationale:** `connection-management` grows from 6 to 7 features, within the limit of 8. The existing scenarios `Parameter validation` and `No credential logging` stay accurate.
- **Promotes to ADR:** no

### [8] Driver manager tests use a temporary database user for the password cases

- **Decision:** The password tests create a uniquely named user through an admin connection, grant `CREATE SESSION`, set each test password with `ALTER USER ... IDENTIFIED BY "<password>"`, connect through the driver manager, and drop the user at the end. The other credential tests use the configured `sys` credentials. Every test that needs Exasol starts with `skip_if_no_library!()` and `skip_if_no_exasol!()`, and the checklist runs them with `REQUIRE_EXASOL=1`, so they fail instead of skipping. The missing-username test needs only the library.
- **Alternatives:** Change the `sys` password: rejected, because it breaks every other test that runs at the same time.
- **Rationale:** The `sys` test password, `exasol`, has none of the characters under test. `CREATE USER` and `ALTER USER` take the password as a delimited identifier (`IDENTIFIED BY "h12_Xhz"` in the Exasol `CREATE USER` reference), and the Exasol basic language elements reference states that all characters are allowed in a delimited identifier, with `""` for a double quote. None of the test passwords contains a double quote.
- **Consequences:** CI runs only the `arrow_value_to_parameter` FFI unit tests (`.github/workflows/ci.yml`), so the new FFI unit tests run locally only, through `cargo test --lib --features ffi -- --skip test_autocommit_set_false --skip test_ffi_connection_options`. The command skips two existing tests that dial the server from `set_option(AutoCommit, "false")` and fail with or without Exasol. The driver manager tests carry the CI evidence for the FFI path. The CI filter stays unchanged.
- **Promotes to ADR:** no

### [9] Changelog under `## [Unreleased]`, no version bump

- **Decision:** Add a `## [Unreleased]` section above `## 0.18.0` with `Fix:`, `Breaking:`, and `Changed:` entries. `Cargo.toml` keeps version 0.18.0.
- **Alternatives:** none
- **Rationale:** AGENTS.md puts the entries of a PR without a version bump under `## [Unreleased]`, and the user picks the release number.
- **Promotes to ADR:** no

### [10] Scope boundary

- **Decision:** The plan leaves out: an error for conflicting URI and option credentials; parse errors of the URI path that echo a malformed query pair, which can contain part of an unencoded password (`exasol://u:pa?ss@host` reports `Invalid query parameter format: ss@host`); unknown-parameter rejection and sending `client_name` (#73, #75); the stale WebSocket statement in the `adbc-driver/driver-interface` Background; the CI filter on FFI unit tests.
- **Alternatives:** none
- **Rationale:** The orchestrator scoped the plan strictly to #74. Option credentials no longer pass through the URI string, so the echoed-pair defect affects only callers that put an unencoded password into the URI, which the documentation already forbids. Sending `client_name` to the server stays with #75, so the scenario "An at sign in a query parameter value does not change the host" checks the `client_name` value of the connection parameters and not the client name of the server session.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] The FFI unit test step cannot pass on the current code

- **Finding:** Round 1 flagged an `[UNSTATED_ASSUMPTION]` BLOCKER. The Checklist step `cargo test --lib --features ffi` expected 0 failures, but two existing tests fail on HEAD 3601116. `test_autocommit_set_false` and `test_ffi_connection_options` call `set_option(AutoCommit, "false")` on an unconnected `FfiConnection`, which dials `localhost:8563` without `validateservercertificate=0`. The step is the only place where the plan runs the new FFI unit tests of task 2.5.
- **Direction change:** The Checklist step "Unit test (FFI)" now runs `cargo test --lib --features ffi -- --skip test_autocommit_set_false --skip test_ffi_connection_options`. A plan.md § Context bullet states why the two tests fail and that they stay out of scope. The last bullet of § Scenario Coverage and entry [8] Consequences name the new command.
- **Promotes to ADR:** no

### [plan-review] The client name step claimed behavior that issue #75 reports as missing

- **Finding:** Round 1 flagged an `[AMBIGUOUS_REQUIREMENT]` BLOCKER. The step "the client name SHALL be `dbt@ci`" reads as the client name of the server session. Both transports send the fixed name `exarrow-rs` at login, and the planned tests check only the parsed field.
- **Direction change:** The step in the scenario "An at sign in a query parameter value does not change the host" now reads "the connection parameters SHALL hold the `client_name` value `dbt@ci`". Entry [10] states that sending `client_name` to the server stays with #75.
- **Promotes to ADR:** no

### [plan-review] No scenario depended on the order of the `user` and `username` query keys

- **Finding:** Round 1 flagged an `[IMPLEMENTATION_LEAKAGE]` BLOCKER. The Background states that the `user` query key comes before `username`, but no scenario step depends on that order. The driver manager consequence of entry [2], a login as the `user` query value when only the `password` option is set, had no test.
- **Direction change:** The credential-sources spec gains the scenario "Query user applies when only the password option is set", so the feature holds 9 scenarios. Task 1.4 adds `test_parse_with_credentials_query_user_with_password_option`, task 2.5 adds `test_ffi_database_query_user_with_password_option`, and § Scenario Coverage lists both. Entry [2] Consequences names the scenario, and entry [7] counts nine scenarios.
- **Promotes to ADR:** no
