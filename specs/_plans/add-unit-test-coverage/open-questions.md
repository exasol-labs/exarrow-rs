# Open Questions: add-unit-test-coverage

speq-plan-pr could not complete this plan without human input. What's done so far is committed on this branch. Reply inline on the PR, or resume with `/speq:plan add-unit-test-coverage` locally, or re-run `/speq:plan-pr add-unit-test-coverage` after commenting.

- [ ] **Spec-delta mechanics:** both `code-quality/core/spec.md` and `native-client/protocol/spec.md` deltas carry their most important new rules inside an unmarked `## Background` edit (the 82%/per-file-floor MUST, and the "decoders MUST NOT panic" MUST). `/speq:record`'s marker rules only merge marked `## Scenarios` content — an unmarked `## Background` edit has no defined recorder behavior, so those rules could be silently dropped on record. Fix: move each sentence into a marked scenario (round-2 review proposes exact wording) and revert `## Background` to match the recorded library spec verbatim. This looks mechanical rather than a judgment call — confirm the planner should just do it, or flag if you want the Background text kept as-is for a different reason.
- [ ] **Per-file coverage floor `N` is unfalsifiable as designed:** task 6.1 sets `N` to "the largest multiple of 5 below the post-change per-file minimum" — i.e., after measuring the crate's own worst file, chosen to sit under it. That guarantees the floor passes by construction and can never catch a regression. Two ways to fix it, pick one:
  - (a) Require `N >= 50`, and explicitly exclude (via `--ignore-filename-regex` + an `AGENTS.md` note) any file that measures below 50% today (`src/transport/http_transport.rs` is the likely candidate, since its 276 LOCAL-IO lines are already deferred out of scope in decision [8]).
  - (b) Drop the per-file floor entirely and rely on the 3,554-uncovered-line ceiling plus SonarCloud's new-code condition as the only regression guards.

## Also flagged (non-blocking, carried from round-2 review — informational)

- Task 6.1's coverage-floor check must run as a separate CI step with `if: always()` on `Upload unit coverage`, not inlined onto the existing lcov-generation step — otherwise a below-floor run skips the artifact upload and the whole `sonar` job (which `needs: [unit-tests]`), leaving no report to diagnose the failure.
- Task 6.3 should only *record* the `--features websocket` coverage number, not decide whether to widen the CI command from it — that decision would silently invalidate the 3,554-line ceiling and 82.0% floor (both measured with default features).
- Recording this plan will push `native-client/protocol` to 12 scenarios and `code-quality/core` to 10, tripping `/speq:spec-merge`'s ">10 scenarios per spec" reorganization threshold — expect `/speq:record` to pause on that feature.
- The 82.0% floor's sensitivity direction was stated backwards in the plan (more test code raises the reported ratio, it does not lower it) — conclusion still holds, derivation needs correcting.
- Several prose-tightening asks (line-length, passive voice, one hedge) — cosmetic, not load-bearing.

## Design decisions worth a sanity check (from decision-log.md, not blocking)

- **[7] Fixing a real bug, not just adding tests:** `exasol_encode_pwd` panics on a remainder-by-zero if a server sends an empty `ATTR_RANDOM_PHRASE`. The plan changes this to return an error instead of pinning the panic with `#[should_panic]`. This is the one behavior change in an otherwise test-only plan — confirm you're fine with a production fix riding in a "raise coverage" PR.
- **[6] Keeping unused public API:** 8 unused `pub` items in `src/transport/messages.rs` are kept and tested rather than deleted, since the crate is published to crates.io and deleting `pub` API breaks downstream users for no coverage gain.
- **[10] This plan will not turn the SonarCloud gate green by itself:** 29 open code-smell issues (22 cognitive-complexity, 6 wildcard-import, 1 already handled by PR #49) plus 3.8% duplicated lines (vs. a 3% threshold) remain after this plan — explicitly out of scope, would need a separate plan.
