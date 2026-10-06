# Plan Review Findings: fix-pre-1970-date-conversion (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 4, Advisory: 5)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now the plan has failed. Three ways it could happen:

1. Task 4.2 cannot finish as written. The existing unit test `test_format_timestamp_micros_truncates_negative_day_offset` asserts the truncating behavior and fails after the fix. The implementer has two choices: change a test without plan authority, or stop. Routed to Feasibility, `[UNSTATED_ASSUMPTION]`.
2. The fix ships as 0.17.1, not as the minor release the requester asked for. `/speq:implement-pr` step A3 applies the Conventional Commits patch bump to a fix-only plan because no artifact tells it otherwise. The version can also collide with fix-paged-fetch-position. Routed to Feasibility, `[HIDDEN_DEPENDENCY]`.
3. A user with historical data finds two gaps. The recorded spec contradicts itself for `DATE '1500-02-29'`. An imported Date32 value for 1582-10-10 comes back as 1582-10-15, and no document mentions it. Routed to Requirement Quality, `[REQUIREMENT_CONFLICT]` and `[COMPLETENESS_GAP]`.

## Entry [2] Assessment

Decision-log entry [2] (convert by calendar label in the proleptic Gregorian calendar) is neither an Intent Fidelity defect nor a HUMAN escalation.

- The requester chose the oracle. The interview names "chrono ... or a civil-from-days algorithm ... over the full 0001-01-01 to 9999-12-31 range" as the oracle. Both count days in the proleptic Gregorian calendar, and issue #84 proposes `chrono::NaiveDate`. Entry [2] implements that choice. The rejected alternative, Exasol's Julian elapsed-day count, would contradict it.
- The entry keeps the current semantics. The current formula already converts by calendar label, and the plan corrects only its leap-day count. The rejected alternative is the one that would change what users see.
- The entry's facts hold. I re-ran them on `exasol-test` (2026.1.0). `DAYS_BETWEEN(DATE '1970-01-01', DATE '0001-01-01')` returns 719164, where the proleptic count is 719162. `ADD_DAYS(DATE '1582-10-04', 1)` returns 1582-10-15. `DATE '1500-02-29'` is a valid value.
- The human still decides. The entry is an ADR candidate, and recording the plan is its acceptance (`/speq:adr-rules` rule 8). Task 6.1 documents the effects for users.
- The entry passes `/speq:adr-rules`. Its Rationale names criteria 4 and 2 and states the search result (ten ADRs, none about dates or calendars). Its Decision holds no paths or signatures. It is the only `yes` entry.
- Its remaining defects are mechanical. They are the self-contradicting scenario step (Requirement Quality), the missing import-side loss (Requirement Quality), and the Architecture line that contradicts the entry's own criterion 2 (Design Depth).

## Intent Fidelity

[no objection — axis checked: every proposed-fix item of issue #84 and its § Related import defects map to tasks 1.1 to 5.5; the requester's check of negative sub-second values in `ymd_hms_nanos_to_micros` is covered by entry [1] and `test_ymd_hms_nanos_to_micros_before_epoch`; entry [2] follows the requester's own oracle choice, see § Entry [2] Assessment; the minor-bump convention is stated in entry [7], and its execution gap is raised under Feasibility]

## Feasibility

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Implementation Tasks, tasks 4.1 and 4.2; plan.md § Dead Code Removal
- Issue: Task 4.2 ends with "The tests of task 4.1 and the existing tests of the file pass." That is false. `src/import/arrow.rs` holds `test_format_timestamp_micros_truncates_negative_day_offset` (about line 1669), a characterization test of the defect. It asserts `format_timestamp_micros(-1_000_000)` equals `"1970-01-01 00:00:01.000000"`. With `div_euclid` and `rem_euclid` the function returns `1969-12-31 23:59:59.000000`, so the test fails. No task deletes or rewrites it.
- Fix: In plan.md task 4.1, add two cases to `test_format_timestamp_micros_before_epoch`: -1000000 is `1969-12-31 23:59:59.000000`, and -86400000000 is `1969-12-31 00:00:00.000000`. In task 4.2, add: "Delete `test_format_timestamp_micros_truncates_negative_day_offset` from `src/import/arrow.rs`. It asserts the truncating split that this task removes, and task 4.1 covers both of its inputs." Change the last sentence of task 4.2 to "The tests of task 4.1 and the remaining existing tests of the file pass." Add a `Test` row for the deleted test to plan.md § Dead Code Removal.
- Escalation: MECHANICAL. Reading the test file settles it.

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: decision-log.md § Design Decisions [7]; plan.md § Parallelization
- Issue: Entry [7] says "The implement step chooses the release version: the next 0.x minor version ..." and "folds `[Unreleased]` into the matching `## X.Y.Z` header". No artifact gives that instruction to the step that bumps the version. `/speq:implement-pr` step A3 bumps "per the plan's `workspace/version` spec delta if it specifies one. Otherwise apply the conventional next version per Conventional Commits semantics (`feat` → minor bump; a purely `fix` plan → patch)". This plan has no `workspace/version` delta (`speq domain list` shows no `workspace` domain) and is a pure fix, so A3 sets 0.17.1. That contradicts the requester's convention, "the bump to the next 0.x minor version". A3 also does not fold `CHANGELOG.md`. The CI release job takes the release notes from the `## X.Y.Z` section, so the release would have empty notes. The number that entry [7] must avoid also moves. The committed fix-paged-fetch-position plan names 0.18.0, and its uncommitted working copy in the main checkout names 0.17.1.
- Fix: Add a `### Release` subsection to plan.md § Parallelization, addressed to the `/speq:implement-pr` orchestrator: "At step A3, do not apply the Conventional Commits patch default. Set `version` in `Cargo.toml` to X.(Y+1).0. X.Y is the highest of the `vX.Y.Z` tags and of the versions that open pull requests set in `Cargo.toml` or name in a `CHANGELOG.md` header (read each with `ghbrk gh pr diff <number>`). Run `cargo build`. Rename `## [Unreleased]` in `CHANGELOG.md` to `## X.(Y+1).0`, and merge in any `[Unreleased]` entries already on `main`." In decision-log.md entry [7], name step A3 of `/speq:implement-pr` as the actor in the Decision line. Replace "fix-paged-fetch-position claims 0.18.0" with "fix-paged-fetch-position also plans a release, and its version number is not fixed".
- Escalation: MECHANICAL. The requester already set the minor-bump rule. The fix gives the rule an actor.

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md task 2.3; plan.md § Dead Code Removal, `Test` row
- Issue: Task 2.3 deletes "the sections for `ResultSet::parse_date_to_days` and `ResultSet::parse_timestamp_to_micros`" (about lines 1213 to 1390 of `src/query/results.rs`). Seven more tests call the removed functions. Six are in the section "Tests for the parse_date_to_days month table" (about lines 2650 to 2713). The seventh is `test_parse_timestamp_to_micros_time_part_without_a_colon_adds_nothing` (about line 2791), which also uses the constants `SECONDS_PER_DAY` and `MICROS_PER_SECOND` that task 2.2 deletes. The build fails until they are gone. Each has a counterpart in the test module of `src/types/conversion.rs` (about lines 372, 378, 385, 404 to 417, 428, 523, and 578), so no case is lost.
- Fix: In plan.md task 2.3 and in the Dead Code Removal `Test` row, add the section "Tests for the parse_date_to_days month table" and `test_parse_timestamp_to_micros_time_part_without_a_colon_adds_nothing` to the tests to delete.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: specs/_plans/fix-pre-1970-date-conversion/type-mapping/exasol-to-arrow/spec.md § Scenario "DATE and TIMESTAMP values count days in the proleptic Gregorian calendar", lines 27 and 28
- Issue: Line 27 says that a "date before 1582-10-15 ... SHALL convert by its year, month, and day, so that a reader that decodes Date32 in the proleptic Gregorian calendar, such as chrono or pyarrow, shows the same year, month, and day as Exasol". Line 28 says that February 29 of a Julian-only leap year "SHALL convert to the same Date32 value as March 1 of that year". For `DATE '1500-02-29'` chrono shows 1500-03-01. A test of line 27 over the pre-1582 range fails on twelve dates while a test of line 28 passes. The permanent spec would carry two SHALL steps that cannot both hold.
- Fix: In that scenario, change the end of line 27 to "... shows the same year, month, and day as Exasol, except for the dates of the next step".
- Escalation: MECHANICAL. Reading the two steps settles it.

#### [COMPLETENESS_GAP] ADVISORY
- Location: decision-log.md [2] § Consequences; plan.md § Impact; plan.md task 6.1
- Issue: Entry [2] says "Import converts Arrow values back to Exasol text by the same calendar" and that a Date32 value "shows the same date text in Arrow tools as in Exasol". The import side has its own loss, and no artifact names it. The proleptic Gregorian dates 1582-10-05 to 1582-10-14 do not exist in Exasol, and Exasol maps them to 1582-10-15 without an error. On `exasol-test` (2026.1.0), `TO_CHAR(DATE '1582-10-10', 'YYYY-MM-DD')` returns `1582-10-15`. An imported Date32 value for one of these ten days is therefore stored as 1582-10-15. The behavior exists today and the plan does not change it. The new documentation describes only the read side.
- Fix: Add a Consequences bullet to decision-log.md entry [2]: "On import, a Date32 value for 1582-10-05 to 1582-10-14 becomes Exasol text that Exasol stores as 1582-10-15." Add the same sentence to plan.md § Impact and to the description of `docs/type-mapping.md` in task 6.1.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: specs/_plans/fix-pre-1970-date-conversion/type-mapping/boundaries-and-validation/spec.md, Background and § Scenario "Lossless conversion validation" line 20; plan.md task 1.1, `test_ymd_to_days_julian_only_leap_day_reads_as_march_first`
- Issue: The new exception covers "a loss that a `type-mapping` scenario specifies". No test can list which scenarios specify a loss. For example, it is unclear whether "TIMESTAMP with fractional precision 0-9 SHALL map to Arrow Timestamp" in `type-mapping/exasol-to-arrow`, which drops nanoseconds, now counts. The plan needs the exception for one loss only, so the general form weakens the recorded rule for future work with no stated need. The test that covers the step calls `ymd_to_days`, which cannot fail. It therefore does not check "without a warning or an error" on a path that could fail.
- Fix: In the boundaries-and-validation delta, replace "a loss that a `type-mapping` scenario specifies, such as February 29 of a Julian-only leap year in `type-mapping/exasol-to-arrow`," with "February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow` maps to March 1,". Make the same change in the Background. In plan.md task 1.1, add to `test_ymd_to_days_julian_only_leap_day_reads_as_march_first` the assertion that `parse_date_to_days("1500-02-29")` returns `Ok(-171605)`.

## Task Breakdown

[no objection — axis checked: every new and changed scenario has an implementing task and a test in § Scenario Coverage; the one group is justified by shared files (`tests/import_export_tests.rs`) and by task 5.4 needing tasks 1.2 and 4.2; the order 1 to 6 is stated; task 3.2 removes an expectation derived from the code under test, in a file the plan already edits; every literal expected value in tasks 1.1, 2.1, 3.1, 3.2, 4.1, 4.3, and 5.1 to 5.5 matches Python's `datetime`, and the fixed formula matches it on all 3,652,059 days]

## Design Depth

#### [ARCHITECTURE_DRIFT] BLOCKER
- Location: decision-log.md [2] § Rationale and `Architecture:` line; plan.md § Context, last bullet
- Issue: Entry [2] is `Promotes to ADR: yes`, and its Rationale names rule-2 criterion 2: "the rule binds every temporal conversion the driver adds later". Its Architecture line says "no change: the rule lives inside the value conversion of the types and import modules, and no component, boundary, interface, data flow, constraint, or external dependency changes". The reason contradicts the entry. A rule that binds every future temporal conversion is a constraint. It also spans more than types and import: the native transport, query, export, arrow_conversion, and adbc_ffi (parameter binding) all convert DATE or TIMESTAMP values under it. `specs/architecture.md` § Constraints already holds conversion-level constraints, for example "Results stream as Arrow RecordBatches, and conversion is Arrow-native and zero-copy where possible".
- Fix: Create `specs/_plans/fix-pre-1970-date-conversion/architecture.md` per `/speq:plan`'s `references/architecture-delta-template.md`. It holds the H1 `# Architecture Delta: fix-pre-1970-date-conversion`, the BASE comment from `git hash-object specs/architecture.md`, and one `DELTA:CHANGED` block for `## Constraints`. The block copies every current bullet of that section and adds "- DATE and TIMESTAMP values convert between Exasol and Arrow by the year, month, and day that Exasol reports, counted in the proleptic Gregorian calendar, on every read and write path". Set entry [2]'s `Architecture:` line to `§ Constraints`. Replace the last bullet of plan.md § Context with a pointer to the delta. In decision-log.md entry [8], add that fix-paged-fetch-position also changes § Constraints, so the plan recorded second re-bases its block.
- Escalation: MECHANICAL. The entry's own Rationale and `specs/architecture.md` settle it.

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: decision-log.md [3] § Consequences
- Issue: After this plan, the read side has one owner of the calendar rule, `types::conversion`. The write side keeps the rule in six code sites across three files. Arrow timestamps become Exasol text in `format_timestamp_micros` (`src/import/arrow.rs`), `format_timestamp` (`src/import/parquet.rs`), and the Timestamp arm of the FFI parameter binding (`src/adbc_ffi.rs`). Date32 values become text in `days_to_ymd` (`src/import/arrow.rs`) and through chrono in `src/import/parquet.rs` and `src/adbc_ffi.rs`. Issue #84 exists because two copies of one rule drifted apart. Entry [3] keeps the copies and schedules no follow-up.
- Fix: In decision-log.md entry [3] § Consequences, add a follow-up bullet: open a GitHub issue that moves Arrow-to-Exasol date and timestamp formatting into `types::conversion` with one error contract, and name that issue in the bullet.

Also checked, no finding: `[ADR_OVERPROMOTION]` (entry [2] passes rules 1 to 6, see § Entry [2] Assessment); `[ADR_CONFLICT]` (none of the ten accepted ADRs from `speq decision-log show` covers dates, timestamps, calendars, or release versions); `[SHALLOW_DESIGN]` and `[BOUNDARY_VIOLATION]` (the plan adds no module or interface, and deleting the five WebSocket parsers deepens `types::conversion`).

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: specs/_plans/fix-pre-1970-date-conversion/import-export/arrow-recordbatch/spec.md § Scenario "RecordBatch import converts pre-epoch timestamps of every time unit to the earlier instant"; specs/_plans/fix-pre-1970-date-conversion/import-export/parquet-io/spec.md § Scenario "CSV-path Parquet import converts pre-epoch timestamps of every time unit to the earlier instant"; plan.md § Impact; decision-log.md [3] § Rationale
- Issue: "converts ... to the earlier instant" reads as if every value moves to an earlier time. Only the sub-microsecond digits of a nanosecond value round down. Second, Millisecond, and Microsecond values convert exactly. "drops them toward the earlier instant" in plan.md § Impact and entry [3] has the same problem.
- Fix: Rename both scenarios to end "... formats pre-epoch timestamps of every time unit as times before the epoch". Update the `/// Scenario:` lines in tasks 4.1 and 4.3 and the rows of § Scenario Coverage to the new titles. In plan.md § Impact and decision-log.md entry [3], replace "drops them toward the earlier instant" with "rounds them down to the microsecond".
