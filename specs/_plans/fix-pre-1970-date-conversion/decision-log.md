# Decision Log: fix-pre-1970-date-conversion

## Interview

This plan ran in headless mode. No live interview took place. The orchestrator passed the following intent and guidance.

**Q:** What should the plan do?
**A:** Plan the fix for GitHub issue exasol-labs/exarrow-rs#84, "Pre-1970 DATE and TIMESTAMP values are converted to the wrong day". `SELECT DATE '1968-01-01'` returns Date32 -730 (1968-01-02) instead of -731, and `TIMESTAMP '1950-06-15 00:00:00'` comes back as 1950-06-16. Both transports are affected. The root cause is the days-from-year formula, which uses Rust's truncating integer `/` where years before 1970 need floor division. Exasol's DATE range is 0001-01-01 to 9999-12-31.

**Q:** Which design does the requester prefer?
**A:** One shared pure function with floor (Euclidean) arithmetic. The WebSocket copy in `src/query/results.rs` is deleted and calls the shared function. Validate the design against the code, and check for other copies, including import, export, Parquet, the Arrow converters, the reverse direction (days to date), and negative sub-second values in `ymd_hms_nanos_to_micros`.

**Q:** How should the fix be tested?
**A:** Compare against an independent oracle, such as a civil-from-days algorithm or the chrono crate if it is already a dependency, over the full 0001-01-01 to 9999-12-31 range. Include the issue's examples (1968-01-01 is -731, the 1950-06-15 timestamp), year 1, year 9999, and the 1900 and 2000 leap-year edge cases, on both the native binary path and the WebSocket string path. Add an integration test against Docker Exasol if the repository's test structure makes it cheap.

**Q:** Which project rules apply to the release?
**A:** `AGENTS.md` and `specs/mission.md` apply: tests fail rather than skip, a user-facing fix gets a `CHANGELOG.md` entry under `## [Unreleased]`, and the bump to the next 0.x minor version is decided in this log per the existing convention. The plan fix-paged-fetch-position is in progress on another branch and plans release 0.18.0. This plan is independent of it, must not assume its version number, and must not conflict with it.

## Design Decisions

### [1] One day-count function with floor division serves every read path

- **Decision:** `ymd_to_days` in `src/types/conversion.rs` stays the only function that turns a year, month, and day into a Date32 day count. Its three leap-year terms use `i32::div_euclid` instead of `/`. `ResultSet` in `src/query/results.rs` drops its private date and timestamp parsers and calls `types::conversion::parse_date_to_days` and `types::conversion::parse_timestamp_to_micros`. It keeps its NULL-on-failure behavior by calling `.ok()` on their results.
- **Alternatives:** (a) Replace the formula with `chrono::NaiveDate::from_ymd_opt`. Rejected: chrono rejects day numbers that the current parsers accept, such as `2024-02-30` and Exasol's `1500-02-29` (entry [2]), so every caller would need a new error path, and the native decoder would validate every value. (b) Fix both copies in place. Rejected: the two copies already carried the same defect, and keeping both keeps two places that must agree.
- **Rationale:** With floor division the formula is exact for every integer year. Each floor term counts the leap years between 1970 and the target year, also when that count is negative. A check during planning compared the fixed formula with Python's proleptic Gregorian `datetime.date` on all 3,652,059 days from 0001-01-01 to 9999-12-31 and found no difference. The current formula differs on 487,680 of those days. The parsers in `types::conversion` and the private copies in `ResultSet` accept the same inputs: both check the month range 1 to 12 and the day range 1 to 31, both read `HH:MM` without seconds as a valid time, and both pad short fractions to six digits. The WebSocket path therefore changes only in the corrected values. `ymd_hms_nanos_to_micros` and `parse_timestamp_to_micros` add a non-negative time of day to the day count, so they are correct once the day count is.
- **Consequences:**
  - One fix reaches the native decoder (`read_packed_date_days`, `read_timestamp_micros`), the WebSocket query results, the public `ArrowConverter`, and the typed CSV conversions of `export_to_parquet`, `export_to_record_batches`, `export_to_arrow_ipc`, and the CSV-bytes Parquet entry points.
  - `src/query/results.rs` loses five functions and five time constants. Its parser unit tests are removed. The one case that `src/types/conversion.rs` does not cover, a time of day without seconds, moves there.
  - The decision of how Exasol date text becomes a day count now has one owner, `types::conversion`.
  - The native decoder calls `ymd_to_days` once per DATE and TIMESTAMP value. `div_euclid` by a constant adds a sign correction to the division, so the per-value cost stays a few integer operations, and the decoder still allocates nothing per value (`native-client/zero-copy-fetch`).
- **Promotes to ADR:** no

### [2] DATE and TIMESTAMP values convert by their calendar label in the proleptic Gregorian calendar

- **Decision:** The driver converts a DATE or TIMESTAMP value by the year, month, and day that Exasol reports, counted in the proleptic Gregorian calendar for every year. Import converts Arrow values back to Exasol text by the same calendar. A Date32 value therefore shows the same date text in Arrow tools as in Exasol, also before 1582-10-15, where Exasol labels dates in the Julian calendar.
- **Alternatives:** (a) Convert by the elapsed day count that Exasol's own date arithmetic uses, which counts Julian days before 1582-10-15. Rejected: chrono, pyarrow, and Parquet readers decode Date32 in the proleptic Gregorian calendar, so `DATE '0001-01-01'` would display as 0000-12-30, and a pre-1582 date would display up to 10 days away from its Exasol text. The import paths and parameter binding would also need a Julian conversion to keep round trips intact. (b) Return an error for a Julian-only leap day such as `1500-02-29`. Rejected: one stored value would make a whole result unreadable.
- **Rationale:** `/speq:adr-rules` rule 2, criterion 4: the decision rejects a plausible alternative, and the reason matters to a reader who compares the driver's Date32 values with Exasol's `DAYS_BETWEEN` or with a JDBC client. Criterion 2 also applies: the rule binds every temporal conversion the driver adds later. Search: `speq decision-log show` lists ten ADRs, and none covers dates, timestamps, or calendars. Evidence from `exasol/docker-db` 2026.1.0 during planning: `DAYS_BETWEEN(DATE '0001-01-01', DATE '1970-01-01')` returns -719164, where the proleptic Gregorian count is -719162. `TO_CHAR(ADD_DAYS(DATE '1582-10-04', 1), 'YYYY-MM-DD')` returns 1582-10-15. `TO_CHAR(ADD_DAYS(DATE '1500-02-28', 1), 'YYYY-MM-DD')` returns 1500-02-29. Issue #84 states its expected values and suggests `chrono::NaiveDate`, both proleptic Gregorian.
- **Consequences:**
  - A day difference across 1582-10-15 that a caller computes from Date32 values differs from Exasol's `DAYS_BETWEEN`.
  - February 29 of the years 100, 200, 300, 500, 600, 700, 900, 1000, 1100, 1300, 1400, and 1500 converts to the Date32 value of March 1 of the same year. These twelve Exasol days share a Date32 value with March 1. The current formula already maps them this way, one day off.
  - That loss contradicts the recorded rule "it SHALL warn or error on lossy conversions" in `type-mapping/boundaries-and-validation`. The driver has no warning channel for result values, and an error is rejected above, so the plan's delta for that feature adds an exception for February 29 of a Julian-only leap year.
  - On import, a Date32 value for 1582-10-05 to 1582-10-14 becomes Exasol text that Exasol stores as 1582-10-15. Exasol's `TO_DATE` and `CAST` map these ten dates to 1582-10-15 without an error (verified on `exasol/docker-db` 2026.1.0). The behavior exists before this plan, and the plan does not change it.
  - `docs/type-mapping.md` states these effects.
- **Architecture:** § Constraints
- **Promotes to ADR:** yes

### [3] The import formatters split timestamps with Euclidean division in place

- **Decision:** `format_timestamp_micros` in `src/import/arrow.rs` splits microseconds into days, seconds of the day, and the microsecond fraction with `div_euclid` and `rem_euclid`. `format_timestamp` in `src/import/parquet.rs` splits microseconds into seconds and the microsecond fraction the same way. Both convert nanoseconds to microseconds with `div_euclid(1_000)`.
- **Alternatives:** (a) Merge the two import formatters and the FFI parameter formatter into one shared function. Rejected for this plan: their error contracts differ (`import::arrow` returns a `String`, `import::parquet` returns a `Result` that rejects values chrono cannot represent), so a merge changes behavior beyond the defect, and each formatter is correct after this change. (b) Use `chrono::DateTime::from_timestamp_micros` in `import::arrow`. Rejected: it adds a failure path to an infallible function.
- **Rationale:** Issue #84 § Related names the truncating split as the import defect, and Date32 import is already correct (`days_to_ymd` in `src/import/arrow.rs` uses floor arithmetic, and `src/import/parquet.rs` uses chrono). Floor division keeps the time of day between 00:00:00 and 23:59:59.999999 for a pre-epoch value. With the nanosecond floor, a pre-epoch value with sub-microsecond digits rounds down to the microsecond, as a positive value already does, so the CSV text holds the first six fractional digits of the value. The FFI parameter binding in `src/adbc_ffi.rs` already uses `div_euclid` and `rem_euclid` with chrono, and stays unchanged.
- **Consequences:**
  - The write side keeps the calendar rule in six code sites across three files. Arrow timestamps become Exasol text in `format_timestamp_micros` (`src/import/arrow.rs`), `format_timestamp` (`src/import/parquet.rs`), and the Timestamp arm of the parameter binding (`src/adbc_ffi.rs`). Date32 values become text in `days_to_ymd` (`src/import/arrow.rs`) and through chrono in `src/import/parquet.rs` and `src/adbc_ffi.rs`. This duplication exists before this plan, and this plan does not add to it. A later change to the Exasol date or timestamp text form edits all six sites.
  - Follow-up: a GitHub issue titled "Move Arrow-to-Exasol date and timestamp formatting into types::conversion with one error contract" covers merging the six sites. The planner did not open it, because creating a GitHub issue is a remote write that the planning task does not authorize. The orchestrator or a maintainer opens it and replaces this sentence with its number. Open issue #38 also concerns Arrow timestamps serialized as Exasol text, so the new issue should link it.
- **Promotes to ADR:** no

### [4] chrono and literal values are the test oracles

- **Decision:** Unit tests compare `ymd_to_days` with `chrono::NaiveDate` for every day from 0001-01-01 to 9999-12-31, and `parse_timestamp_to_micros` with `chrono::NaiveDateTime` for one timestamp in every year from 1 to 9999. Integration tests assert literal expected values, which Python's `datetime` module computed during planning, and read Exasol's `TO_CHAR` text to check what an import stored.
- **Alternatives:** (a) Use Exasol's `DAYS_BETWEEN` as the integration oracle. Rejected: it counts Julian days before 1582-10-15 and so disagrees with entry [2] for `DATE '0001-01-01'`. (b) Compare `ymd_to_days` with `days_to_ymd` in `src/import/arrow.rs`. Rejected: both are crate code, so a shared misconception would pass.
- **Rationale:** chrono 0.4 is already a regular dependency (`Cargo.toml`, "Date/time formatting for import/export"), so the oracle adds no dependency. The existing unit tests check dates from 1969 onward only, which the defective formula gets right. `TO_CHAR` text comes from the server, so it checks an import independently of the driver's own date decoding.
- **Consequences:** The full-range unit test runs 3,652,059 iterations in `cargo test --lib`.
- **Promotes to ADR:** no

### [5] Spec placement: one cross-transport value rule, plus one scenario per import and export entry point

- **Decision:** The query-result scenarios go into `type-mapping/exasol-to-arrow`, which both transports implement. The export scenario goes into `import-export/parquet-export`. The import scenarios go into `import-export/arrow-recordbatch` and `import-export/parquet-io`. `type-mapping/boundaries-and-validation` gets the exception of entry [2]. `native-client/type-conversion`, `native-client/zero-copy-fetch`, `arrow-conversion/type-converters`, and `type-mapping/arrow-to-exasol` get no delta.
- **Alternatives:** (a) One scenario per transport in `native-client/type-conversion` and the WebSocket specs. Rejected: the defect lives in one shared function, so per-transport scenarios would repeat one rule. (b) Put the import scenarios into `type-mapping/arrow-to-exasol`. Rejected: that feature describes parameter binding and DDL generation, and this plan changes neither.
- **Rationale:** Each delta owns the code it governs: the shared conversion for the read paths, and one import formatter each for the two import features. `export_to_record_batches` is checked through the round-trip scenario, `export_to_parquet` through its own scenario.
- **Consequences:** ADBC bulk ingestion calls `import_from_record_batch`, and `export_to_arrow_ipc` and `ArrowConverter` call the shared conversion, so the scenarios cover them through shared code. They get no separate integration test, because driver manager tests need the release cdylib and these paths add no conversion code of their own.
- **Promotes to ADR:** no

### [6] New tests fail when Exasol is unavailable

- **Decision:** The new tests in `tests/integration_tests.rs` and `tests/websocket_integration_tests.rs` connect with `.expect(...)` and do not call `skip_if_no_exasol!()`. The new tests in `tests/import_export_tests.rs` follow that file's current pattern: `#[tokio::test]`, `#[ignore]`, and `assert!(common::is_exasol_available(), ...)` as the first statement. No new test edits `tests/common/mod.rs`.
- **Alternatives:** (a) Start with `skip_if_no_exasol!()` like the neighboring tests. Rejected: the macro skips unless `REQUIRE_EXASOL` is set, and `AGENTS.md` § Testing requires a failure.
- **Rationale:** `AGENTS.md` § Testing: tests must fail, not skip, when Exasol is unavailable. The plan fix-paged-fetch-position edits `tests/common/mod.rs`, so leaving it untouched avoids a merge conflict.
- **Consequences:** If fix-paged-fetch-position merges first, it removes the bare `#[ignore]` from `tests/import_export_tests.rs`, adds a CI check that rejects a bare `#[ignore]`, and changes the import/export test command. The rebase of this branch then removes `#[ignore]` from the three new import/export tests and uses that command.
- **Promotes to ADR:** no

### [7] Changelog entry under `[Unreleased]`, release version chosen at implement time

- **Decision:** `CHANGELOG.md` gets a `## [Unreleased]` section above the newest release header, with one `Fix:` entry for query results and exports and one for imports. Step A3 of `/speq:implement-pr` chooses the release version, following plan.md § Parallelization › Release: the next 0.x minor version above the highest `vX.Y.Z` tag and above any version that another open plan or PR already claims. fix-paged-fetch-position also plans a release, and its version number is not fixed. If that plan has not released when this plan merges, the two releases must still differ. Step A3 folds `[Unreleased]` into the matching `## X.Y.Z` header.
- **Alternatives:** (a) A patch release, such as 0.17.1. It would reach dependents on `exarrow-rs = "0.17"` through `cargo update`. Rejected per the requester's convention: the fix changes the values that existing queries return and that imports store for pre-1970 data, and a minor version makes that change a visible step that a dependent takes deliberately. (b) Fix the number now. Rejected: both plans branch from `main`, so a fixed number can collide.
- **Rationale:** `AGENTS.md` § Changelog requires the entry in the same PR. `CONTRIBUTING.md` § Releasing requires a SemVer bump and an exact `## X.Y.Z` header. Tag `v0.17.0` is the newest tag on `main` today.
- **Consequences:** Both plans add a `## [Unreleased]` section. The branch that merges second merges the two sections into one.
- **Promotes to ADR:** no

### [8] Merge overlap with fix-paged-fetch-position

- **Decision:** This plan keeps its edits in `src/query/results.rs` to `json_column_to_array`, the removed parsers and constants, and the parser tests. It adds new test functions to the integration test files instead of editing existing ones.
- **Alternatives:** none
- **Rationale:** fix-paged-fetch-position edits `paginate_remaining`, `ResultSetIterator`, and `from_transport_result` in `src/query/results.rs`, and adds tests to `tests/integration_tests.rs`, `tests/websocket_integration_tests.rs`, and `tests/import_export_tests.rs` (its plan.md § Parallelization). Disjoint functions keep the textual conflicts to adjacent insertions. fix-paged-fetch-position also changes `specs/architecture.md` § Constraints in its architecture delta, so the plan recorded second re-bases its § Constraints block onto the recorded section.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] Task 4.2 breaks an existing characterization test

- **Finding:** Task 4.2 said the existing tests of `src/import/arrow.rs` pass after the fix. `test_format_timestamp_micros_truncates_negative_day_offset` asserts the truncating split (`format_timestamp_micros(-1_000_000)` is `1970-01-01 00:00:01.000000`), so it fails once task 4.2 switches to Euclidean division, and no task removed it.
- **Direction change:** Task 4.1 adds the test's two inputs to `test_format_timestamp_micros_before_epoch` with their correct results (-1000000 is `1969-12-31 23:59:59.000000`, -86400000000 is `1969-12-31 00:00:00.000000`). Task 4.2 deletes the characterization test and expects only the remaining existing tests to pass. plan.md § Dead Code Removal lists the deleted test.
- **Promotes to ADR:** no

### [2] [plan-review] The minor-version rule had no actor at release time

- **Finding:** Entry [7] said "the implement step" picks the next 0.x minor version and folds `[Unreleased]`. `/speq:implement-pr` step A3 applies the Conventional Commits patch default to a pure fix plan unless the plan says otherwise, and it does not fold `CHANGELOG.md`, so the release would be a patch with empty release notes. The fix-paged-fetch-position version number also moves.
- **Direction change:** plan.md § Parallelization gains a `### Release` subsection that tells step A3 to skip the patch default, compute X.(Y+1).0 from the tags and the versions of open pull requests, run `cargo build`, and rename and merge the `[Unreleased]` section. Entry [7] names step A3 as the actor and no longer states a version number for fix-paged-fetch-position.
- **Promotes to ADR:** no

### [3] [plan-review] Two SHALL steps of the calendar scenario contradicted each other

- **Finding:** In "DATE and TIMESTAMP values count days in the proleptic Gregorian calendar", one step required that a chrono or pyarrow reader shows Exasol's year, month, and day for every date before 1582-10-15, and the next step mapped February 29 of a Julian-only leap year to March 1. Both cannot hold for `DATE '1500-02-29'`.
- **Direction change:** The first step now ends "..., except for the dates of the next step", so the Julian-only leap days are the stated exception.
- **Promotes to ADR:** no

### [4] [plan-review] Entry [2] named a long-lived constraint but claimed no architecture change

- **Finding:** Entry [2] promotes to an ADR under criterion 2, a rule that binds every future temporal conversion, while its Architecture line said "no change" and confined the rule to the types and import modules. The rule is a constraint, and it spans the native transport, query, export, arrow_conversion, import, and adbc_ffi.
- **Direction change:** The plan now has an architecture delta, `architecture.md`, with a `DELTA:CHANGED` block for `## Constraints` that copies every current bullet and adds the calendar rule. Entry [2]'s Architecture line is `§ Constraints`. The last bullet of plan.md § Context points to the delta. Entry [8] notes that fix-paged-fetch-position also changes § Constraints, so the plan recorded second re-bases its block.
- **Promotes to ADR:** no

### [5] [plan-review] Task 2.3 missed seven tests that call the removed parsers

- **Finding:** Task 2.3 deleted only the two parser test sections of `src/query/results.rs`. Seven more tests call the parsers that task 2.2 removes, so the build fails until they are gone: the six tests of the section "Tests for the parse_date_to_days month table" and `test_parse_timestamp_to_micros_time_part_without_a_colon_adds_nothing`, which also uses `SECONDS_PER_DAY` and `MICROS_PER_SECOND`. The planner flagged this in round 1 as well.
- **Direction change:** Task 2.3 lists the seven tests by name and ends with a grep that prints nothing once every caller is gone. The Dead Code Removal rows for the tests and the constants name them. A check against the source found no further callers. Each deleted case has a counterpart in `src/types/conversion.rs`: `test_parse_date_to_days_each_month_of_a_non_leap_year`, `test_ymd_to_days_leap_year_after_feb` and the new `test_parse_date_to_days_century_leap_years`, `test_parse_timestamp_to_micros_empty_string_reports_date_format`, `test_parse_timestamp_to_micros_ignores_segments_after_the_time`, `test_fractional_seconds_to_micros_empty_is_zero`, the three `test_parse_timestamp_to_micros_invalid_*` tests, and `test_parse_timestamp_to_micros_time_without_colon_is_ignored`.
- **Promotes to ADR:** no

### [6] [plan-review] No artifact named the import-side loss at the 1582 calendar reform

- **Finding:** Entry [2] described the read side only. The proleptic Gregorian dates 1582-10-05 to 1582-10-14 do not exist in Exasol, so an imported Date32 value for one of them is stored as 1582-10-15 without an error.
- **Direction change:** Entry [2] § Consequences, plan.md § Impact, and the `docs/type-mapping.md` description of task 6.1 state that on import, a Date32 value for 1582-10-05 to 1582-10-14 becomes Exasol text that Exasol stores as 1582-10-15. Exasol's IMPORT parses date text, so the planner checked the text paths as well as the literal the reviewer used. On `exasol/docker-db` 2026.1.0, `TO_DATE('1582-10-10', 'YYYY-MM-DD')`, `CAST('1582-10-05' AS DATE)`, and `CAST('1582-10-14' AS DATE)` all return 1582-10-15, and `CAST('1582-10-04' AS DATE)` stays 1582-10-04.
- **Promotes to ADR:** no

### [7] [plan-review] The boundaries-and-validation exception was broader than the plan needs

- **Finding:** The new exception covered "a loss that a `type-mapping` scenario specifies". No test can enumerate those scenarios, it left open whether nanosecond truncation now counts, and it weakened the recorded rule for future work. The covering test called only `ymd_to_days`, which cannot fail.
- **Direction change:** The Background and the scenario step of the `type-mapping/boundaries-and-validation` delta now name only February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow` maps to March 1. `test_ymd_to_days_julian_only_leap_day_reads_as_march_first` also asserts that `parse_date_to_days("1500-02-29")` returns `Ok(-171605)`, a path that can return an error. Entry [2], the Scenario Coverage row, and the coverage note use the narrow wording.
- **Promotes to ADR:** no

### [8] [plan-review] The write side keeps the calendar rule in six sites with no scheduled follow-up

- **Finding:** After this plan the read side has one owner of the calendar rule, while the write side keeps it in six code sites across `src/import/arrow.rs`, `src/import/parquet.rs`, and `src/adbc_ffi.rs`. Issue #84 itself came from two copies of one rule that drifted apart, and entry [3] scheduled no follow-up.
- **Direction change:** Entry [3] § Consequences lists the six sites and adds a follow-up bullet with the proposed issue title "Move Arrow-to-Exasol date and timestamp formatting into types::conversion with one error contract". The finding is only partly resolved: the planner did not open the issue, because creating a GitHub issue is a remote write that the planning task does not authorize, so the bullet carries no issue number yet. A read-only search of the repository's issues found no existing issue for this refactor. Open issue #38 reports a different defect in the same conversion, the `T` separator, and the bullet says the new issue should link it.
- **Promotes to ADR:** no

### [9] [plan-review] "Converts to the earlier instant" read as if every value moved

- **Finding:** The two time-unit scenario titles said the import "converts ... to the earlier instant", and plan.md § Impact and entry [3] said a value "drops them toward the earlier instant". Only the sub-microsecond digits of a nanosecond value round down. Second, Millisecond, and Microsecond values convert exactly.
- **Direction change:** The scenarios are now "RecordBatch import formats pre-epoch timestamps of every time unit as times before the epoch" and "CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch". The `/// Scenario:` lines of tasks 4.1 and 4.3 and the Scenario Coverage rows use the new titles. plan.md § Impact and entry [3] say the sub-microsecond digits round down to the microsecond.
- **Promotes to ADR:** no
