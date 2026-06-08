# Tasks: is-there-a-way-adaptive-thacker

## Phase 1: Dependency Changes
- [x] 1.1 Upgrade arrow and parquet to 58.x in Cargo.toml
- [x] 1.2 Test [patch.crates-io] thrift v0.23.0 — rejected (^0.17 semver incompatible); added deny.toml ignore instead

## Phase 2: Tests and Docs
- [x] 2.1 Add invariant test test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates
- [x] 2.2 Update CHANGELOG.md with 0.12.5 entry

## Phase 3: Verification
- [x] 3.1 cargo build --release --features ffi — exit 0
- [x] 3.2 cargo fmt + cargo clippy — clean
- [x] 3.3 cargo test --lib — 1021 passed
- [x] 3.4 cargo deny check advisories — advisories ok
