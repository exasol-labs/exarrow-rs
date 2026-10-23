# Plan Review Findings: fix-bulk-ingest-type-coverage

## Summary
- Axes checked: 6/6
- Total findings: 13 (Blockers: 2, Findings: 11)
- Intent Fidelity blockers: 0

## Ways This Plan May Fail

1. Native Parquet auto-create starts accepting FixedSizeBinary, BinaryView, Utf8View, and string dictionaries. It creates a `VARCHAR(2000000)` table, and Exasol's native Parquet reader then fails or stores different text than the CSV path. That repeats the #111 defect in a third entry point. Routed to Feasibility `[UNSTATED_ASSUMPTION]`.
2. Task 1.4 deletes the Parquet-local helpers and "the unit tests that call only the deleted helpers". That removes the only tests of two recorded `import-export/parquet-io` scenarios. Routed to Task Breakdown `[TRACEABILITY_GAP]`.
3. Date64 and Decimal256 conversion reuse helpers with narrower ranges: `format_date32` takes `i32` days, and `format_decimal128` multiplies for a negative scale. An unchecked narrowing or multiplication writes a wrong value or panics. Routed to Requirement Quality `[COMPLETENESS_GAP]`.

## Intent Fidelity

#### [INTENT_DRIFT] FINDINGS
- Location: decision-log.md § Design Decisions [2]; architecture.md § Constraints (the conversion line); plan.md § Impact bullet 2
- Issue: The interview answer is "Which types are added? → A: Date64, Decimal256 up to 36 digits, ...". Decision [2] and the new constraint accept "Decimal128 and Decimal256 with a precision above 36 or a scale outside 0 to the precision" in append mode. For Decimal128(38, s), the rationale argues that this keeps today's behavior. Decimal256 never worked, so accepting Decimal256 above 36 digits in append mode is new scope beyond the interview answer, and no entry asks the user to confirm it. plan.md § Impact then states "ADBC ingest accepts ... Decimal256 up to precision 36". That is wrong for append mode, and task 3.4 copies it into CHANGELOG.md.
- Fix: In decision-log.md [2], add the sentence "Confirm in PR review: append mode also accepts Decimal256 with a precision above 36, which extends the interview answer 'Decimal256 up to 36 digits'." In plan.md § Impact bullet 2, replace "Decimal256 up to precision 36" with "Decimal256 up to precision 36 in every mode, and Decimal128 and Decimal256 of any precision and scale in append mode".

#### [SCOPE_REDUCTION] FINDINGS
- Location: plan.md § Impact
- Issue: Both issues name the adbc-drivers.org driver page as out of date. #111 says the page "lists every Arrow type as unsupported for ingestion". #109 says "Then the page can list target catalog as supported for `EXA`". The page lives outside this repository (README.md links github.com/adbc-drivers/exasol). The plan neither updates the page nor records it as follow-up work, so this outcome of both issues can be lost when they close.
- Fix: Add one bullet to plan.md § Impact: "Follow-up outside this repository: after the release, update the driver page in adbc-drivers/exasol to list the ingestible Arrow types and the target catalog `EXA` as supported."

## Feasibility

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Impact bullets 4 and 5; decision-log.md [1] Consequences; plan.md task 3.1; architecture.md § Constraints (the conversion line)
- Issue: `import_from_parquet` (src/import/parquet.rs:268-274) and `import_from_parquet_files` (:697-706) run auto-create through `infer_schema_from_parquet*` and `TypeMapper::arrow_to_exasol` before they branch on `use_native`. Today, a Parquet file whose Arrow schema holds FixedSizeBinary, BinaryView, Utf8View, or a string dictionary fails schema inference before `CREATE TABLE`. A Parquet UUID column, read as `FixedSizeBinary(16)`, is a common example. After task 1.1, the native path creates `VARCHAR(2000000)` columns for these types, and Exasol's native Parquet reader loads the raw Parquet column, not the driver's hex text. The plan never states or verifies what Exasol does with a FIXED_LEN_BYTE_ARRAY or an unannotated BYTE_ARRAY column in a VARCHAR target. If the native IMPORT fails, the empty table stays behind. Decision [1] names exactly that defect as the reason for the refactor. If the IMPORT succeeds, the stored value differs from the CSV path, which stores `abcd` in the new parquet-io scenario. In both cases, plan.md states "Native Parquet import does not change", and task 3.1 documents "binary types are stored as lowercase hex text" and "Parquet auto-create uses the same mapping". No scenario or test covers native auto-create of these types. The local container (exasol/docker-db 2025.2.1) supports the native path, so the behavior can be verified.
- Fix: Choose one native-path behavior and state it. Option (a): import a Parquet file with FixedSizeBinary, BinaryView, Binary, Utf8View, and `Dictionary(Int32, Utf8)` columns into the local container with `with_native_parquet(Some(true))` and `with_create_table_if_not_exists(true)`. Then add a DELTA:NEW scenario to `import-export/parquet-io/spec.md` that pins the created column types and the stored values, and add its integration test to task 1.5. Option (b): pass the path choice from `import_from_parquet` and `import_from_parquet_files` into Parquet schema inference, and reject before `CREATE TABLE` each type that the native reader cannot load into its mapped column. Add a DELTA:NEW scenario for that `SchemaInferenceError`. With either option, replace "Native Parquet import does not change" in plan.md § Impact and in decision-log.md [1] Consequences with the chosen behavior. Add any native-path exception to the task 3.1 docs text and to the architecture.md conversion constraint.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Verification › Manual Testing, script M1
- Issue: M1 ingests `pa.table({"c": ...})`. `generate_create_table_ddl` (src/adbc_ffi.rs) quotes each field name as written, so the created column is the lowercase `"c"`. The script then runs `SELECT c FROM PLAN_CHECK.D64`, and Exasol resolves the unquoted `c` as `C`. On the local Exasol 2025.2.1 container, `SELECT c` against a table with the column `"c"` fails with `object C not found`. M1 stops at that line, before the replace and create checks that it exists to show.
- Fix: In script M1, change `cur.execute("SELECT c FROM PLAN_CHECK.D64")` to `cur.execute('SELECT "c" FROM PLAN_CHECK.D64')`.

## Requirement Quality

#### [COMPLETENESS_GAP] FINDINGS
- Location: import-export/arrow-recordbatch/spec.md § "RecordBatch CSV conversion covers every Arrow type that has an Exasol column type"; plan.md task 1.2
- Issue: The only Date64 example is `1_577_836_800_000` ms. Neither the spec nor task 1.2 says what a pre-epoch value that is not a whole day becomes, such as `-1` ms. Neither says what happens to a Date64 value outside 0001-01-01 to 9999-12-31. `format_date32` takes `i32` days, and `i64::MAX` ms is about 1.07e11 days. An `as i32` narrowing wraps that to a wrong date inside Exasol's range instead of failing. The recorded feature `import-export/arrow-recordbatch-dates` owns these rules for Date32 and Timestamp, in its pre-epoch scenarios and in "RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range". The plan does not extend that feature to Date64.
- Fix: Add a delta to `import-export/arrow-recordbatch-dates/spec.md`. Name Date64 in its Background. Add the Date64 values `253_402_300_800_000` (10000-01-01) and `9_223_372_036_854_775_807` to the GIVEN of "RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range". Add a step stating that the Date64 value `-1` converts to `1969-12-31`. In task 1.2, state: "floor the millisecond value to days with `div_euclid(86_400_000)`, and convert the day count to `i32` with a checked conversion that fails with `ImportError::ConversionError`". Add the matching unit tests to § Scenario Coverage.

#### [COMPLETENESS_GAP] FINDINGS
- Location: plan.md task 1.2; decision-log.md [2]
- Issue: Task 1.2 formats Decimal256 "including a negative scale, as `format_decimal128` does". `format_decimal128` (src/import/arrow.rs:484) computes `value * 10_i128.pow(-scale)`. That computation overflows for a scale of -39 or below, and for large values at smaller negative scales. An overflow panics in debug builds and wraps to a wrong number in release builds. Decision [2] makes negative scales reachable on purpose, because append mode accepts every decimal that the conversion converts. No scenario or test covers a negative scale.
- Fix: In task 1.2, specify that for a negative scale, Decimal128 and Decimal256 formatting both write the integer text followed by `-scale` zeros, with no multiplication. Add unit assertions for `Decimal128(38, -5)` holding `10^37` and for `Decimal256(76, -10)` holding a value near the type maximum, each expecting the full digit string.

#### [COMPLETENESS_GAP] FINDINGS
- Location: architecture.md § Constraints (the ingest check line); plan.md task 3.2
- Issue: The new constraint ends "so a rejected ingest drops, creates, and writes no table". The checks cover column types and the catalog only. A batch of accepted types with a rejected value, such as the Date32 value `2932897`, passes the checks. In replace mode, `execute_bulk_ingest` then runs `DROP TABLE` and `CREATE TABLE`, and only the conversion inside `import_from_record_batch` fails, so the old rows are lost. The constraint reads as a guarantee for every rejected ingest, and the docs task does not warn users about this case.
- Fix: In architecture.md, change the ending of the constraint to "so an ingest that these checks reject drops, creates, and writes no table". In task 3.2, add that a value the conversion rejects, such as a date outside 0001-01-01 to 9999-12-31, fails the ingest after replace mode has dropped the table.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: import-export/arrow-recordbatch/spec.md § "RecordBatch CSV conversion covers every Arrow type that has an Exasol column type", GIVEN step
- Issue: "one column of each Arrow type that `TypeMapper::arrow_to_exasol` maps" names an unbounded set: every decimal precision and scale, every FixedSizeBinary width, every time zone, and eight dictionary key types for two value types. A RecordBatch cannot hold one column of each, and the scenario does not say which representative columns count as coverage.
- Fix: Replace the GIVEN with an explicit column list, for example: Boolean, each integer type, Float32, Float64, Utf8, LargeUtf8, Utf8View, `Dictionary(Int8, Utf8)`, `Dictionary(UInt64, LargeUtf8)`, Binary, LargeBinary, BinaryView, `FixedSizeBinary(3)`, Date32, Date64, Timestamp of each unit with and without a time zone, `Decimal128(36, 2)`, `Decimal256(36, 2)`, and `Decimal128(38, 2)`.

## Task Breakdown

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md task 1.4; plan.md § Dead Code Removal
- Issue: Task 1.4 says "Delete `format_arrow_value` and its Parquet-local helpers. Delete the unit tests that call only the deleted helpers, and move any assertion that `src/import/arrow.rs` tests do not already cover." Two of those tests are the only tests of recorded scenarios that stay in the spec. `test_format_timestamp_before_epoch_in_every_unit` (src/import/parquet.rs:1849, `/// Scenario: CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch`) calls only the Parquet-local `format_timestamp`. `test_format_arrow_value_rejects_dates_and_timestamps_outside_exasol_range` (:1879, `/// Scenario: CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range`) calls only `format_arrow_value`. `src/import/arrow.rs` already holds the same assertions under the `arrow-recordbatch-dates` scenario titles (:1553, :1590), so a literal reading of the task deletes both tests. The two parquet-io scenarios then have no test, which breaks the AGENTS.md rule that a test implementing a scenario carries its `/// Scenario:` line. § Dead Code Removal lists only ~1203-1262 and ~1928-1951 and does not mention these two tests.
- Fix: In task 1.4, add: "Rewrite `test_format_timestamp_before_epoch_in_every_unit` and `test_format_arrow_value_rejects_dates_and_timestamps_outside_exasol_range` to convert through `record_batch_to_csv` of `src/import/parquet.rs`, and keep their `/// Scenario:` lines. Delete only the other tests that call the deleted helpers." Add both scenarios to § Verification › Scenario Coverage as regression rows that name these two tests.

Other checks on this axis: no objection. Evidence: every delta scenario has a row in § Scenario Coverage. Tasks 2.1 and 2.2 implement the Interfaces and Constraints changes of the architecture delta, and tasks 3.1 and 3.2 implement point 4 of #111. Groups A, B, and C run in sequence, and their Knowledge entries share no source file across groups.

## Design Depth

#### [INFORMATION_LEAKAGE] FINDINGS
- Location: plan.md tasks 1.2 and 1.3; architecture.md § Constraints (the conversion line)
- Issue: After the plan, three hand-written lists encode the same set of Arrow types: the `match` in `format_value`, the new `pub(crate)` schema check of task 1.3, and `TypeMapper::arrow_to_exasol`. Task 1.3 does not say how the schema check learns which types `format_value` converts, so the likely implementation is a second `match` in the same file. Only the task 1.3 unit test keeps the three lists in agreement. That test lists its sample types by hand, so a type added to one list but not to the test still passes CI. This is back-door leakage of the decision about which types convert.
- Fix: In tasks 1.2 and 1.3, specify one dispatch: a function in `src/import/arrow.rs` that resolves the value formatter of a column from its `DataType` once, or returns `ImportError::ConversionError` that names the type. The schema check calls that function for every field, and `format_row` uses the formatters it returns. The check then cannot disagree with the conversion, and the task 1.3 test only needs to compare `arrow_to_exasol` with that one function.

ADR and architecture checks: no objection. Evidence: no decision-log entry sets `Promotes to ADR: yes`, so `[ADR_OVERPROMOTION]` does not apply. The ADR digest lists 7 accepted ADRs (query timeout, give-up termination, zero-row result schema, placeholder lexer, Parquet export schema, batch arity check, export timer), and none covers ingest or type mapping, so `[ADR_CONFLICT]` does not apply. The delta BASE `af9b006de81e759c97524e6b69cae4091d9dac9c` equals `git hash-object specs/architecture.md`. Both CHANGED blocks copy the full current section, and a diff shows only the ADBC C ABI line changed and two constraint lines added. Every architectural fact in plan.md appears in the delta, so `[ARCHITECTURE_DRIFT]` does not apply.

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md § Impact bullet 4; plan.md task 3.4
- Issue: Bullet 4 says `arrow_to_exasol` "maps the six added types" and "Parquet columns of the six added types now get a column type instead of a schema inference error". `arrow_to_exasol` already maps Date64 and Decimal256 (the `Date32 | Date64` and `Decimal128 | Decimal256` arms in src/types/mapping.rs). For those two types, the only change is the new precision limit. The newly mapped types are Utf8View, string dictionaries, BinaryView, and FixedSizeBinary. Task 3.4 copies the Impact entries into CHANGELOG.md, so the release notes would misstate the change.
- Fix: Rewrite bullet 4 to say that `arrow_to_exasol` newly maps Utf8View, string dictionaries, BinaryView, and FixedSizeBinary, and that Parquet columns of these four types now get a column type instead of a schema inference error.

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md § Verification › Manual Testing table and the "Expected:" lines below scripts M1 and M2
- Issue: The table says "The four lines listed under M1" and "The two lines listed under M2". The Expected lines list five outputs for M1 and three for M2.
- Fix: In the Manual Testing table, change "four lines" to "five lines" and "two lines" to "three lines".

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md § Summary and tasks 1.1 to 1.3; decision-log.md [1] and [2]; architecture.md § Constraints
- Issue: The artifacts use several names for one thing: "One Arrow-to-CSV conversion", "the import CSV conversion", "the shared CSV conversion", "the RecordBatch CSV conversion", "the CSV conversion", "the import conversion", and "that conversion". Writing guardrails rule 4 requires one name. A reader of decision [2] cannot tell at once whether "the CSV conversion" and "the import conversion" are the same thing.
- Fix: Pick one name, such as "the import CSV conversion". Define it once in plan.md § Summary as the value conversion in `src/import/arrow.rs`, and use that name everywhere in plan.md, decision-log.md, and architecture.md.
