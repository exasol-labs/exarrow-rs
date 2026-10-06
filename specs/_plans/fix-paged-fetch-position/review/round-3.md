# Plan Review Findings: fix-paged-fetch-position (round 3)

## Summary
- Axes checked: 6/6
- Total findings: 7 (Blockers: 2, Advisory: 5)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Round-2 Blocker Recheck

- Resolved: [UNSTATED_ASSUMPTION] The native Parquet import hang was attributed without a cause check. Decision [15] records the five-configuration cause check. Decisions [19] and [20] name both causes: Exasol 2025.2.0 rejects HTTP Parquet sources with `ETL-2210`, and the driver waits on the tunnel task after a failed IMPORT statement. Task 7.2 leaves no native Parquet test ignored, and the Checklist row expects 1 ignored test.
- Resolved: [NFR_IGNORED] The release version of the breaking Arrow upgrade was left open. Decision [10] and plan.md § Impact state the 0.x minor rule and 0.18.0. `v0.17.0` resolves to `f9388a1`, which is `origin/main`, as plan.md § Context states. A residual risk remains: no pipeline step reads the rule. It is part of the new `[HIDDEN_DEPENDENCY]` blocker below.
- Resolved: [COMPLETENESS_GAP] The eight-minute opt-in export test lost its `#[ignore]`. Task 7.2 gives it a reasoned `#[ignore = "..."]`, keeps its early return, and rewrites its doc sentence. Decision [15] names it.
- Resolved: [REQUIREMENT_CONFLICT] Three unchanged advisory scenarios named the default-feature command. The `code-quality/dependencies` delta has a `DELTA:CHANGED` block for each of the three, and each names `cargo deny --all-features check advisories`.
- Resolved: [TRACEABILITY_GAP] `driver_manager_tests` skipped instead of failing under `REQUIRE_EXASOL`. Task 7.2 changes the macro at `tests/driver_manager_tests.rs` lines 87-98, and the Scenario Coverage row names both macros.
- Resolved: [ADR_OVERPROMOTION] The ADR candidate pinned versions. Entry [11] has a version-free Decision and Alternatives and two Consequences bullets. Entry [18], which is not an ADR candidate, holds the versions.

## Premortem

Six months from now this plan failed. Three ways it could happen:

1. PR #85 is squash-merged, as every recent PR on `main` is. The A, C, B commit boundaries never reach `main`. The pipeline also made one commit, the code-review fixes to `src/query/results.rs` landed after group B, and step A3 bumped the version to 0.17.1 because the plan is named `fix-...`. When exapump needs the issue #80 fix on arrow 58, no clean commit exists. Routed to `[HIDDEN_DEPENDENCY]` BLOCKER.
2. A service runs multi-file Parquet imports in a loop, and some target tables are missing. Each failed import now returns its error, but its per-connection tunnel tasks were detached rather than aborted. Task 8.2 had no test, so the tests passed without it. Each task holds a socket that Exasol keeps open, and the process runs out of file descriptors. Routed to `[TRACEABILITY_GAP]` BLOCKER.
3. A user on Exasol 2025.2.0 upgrades. Parquet import now fails by default with `ETL-2210`. The spec library still says native Parquet import "is available from Exasol 2025.1.11 onward", and the requester expected 2025.2.0 support to be dropped outright, not only for one feature. Routed to `[INTENT_DRIFT]` ADVISORY and `[REQUIREMENT_CONFLICT]` ADVISORY.

## Intent Fidelity

Checked: each scope addition and user decision maps to tasks.
- Issue #80: tasks 1.x to 5.1.
- Advisories: tasks 6.1 to 6.8.
- New tests in CI: group A's tests run in `integration_tests` and `websocket_integration_tests`. Task 7.7 adds the WebSocket unit tests. Task 7.1 adds `import_export_tests`, which holds the group C tests.
- Root cause instead of ignoring tests: decisions [19] and [20] give the root cause. Task 7.2 ignores no native Parquet test.
- 2025.2.0 removal: tasks 8.6, 8.7, and 8.9 and the architecture delta cover every reference outside the plan directory: `.github/workflows/ci.yml:237`, `scripts/run_all_tests.sh:10,14`, `specs/architecture.md:82`, `src/connection/session.rs:556`, `src/connection/version.rs:49,82`, and the recorded scenario "Native Parquet import threshold".
- Decision [4] (mismatch error) and decision [11] (arrow 59) are unchanged. How the plan carries out the group A commit decision is challenged under Feasibility.

#### [INTENT_DRIFT] ADVISORY
- Location: plan.md § Summary ("Exasol 2025.2.0 ... is not supported for native Parquet import"); plan.md § Impact; plan.md tasks 8.6, 8.8, and 8.10; decision-log.md [19]
- Issue: The requester asked to "drop 2025.2.0 support entirely, pinning no tests or code to outdated Exasol versions". The plan narrows this to "not supported for native Parquet import". That wording is accurate: only native Parquet import fails on 2025.2.0, and the repository states no supported-version range. The plan also writes the version into code. Task 8.6 puts "Exasol 2025.2.0 rejects HTTP Parquet sources with `ETL-2210` and is not supported for native Parquet import" into the doc comment of `supports_native_parquet_import` in `src/connection/version.rs`. Decision [19] says that tests, scripts, CI, and specs no longer name 2025.2.0, but it is silent on source code.
- Fix: In task 8.6, keep the doc comment of `supports_native_parquet_import` free of 2025.2.0. Replace the accepted example `(2025, 2, 0)` with `(2025, 2, 1)`. Point to `docs/import-export.md` § Native Parquet Import for server releases that reject HTTP Parquet sources. In decision [19] Decision, add one sentence that defines the term: dropping 2025.2.0 support means that no code, test, script, CI step, or spec names it, and only `docs/import-export.md` and `CHANGELOG.md` state the native Parquet import limitation. Put the same sentence in the PR description so that the requester can confirm the interpretation.

## Feasibility

Checked and confirmed:
- The five call sites of task 8.3 exist: `src/import/csv.rs:567`, and `src/import/parquet.rs:356`, `:400`, `:625`, and `:671`. These are all the callers of `resolve_stream_task`.
- The single-file CSV path uses `tokio::select!` at `src/import/csv.rs:821`. Arrow import reaches that path through `import_from_stream`, so the five paths are complete.
- `FakeExasolServer::silent_after_handshake` exists at `src/transport/test_support.rs:186`. It accepts one connection, which is enough for the `serve_parquet_bytes` unit test.
- CSV export already aborts its tunnel task after a failed EXPORT statement (`src/export/csv.rs:571-573`), so task 8.1 follows a pattern that the crate already uses.
- Task 8.1 aborts the tunnel task only after the IMPORT statement has returned. It abandons no in-flight query, so it does not trigger the ADR `client-give-up-terminates-connection`.
- `ParquetImportOptions` defaults to `use_tls: false` (`src/import/parquet.rs:99`), so the default native import sends the `http://` URL that 2025.2.0 rejects.

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: plan.md § Parallelization (bullet "Commits: group A lands first as its own commit ..."); decision-log.md [22]; decision-log.md [10]; plan.md § Impact (bullet "Release version")
- Issue: Nothing in the plan or the pipeline produces the commit order A, C, B or the version 0.18.0. No actor is named for either. Four facts outside the plan work against them.
  - `/speq:implement-pr` step A4 makes one commit after `/speq:implement` finishes. That commit holds "implementation files, version bump, specs/_plans/<plan-name>/". Step 6 of the user command `~/.claude/commands/speq-implement-pr.md` also commits once. The implementer agents work under read-only git rules. No step commits between groups. `/speq:implement` Phase 4 also states that "Implementation work is uncommitted at this point".
  - `/speq:implement` runs `code-reviewer` once, after every group is done. Fixes to group A's files therefore land after groups C and B, and a cherry-pick of the group A commit would miss them.
  - `main` squash-merges every PR. `f9388a1` (#83), `c0de678` (#82), `6050ceb` (#79), and `122dec2` (#61) each have one parent. A squash merge of PR #85 removes the group boundaries from `main`, so the promised commit would exist only on the feature branch.
  - Without a `workspace/version` spec delta, step A3 bumps by Conventional Commits ("a purely `fix` plan → patch"). This project has no `workspace/version` feature, so the 0.18.0 rule of decision [10] depends on the orchestrator reading plan prose.
  - With all four in place, user decision (3), "group A lands as its own backportable commit (order A, C, B)", fails without any error.
- Fix: Add a `### Commits` subsection to plan.md § Parallelization that names the actor and each checkpoint:
  - (1) The `/speq:implement-pr` orchestrator is the only actor with git write authority, and implementer agents never commit. It commits after group A's tasks and unit tests pass and before group C starts, then again after group C. It commits group B at step A4.
  - (2) The code-review base stays the commit before implementation, so the review still covers the committed groups.
  - (3) Code-review fixes to a group's files go into a separate commit whose subject names the group.
  - (4) Step A3 sets `version = "0.18.0"` and folds `[Unreleased]` into `## 0.18.0`, per decision [10] and not by Conventional Commits. This change goes in the group B commit.
  - (5) The PR description lists the backport set by SHA: the group A commit and its review-fix commit, and optionally the group C commits. It also states that a squash merge drops the group boundaries from `main`, so a maintainer who wants a backport keeps the feature branch or merges without squashing.
  - Update decision [22] Consequences to name the same actor and to state the squash-merge caveat.
- Escalation: MECHANICAL. The single commit step, the read-only implementers, the review timing, and the squash-merge history are all in the skill files and `git log`. The Fix names an actor and checkpoints and leaves the backport release to a human, as decision [22] already does.

## Requirement Quality

Checked: the deltas are testable, and `speq plan validate fix-paged-fetch-position` passes. Its one warning is about a REMOVED scenario. The new http-transport scenario does not conflict with the recorded `csv-import` scenarios "Import failure cleanup" and "Stream error aborts import immediately", or with the http-transport scenario "Import data flow". The `version-capability` delta changes only the boundary case `(2025,2,0)` to `(2025,2,1)`, and the gate it describes is unchanged.

#### [COMPLETENESS_GAP] ADVISORY
- Location: import-export/http-transport/spec.md § "Failed IMPORT statement returns its error without waiting for the tunnel" (last step, "as it does today"); decision-log.md [20] Consequences ("A tunnel task's protocol error still takes precedence over the statement's error, as before"); plan.md tasks 8.1 and 8.4
- Issue: The plan describes the error precedence as unchanged, but it changes. Today all five paths await the tunnel task after the statement, so a tunnel error wins even when it happens after the statement returned. Task 8.1 returns the task's error only if the task finished before the statement returned. Otherwise it aborts the task and returns the statement's error. Example: Exasol requests data, rejects the statement partway through (a reject limit or a type error), and closes the tunnel. Today the caller gets `ParallelImportError("Stream 0 failed: ...")`. After the change, the caller usually gets `SqlError`, depending on timing. The new behavior is better, because the statement's error explains the failure. However, "as before" and "as it does today" misstate it, and no test covers this case.
- Fix: Reword the delta's last step to "when a tunnel task has failed before the IMPORT statement returns its error, the system SHALL return that task's error". In decision [20] Consequences, state that a tunnel task still running when the statement fails is aborted and the statement's error is returned. Add a fourth case to task 8.4: a failed statement, with a tunnel task that is still running and fails later, returns `ImportError::SqlError`.

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: recorded `specs/connection-management/version-capability/spec.md` § Background ("Native Parquet import is available from Exasol 2025.1.11 onward"); recorded `specs/import-export/parquet-io/spec.md` § Background ("On Exasol 2025.1.11 and newer the server requests the Parquet file from the driver using HTTP range requests"); decision-log.md [19] (no `Architecture:` line)
- Issue: Decision [19] shows that Exasol 2025.2.0 rejects HTTP Parquet sources. Both Backgrounds stay unchanged, and both state availability from 2025.1.11 onward with no exception, which the plan has disproved. The architecture bullet "Native Parquet import requires Exasol 2025.1.11 or later" uses "requires", so it stays true. Decision [19] puts the limitation only in `docs/import-export.md`, which matches the request not to name 2025.2.0. Decision [19] states no `Architecture:` line for that choice. This is ADVISORY because no scenario step depends on the availability claim.
- Fix: In `specs/_plans/fix-paged-fetch-position/connection-management/version-capability/spec.md`, add a `DELTA:CHANGED` Background that replaces "Native Parquet import is available from Exasol 2025.1.11 onward" with "The driver selects native Parquet import from Exasol 2025.1.11 onward", and keep the rest of the Background. Add a `parquet-io` delta whose `DELTA:CHANGED` Background replaces "On Exasol 2025.1.11 and newer the server requests" with "On Exasol 2025.1.11 and newer the driver selects native Parquet import, and the server requests". Add `Architecture: no change: § Constraints states 2025.1.11 as a requirement, which stays true, and the server-side limitation lives in docs/import-export.md` to decision [19].

## Task Breakdown

Checked:
- The http-transport delta maps to tasks 8.1 to 8.5, and the version-capability delta maps to task 8.6.
- Groups run in sequence A, C, B, and the plan claims no parallelism.
- Groups C and B share `.github/workflows/ci.yml`, `scripts/run_all_tests.sh`, `tests/import_export_tests.rs`, and `CHANGELOG.md`. The sequence settles this overlap, and the user chose the commit order.
- Group A has no dependency on arrow 59 or on the later groups. Its base `v0.17.0` is the branch's own base, so its changes apply to unchanged files.

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md task 8.2; plan.md task 8.4; plan.md § Scenario Coverage (rows for "Failed IMPORT statement returns its error without waiting for the tunnel"); import-export/http-transport/spec.md step "the system SHALL stop serving the tunnel connections"; plan.md § Parallelization, group C Knowledge (`src/import/parquet.rs` (... `stream_parquet_files_parallel`))
- Issue: Task 8.2 is the only task that stops the per-connection tunnel tasks of the multi-file paths, and no test checks it.
  - The unit tests of task 8.4 use a synthetic tunnel task, and its `serve_parquet_bytes` test covers a path with one connection. The integration tests of task 8.5 only check that the error returns within 60 seconds.
  - If task 8.2 is skipped or done wrong, every planned test still passes. Task 8.1 aborts the parent task. `join_stream_handles` then drops its `Vec<JoinHandle>`, and tokio detaches a task when its `JoinHandle` drops. It does not cancel the task. The per-connection tasks keep their `HttpTransportClient` sockets open and wait for a GET that never comes. The plan's own root cause states that Exasol keeps the tunnel socket open. A long-running caller therefore leaks N sockets and N tasks per failed multi-file import.
  - No planned test checks the delta step "the system SHALL stop serving the tunnel connections" on any path.
  - Task 8.2 offers "a `tokio::task::JoinSet`" and also requires that "the existing `test_join_stream_handles_*` tests stay unchanged and pass". Those three tests (`src/import/parallel.rs` lines 660, 673, and 688) pass a `Vec<JoinHandle>` to `join_stream_handles`. A `JoinSet` changes that signature, so it breaks the tests. Only an abort-on-drop guard inside `join_stream_handles` meets both requirements.
  - `stream_parquet_files_parallel` is in `src/import/parallel.rs` (line 322), not in `src/import/parquet.rs` as task 8.2 and the group C Knowledge column say.
- Fix: Rewrite task 8.2 as follows. In `join_stream_handles` (`src/import/parallel.rs`), hold the handles in a guard that aborts every handle not yet joined when the guard is dropped. The guard stops the per-connection tasks of `stream_files_parallel` and `stream_parquet_files_parallel`, both in `src/import/parallel.rs`, when their parent task is aborted. It also stops the remaining tasks after the first failure, as the recorded scenario "Fail-fast on streaming error" requires. Keep the failing index in the errors and the existing tests unchanged. Remove the `JoinSet` option.

  Then extend task 8.4:
  - Add a unit test in `src/import/parallel.rs`. It spawns a parent task that calls `join_stream_handles` on two child tasks. Each child holds a `tokio::sync::oneshot::Sender` and never finishes. The test aborts the parent and asserts that both receivers return `RecvError` within 5 seconds.
  - Extend the `serve_parquet_bytes` unit test so that it also asserts that the fake server's connection reaches end of stream within 5 seconds. For this, add a `FakeExasolServer` method that waits for its peer task.

  Add both tests to the Scenario Coverage row. Correct the file of `stream_parquet_files_parallel` in task 8.2 and in the group C Knowledge column.
- Escalation: MECHANICAL. Tokio's drop semantics, the existing test signatures, and the function's location are all in the code. The Fix adds tests and removes one option.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md task 8.5; plan.md § Verification › Manual Testing (import-export/http-transport row, "The four missing-table tests"); plan.md § Parallelization ("Order inside group C"); plan.md task 8.9
- Issue: Task 8.5 says it adds one test for each path that task 8.3 changes. Three details weaken that mapping, and one comment goes stale.
  - The two native-path tests, `test_parquet_import_into_missing_table_returns_error` and `test_parallel_parquet_import_into_missing_table_returns_error`, set no `with_native_parquet` override. They use the native path only because the CI image reports 2025.2.1. Against a server below 2025.1.11 they pass through the CSV path, and the native paths go untested without any failure.
  - Task 8.5 adds five tests, but the Manual Testing row expects four.
  - "Order inside group C" runs 8.4 and 8.5 after 8.1 to 8.3. The 8.5 tests reproduce the hang, so failing-test-first (`/speq:code-guardrails`) puts them before the fix. Without the fix they fail at the 60-second timeout.
  - Task 8.9 updates the doc comment of `test_parquet_import_native_path_when_supported` only. `test_parquet_stream_import_native_path` (line 2573) and `test_parallel_parquet_import_native_path` (line 2621) also say "Skips gracefully on older server versions.", which becomes false.
- Fix: In task 8.5, give the two native-path tests `with_native_parquet(Some(true))`. In the Manual Testing row, change "four" to "five". In "Order inside group C", run 8.5 and the tests of 8.4 first, then 8.1 and 8.2, then 8.3, then 8.6, 8.7, and 8.9, then 8.10 and 8.8. In task 8.9, update the doc comments of all three tests.

## Design Depth

No objection, axis checked:
- Task 8.1's function is a deep module. One function replaces the code at five call sites and hides the abort and the error precedence behind one call. `resolve_stream_task` becomes its internal helper.
- `[ADR_OVERPROMOTION]`: entry [11] is the only `Promotes to ADR: yes` entry. It names criterion 3, states the search result, and holds no versions.
- `[ARCHITECTURE_DRIFT]`: decision [11] names § Constraints, [15] § Constraints, [20] § Data Flow, and [21] § Constraints, and the delta has each section. The BASE `2078b477` equals `git hash-object specs/architecture.md`. The Constraints block keeps all twelve current bullets. Decision [19]'s missing `Architecture:` line is raised under Requirement Quality.
- `[ADR_CONFLICT]`: `speq decision-log show` lists no conflicting ADR. Task 8.1 aborts the tunnel task only after the statement returns, so `client-give-up-terminates-connection` does not apply. `export-timer-stays-opt-in` covers export only. Entry [11] supersedes ADR-003 by its slug.

## Prose Quality

Checked: no em or en dashes in plan.md, decision-log.md, architecture.md, or the seven spec deltas (grep).

#### [PROSE_BLOAT] ADVISORY
- Location: decision-log.md [7] Rationale; decision-log.md [10] Decision; decision-log.md § Review Findings [2], [3], [7], and [8]
- Issue: Several statements no longer describe the current plan, which breaks writing rule 8 (state the current result).
  - [7]: "the default 64 MiB data message size, which the CI image `exasol/docker-db:2025.2.0` uses". CI uses 2025.2.1 (entry [21]).
  - [10]: the branch "If tag `v0.17.0` does not exist when this PR merges ..." can no longer apply. The tag exists, as [10]'s own Rationale states.
  - Review finding [2]: "§ Context states that this branch is stacked on the unmerged `feat/fix-export-parquet-transport-roundtrip`". § Context now says that the branch is based on `main`.
  - Review finding [3]: "The Checklist row expects 12 ignored tests". The row now expects 1.
  - Review finding [7]: "Task 7.0 covers the follow-up issue finding". Review finding [8] removed task 7.0.
  - Review finding [8]: "New group C (tasks 8.1 to 8.8) ... excludes 2025.2.0 from native Parquet import". Group C is tasks 8.1 to 8.10, and review finding [9] replaced the exclusion.
- Fix: In [7], replace "which the CI image `exasol/docker-db:2025.2.0` uses" with "the server default". In [10] Decision, delete the untagged-`v0.17.0` branch and state 0.18.0. In review findings [2], [3], [7], and [8], correct each statement to the current plan, or end it with "(superseded by review finding [9])".
