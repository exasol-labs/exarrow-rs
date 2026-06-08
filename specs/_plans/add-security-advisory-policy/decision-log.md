# Decision Log: add-security-advisory-policy

Date: 2026-06-08

## Interview

**Q:** Should we fix or suppress the thrift CVE?
**A:** Suppress with documented rationale — no patch exists on crates.io (`thrift 0.23.0` not published; `parquet ^0.17` blocks a patch override; `parquet 59.x` which removes thrift entirely not yet released).

**Q:** Additional dep bumps?
**A:** Yes — run `cargo update` for patch-level bumps (no breaking changes, no downgrades).

**Q:** New CI step for advisories?
**A:** Yes — add `cargo deny check advisories` to CI alongside the existing `cargo deny check licenses`.

## Design Decisions

### [1] Suppress GHSA-2f9f-gq7v-9h6m rather than patch

- **Decision:** Add a `deny.toml` suppression entry for GHSA-2f9f-gq7v-9h6m with a structured rationale comment instead of attempting a `[patch.crates-io]` override or a version downgrade.
- **Alternatives:** (a) `[patch.crates-io]` pointing to the `thrift` git repository at tag `v0.23.0` — rejected because `parquet`'s `^0.17` semver constraint does not admit 0.23.0 and Cargo rejects the override. (b) Downgrade to `arrow/parquet 57.x` — already rejected in prior work (PR#36 confirmed 58.x resolves the `adbc_core 0.23` semver conflict). (c) Wait without action — leaves an open Dependabot alert with no documented closure.
- **Rationale:** Documented suppression is the canonical `cargo-deny` pattern when no patch is available. The re-evaluation trigger (`parquet 59.x` or `adbc_core` lifting the `arrow-schema <59` cap) provides an audit trail for future maintainers.
- **Promotes to ADR:** yes

### [2] Co-locate advisories check in the existing `licenses` CI job

- **Decision:** Add `cargo deny check advisories` as a second step inside the existing `licenses` job rather than creating a new CI job.
- **Alternatives:** A standalone `advisories` job — would require its own `cargo-deny` install action, adding runner cost and parallelism complexity for a single command.
- **Rationale:** `cargo-deny` is already installed in the `licenses` job via `taiki-e/install-action@cargo-deny`. Both checks share no output artifacts with other jobs and both must pass before integration tests. Co-location is the simplest change with the smallest CI diff.
- **Promotes to ADR:** no

### [3] Restrict `cargo update` to patch-level; no minor/major bumps

- **Decision:** Run `cargo update` without explicit `--precise` flags, accepting all patch-level resolver advances but excluding any manual minor/major version edits to `Cargo.toml`.
- **Alternatives:** Selective `--precise` updates for specific crates — more surgical but also more error-prone and time-consuming for a routine lockfile refresh.
- **Rationale:** Patch-level bumps are safe by semver contract. The spec (scenario: "Patch-level dep bump applied without breaking-change review") formalises this policy. The full test suite confirms no regressions.
- **Promotes to ADR:** no

### [4] Introduce `code-quality/dependencies` as a new spec feature

- **Decision:** Create a new feature spec at `specs/code-quality/dependencies/spec.md` rather than extending the existing `code-quality/core` spec.
- **Alternatives:** Add dependency-policy scenarios to `code-quality/core` — rejected because `core` covers compiler-level quality gates (clippy, tests, Arrow invariants), while dependency policy is a separate concern with distinct actors and triggers.
- **Rationale:** Separation by concern makes each spec independently validatable and avoids inflating `core` with policy scenarios that have no automated test counterpart in the unit-test suite.
- **Promotes to ADR:** no

## Review Findings

<!-- Significant code-review findings that changed implementation direction. -->
<!-- Populated by speq-implement after code review. -->
