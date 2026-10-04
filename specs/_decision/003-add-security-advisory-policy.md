# Decisions: add-security-advisory-policy

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-003: Suppress GHSA-2f9f-gq7v-9h6m (Apache Thrift) via documented deny.toml entry rather than patching

**ID:** suppress-ghsa-2f9f-gq7v-9h6m-via-deny-toml
**Plan:** add-security-advisory-policy
**Status:** Accepted

### Context

GHSA-2f9f-gq7v-9h6m (CWE-789, CVSS 5.3) affects `thrift 0.17.0`, pulled in by `parquet 58.x`. The fixed `thrift 0.23.0` is not on crates.io. `parquet` pins `^0.17`, so a `[patch.crates-io]` override is rejected by Cargo. `parquet 59.x` drops `thrift` but is unreleased, and `adbc_core 0.23` caps `arrow-schema` below 59.

### Decision

`deny.toml` suppresses GHSA-2f9f-gq7v-9h6m with a structured `reason` that records the upstream fix status, the semver blocker, and the re-evaluation trigger. The trigger is the release of `parquet 59.x` or `adbc_core` lifting the `arrow-schema <59` cap. `cargo deny check advisories` is a required CI gate in the `licenses` job.

### Options Considered

| Option | Verdict |
|--------|---------|
| Documented `deny.toml` suppression with re-evaluation trigger | ✓ Chosen. Standard `cargo-deny` pattern when no patch exists, with an audit trail |
| `[patch.crates-io]` to `thrift` 0.23.0 from git | ✗ Rejected. `parquet ^0.17` does not admit 0.23.0 |
| Downgrade to arrow/parquet 57.x | ✗ Rejected. 58.x is required to satisfy `adbc_core 0.23` |
| Wait without action | ✗ Rejected. The Dependabot alert stays open with no closure or trigger |

### Consequences

The CI gate blocks merges on any new unacknowledged advisory. The maintainer removes the suppression and retries `cargo update` when the trigger fires.
