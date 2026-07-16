# Plan: fix-dependabot-security-alerts

> **Status:** blocked — see open-questions.md

## Summary

Reconcile the sole open Dependabot alert (#18, GHSA-2f9f-gq7v-9h6m, Apache Thrift) with the documented `deny.toml` suppression policy, then release `0.14.1`. The advisory is not fixable today because `adbc_core 0.23.0` caps `arrow-schema` at `<59`, blocking the `parquet` 59.x upgrade that would drop `thrift`; the release refreshes the stale suppression rationale, dismisses alert #18 as `tolerable_risk`, and records the re-evaluation.

This is a documentation and audit-trail release. No vulnerable code is removed — `thrift 0.17.0` remains present and suppressed. The `cargo deny` gate and the GitHub Dependabot alert are brought into agreement with the standing policy decision so both surfaces state the same rationale.

## Design

### Context

Dependabot alert #18 (GHSA-2f9f-gq7v-9h6m / CVE-2026-43868, Apache Thrift, CWE-789, CVSS 5.3 Medium) is the only open alert on the repository. ADR-003 suppressed this advisory in `deny.toml` with a dual re-evaluation trigger: `parquet 59.x` released, OR `adbc_core` lifts its `arrow-schema <59` cap. `parquet` 59.0.0 and 59.1.0 are now published on crates.io and drop the `thrift` dependency entirely, so the first trigger has fired. The suppression rationale is now stale: it still claims `parquet 59.x` is unreleased.

Verification of the second trigger shows the advisory is still not fixable. `adbc_core` latest published version remains 0.23.0, and its manifest still requires `arrow-schema >=53.1.0, <59`. A direct experiment (temporarily bumping `arrow`/`parquet` to `59` and running `cargo build --features ffi`) failed with 12 `E0053` "incompatible type for trait" errors in `src/adbc_ffi.rs`: two incompatible major versions of `arrow-array`/`arrow-schema` land in the build graph simultaneously and Rust treats them as distinct types across the `adbc_core::Statement`/`RecordBatchReader` trait boundary.

- **Goals** — Record the re-evaluation, refresh the suppression rationale to match reality, narrow the re-evaluation trigger, dismiss GitHub Dependabot alert #18 so its state matches the standing policy, and ship a patch release.
- **Non-Goals** — Bumping `arrow`/`parquet` to 59.x (verified to break the `ffi` build). No source-code change, no `Cargo.lock` dependency change. No downstream release chain: the workspace "Release chains" rule (new `exapump` release, `adbc-driver-exasol` PR) is deliberately skipped because this patch ships no functional, API, or dependency change for downstreams to pick up.

### Decision

Keep the GHSA-2f9f-gq7v-9h6m suppression in `deny.toml`. Rewrite its comment block and `reason` field so they state the current facts: `parquet 59.x` is released and removes `thrift`, but `adbc_core 0.23.0` caps `arrow-schema` at `<59`, so adopting `parquet` 59.x forces two incompatible major versions of `arrow-schema` into the build graph and breaks the `ffi` build. Narrow the re-evaluation trigger to a single condition: `adbc_core` lifting the `arrow-schema <59` cap. Update the corresponding spec scenario to match.

Reconcile GitHub Dependabot alert #18 with this policy by dismissing it as `tolerable_risk`, with a dismissal comment citing the `deny.toml` suppression and the re-evaluation trigger. Editing `deny.toml` alone leaves the GitHub alert `open`, so the alert state would contradict the standing decision. The dismissal is reversible: when `adbc_core` lifts the cap and the `parquet` 59.x upgrade drops `thrift`, the alert resolves automatically. Release `0.14.1`.

#### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Refresh suppression rationale, keep suppression | Remove suppression and bump `arrow`/`parquet` to 59.x | The 59.x bump breaks the `ffi` build (verified: 12 `E0053` trait-mismatch errors); the advisory is not fixable today. |
| Narrow re-evaluation trigger to the `adbc_core` cap | Keep the dual `parquet 59.x` OR `adbc_core` trigger | The `parquet 59.x` condition has fired; only the `adbc_core` cap still blocks the upgrade, so it is the sole remaining trigger. |
| Dismiss GitHub alert #18 as `tolerable_risk` | Leave the alert `open` | Leaving it open contradicts the standing suppression policy and overstates what an open alert means; dismissal makes both surfaces agree and auto-resolves on a future fix. |
| Ship as a doc-only patch release `0.14.1` | Fold into a later release | AGENTS.md requires a CHANGELOG entry per version bump; a dedicated patch release gives the audit-trail correction a traceable version. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| code-quality/dependencies | CHANGED | `code-quality/dependencies/spec.md` |

## Implementation Tasks

- [ ] 1.1 Rewrite the GHSA-2f9f-gq7v-9h6m comment block and `reason` field in `deny.toml`: state `parquet 59.x` is released and removes `thrift`, that `adbc_core 0.23.0` caps `arrow-schema` at `<59` so adopting `parquet` 59.x forces two incompatible `arrow-schema` majors into the build graph and breaks the `ffi` build, and that re-evaluation triggers when `adbc_core` lifts the `arrow-schema <59` cap.
- [ ] 1.2 Bump the `[package]` version in `Cargo.toml` from `0.14.0` to `0.14.1`.
- [ ] 1.3 Add a `## 0.14.1` entry to `CHANGELOG.md` above `## 0.14.0`. Label it as a documentation and audit-trail change: the entry MUST state that the `deny.toml` suppression rationale was refreshed and Dependabot alert #18 dismissed as `tolerable_risk`, and MUST NOT imply the Thrift vulnerability was fixed or removed (`thrift 0.17.0` remains present and suppressed).
- [ ] 1.4 Run `cargo deny check advisories` (exit 0), `cargo fmt --all -- --check`, and `cargo build` to confirm the working tree is clean and no dependency changed.
- [ ] 1.5 Dismiss GitHub Dependabot alert #18 (delegated to `git-agent` as a generic `gh api` call; the git-operations catalog has no dedicated alert-dismiss verb): `ghbrk gh api -X PATCH repos/exasol-labs/exarrow-rs/dependabot/alerts/18 -f state=dismissed -f dismissed_reason=tolerable_risk -f dismissed_comment="<rationale citing the deny.toml GHSA-2f9f-gq7v-9h6m suppression and the adbc_core arrow-schema <59 re-evaluation trigger>"`. Then confirm the alert reads `dismissed` via `ghbrk gh api repos/exasol-labs/exarrow-rs/dependabot/alerts/18 --jq .state`.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1, 1.2, 1.3 |
| Group B | 1.4 |
| Group C | 1.5 |

Sequential dependencies:
- Group A → Group B (verification runs after the edits).
- Group A → Group C (the alert dismissal cites the refreshed rationale, so it runs after 1.1 lands). Group C is independent of Group B.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None | — | Documentation and version-bump only; no code or dependency changes. |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift | Command gate | CI job `licenses` (`.github/workflows/ci.yml`), `scripts/run_all_tests.sh` | `cargo deny check advisories` |
| GHSA-2f9f-gq7v-9h6m suppression for Apache Thrift (alert reconciliation) | GitHub API check | Manual / one-shot | `gh api .../dependabot/alerts/18 --jq .state` returns `dismissed` |

This feature's verifiable behavior is a tool exit code plus a GitHub-side alert state, not application logic. Consistent with ADR-003, the suppression is proven by the `cargo deny check advisories` gate, not a Rust `#[test]`. The alert-reconciliation clause is proven by the GitHub API returning `dismissed`; it is a one-shot GitHub-state assertion, not a CI gate.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| code-quality/dependencies | `cargo deny check advisories` | Exit 0; GHSA-2f9f-gq7v-9h6m suppressed. |
| code-quality/dependencies | `grep -A2 GHSA-2f9f deny.toml` | `reason` states `parquet 59.x` released, `adbc_core 0.23.0` caps `arrow-schema <59`, single re-evaluation trigger. |
| code-quality/dependencies | `grep '^version' Cargo.toml` | `version = "0.14.1"` |
| code-quality/dependencies | `head CHANGELOG.md` | `## 0.14.1` entry present, labelled documentation/audit-trail. |
| code-quality/dependencies | `ghbrk gh api repos/exasol-labs/exarrow-rs/dependabot/alerts/18 --jq .state` | `dismissed` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Advisories | `cargo deny check advisories` | Exit 0 |
| Build | `cargo build` | Exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Lockfile | `git diff --stat Cargo.lock` | No change |
| Alert state | `ghbrk gh api repos/exasol-labs/exarrow-rs/dependabot/alerts/18 --jq .state` | `dismissed` |
