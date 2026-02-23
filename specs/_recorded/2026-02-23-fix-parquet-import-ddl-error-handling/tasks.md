# Tasks: fix-parquet-import-ddl-error-handling

## Phase 1: Implementation (Group A — DDL error handling fix)
- [x] 1.1 Fix `import_from_parquet` DDL error handling — replace `let _ = execute_sql(ddl).await` with selective suppression
- [x] 1.2 Fix `import_from_parquet_files` DDL error handling — same pattern

## Phase 2: Tests (Group B — Unit & Integration tests)
- [x] 2.1 Add unit test: non-"already exists" DDL error is propagated
- [x] 2.2 Add unit test: "already exists" DDL error is silently ignored
- [x] 2.3 Add integration test: import_from_parquet with nonexistent schema returns error

## Phase 3: Version Bump
- [x] 3.1 Bump version from 0.6.0 to 0.6.1 in Cargo.toml
- [x] 3.2 Add changelog entry for 0.6.1

## Phase 4: Verification
- [x] 4.1 Run build
- [x] 4.2 Run unit tests
- [x] 4.3 Run integration tests
- [x] 4.4 Run linter
- [x] 4.5 Run formatter check
