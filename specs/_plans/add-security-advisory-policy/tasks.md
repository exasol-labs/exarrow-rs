# Tasks: add-security-advisory-policy

## Phase 2: Implementation (Group A — parallel)
- [x] 2.1 Add GHSA-2f9f-gq7v-9h6m suppression entry to `deny.toml` with structured rationale
- [x] 2.2 Add `cargo deny check advisories` step to `.github/workflows/ci.yml` (after existing Check licenses step)
- [x] 2.3 Run `cargo update` to refresh `Cargo.lock` with latest patch-level versions

## Phase 2: Implementation (Group B — after Group A)
- [x] 2.4 Bump `version` in `Cargo.toml` from `0.12.5` to `0.12.6`
- [x] 2.5 Add `0.12.6` section to `CHANGELOG.md`

## Phase 3: Verification
- [x] 3.1 Run `cargo build` — exit 0
- [x] 3.2 Run `cargo test --lib` — 1023 passed, 0 failed
- [x] 3.3 Run `cargo fmt --all -- --check` — exit 0
- [x] 3.4 Run `cargo clippy --all-targets --all-features -- -W clippy::all` — 0 warnings
- [x] 3.5 Run `cargo deny check advisories` — advisories ok
- [x] 3.6 Run `cargo deny check licenses` — licenses ok
