# Plan: add-security-advisory-policy

## Summary

Formally closes the only open Dependabot alert (GHSA-2f9f-gq7v-9h6m, Apache Thrift CWE-789) by adding a documented suppression to `deny.toml`, introduces `cargo deny check advisories` as a required CI gate alongside the existing license check, refreshes `Cargo.lock` with patch-level dep bumps, and releases this as version 0.12.6.

## Design

### Context

Dependabot has one open alert: GHSA-2f9f-gq7v-9h6m (Apache Thrift, Memory Allocation with Excessive Size Value, CVSS 5.3 Medium). The fix (`thrift 0.23.0`) does not exist on crates.io. A `[patch.crates-io]` override is blocked by `parquet`'s `^0.17` semver constraint, which does not admit 0.23.0. The upstream removal of the `thrift` dependency in `parquet 59.x` is not yet released. Until then the only actionable resolution is a documented suppression in `deny.toml`.

The project has never had a formal dependency security policy. CI only checks licenses; advisories are not gated. This plan introduces both the policy spec and the CI gate simultaneously.

- **Goals** — Suppress GHSA-2f9f-gq7v-9h6m with full rationale; add `cargo deny check advisories` to CI; establish a spec for the dependency security policy; bump to 0.12.6
- **Non-Goals** — Upgrading `parquet` beyond 58.x; changing the licenses allow-list; adding vulnerability scanning beyond `cargo deny`

### Decision

#### Architecture

```
deny.toml
  [advisories]
    ignore = [RUSTSEC-2024-0436, GHSA-2f9f-gq7v-9h6m]   ← add new entry

.github/workflows/ci.yml
  licenses job:
    - Check licenses    (existing)
    - Check advisories  ← new step

Cargo.lock              ← refreshed via cargo update (patch-level only)
Cargo.toml              ← version 0.12.5 → 0.12.6
CHANGELOG.md            ← 0.12.6 section added
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Documented suppression | `deny.toml` | No patch available; suppression with rationale is the correct resolution per `cargo-deny` advisory policy |
| Advisory gate in CI | `licenses` job | Runs immediately after build; no Exasol instance required; fail-fast before integration tests |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Suppress GHSA-2f9f-gq7v-9h6m | `[patch.crates-io]` override; stay on arrow 57 | `thrift 0.23.0` not on crates.io; `^0.17` blocks override; arrow 57 downgrade already rejected in prior work |
| Add advisories check to existing `licenses` job | Create a new `advisories` job | Keeps `cargo-deny` invocations co-located; `cargo-deny` is already installed in that job; no extra runner cost |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| code-quality/dependencies | NEW | `specs/_plans/add-security-advisory-policy/code-quality/dependencies/spec.md` |

## Implementation Tasks

1. Add GHSA-2f9f-gq7v-9h6m suppression entry to `deny.toml` with structured rationale: `thrift 0.23.0` not published on crates.io; `parquet ^0.17` semver constraint prevents a `[patch]` override to 0.23.0; `parquet 59.x` (which removes thrift) not yet released; `adbc_core 0.23` caps `arrow-schema <59`; re-evaluate when `parquet 59.x` ships or `adbc_core` supports `arrow-schema >=59`
2. Add `cargo deny check advisories` step to `.github/workflows/ci.yml` inside the `licenses` job, immediately after the existing `Check licenses` step
3. Run `cargo update` in the repository to refresh `Cargo.lock` with latest patch-level versions (no minor/major bumps, no downgrades)
4. Bump `version` in `Cargo.toml` from `0.12.5` to `0.12.6`
5. Add `0.12.6` section to `CHANGELOG.md` documenting: formal suppression of GHSA-2f9f-gq7v-9h6m in `deny.toml` with traceable rationale, `cargo deny check advisories` CI gate added, and `Cargo.lock` lockfile refresh with patch-level dep bumps

## Parallelization

Tasks 1, 2, 3 have no interdependencies and can be done concurrently.  Tasks 4 and 5 depend on all prior tasks being complete but are themselves independent of each other.

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1, 2, 3 |
| Group B | 4, 5 |

Sequential dependencies:
- Group A → Group B (version bump and changelog must reflect the final state of deny.toml and CI)

## Dead Code Removal

None. This plan adds new content only.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Advisory suppression documents rationale | Integration | `tests/integration_tests.rs` | `test_deny_toml_advisory_suppression_has_rationale` |
| Patch-level dep bump applied without breaking-change review | Integration | `tests/integration_tests.rs` | `test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates` |
| Minor or major dep bump requires explicit evaluation | Manual | — | PR process verification (no automated test; enforced by policy) |
| Advisory CI gate blocks merge on unacknowledged advisory | Integration | CI | `cargo deny check advisories` step in `licenses` job exits non-zero when an unacknowledged advisory is present |
| GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift | Integration | CI | `cargo deny check advisories` exits 0 after suppression entry is added |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| code-quality/dependencies | `cargo deny check advisories` | Exit 0; no unacknowledged advisories reported |
| code-quality/dependencies | `cargo deny check licenses` | Exit 0; license allow-list unchanged |
| code-quality/dependencies | `cargo build` | Exit 0; no compilation errors |
| code-quality/dependencies | `cargo tree -i thrift` | Shows `thrift v0.17.0` as a transitive dep of `parquet`; no other thrift version present |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Advisories | `cargo deny check advisories` | Exit 0 |
| Licenses | `cargo deny check licenses` | Exit 0 |
