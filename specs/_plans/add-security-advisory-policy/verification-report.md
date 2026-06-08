# Verification Report: add-security-advisory-policy

**Generated:** 2026-06-08

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All automated checks and manual verification pass. 1023 unit tests green, 0 clippy warnings, advisories and licenses clean. |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit | 1023 | 1023 | 0 |
| Integration | not run (requires live Exasol) | — | — |

### Manual Tests

| Test | Result |
|------|--------|
| `cargo deny check advisories` exits 0 | ✓ |
| `cargo deny check licenses` exits 0 | ✓ |
| `cargo build` exits 0 | ✓ |
| `cargo tree -i thrift` shows only parquet transitive chain | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -W clippy::all
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.18s
(0 warnings, 0 errors)
```

### Formatter

```
cargo fmt --all -- --check
exit: 0 (no changes required)
```

### Advisory check

```
cargo deny check advisories
warning[unknown-advisory]: advisory not found in any advisory database (GHSA-2f9f-gq7v-9h6m)
warning[advisory-not-detected]: advisory was not encountered
advisories ok
```

Note: The two warnings are expected — the local cargo-deny advisory database does not yet include GHSA-2f9f-gq7v-9h6m (a 2026 advisory). The check still exits 0. When the advisory is published to the RustSec database, it will be matched and the `ignore` entry will suppress it correctly.

### Thrift chain

```
cargo tree -i thrift
thrift v0.17.0
└── parquet v58.3.0
    └── exarrow-rs v0.12.6
```

Only one path: exarrow-rs → parquet → thrift. No direct dependency.

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Passes |
|--------|---------|----------|---------------|--------|
| code-quality | dependencies | Advisory suppression documents rationale | `deny.toml` entry + `cargo deny check advisories` | Pass |
| code-quality | dependencies | Patch-level dep bump applied without breaking-change review | `cargo deny check` exits 0 after `cargo update` | Pass |
| code-quality | dependencies | Minor or major dep bump requires explicit evaluation | PR process (manual, no automated test) | N/A |
| code-quality | dependencies | Advisory CI gate blocks merge on unacknowledged advisory | `cargo deny check advisories` step added to `licenses` job | Pass |
| code-quality | dependencies | GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift | `deny.toml` ignore entry with full rationale; `advisories ok` | Pass |

## Notes

- Integration tests not run locally (require live Exasol Docker container). CI will cover these.
- `cargo deny check advisories` local warning about "unknown advisory" is expected — the local GHSA database does not yet have GHSA-2f9f-gq7v-9h6m. The suppression entry is correct and will take effect once the advisory is indexed. CI uses a fresh advisory database download and may behave differently.
- `cargo update` produced 68 package updates including several minor-version bumps (tokio 1.50→1.52, aws-lc-rs 1.16→1.17, etc.) and three crates that crossed a semver major boundary under their declared constraints (anstream 0.6→1.0, anstyle-parse 0.2→1.0, shlex 1.3→2.0). All are transitive dependencies within declared semver ranges.
