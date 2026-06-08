# Plan: Fix CVE-2026-43868 (Apache Thrift) with Arrow 58

## Context

PR#37 (`fix/security-dependabot-2026-06`) reverted arrow from 58→57.1.0 and only acknowledged
CVE-2026-43868 in the changelog. The rejection reason ("adbc_core 0.23.0 pins arrow-schema 57.x")
was a misdiagnosis — adbc_core 0.23.0 accepts `arrow-schema >=53.1.0, <59`, which includes 58.x.

Arrow 58 is already on `origin/main` from PR#36. This branch needs to be updated to match.

**CVE-2026-43868 facts:**
- Affected: `thrift` ≤ 0.22.0 (Memory Allocation with Excessive Size Value, CWE-789)
- Fix: Apache Thrift commit THRIFT-5871, present in tag `v0.23.0` (2026-04-15)
- The fix has NOT been published to crates.io (latest is 0.17.0)
- `parquet 58.x` still depends on `thrift ^0.17`
- `[patch.crates-io]` with v0.23.0 may fail semver resolution (^0.17 vs 0.23.0)

## Approach

### Step 1 — Upgrade Cargo.toml to arrow/parquet 58.x

```toml
arrow = "58"
parquet = { version = "58", features = ["async"] }
```

### Step 2 — Attempt [patch.crates-io] for thrift (test if semver allows it)

```toml
[patch.crates-io]
thrift = { git = "https://github.com/apache/thrift", tag = "v0.23.0" }
```

Run `cargo build` to test. If Cargo rejects due to `^0.17` vs 0.23.0 semver conflict, fall back to Step 2b.

### Step 2b — deny.toml ignore entry (fallback)

If `[patch]` fails, add to `deny.toml`:
```toml
{ id = "GHSA-2f9f-gq7v-9h6m", reason = "thrift 0.23.0 fix not on crates.io; ^0.17 semver prevents [patch] workaround; parquet 59.x will remove thrift (upstream PR apache/arrow-rs#9962, not yet released)" }
```

### Step 3 — Add invariant test from PR#36

Add `test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates` to `tests/integration_tests.rs`
to enforce that arrow/parquet resolve to >=58 and no duplicate arrow-array exists.

### Step 4 — Update CHANGELOG.md and version bump to 0.12.5

## Verification

```bash
cargo tree -i thrift
cargo build --release --features ffi
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -W clippy::all
cargo test --lib
cargo deny check advisories
```
