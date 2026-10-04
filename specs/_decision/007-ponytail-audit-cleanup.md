# Decisions: ponytail-audit-cleanup

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-007: In a library, "unused internally" is not "dead", and tests must not mutate global env

**ID:** unused-internally-is-not-dead-tests-must-not-mutate-env
**Plan:** ponytail-audit-cleanup
**Status:** Accepted

### Context

A library exports surface that its own code never calls, because downstream consumers use it. Zero internal callers therefore does not make a symbol dead. Separately, unit tests in `tests/common/mod.rs` called `env::remove_var` on the `EXASOL_*` variables. That module compiles into every integration-test binary, so the tests wiped the connection settings mid-run and later tests fell back to `localhost:8563`.

### Decision

1. **Deletion criterion.** Remove a symbol only when it has zero non-test callers and is not public API. Public API means re-exported through `lib.rs` or `pub use`, or reachable through a `pub mod`. Exported surface such as `ArrowConverter`, `ResultSetIterator`, the `blocking_*` API, `ArrowToCsvWriter`, and `CsvToArrowReader` stays even when unused internally. Removing it is a separate, deliberate breaking change.
2. **No env mutation in tests.** No test calls `env::set_var` or `env::remove_var`. Tests exercise env-derived logic through pure helpers with explicit arguments, such as `common::connection_string`.

Removing unreachable public items (`SessionManager`, unused `ExportError` variants) is a breaking change released as `0.13.0`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Delete only verified-dead, non-exported code | ✓ Chosen. Keeps the public contract and removes real bloat |
| Delete everything with zero internal callers | ✗ Rejected. It strips public API meant for consumers |
| Save/restore env with a serial mutex | ✗ Rejected. It still mutates global env, and `set_var` is `unsafe` in edition 2024 |
| Patch bump to 0.12.9 | ✗ Rejected. Removing public API is breaking under 0.x semver |

### Consequences

Future audits MUST apply the deletion criterion before removing a `pub` symbol. The test suite is independent of thread count and order. Because `0.13.0` is breaking for `^0.12` users, the `exapump` `exarrow-rs` constraint and the `adbc-driver-exasol` submodule pointer need updating.
