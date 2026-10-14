# Tasks: fix-connection-parameters

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1-1.5 Connection-string parameter validation (plan.md group 1)

## Phase 2: Implementation (Group B)
- [x] 2.1-2.5 Client identification at login (plan.md group 2)

## Phase 2: Implementation (Group C)
- [x] 3.1-3.3 Docs and release (3.4 and the final gates 3.5 are the orchestrator's) (plan.md group 3)

## Phase 4: Review Fixes
- [x] 4.1 tests/integration_tests.rs: gate `test_client_name_and_version_identify_the_session_native` with `#[cfg(feature = "native")]`, build its connection string from `get_test_connection_string_with_transport("native")`, and import that helper in the native `use common::{...}`
- [x] 4.2 src/lib.rs, src/transport/protocol.rs, src/connection/params.rs: add `pub(crate) const DRIVER_VERSION` and use it in `Credentials::new` and `ConnectionBuilder::build`
- [x] 4.3 src/transport/native/handshake.rs: document in `build_login_packet` that the packet never carries the password
