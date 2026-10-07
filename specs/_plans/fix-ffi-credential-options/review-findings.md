# Code Review Findings: fix-ffi-credential-options

## Summary
- Files reviewed: 9
- Total findings: 5 (standard: 5, expert: 0)
- Evidence: `cargo clippy --all-targets --all-features -- -W clippy::all` reports no warnings. `cargo test --lib --features ffi` reports 1717 passed and 0 failed.

## Standard fixes

### src/adbc_ffi.rs

#### [CONTEXTLESS_ERROR] Unset-URI error tells the caller to set an option the driver never reads
- Location: line 1054, `FfiDatabase::build_connection_params`
- Issue: The error text is `Database URI not set. Set adbc.exasol.uri option.`. `FfiDatabase::set_option` stores the key `adbc.exasol.uri` under `OptionDatabase::Other` in `options`. `build_connection_params` reads only `self.uri`, which only `OptionDatabase::Uri` (the key `uri`) sets. A caller who follows the message gets the same error again. No test covers this failure path.
- Fix: In src/adbc_ffi.rs, `FfiDatabase::build_connection_params`, change the message to `"Database URI not set. Set the uri database option."` and keep the status `AdbcStatus::InvalidState`. In the `Credential sources` section of `mod tests`, add `test_ffi_database_without_uri_rejects_connection`: build `FfiDatabase::new()`, set only `OptionDatabase::Password` to `"Secret1"`, call `new_connection()`, and assert that the error status is `AdbcStatus::InvalidState`, that the message contains `uri database option`, and that the message does not contain `Secret1`. Write the test first and run it to see it fail on the old text.

#### [DUPLICATE_TEST] `test_autocommit_set_false` repeats `test_ffi_connection_options`
- Location: line 3836, `test_autocommit_set_false`
- Issue: Both tests build the same `MockTransport` with the same five expectations, including `expect_set_autocommit().with(eq(false)).times(1)`, and both call `set_option(OptionConnection::AutoCommit, "false")` on `connected_connection(transport)`. They differ only in the assertion. `test_autocommit_set_false` asserts the private field `conn.auto_commit`, which is internal state. `test_ffi_connection_options` asserts the observable `get_option_string(OptionConnection::AutoCommit)`, and its mock already checks the single `set_autocommit(false)` call.
- Fix: In src/adbc_ffi.rs `mod tests`, delete the function `test_autocommit_set_false`. Keep `test_ffi_connection_options` with its mock expectations unchanged.

#### [VAGUE_TEST_NAME] `test_ffi_connection_options` does not state the condition or the expected result
- Location: line 3857, `test_ffi_connection_options`
- Issue: The name does not say that autocommit is turned off on a connected connection, or that the connection opens one transaction and reports `false`. The other tests in the `Connection options` section use names that state behavior, such as `set_option_auto_commit_to_the_value_already_in_force_needs_no_connection`.
- Fix: In src/adbc_ffi.rs `mod tests`, rename `test_ffi_connection_options` to `set_option_auto_commit_false_opens_a_transaction_and_reports_false`. Leave its body unchanged.

### src/connection/params.rs

#### [MISSING_BOUNDARY_TEST] Query parameter error position is tested only at position 1
- Location: line 454, `parse_query_params`; tests at line 1464 onward
- Issue: `parse_query_params` reports `index + 1` from `enumerate()` over every `&`-separated part, and it counts empty parts it skips. Every test uses a bad part at position 1. A change that counts only non-empty parts, or that reports a 0-based index, still passes for position 1 when the bad part is first and no part is empty. No test pins the position of a later part or the counting of an empty part.
- Fix: In src/connection/params.rs `mod tests`, `Parse errors` section, add `test_parse_query_param_error_counts_every_ampersand_separated_part` without a `/// Scenario:` line. Assert that `ConnectionParams::from_str("exasol://alice@db.example.com:8563?tls=true&Kp9")` fails with a `to_string()` that contains `position 2`. Assert that `ConnectionParams::from_str("exasol://alice@db.example.com:8563?tls=true&&Kp9")` fails with a `to_string()` that contains `position 3`. Assert that neither message contains `Kp9`.

### tests/driver_manager_tests.rs

#### [TOO_MANY_ARGUMENTS] `open_database` takes 4 arguments
- Location: line 2758, `open_database`; line 2798, `login_with_options`
- Issue: `open_database(driver, uri, username, password)` takes 4 arguments. Every caller passes a driver from `load_driver()` only to create the database. In adbc_driver_manager 0.23, `ManagedDatabase` holds an `Arc` of the driver internals (`ManagedDatabaseInner::driver`), so a database outlives the `ManagedDriver` value that created it, and the helper can load the driver itself.
- Fix: In tests/driver_manager_tests.rs, change `open_database` to `fn open_database(uri: &str, username: Option<&str>, password: Option<&str>) -> ManagedDatabase` and make its first line `let mut driver = load_driver();`. Change `login_with_options` to `fn login_with_options(username: &str, password: &str) -> String` and drop its `driver` argument from the `open_database` call. In the seven tests from `test_driver_manager_option_credentials_override_uri_credentials` to `test_driver_manager_uri_parse_error_omits_uri_values`, delete `let mut driver = load_driver();` and remove the `&mut driver` argument from every `open_database` and `login_with_options` call. Run `cargo clippy --all-targets --all-features -- -W clippy::all`. Then run `cargo build --release --features ffi` and `REQUIRE_EXASOL=1 cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1`.

## Expert fixes
[none]
