# Tasks: fix-execute-schema

## PR Lifecycle
- [x] resolved
- [ ] implemented
- [ ] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1 Docker exasol-test running
- [x] 1.2 Helper + test_execute_schema_reports_columns_without_running_the_query (+ websocket)
- [x] 1.3 test_execute_schema_of_parameterized_select_needs_no_binding
- [x] 1.4 test_execute_schema_of_row_count_statement_is_empty_and_runs_nothing
- [x] 1.5 test_execute_schema_follows_sql_set_after_prepare
- [x] 1.6 test_execute_schema_without_sql_fails_with_invalid_state
- [x] 1.7 Confirm new tests fail on unchanged code
- [x] 2.1 Unit tests in src/adbc_ffi.rs
- [x] 2.2 Scenario lines + InvalidState assertions on test_get_parameter_schema_not_prepared
- [x] 2.3 Confirm unit tests fail on unchanged code
- [x] 3.1 build_schema pub(crate)
- [x] 3.2 execute_schema body
- [x] 3.3 set_sql_query discards prepare
- [x] 3.4 module doc table row
- [x] 3.5 Run lib + driver_manager tests green

## Phase 2: Implementation (Group B)
- [x] 4.1 Version bump 0.21.3 + cargo check
- [x] 4.2 CHANGELOG section
- [ ] 4.3 Run Checklist

## Phase 4: Review Fixes
- [x] 4.4 src/adbc_ffi.rs: delete `connected_transport` and its doc comment; use `schema_transport()` in `transport_that_never_executes` and `bound_execute_after_set_sql_query_runs_the_new_sql`
- [x] 4.5 src/adbc_ffi.rs: delete the doc comment above `transport_that_never_executes`
- [x] 4.6 tests/driver_manager_tests.rs: in `open_connection`, replace the `ManagedDriver::load_dynamic_from_filename(...)` expression with `load_driver()`
- [x] 4.7 tests/driver_manager_tests.rs: delete the `// Statement Schema Lookup: execute_schema` banner and the blank line after it
