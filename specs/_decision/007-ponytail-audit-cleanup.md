# Decisions: ponytail-audit-cleanup

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR: In a library, "unused internally" is not "dead" — and tests must not mutate global env

**ID:** unused-internally-is-not-dead-tests-must-not-mutate-env
**Plan:** ponytail-audit-cleanup
**Status:** Accepted

### Context

A whole-repo over-engineering audit flagged ~3,500 lines as removable. Much of it was a category error: a *library* legitimately exports surface its own internals never call (it exists for downstream consumers), so "zero internal callers" alone does not make a symbol dead. Separately, the audit surfaced a long-standing test-isolation defect: non-`#[ignore]` unit tests in `tests/common/mod.rs` called `env::remove_var` on the `EXASOL_*` connection vars. Because that module compiles into every integration-test binary and the suite runs `--test-threads=1`, those tests wiped the configured connection parameters mid-run, so later tests silently fell back to `localhost:8563` and failed against any non-default container.

### Decision

Two rules, both from this cleanup:

1. **Deletion criterion.** Only remove a symbol when it has zero non-test callers **and** is not part of the public API (not re-exported via `lib.rs` or any `pub use`, and not reachable through a `pub mod`). Documented or exported surface — `ArrowConverter`, `ResultSetIterator`, the `blocking_*` sync API, `ArrowToCsvWriter`, `CsvToArrowReader` — is retained even when internally unused; its removal is a deliberate, separately-decided breaking change. Genuinely-unreachable internals (e.g. `SessionManager`, dead protocol constants) are removed.
2. **Tests never mutate the shared process environment.** No test calls `env::set_var`/`remove_var`. Env-derived logic is exercised through pure helpers taking explicit arguments (e.g. `common::connection_string`). See the code-quality scenario "Tests do not mutate shared process environment".

Removing the unreachable public items (`connection::session::SessionManager`, the unused `ExportError` variants) is a breaking change, released as `0.13.0`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Delete only verified-dead, non-exported code; retain public surface | ✓ Chosen — preserves the library's contract with downstreams while still removing genuine bloat |
| Delete everything with zero internal callers | ✗ Rejected — strips legitimate public API that exists for consumers, not internal use |
| Fix env-mutating tests via save/restore + a serial mutex | ✗ Rejected — still mutates global env (a window remains; `set_var` is `unsafe` in edition 2024); pure helpers eliminate the hazard entirely |
| Patch-bump (0.12.9) despite removing public items | ✗ Rejected — removing reachable `pub` API is breaking under Cargo 0.x semver |

### Consequences

Future audits MUST apply the deletion criterion before removing any `pub` symbol. The test suite is now order-independent regardless of thread count or execution order, asserted by the new code-quality scenario. Separately, investigating the full `--include-ignored` run surfaced two pre-existing, `#[ignore]`-gated integration tests (never run by the CI integration stage, which omits `--include-ignored`) that were broken independently of this cleanup and were brought into line with the recorded behavior: `test_connect_with_nonexistent_uri_schema_succeeds` used `use_tls(false)` against the TLS-only container (fixed to accept the self-signed cert), and `test_uri_schema_activation_failure_returns_error` asserted the pre-ADR fatal behavior — it was reconciled with the URI-schema-best-effort decision and renamed `test_uri_schema_missing_is_best_effort_via_adbc`, asserting best-effort success on the ADBC URI path. Because `0.13.0` is breaking for downstreams pinned to `^0.12`, the release chain requires bumping `exapump`'s `exarrow-rs` constraint and advancing the `adbc-driver-exasol` vendored submodule pointer.
