# Decisions: add-security-advisory-policy

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-003: Suppress GHSA-2f9f-gq7v-9h6m (Apache Thrift) via documented deny.toml entry rather than patching

**ID:** suppress-ghsa-2f9f-gq7v-9h6m-via-deny-toml
**Plan:** add-security-advisory-policy
**Status:** Accepted

### Context

Dependabot opened an alert for GHSA-2f9f-gq7v-9h6m (Apache Thrift, CWE-789, CVSS 5.3 Medium), which affects `thrift 0.17.0` pulled in transitively by `parquet 58.x`. The canonical fix (`thrift 0.23.0`) is not published on crates.io. A `[patch.crates-io]` override is blocked by `parquet`'s `^0.17` semver constraint, which does not admit 0.23.0. The upstream removal of the `thrift` dependency (in `parquet 59.x`) is also not yet released, and `adbc_core 0.23` caps `arrow-schema` at `<59`, preventing an upgrade to `arrow/parquet 59.x`. A version downgrade to `arrow/parquet 57.x` was already rejected in prior work (PR#36 confirmed 58.x resolves the `adbc_core 0.23` semver conflict).

### Decision

Add a suppression entry for GHSA-2f9f-gq7v-9h6m to `deny.toml` with a structured `reason` field documenting the upstream fix status, the semver blocker, and the re-evaluation trigger (`parquet 59.x` released, or `adbc_core` lifts the `arrow-schema <59` cap). Introduce `cargo deny check advisories` as a required CI gate in the existing `licenses` job.

### Options Considered

| Option | Verdict |
|--------|---------|
| Documented suppression in `deny.toml` with re-evaluation trigger | ✓ Chosen — canonical `cargo-deny` pattern when no patch is available; provides audit trail for future maintainers |
| `[patch.crates-io]` pointing to `thrift` git repo at `v0.23.0` | ✗ Rejected — `parquet ^0.17` does not admit 0.23.0; Cargo rejects the override |
| Downgrade to `arrow/parquet 57.x` | ✗ Rejected — already rejected in PR#36; 58.x is required to resolve the `adbc_core 0.23` semver conflict |
| Wait without action | ✗ Rejected — leaves an open Dependabot alert with no documented closure or re-evaluation trigger |

### Consequences

The advisory is formally closed in Dependabot with a traceable rationale. Future maintainers have a concrete re-evaluation trigger. The `cargo deny check advisories` CI gate ensures any new unacknowledged advisory blocks merge automatically. When `parquet 59.x` is released or `adbc_core` lifts the `arrow-schema <59` cap, the suppression entry should be removed and a clean `cargo update` attempted.
