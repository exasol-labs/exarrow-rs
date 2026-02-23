# Plan: fix-parquet-import-ddl-error-handling

## Summary

Fix a hang in `import_from_parquet` and `import_from_parquet_files` when `create_table_if_not_exists` is enabled and the CREATE TABLE DDL fails for reasons other than "table already exists" (e.g., nonexistent schema). The fix replaces the blanket error suppression (`let _ = execute_sql(ddl).await`) with selective suppression that only ignores "object already exists" errors.

## Design

### Goals / Non-Goals

- Goals
    - Propagate DDL errors as `ImportError::SqlError` when the CREATE TABLE fails for non-idempotency reasons
    - Prevent indefinite hangs when DDL fails (no HTTP transport setup after DDL failure)
    - Preserve existing behavior: silently ignore "table already exists" errors
- Non-Goals
    - Auto-creating schemas — that is a policy decision for higher-level tools
    - Changing the DDL generation logic itself
    - Changing the CSV import path (it does not have auto-create table logic)

### Trade-offs

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Check error string for "already exists" pattern | Use Exasol error codes; execute a separate check query first | Error string matching is simple, consistent with the `execute_sql` closure returning `Result<u64, String>`, and Exasol's error messages are stable. A separate check query adds latency and complexity. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Auto table creation | CHANGED | `import-export/auto-table-creation/spec.md` |

## Implementation Tasks

1. In `import_from_parquet` (`src/import/parquet.rs`), replace `let _ = execute_sql(ddl).await;` with logic that checks whether the error indicates "object already exists" and only suppresses that specific case; propagate all other DDL errors as `ImportError::SqlError`.
2. Apply the same fix in `import_from_parquet_files` (`src/import/parquet.rs`), which has the identical `let _ = execute_sql(ddl).await;` pattern.
3. Add a unit test verifying that a non-"already exists" DDL error is propagated (mock `execute_sql` returning an error without "already exists").
4. Add a unit test verifying that an "already exists" DDL error is silently ignored and import proceeds.
5. Add an integration test that calls `import_from_parquet` with `create_table_if_not_exists=true` targeting a nonexistent schema and asserts it returns an error (not a hang).

## Dead Code Removal

None. This is a targeted bug fix with no code being removed.

## Verification

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Integration tests | `cargo test --test import_export_tests -- --ignored` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 errors/warnings |
| Format | `cargo fmt --all -- --check` | No changes |

### Manual Testing

| Feature | Test Steps | Expected Result |
|---------|------------|-----------------|
| DDL error propagation | Call `import_from_parquet` with `create_table_if_not_exists=true` targeting `nonexistent_schema.test_table` | Returns `ImportError::SqlError` containing schema error message |

### Scenario Verification

| Scenario | Test Type | Test Location |
|----------|-----------|---------------|
| Auto-create table fails due to nonexistent schema | Integration | `tests/import_export_tests.rs` |
| Auto-create table fails due to nonexistent schema (multi-file) | Unit | `src/import/parquet.rs::tests` |
| Auto-create table fails due to other DDL errors | Unit | `src/import/parquet.rs::tests` |
| Auto-create with existing table (updated) | Unit | `src/import/parquet.rs::tests` |
