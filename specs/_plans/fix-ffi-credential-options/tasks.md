# Tasks: fix-ffi-credential-options

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1 Change `parse_auth` to return `(String, Option<String>)` (plan.md task 1.1)
- [x] 1.2 Add `parse_with_credentials` and make `from_str` call it (plan.md task 1.2) [expert]
- [x] 1.3 Manual redacting `Debug` for `ConnectionBuilder` (plan.md task 1.3)
- [x] 1.4 Unit tests for credential sources in `src/connection/params.rs` (plan.md task 1.4)
- [x] 1.5 Remove URI text from parse errors (plan.md task 1.5)
- [x] 1.6 Unit tests for parse errors (plan.md task 1.6)
- [x] 2.1 `FfiDatabase::build_connection_params` replaces `build_connection_uri` (plan.md task 2.1)
- [x] 2.2 `FfiConnection` holds `params: ConnectionParams` (plan.md task 2.2)
- [x] 2.3 Remove `FfiStatement::uri` and its fallback connection (plan.md task 2.3)
- [x] 2.4 Update existing FFI unit tests to the new constructors (plan.md task 2.4)
- [x] 2.5 Add FFI unit tests (plan.md task 2.5)
- [x] 3.1 Driver manager test helpers (plan.md task 3.1)
- [x] 3.2 Test: option credentials override URI credentials (plan.md task 3.2)
- [x] 3.3 Test: password option keeps URI username (plan.md task 3.3)
- [x] 3.4 Test: option password reaches server verbatim (plan.md task 3.4)
- [x] 3.5 Test: at sign in query value keeps host (plan.md task 3.5)
- [x] 3.6 Test: missing username is rejected (plan.md task 3.6)
- [x] 3.7 Test: URI credentials apply without options (plan.md task 3.7)
- [x] 3.8 Test: URI parse error omits URI values (plan.md task 3.8)
- [x] 3.9 Skip macros and scenario lines on tests 3.2 to 3.8 (plan.md task 3.9)
- [x] 4.1 `connect_with_transport` to `pub(crate)`; move `transport_session_info` (plan.md task 4.1)
- [x] 4.2 `connected_connection` helper; rewrite two autocommit tests over a mock transport (plan.md task 4.2)
- [x] 4.3 CI step runs `cargo test --lib --features ffi` (plan.md task 4.3)
- [x] 5.1 Update `docs/setup-and-connect.md` (plan.md task 5.1)
- [x] 5.2 Update `docs/driver-manager.md` (plan.md task 5.2)
- [x] 5.3 Add `## [Unreleased]` entries to `CHANGELOG.md` (plan.md task 5.3)

## Phase 3: Verification
- [x] 6.1 Start Exasol and wait for readiness
- [x] 6.2 Run every Checklist step from plan.md

## Phase 4: Review Fixes
- [x] 4.1 `FfiDatabase::build_connection_params`: change the unset-URI message to `Database URI not set. Set the uri database option.` and add `test_ffi_database_without_uri_rejects_connection` first
- [x] 4.2 Delete the duplicate test `test_autocommit_set_false` in `src/adbc_ffi.rs`
- [x] 4.3 Rename `test_ffi_connection_options` to `set_option_auto_commit_false_opens_a_transaction_and_reports_false`
- [x] 4.4 Add `test_parse_query_param_error_counts_every_ampersand_separated_part` in `src/connection/params.rs`
- [x] 4.5 Reduce `open_database` and `login_with_options` arguments in `tests/driver_manager_tests.rs`
