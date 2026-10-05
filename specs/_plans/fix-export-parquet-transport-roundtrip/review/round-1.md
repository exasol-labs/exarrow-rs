# Plan Review Findings: fix-export-parquet-transport-roundtrip (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 13 (Blockers: 2, Advisory: 11)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now this plan failed catastrophically. Three ways it got there:

1. A user exports a table that has a `TIMESTAMP WITH LOCAL TIME ZONE` column. Today that export succeeds with Utf8 columns. After the fix, the schema types the column `Timestamp(Microsecond, Some("UTC"))`, `build_timestamp_array_from_csv` ignores the time zone and returns `Timestamp(Microsecond, None)`, and `RecordBatch::try_new` rejects the batch. Every export of that table fails. If an implementer patches the builder to apply the label, the file instead shifts each value by the session's UTC offset. Routed to Requirement Quality, `[COMPLETENESS_GAP]` BLOCKER.
2. A service converts a 2 GB CSV buffer with `export_to_parquet_stream`. The old code parsed 1024 lines at a time. The rewritten `csv_to_record_batches` first turns every field of the input into its own `String` through `parse_csv`, and the process runs out of memory. Decision [3] claims the opposite. Routed to Feasibility, `[NFR_IGNORED]`.
3. A VARCHAR value contains a lone carriage return. If Exasol does not quote it under `DELIMIT AUTO` with an LF row separator, `parse_csv` ends the row at the carriage return, and the export fails with a field-count error. This is the issue #58 symptom in a new form. Routed to Requirement Quality, `[COMPLETENESS_GAP]`.

## Intent Fidelity

[no objection, axis checked: issue #58's three defects each map to the plan. Defect 1 maps to task 3.2 (no `row.join` re-serialization) and task 2.3 (`csv_str.lines()` replaced by `parse_csv`). Defect 2 maps to decision [1] and tasks 3.1 and 3.2, which use the #60 API that the issue comment names. Defect 3 maps to decisions [4] and [5] and task 2.2. The brief left the trim on the CSV-bytes path open, and decision [4] decides it with alternatives and rationale. The issue's suggested shape, arrays built directly from the `export_to_list` rows, is the shared converter of decision [3]. The schema-only file for an empty export (decision [6]) and Utf8 for INTERVAL, GEOMETRY, and HASHTYPE (decision [2]) follow from taking the real schema and prevent regressions, so they are not scope creep.]

## Feasibility

#### [NFR_IGNORED] ADVISORY
- Location: decision-log.md § [3] Rationale; plan.md § Implementation Tasks 2.1 and 2.3
- Issue: Decision [3] states "Removing the `Vec<char>` collection keeps the CSV-bytes path's peak memory near its input size after it moves to `parse_csv`." The claim is false. `parse_csv` returns `Vec<Vec<String>>` for the whole input. Each field costs a 24-byte `String` header plus its own heap allocation, so a CSV with short numeric fields needs many times its input size. Today `csv_to_record_batches` holds parsed values for one `batch_size` chunk at a time. After task 2.3 it holds parsed values for every row at once, next to the input bytes and the output batches. The transport path does not regress, because `export_to_list` already collects every row.
- Fix: In plan.md task 2.1, give `parse_csv` a row-at-a-time form (an iterator or a per-row callback) and have task 2.3 convert one `batch_size` chunk at a time, while `export_to_list` keeps collecting all rows. Alternatively, keep tasks 2.1 and 2.3 unchanged, delete the memory sentence from decision [3] Rationale, and add a plan.md § Impact bullet stating that the CSV-bytes entry points now hold every parsed field in memory before conversion.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: decision-log.md § [1] Decision and Consequences; spec.md § Scenario "Export source that produces no result set is rejected before the export runs"
- Issue: Today a query source reaches Exasol only inside `EXPORT (<query>)`, so text that is not a query fails to parse and never runs. After this plan, `create_prepared_statement` receives the raw source text as a standalone statement before any EXPORT. The plan assumes that Exasol's prepare step executes no statement class. The scenario and its integration test check this for `DELETE FROM T` only. DDL and `EXECUTE SCRIPT` sources stay unchecked.
- Fix: In plan.md task 3.4, extend `test_parquet_export_rejects_source_without_result_set` with a DDL source such as `CREATE TABLE <test schema>.T2 (X DECIMAL(1,0))`, and assert that `T2` does not exist afterwards. State the verified assumption in decision [1] Consequences.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Context bullet 5 and § Impact last bullet; decision-log.md § [6] Rationale
- Issue: The plan presents exapump's empty split export as a beneficiary of decision [6]. That path calls `Connection::export_to_parquet` with `SELECT 1 WHERE FALSE` (`labs-exapump/src/commands/export.rs`). After the fix, the call writes a file whose schema is the single column of `SELECT 1`, not the schema of the source the user exported. The Impact line "exapump 0.12.0 picks up the fix without code changes" holds for exapump's non-split path only.
- Fix: In plan.md § Impact, state that exapump's empty split export now writes a schema-only file with the `SELECT 1` column, and that exapump needs its own change to pass the real source. Remove the exapump sentence from decision [6] Rationale, or reword it so it does not present that path as fixed.

Other checks, no objection: the mock unit tests are buildable, because `MockTransport` mocks `create_prepared_statement` and `close_prepared_statement` and `PreparedStatementHandle::with_result_columns` is `pub(crate)`. Decision [10] states the DECIMAL text-form assumption and attaches task 3.5 to verify it. `export_to_callback` opens the HTTP tunnel only after the schema step, so a prepare failure leaves no tunnel open.

## Requirement Quality

#### [COMPLETENESS_GAP] BLOCKER
- Location: spec.md § Background (type-mapping paragraph); plan.md § Implementation Tasks 1.3 and 3.1; decision-log.md § [2]
- Issue: The plan names BOOLEAN, CHAR, VARCHAR, DECIMAL, DOUBLE, DATE, TIMESTAMP, and four text-only types, but never `TIMESTAMP WITH LOCAL TIME ZONE`. The lookup from task 1.2 turns that type into `ExasolType::Timestamp { with_local_time_zone: true }`. `types::conversion::exasol_type_to_arrow` maps it to `Timestamp(Microsecond, Some("UTC"))`. `build_timestamp_array_from_csv` ignores its `_tz` argument and returns `Timestamp(Microsecond, None)`. `RecordBatch::try_new` then rejects the batch with "column types must match schema types" (arrow-array 58, `record_batch.rs`). Every transport Parquet export of a table with such a column fails, although today it succeeds with Utf8 text. This contradicts decision [2] Rationale: "No export that succeeds today starts failing." Applying the UTC label inside the builder would also be wrong. Exasol writes these values as session-local wall-clock text, so a UTC label shifts each value by the session's UTC offset. The native transport already decodes session-local values of this type as `Timestamp(Microsecond, None)` (`src/transport/native/result_parser.rs`, test `single_pass_timestamp_with_local_time_zone_has_no_arrow_timezone`).
- Fix: In plan.md task 1.3, also map `ExasolType::Timestamp { with_local_time_zone: true }` to `Timestamp(Microsecond, None)` in the private `exasol_type_to_arrow` of `src/export/parquet.rs`, and assert it in the task 1.3 unit test through `exasol_types_to_arrow_schema`. In spec.md § Background, name `TIMESTAMP WITH LOCAL TIME ZONE` and state that it maps to a timestamp without a time zone that holds the session-local value. Add a column `TS_LTZ TIMESTAMP WITH LOCAL TIME ZONE` to the "Export preserves schema" scenario with the expected type `Timestamp(Microsecond, None)`. Have `test_parquet_export_preserves_schema` set a non-UTC session time zone (`ALTER SESSION SET TIME_ZONE = 'EUROPE/BERLIN'`) and check that the value read back matches the session-local text that `Connection::export_csv_to_list` returns for the same row. Add the mapping to decision [2], plan.md § Impact, and the decision [11] CHANGELOG list.
- Escalation: MECHANICAL. The code path and the native transport's existing mapping settle the behavior. No requester judgment is needed.

#### [IMPLEMENTATION_LEAKAGE] BLOCKER
- Location: specs/_plans/fix-export-parquet-transport-roundtrip/import-export/parquet-io/spec.md § Background, the type-mapping paragraph and the NULL paragraph
- Issue: Three new Background statements are facts that no scenario step in the target spec depends on:
  1. "The native transport also returns INTERVAL result-set columns as Utf8 strings (`native-client/type-conversion`)." No parquet-io scenario involves native-transport query results. The sentence is rationale for decision [2].
  2. "because every exported value arrives as CSV text and the export has no text-to-Arrow conversion for these types" is rationale for the same decision.
  3. For the CSV-bytes entry points, "a field equal to `null_value` is NULL". No scenario sets `null_value` on a CSV-bytes entry point. "CSV-bytes export keeps field whitespace" runs with `null_value` unset.
- Fix: Delete statements 1 and 2 from spec.md § Background. Decision [2] Rationale already carries them. For statement 3, add a step or scenario in which `export_to_parquet_stream` runs with `with_null_value("NULL")`, a `NAME` field equal to `NULL` reads back as NULL, and an empty `NAME` field reads back as an empty string. Map the new step to a unit test in plan.md § Scenario Coverage. If you do not add the step, delete the clause from the Background.
- Escalation: MECHANICAL. The Background rule is checked against the delta's own scenarios.

#### [COMPLETENESS_GAP] ADVISORY
- Location: spec.md § Scenario "Values containing the separator, the delimiter, or a line break export intact"; plan.md task 2.1
- Issue: `parse_csv` ends a row at a carriage return outside quotes, and it drops a carriage return that precedes a line feed. The transport export requests `ROW SEPARATOR = 'LF'`. If Exasol's default `DELIMIT AUTO` does not quote a value that contains only a carriage return, that value splits its row. A last-column value that ends with a carriage return loses it. The scenario covers a CRLF value, which contains a line feed and is therefore quoted, but not a lone carriage return. plan.md § Impact claims that every value with "a line break" exports intact.
- Fix: Add a `NAME` value with a lone carriage return and a `NAME` value that ends with a carriage return to the scenario and to `test_parquet_export_keeps_values_with_separators_and_line_breaks` (task 3.4). If Exasol leaves them unquoted, change `parse_csv` in task 2.1 so that a carriage return not followed by a line feed is field data, and record any remaining limitation in plan.md § Impact.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: spec.md § Scenario "CSV-bytes export keeps field whitespace", last step; plan.md task 2.2; decision-log.md § [7]
- Issue: The step "fail with `ParquetExportError::CsvParse` naming the row" does not define the row numbering: 0-based or 1-based, header counted or not. The converter reuses the typed builders, which report the row index inside the current batch (`row_idx` in `build_*_array_from_csv`). A conversion failure in the second or a later batch therefore reports a row that is off by a multiple of `batch_size`. Decision [7] says `CsvParse` "keeps its row", which passes that wrong number on to `ExportError::CsvParseError`.
- Fix: State the numbering in spec.md, for example "the 0-based index of the data row, header excluded". In plan.md task 2.2, add that the converter offsets every builder-reported row by the first row of its batch, and add a unit test whose bad value sits in the second batch.

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: spec.md § Background (type-mapping paragraph); recorded `type-mapping/exasol-to-arrow` § Scenario "Date and time types mapping"
- Issue: The recorded scenario states "INTERVAL types SHALL map to Arrow Duration or Interval types". The delta maps INTERVAL YEAR TO MONTH and INTERVAL DAY TO SECOND to Utf8, and task 1.3 changes the public `exasol_types_to_arrow_schema` to match. The library already holds the same contradiction through `native-client/type-conversion`. This plan adds a second disagreeing spec and does not reconcile them.
- Fix: Add a `DELTA:CHANGED` block for `type-mapping/exasol-to-arrow` § Scenario "Date and time types mapping" that limits the INTERVAL step to the paths that produce Arrow interval values and names the CSV-based Parquet export and the native transport as Utf8 exceptions. Alternatively, add a sentence to decision [2] Consequences that states the conflict is accepted and why.

#### [COMPLETENESS_GAP] ADVISORY
- Location: decision-log.md § [7] Decision and Rationale; plan.md task 3.2
- Issue: Decision [7] states "No caller finds a truncated or empty Parquet file after an error." Task 3.2 guarantees this for conversion errors only, by creating the file after every batch converts. A failure inside the task 2.4 writer helper, such as a disk-full error during `ArrowWriter::write` or `close`, leaves a truncated file at `file_path`.
- Fix: In plan.md task 3.2, remove the output file when writing or closing fails and return the original error. Alternatively, narrow the decision [7] sentence to conversion errors.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Impact bullet 3; decision-log.md § [11]
- Issue: Exasol pads `CHAR(n)` values with trailing spaces to length n. Today's `.trim()` removes that padding from the Parquet file. After the fix, every CHAR column in a Parquet export keeps it. The Impact bullet "Text values keep leading and trailing whitespace" covers this only implicitly, and the CHANGELOG entries in decision [11] do not name it.
- Fix: Add to plan.md § Impact and to the decision [11] CHANGELOG list that `CHAR(n)` values now keep Exasol's space padding to length n, the same value a query returns.

## Task Breakdown

[no objection, axis checked: every scenario in the delta maps to a named test in plan.md § Scenario Coverage. Every task traces to a scenario, a decision, or a § Dead Code Removal row, and every Dead Code Removal row names the task that replaces it. The single Parallelization group is justified, because tasks 2.x and 3.x edit `src/export/parquet.rs` and share one spec delta. The ordering note puts 1.x and 2.1 to 2.4 before 3.1 and 3.2, 3.5 after 3.4, and 4.x last. The existing integration tests `test_parquet_export_to_file` and `test_parquet_round_trip` assert row counts only and stay valid with typed columns.]

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: decision-log.md § [3] Rationale and Alternatives (a)
- Issue: Decision [3] states "One module owns the decision of how a CSV field becomes an Arrow value." After the plan, two modules still own it: `export::parquet` (the shared converter and `build_*_array_from_csv`) and `export::arrow` (`build_array_from_strings` and `is_null_value`). Their NULL rules differ. `export::arrow` treats an empty field as NULL even when `null_value` is set, and `export::parquet` does not. Alternative (a) is rejected partly because it "would leave two string-to-array paths that differ in behavior", but the chosen design also leaves two.
- Fix: Correct decision [3] Rationale to say that the plan unifies the two Parquet entry points only, and that `export::arrow` keeps a separate builder set with a different NULL rule. Add a plan.md § Impact line or a follow-up issue reference for merging the two builder sets.

#### [TACTICAL_SHORTCUT] ADVISORY
- Location: decision-log.md § [7]; plan.md task 3.1
- Issue: Task 3.1 routes a type-lookup failure through `ParquetExportError::Schema`. Decision [7] maps that variant to `ExportError::CsvParseError { row: 0, .. }`. A caller that matches on the variant sees a CSV parse error at row 0 for a column type the driver cannot map, before any CSV exists. Decision [7] already maps a source without a result set to `SqlExecutionError`, the variant it argues "describes a source that is not a query".
- Fix: In plan.md task 3.1, return a type-lookup failure as `ExportError::SqlExecutionError` with a message that names the column and the Exasol type, and list that mapping in decision [7].

Other checks, no objection:
- ADR promotion: decision [1] is the only `Promotes to ADR: yes` entry. Its Rationale names rule-2 criterion 4 and states the `speq decision-log show` result. Its Decision holds no paths, signatures, or flags, so `[ADR_OVERPROMOTION]` does not apply.
- Architecture drift: the delta's BASE equals `git hash-object specs/architecture.md` (`51e1eee...`). The Components block copies the current section with only the export bullet changed. The Data Flow block adds one bullet for the prepare step. No component, boundary, or dependency in plan.md is missing from the delta, because `export` already depends on `query` and `transport core`.
- ADR conflict: the plan agrees with ADR-004 (zero-row results carry their schema). ADR-005, `export-timer-stays-opt-in`, and `client-give-up-terminates-connection` do not apply, because the Parquet export arms no timer and never gives up on a request.
- Task 1.2 removes duplication: the export path reuses the `DataType` lookup in `query/results.rs` instead of adding a fourth copy.

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Impact bullet 8; decision-log.md § [9] (c); spec.md § Scenario "A value that does not match its column type fails the export"
- Issue: plan.md and decision [9] say that the conversion error "names the column and the value". The scenario requires "the column position and the value". The builders report a 0-based position such as `at column 3`, not a name. A reader cannot tell whether the error promises the column name.
- Fix: Use one term in all three places. Either change plan.md § Impact and decision [9] (c) to "the column position", or require the column name in the scenario and add to plan.md task 2.2 that the converter includes the field name in the message.

Other checks, no objection: the four artifacts contain no em dashes, and their sentences lead with the decision or the result.
