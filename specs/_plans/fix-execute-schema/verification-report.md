# Verification Report: fix-execute-schema

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | `execute_schema` now serves the schema from prepare metadata and runs nothing. All checklist commands exit 0. |
| Code review | 4 findings, 4 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ release `ffi websocket` build, websocket-only tests build |
| Tests | ✓ lib 0 failures, lib `ffi` 0 failures, driver_manager_tests 0 failures, integration_tests 0 failures |
| Lint | ✓ clippy `--all-targets --all-features -W clippy::all`, 0 warnings |
| Format | ✓ `cargo fmt --all -- --check` |
| Scenario Coverage | ✓ every scenario in the plan has a passing unit and/or integration test |
| Manual Tests | not run: the Python `adbc_driver_manager` package is not installed. The same checks (no execution, schema, zero-field schema for INSERT) are covered by the driver-manager integration tests, which load the release cdylib. The `exapump` setup and teardown steps ran; a row count of 1 was confirmed before any execute_schema call. |

## Test Evidence

### Regression tests on unchanged code (tasks 1.7, 2.3)

Failure messages are in `notes/A-failures.md`, which stays local.
- Driver-manager: 5 of 6 new tests failed (query ran and failed on cast; "Not enough parameters bound"; "Cannot fetch batches from row count result" after the INSERT ran; `get_parameter_schema` returned Ok instead of `InvalidState`). `test_execute_schema_without_sql_fails_with_invalid_state` passed as the plan expected.
- Unit: 5 of 7 new tests failed. `execute_schema_without_sql_fails_with_invalid_state` passed as expected. `set_sql_query_with_unchanged_sql_keeps_the_prepared_statement` also failed on old code (the old `execute_schema` runs the query), which differs from the plan's expectation. It passes after the fix.

### Results after the fix

| Suite | Result |
|-------|--------|
| `cargo test --lib` | pass |
| `cargo test --lib --features ffi` | 1805 passed |
| driver_manager_tests (`ffi websocket`, `--include-ignored`) | 73 passed |
| integration_tests (`ffi`) | pass |
| coverage `check` | pass |

Version bumped to 0.21.3 (main was 0.21.2); CHANGELOG `## 0.21.3` added with the `Changed:` entry.
