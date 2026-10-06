# Plan Review Findings: fix-paged-fetch-position (round 2)

## Summary
- Axes checked: 6/6
- Total findings: 12 (Blockers: 6, Advisory: 6)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Round-1 Blocker Recheck

- Round 1 raised no BLOCKER, so no blocker needs a recheck.
- Round 1's five ADVISORY findings are not applied. `decision-log.md` § Review Findings is empty, and tasks 1.2, 1.4, and 4.3 and decisions [4] and [5] are unchanged. They stand as written in `review/round-1.md` and are not repeated here.
- One exception: scope addition 2 changes the weight of round 1's WebSocket unit-test finding. It is raised again under Intent Fidelity.

## Premortem

Six months from now this plan failed. Three ways it could happen:

1. `feat/fix-export-parquet-transport-roundtrip` merges first, and CI publishes 0.17.0 on arrow 58. This PR then gets the patch bump that `/speq:implement-pr` step A3 applies to a `fix` plan, and CI publishes 0.17.1 on arrow 59. Every dependent on `exarrow-rs = "0.17"` that passes `RecordBatch` values stops compiling after `cargo update`. Routed to `[NFR_IGNORED]` BLOCKER.
2. CI turns green with all eleven native Parquet import tests ignored, so no CI step runs the native Parquet import path. The hang was not a separate server defect: nine of those tests passed against the same `exasol/docker-db:2025.2.0` image in plan 002's verification, and all 51 tests passed in plan 008's. A driver regression ships, and nobody filed the follow-up issue. Routed to `[UNSTATED_ASSUMPTION]` BLOCKER and `[HIDDEN_DEPENDENCY]` ADVISORY.
3. A maintainer suppresses a `benchmark`-only advisory, as the recorded scenario "Advisory suppression documents rationale" permits. `cargo deny check advisories`, the command that three unchanged scenarios name, now exits 1 with `error[advisory-not-detected]`, while the CI gate passes. Routed to `[REQUIREMENT_CONFLICT]` BLOCKER.

## Intent Fidelity

Checked: issue #80 is still covered by tasks 1.x to 5.1. Scope addition 1 is covered: alert #19 by the parquet 59 upgrade (tasks 6.1, 6.3), alert #20 by `xxhash-rust` 0.8.19 (task 6.2), the two `--all-features` advisories by task 6.2, and later advisories by the re-check in task 6.7. The arrow 59 upgrade is the brief's own path ("If a real `thrift` fix is available, plan it"), so it is not scope creep. Scope addition 2 is covered: the ten new paged-fetch integration tests live in `integration_tests` and `websocket_integration_tests`, which CI already runs, and tasks 7.1 to 7.6 add the three missing targets. The brief authorizes gating a test that cannot run in CI. Whether the eleven gated tests truly cannot run is challenged under Feasibility.

#### [SCOPE_REDUCTION] ADVISORY
- Location: decision-log.md § [5] Consequences ("Adding a CI step for WebSocket unit tests is outside this plan."); decision-log.md § Interview, scope addition 2 ("The new tests must run in CI."); plan.md task 3.6
- Issue: The two new WebSocket unit tests, `test_fetch_results_starts_after_the_inline_rows_and_advances_per_page` and `test_prepared_statement_fetch_starts_after_the_inline_rows`, compile only with the `websocket` feature. The CI `unit-tests` job runs `cargo llvm-cov --lib` with default features, so CI never runs them. They are the only tests that check interleaved handles (`startPosition` 2, 0, then 4), and issue #80 asks for exactly this unit coverage. The user's verbatim request names e2e tests, which the plan satisfies, so this is not a blocker. The plan's own interview record says "The new tests must run in CI", and decision [5] now contradicts that record. The fix costs one CI step.
- Fix: Add task 7.7 to plan.md: in the `unit-tests` job of `.github/workflows/ci.yml`, add a step that runs `cargo test --lib --features websocket` without coverage instrumentation, since `AGENTS.md` § Coverage keeps `websocket` out of the coverage command. Delete the last sentence of decision [5] Consequences and state the new step instead. Add the step to plan.md § Parallelization group B.

## Feasibility

Checked and confirmed:
- `cargo deny --all-features check advisories` fails today on RUSTSEC-2026-0204 and RUSTSEC-2025-0119, and it reports `unknown-advisory` and `advisory-not-detected` for GHSA-2f9f-gq7v-9h6m (run locally with cargo-deny 0.19.6).
- `paste` and `thrift` reach the tree only through `parquet` 58.3.0 (`cargo tree --all-features -i`).
- The crates.io index lists `adbc_core` 0.24.0 with `arrow-array` and `arrow-schema` `>=58, <60`, and 0.23.0 with `<59`.
- cargo-deny 0.19.6 accepts `unused-ignored-advisory = "deny"` and turns an unmatched ignore into an error (scratch config). CI installs the latest cargo-deny through `taiki-e/install-action`.
- The 70 MB tests and the CI timeout raise no objection. `WebSocketTransport` sets `max_message_size` and `max_frame_size` to `None` (`src/transport/websocket.rs` lines 449-450), so a 64 MiB fetch response is accepted. 70 rows of 1,000,000 bytes are 70,000,000 bytes, above 64 MiB (67,108,864 bytes), and the batch-shape assertions fail if the server stops paging. The job has a 30-minute limit against a 6 minute 34 second baseline. The new targets reuse the debug `ffi` build that `driver_manager_tests` already triggers. The import/export step gets its own 10-minute limit. The iterator call cap and the new mismatch error bound every new loop. The native paged-fetch tests run under `cargo llvm-cov` instrumentation, which the planning run did not measure, but decision [17] marks its estimate as unmeasured.

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: decision-log.md § [15] Rationale and Consequences; plan.md § Context (bullet "A planning run against a local `exasol/docker-db:2025.2.0` container ..."); plan.md § Impact ("The hang is a separate defect for a follow-up issue. The driver code for native Parquet import does not change in this plan."); plan.md task 7.2
- Issue: The plan keeps eleven tests out of CI on the belief that their hang is a separate defect. It never establishes the cause, and the repository's own records contradict the framing.
  - `specs/_recorded/002-fix-thrift-cve-upgrade-arrow-58/verification-report.md` ran `import_export_tests -- --ignored` against `exasol/docker-db:2025.2.0`. 40 tests passed, and only `test_parallel_parquet_import_mixed_batch_sizes` and `test_parallel_parquet_import_native_path` hung. The other nine tests in the plan's list existed at that commit (`95819035`) and passed.
  - `specs/_recorded/008-fix-export-parquet-transport-roundtrip/verification-report.md`, from the branch this plan builds on, reports 51 of 51 passed. It does not record the image. `AGENTS.md` starts `exasol/docker-db:latest`, which is 2026.1.0 on this host.
  - The planning run differs from both runs in `--features ffi`, `--test-threads=1`, and possibly the image.
  - The possible causes lead to different plans. If a CI flag causes the hang, the tests can run in CI. If the server version causes it, users on Exasol 2025.2.0 hang, because `supports_native_parquet_import` sends 2025.2.0 to the native path. If the driver regressed after commit `95819035`, a core capability is broken for users.
  - With all eleven tests ignored, no CI step runs the native Parquet import path. Images 2025.2.0, 2025.2.1, and 2026.1.0 are already present on this host, so the planner can settle the cause.
- Fix: Before task 7.2 keeps the eleven ignores, run `test_parquet_import_from_file` and `test_parquet_round_trip` with `REQUIRE_EXASOL=1` in four configurations: the CI flags of task 7.1 on `exasol/docker-db:2025.2.0`; no `--features ffi` and no `--test-threads=1` on 2025.2.0; the CI flags on 2025.2.1; and the CI flags on 2026.1.0. Record each result in decision [15] Rationale, next to the plan 002 and plan 008 results. Then apply the matching rule. If the hang depends on a CI flag, change the task 7.1 step so the tests run, and drop the ignores it frees. If the hang depends on the server version, name the version in the `#[ignore]` reason and add a plan.md § Impact bullet stating that native Parquet import hangs against Exasol 2025.2.0. If the tests hang in every configuration, state in decision [15] that they passed at commit `95819035` and that the regression lies after it.
- Escalation: MECHANICAL: the cause is a fact the planner can settle by running the tests against local images, and the Fix gives a rule for each outcome.

#### [NFR_IGNORED] BLOCKER
- Location: decision-log.md § [10] Rationale ("so the implement step decides whether these entries join 0.17.0 or a later version"); plan.md § Impact (bullet "Breaking change: exarrow-rs moves from arrow and parquet 58 to 59 ..."); plan.md § Context
- Issue: The arrow 59 and adbc 0.24 upgrade breaks downstream crates. The plan leaves the release version to the implement step without stating the constraint that follows. `/speq:implement-pr` step A3 bumps by Conventional Commits when the plan has no `workspace/version` delta: "`feat` → minor bump; a purely `fix` plan → patch". This plan is named `fix-paged-fetch-position`. This branch is stacked on `feat/fix-export-parquet-transport-roundtrip`, which is not on `origin/main` and already sets version 0.17.0 with no `v0.17.0` tag. If that branch merges first, CI publishes 0.17.0 on arrow 58, and a patch bump here publishes 0.17.1 on arrow 59. Cargo treats 0.17.1 as compatible with 0.17.0, so `cargo update` pulls arrow 59 into every dependent on `exarrow-rs = "0.17"`. Code that passes `RecordBatch` values between the two crates then stops compiling. A crates.io release can be yanked but not withdrawn. `CONTRIBUTING.md` § Releasing requires a SemVer bump.
- Fix: In decision-log.md [10], replace "so the implement step decides whether these entries join 0.17.0 or a later version" with this rule: the release that carries this change is a 0.x minor bump over the latest published version. If tag `v0.17.0` does not exist when this PR merges, keep `version = "0.17.0"` and fold `[Unreleased]` into `## 0.17.0`. If `v0.17.0` exists, set `version = "0.18.0"` and fold `[Unreleased]` into `## 0.18.0`. Never bump the patch component. Add the same rule as a plan.md § Impact bullet after the breaking-change bullet. Add a plan.md § Context bullet stating that this branch is stacked on the unmerged `feat/fix-export-parquet-transport-roundtrip`.
- Escalation: MECHANICAL: Cargo's 0.x compatibility rule and the tag state decide the version, so no requester judgment is needed.

#### [NFR_IGNORED] ADVISORY
- Location: plan.md § Impact ("exapump picks up the fixes when it upgrades its exarrow-rs dependency, and it needs arrow 59 to do so."); plan.md § Parallelization
- Issue: The combined scope is coherent as work. Groups A and B are sequenced, and they share only `tests/integration_tests.rs` and `CHANGELOG.md`. As a release, the plan couples the issue #80 fix, which stops silently wrong results, to a breaking Arrow major upgrade. A dependent on arrow 58, such as exapump today, cannot take the #80 fix without moving to arrow 59. The plan states this consequence but keeps no way to ship the fix alone.
- Fix: Add a bullet to plan.md § Parallelization stating that group A lands as its own commit before group B, so a maintainer can cherry-pick it onto an arrow 58 patch release. Add to plan.md § Impact that this plan ships no such backport and that the human decides whether one is needed.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Context ("`parquet` 59.0.0 (2026-06-09) dropped `thrift` and `paste`"); decision-log.md § [11] Consequences ("parquet 59 also drops `paste`"); plan.md task 6.1 (`parquet = { version = "59", features = ["async"] }`)
- Issue: The crates.io index lists `paste ^1.0` as a normal dependency of parquet 59.0.0 and 59.1.0. Release 59.2.0 is the first without it. `thrift` is absent from every 59.x release. The requirement `"59"` admits 59.0.0 and 59.1.0, so removing the RUSTSEC-2024-0436 ignore is safe only while `Cargo.lock` holds parquet 59.2.0 or later. If the lockfile resolves an earlier 59.x, the advisory gate fails on RUSTSEC-2024-0436.
- Fix: Correct both statements to "parquet 59.0.0 dropped `thrift`, and 59.2.0 dropped `paste`". In task 6.1, set `parquet = { version = "59.2", features = ["async"] }`.

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md task 7.2 ("with the number of the follow-up issue"); decision-log.md § [15] Consequences; plan.md § Impact ("The hang is a separate defect for a follow-up issue.")
- Issue: No task creates the follow-up issue, and no actor is named for it. The implementation agents work under read-only git rules, so the implementer may leave the `<follow-up issue>` placeholder in the `#[ignore]` reason or drop the number. Without an issue, nothing tracks the tests that stay out of CI.
- Fix: Add task 7.0 to plan.md, owned by the orchestrator: create the issue with `ghbrk gh issue create --repo exasol-labs/exarrow-rs`, naming the image, the CI flags, the ignored test names, and the result of the cause check in decision [15]. Pass its number to task 7.2. Alternatively, the planner creates the issue now and writes its number into task 7.2 and decision [15].

## Requirement Quality

#### [COMPLETENESS_GAP] BLOCKER
- Location: plan.md task 7.2 ("Remove the bare `#[ignore]` attribute from every test in `tests/import_export_tests.rs`"); plan.md § Verification › Checklist (Import/export test row, "11 ignored with a stated reason") and § Manual Testing (import/export row); decision-log.md § [15]
- Issue: Task 7.2 also removes the bare `#[ignore]` from `test_csv_export_runs_past_the_former_five_minute_limit` (`tests/import_export_tests.rs` line 2733). That test is an eight-minute opt-in check. Its doc comment says it is "Kept `#[ignore]` and gated on `EXARROW_LONG_EXPORT_CHECK` because an eight-minute test has no place in a suite run by default". Without the attribute, CI runs it with the variable unset. The test prints "Skipping" and returns, and cargo counts it as passed. The new scenario lets a test stay out of CI only through `#[ignore = "<reason>"]`, and decision [15] states the same rule, so the task contradicts both. The planning run did not catch this, because the test also returns early under `--include-ignored`. The plan never names the test.
- Fix: In plan.md task 7.2, exclude `test_csv_export_runs_past_the_former_five_minute_limit` from the removal. Replace its bare `#[ignore]` with `#[ignore = "eight-minute opt-in check, run with EXARROW_LONG_EXPORT_CHECK=1"]`, keep its `EXARROW_LONG_EXPORT_CHECK` early return, and rewrite the doc-comment sentence that says `import_export_tests` is not part of the CI job. Name the test in decision [15] Consequences. In the Checklist row, change "11 ignored" to "12 ignored", and extend the Manual Testing row's "eleven native Parquet import tests" to include this test.
- Escalation: MECHANICAL: the task contradicts the plan's own decision [15], which already states the rule to apply.

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: specs/_plans/fix-paged-fetch-position/code-quality/dependencies/spec.md (no block for three recorded scenarios); recorded `specs/code-quality/dependencies/spec.md` § "Advisory suppression documents rationale", § "Patch-level dep bump applied without breaking-change review", § "Minor or major dep bump requires explicit evaluation"; plan.md task 6.3
- Issue: The three recorded scenarios stay unchanged, and each requires "`cargo deny check advisories` MUST exit with code 0". Task 6.3 sets `unused-ignored-advisory = "deny"`, and the CHANGED Background makes `--all-features` the gate. Together they make the default-feature command fail on any suppression for an advisory that only an optional feature pulls in. The first scenario permits such a suppression. Verified with cargo-deny 0.19.6 and a scratch config: an ignore for RUSTSEC-2025-0119, which only `benchmark` reaches, plus `unused-ignored-advisory = "deny"` makes `cargo deny check advisories` print `error[advisory-not-detected]` and exit 1. After the merge, the spec names two advisory commands, and one of them fails in a case the spec allows.
- Fix: In specs/_plans/fix-paged-fetch-position/code-quality/dependencies/spec.md, add one `DELTA:CHANGED` block each for "Advisory suppression documents rationale", "Patch-level dep bump applied without breaking-change review", and "Minor or major dep bump requires explicit evaluation". Copy each scenario from `specs/code-quality/dependencies/spec.md` and replace `cargo deny check advisories` with `cargo deny --all-features check advisories`. Leave `cargo deny check licenses` unchanged.
- Escalation: MECHANICAL: resolved by reading the delta against the recorded spec.

#### [COMPLETENESS_GAP] ADVISORY
- Location: `AGENTS.md` line 34; `CONTRIBUTING.md` lines 37 and 85; `specs/mission.md` line 84 (§ Commands); `tests/common/mod.rs` lines 284-295 (`skip_if_no_exasol!` doc example); `tests/driver_manager_tests.rs` lines 31 and 34; plan.md task 7.2
- Issue: After task 7.2, `cargo test --test import_export_tests -- --ignored` runs only the ignored tests, which are the eleven tests that hang. `AGENTS.md`, `CONTRIBUTING.md`, and `specs/mission.md` still tell agents and contributors to run exactly that command. The `tests/common/mod.rs` doc example pairs `skip_if_no_exasol!()` with a bare `#[ignore]`, which the new lint check rejects. Task 7.2 fixes the same instruction only in the module doc comments of two test files.
- Fix: Extend task 7.2. Change the import/export command in `AGENTS.md`, in both places in `CONTRIBUTING.md`, and in `specs/mission.md` § Commands to `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1`. Rewrite the `tests/common/mod.rs` doc example without `#[ignore]`. Change `-- --ignored` to `-- --include-ignored` in the `tests/driver_manager_tests.rs` module doc comment.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: code-quality/dependencies/spec.md § "Suppression is removed when its advisory no longer applies" ("MUST report no `advisory-not-detected` warning")
- Issue: With `unused-ignored-advisory = "deny"` from task 6.3, cargo-deny 0.19.6 reports a stale ignore as `error[advisory-not-detected]`, not as a warning (verified). The step names a severity that the plan's own configuration never produces, so a literal check of the step passes even when the stale ignore is present.
- Fix: Change the step to "`cargo deny --all-features check advisories` MUST report no `advisory-not-detected` diagnostic for that advisory".

## Task Breakdown

Checked: every new delta has implementing tasks. `code-quality/dependencies` maps to tasks 6.1 to 6.5 and 6.7, and `code-quality/core` maps to tasks 6.4 and 7.1 to 7.5. Group B runs after group A because both edit `tests/integration_tests.rs` and `CHANGELOG.md`, which settles the overlap in their Knowledge columns. Inside group B, the advisory re-check (6.7) and its changelog entry (6.8) run last, after the final lockfile exists. The REMOVED scenario names match the recorded specs, and `speq plan validate fix-paged-fetch-position` passes.

#### [TRACEABILITY_GAP] BLOCKER
- Location: code-quality/core/spec.md § "Every integration test target runs in the CI integration job" (step "a test that needs Exasol MUST fail, not skip, when Exasol is unavailable"); plan.md task 7.2; plan.md § Scenario Coverage (row for that scenario); `tests/driver_manager_tests.rs` lines 87-98
- Issue: `tests/driver_manager_tests.rs` defines its own `skip_if_no_exasol!` macro. That macro prints "Skipping test" and returns whether or not `REQUIRE_EXASOL` is set. The macro in `tests/common/mod.rs` panics instead. CI sets `REQUIRE_EXASOL: "1"` for this target, but the variable has no effect there. The fail-not-skip step of the new scenario is therefore false for this target on the day the plan is recorded. Task 7.2 touches the file only to add an `#[ignore]` reason. The coverage row maps the scenario to the CI `cargo test` steps, which do not check this step.
- Fix: Add to plan.md task 7.2: change the `skip_if_no_exasol!` macro in `tests/driver_manager_tests.rs` to panic with the same message as `tests/common/mod.rs` when `REQUIRE_EXASOL` is set. Name this change in the plan.md § Scenario Coverage row for "Every integration test target runs in the CI integration job".
- Escalation: MECHANICAL: a step of the plan's own delta has no implementing task.

## Design Depth

Checked: no `[ARCHITECTURE_DRIFT]`. The BASE `2078b477` equals `git hash-object specs/architecture.md`. The CHANGED Constraints block copies all twelve current bullets, edits two, and adds one, so no line is dropped. Decisions [11] and [15] name § Constraints, and the delta follows the template's line rules. No `[ADR_CONFLICT]`: decision [11] supersedes ADR-003 by its slug `suppress-ghsa-2f9f-gq7v-9h6m-via-deny-toml`, and `speq decision-log show` lists no other ADR on dependencies, CI gates, or test gating. `scripts/check_ci_test_targets.py` is one script with two path arguments and one exit-code contract, so it raises no depth concern.

#### [ADR_OVERPROMOTION] BLOCKER
- Location: decision-log.md § [11] Decision, Alternatives, Consequences
- Issue: Entry [11] must stay `Promotes to ADR: yes`, because it supersedes ADR-003 (`/speq:adr-rules` rule 5). Its text breaks rule 6, and `/speq:spec-merge` copies Decision, Alternatives, and Consequences into the ADR. Decision pins versions: "exarrow-rs depends on arrow and parquet 59 and on adbc_core, adbc_ffi, and adbc_driver_manager 0.24". Rule 6 says "An ADR names a library but never pins its version." Alternatives also pin versions ("arrow and parquet 60", "adbc_core 0.24 accepts Arrow 58 and 59 only", "parquet 58.4.0"). Consequences hold implementation detail and corollaries, which rule 3 excludes: the locked "arrow 58.3.0" and the "explicit unification step", the `RUSTSEC-2024-0436` ignore removal, and the `specs/mission.md` § Tech Stack edit. These numbers go stale with the next Arrow upgrade, while the ADR stays.
- Fix: Keep `Promotes to ADR: yes`, `Supersedes`, and `Architecture` in entry [11]. Rewrite Decision as policy without version numbers, for example: "exarrow-rs removes the Apache Thrift advisory by upgrading to the first parquet release line without a thrift dependency, together with the arrow and adbc releases that accept it, instead of suppressing the advisory. arrow and parquet stay on one major version that adbc_core accepts." Rewrite Alternatives as version-free one-liners: keep the suppression; move arrow past the range adbc_core accepts (rejected, because adbc_ffi passes Arrow arrays across the C ABI); patch thrift under the current parquet (rejected, because parquet's thrift requirement excludes the fixed release). Reduce Consequences to two bullets: a downstream crate that exchanges Arrow values must move to the same Arrow major version, and an Arrow major upgrade waits until adbc_core accepts it. Move the version numbers, the lockfile unification step, the `paste` ignore removal, and the `specs/mission.md` edit to a new decision-log entry with `Promotes to ADR: no`, or leave them only in plan.md tasks 6.1, 6.3, and 6.6, which already state them.
- Escalation: MECHANICAL: a rule-6 edit to the entry text.

## Prose Quality

No objection, axis checked: plan.md, decision-log.md, architecture.md, and the five spec deltas contain no em or en dashes (grep). The Summary leads with the outcome, and each decision states its Decision first. Names stay consistent: "rows received", "total row count", "integration test target", "advisory gate". Task 7.2 is long, but each sentence makes one claim and names its actor.
