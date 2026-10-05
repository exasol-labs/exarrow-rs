# Decision Log: fix-export-parquet-transport-roundtrip

## Interview

Headless run. No live interview took place. The orchestrator brief is the only input, summarized here as Q/A pairs.

**Q:** What is in scope?
**A:** GitHub issue exasol-labs/exarrow-rs#58. `export_to_parquet_via_transport` re-serializes parsed rows to unquoted CSV and re-parses them. This fails or corrupts values that contain the column separator or a line break, replaces column names and types with `col0..colN: Utf8`, and trims every value in `parse_csv_value`. The suggested fix follows the shape of `export_to_record_batches` and takes the real schema from result-set metadata.

**Q:** Which prerequisite does the fix use?
**A:** Issue #60, prepared-statement result-column metadata on both transports, is closed. The plan uses it. The API is `TransportProtocol::create_prepared_statement(sql)`, which returns a `PreparedStatementHandle` whose `result_columns: Vec<ColumnInfo>` holds each column's `name` and `data_type`, and `TransportProtocol::close_prepared_statement(&handle)`. The behavior is specified in `prepared-statements/result-columns`.

**Q:** How should the `.trim()` in `parse_csv_value` be handled for the direct `export_to_parquet(csv_data, schema, ...)` path?
**A:** The planner decides and documents it. See entry [4].

**Q:** How are version and changelog handled?
**A:** By the repository conventions in `AGENTS.md` and `CONTRIBUTING.md`. See entry [11].

## Design Decisions

### [1] A transport Parquet export takes its schema from prepared-statement result-set metadata

- **Decision:** Before a transport Parquet export runs its EXPORT statement, the driver prepares the SELECT statement that the export source describes, builds the Arrow schema from the prepared statement's result-set column metadata, and closes the prepared statement.
- **Alternatives:** (a) Run a zero-row probe such as `SELECT * FROM (<query>) WHERE FALSE`, as the exapump `--split` path does. Rejected: it executes and rewrites user SQL. (b) Request `WITH COLUMN NAMES` on the EXPORT. Rejected: the header carries names but no types. (c) Infer types from the exported values. Rejected: it guesses, and an empty export has nothing to infer from. (d) Require the caller to pass a schema, as `ArrowExportOptions::with_schema` does. Rejected: every caller would have to look up the schema itself, which is the defect this fix removes.
- **Rationale:** `/speq:adr-rules` rule 2, criterion 4: the decision rejects plausible alternatives, and a later reader will ask why the export spends a prepare round trip instead of a probe query. Prepare runs no user statement and reports the same metadata on both transports (`prepared-statements/result-columns`). Search: `speq decision-log show` lists ADR-001 (placeholder lexer), ADR-003 (Thrift advisory), ADR-004 (zero-row result schema), ADR-005 (batch arity check), ADR-006 (URI schema), and the 008 and 010 fragments (query timeout, export timer, `terminate()`). None covers how an export obtains its schema. ADR-004 is consistent with this decision.
- **Consequences:** A transport Parquet export costs one prepare and one close round trip more than before. The export source must be a result-set-producing statement, which the EXPORT statement already requires. The SELECT text embeds the caller's identifiers and query exactly as the EXPORT statement does, so the change adds no SQL injection surface. The decision relies on Exasol's prepare step describing a statement without executing it. This holds for DML, DDL, and `EXECUTE SCRIPT` sources on both transports, and `test_parquet_export_rejects_source_without_result_set` checks it for a DML source and a DDL source.
- **Architecture:** § Components, § Data Flow
- **Promotes to ADR:** yes

### [2] Text-only types export as Utf8, and local-time-zone timestamps export without a time zone

- **Decision:** The Parquet export type mapping keeps the `type-mapping/exasol-to-arrow` mapping for BOOLEAN, CHAR, VARCHAR, DECIMAL, DOUBLE, DATE, and TIMESTAMP. It maps TIMESTAMP WITH LOCAL TIME ZONE to `Timestamp(Microsecond, None)` that holds the session-local wall-clock value. It maps INTERVAL YEAR TO MONTH, INTERVAL DAY TO SECOND, GEOMETRY, and HASHTYPE to Utf8 that holds Exasol's CSV text. The private `exasol_type_to_arrow` in `src/export/parquet.rs` owns this mapping, and both `exasol_types_to_arrow_schema` and the transport export use it.
- **Alternatives:** (a) Reject the four text-only types, as `exasol_types_to_arrow_schema` does today. Rejected: a table with such a column exports today as Utf8 strings and would fail after the fix. (b) Parse interval text into `Interval(MonthDayNano)`. Rejected for this plan: the interval text parsers are private to `arrow_conversion`, which has no caller inside the crate, so reusing them adds a module dependency and new conversion code beyond issue #58. A follow-up issue can add typed intervals. (c) Keep the `Timestamp(Microsecond, Some("UTC"))` that `types::conversion::exasol_type_to_arrow` returns for TIMESTAMP WITH LOCAL TIME ZONE, and make the timestamp builder apply the label. Rejected: Exasol writes the value as wall-clock text in the session time zone, so a UTC label shifts each value by the session's UTC offset. (d) Convert the session-local text to UTC. Rejected for this plan: the driver would need the session time zone and a time-zone database, which is new conversion code beyond issue #58.
- **Rationale:** No export that succeeds today starts failing. Every exported value arrives as CSV text, and the export has no text-to-Arrow conversion for INTERVAL, GEOMETRY, or HASHTYPE, so these values stay lossless as text. The native transport already returns INTERVAL result-set columns as Utf8 strings (`native-client/type-conversion`, scenario "Direct binary to Arrow conversion for interval types"), and the `type-mapping/exasol-to-arrow` GEOMETRY scenario allows Utf8. For TIMESTAMP WITH LOCAL TIME ZONE, `build_timestamp_array_from_csv` ignores its time-zone argument and returns `Timestamp(Microsecond, None)`, so a UTC-labeled schema field fails `RecordBatch::try_new` and would break every export of such a column. The native transport decodes wire type 124 (timestamp with local time zone) as `Timestamp(Microsecond, None)` as well (`src/transport/native/result_parser.rs`, test `single_pass_timestamp_with_local_time_zone_has_no_arrow_timezone`). The integration test for "Export preserves schema" runs on a `EUROPE/BERLIN` session and checks the value against `Connection::export_csv_to_list`, which verifies the session-local text form. The Background of `import-export/parquet-io` states the mapping.
- **Consequences:** `exasol_types_to_arrow_schema` returns Utf8 for the four text-only types where it returned an error, and `Timestamp(Microsecond, None)` for TIMESTAMP WITH LOCAL TIME ZONE where it returned `Timestamp(Microsecond, Some("UTC"))`. A Parquet file does not record the session time zone of a TIMESTAMP WITH LOCAL TIME ZONE value. The `CHANGELOG.md` entries name both changes. The `type-mapping/exasol-to-arrow` scenario "Date and time types mapping" limits its INTERVAL step to the conversions that produce Arrow interval values and names the native transport and the CSV-based Parquet export as Utf8 exceptions.
- **Promotes to ADR:** no

### [3] One row-to-batch converter serves both Parquet entry points

- **Decision:** A single function in `src/export/parquet.rs` converts parsed rows into RecordBatches against a schema. It reads the rows from an iterator and converts one `batch_size` chunk at a time. It checks that every row has one field per schema column, applies the NULL rule from entry [5], and calls the existing typed builders (`build_array_from_csv_column`). The transport export feeds it the rows from `export_to_list`. `csv_to_record_batches` feeds it a row iterator that `export::csv` provides. The iterator reads the input without first collecting it into a `Vec<char>`, and `parse_csv` collects the same iterator for `export_to_list`.
- **Alternatives:** (a) Call `export::arrow::export_to_record_batches` with the derived schema. Rejected: it pads a short row with NULLs (`unwrap_or("")`) instead of failing, and it reads an empty field as NULL even when `null_value` is set, which breaks the `null_value` rule of the CSV-bytes entry points. (b) Keep `csv_str.lines()` and `parse_csv_line` in the CSV-bytes path. Rejected: issue #58 names the `lines()` split at `parquet.rs:342` as the cause of the line-break failure, and the line split stays wrong for any quoted line break. (c) Have the CSV-bytes path collect every row with `parse_csv` and convert the whole list. Rejected: the path would then hold every parsed field of its input at once, where today it holds one `batch_size` chunk.
- **Rationale:** The two Parquet entry points apply one rule for how a CSV field becomes an Arrow value. `export::arrow` keeps a separate builder set (`build_array_from_strings`, `is_null_value`) with a different NULL rule, and merging the two builder sets is a separate change (plan.md § Impact). The transport export no longer serializes or re-parses rows. The CSV-bytes path parses its input one row at a time and holds parsed fields for one `batch_size` chunk, next to the input bytes and the output batches, as the line-based path does today. The transport path holds every exported row before conversion, as it does today, because `export_to_list` collects all rows. Reading the input through a `chars()` iterator instead of a `Vec<char>` lowers the peak memory of `export_to_list`.
- **Consequences:** `parse_csv_line`, `parse_csv_value`, and `csv_chunk_to_record_batch` are deleted. A row whose field count differs from the schema fails with `ParquetExportError::CsvParse` on both entry points. On the transport path this guards against a column change between the prepare and the EXPORT. Every `CsvParse` row is the 0-based index of the data row in the whole input, header excluded, because the converter offsets each row that a typed builder reports by the first row of its chunk.
- **Promotes to ADR:** no

### [4] The CSV-bytes entry points do not trim fields

- **Decision:** `csv_to_record_batches`, `export_to_parquet`, and `export_to_parquet_stream` keep every field verbatim. This applies to typed columns too: a typed field with surrounding whitespace fails to parse.
- **Alternatives:** (a) Trim only fields of non-Utf8 columns. Rejected: the parser would carry two whitespace rules, and a whitespace-only typed field would need a third rule. (b) Add a trim option. Rejected: an option is a decision the module declines to make, and no caller has asked for it.
- **Rationale:** RFC 4180 treats spaces as part of a field. The crate's other CSV reader, `CsvToArrowReader` in `export::arrow`, does not trim. The doc comments of these functions describe their input as the CSV that the HTTP transport delivers, and the transport path keeps fields verbatim after this fix.
- **Consequences:** A caller that fed padded numeric, date, or boolean fields to these functions now gets `ParquetExportError::CsvParse`. String fields keep their whitespace. A whitespace-only field is a value, not NULL. `CHANGELOG.md` names the change.
- **Promotes to ADR:** no

### [5] The transport export reads NULL only from an empty field

- **Decision:** The transport export treats an empty field as NULL and keeps every other field verbatim. It ignores `ParquetExportOptions::null_value`. The field's doc comment states that `null_value` applies only to the CSV-bytes entry points.
- **Alternatives:** (a) Apply `null_value` as the old path did. Rejected: the driver passes no NULL clause to the EXPORT statement, so Exasol never writes the marker for a NULL, and matching it can only turn a real value into NULL. With `null_value` set, the old path also turned every real NULL into an empty string. (b) Forward `null_value` into the EXPORT statement's NULL clause. Rejected: it adds a configuration path without a caller, and a value equal to the marker would still be ambiguous.
- **Rationale:** Exasol writes NULL as an empty field and stores an empty string as NULL, so an empty field is the exact NULL encoding.
- **Promotes to ADR:** no

### [6] An empty export writes a Parquet file that carries the schema

- **Decision:** When the export returns zero rows, the transport export writes a Parquet file with the derived schema and zero rows, and returns 0.
- **Alternatives:** Keep returning 0 without a file. Rejected: the old reason, that no first row exists to invent a schema from, no longer applies once the schema comes from metadata.
- **Rationale:** ADR-004 established that a zero-row result carries its schema.
- **Consequences:** exapump's empty split export calls `Connection::export_to_parquet` with the source `SELECT 1 WHERE FALSE`, so it gets a schema-only file whose only column is the `SELECT 1` column, not the columns of the exported source. exapump needs its own change to pass the real source.
- **Promotes to ADR:** no

### [7] Failure handling keeps the public error type and leaves no partial file

- **Decision:** A prepare failure maps to `ExportError::SqlExecutionError`. A source that reports zero result-set columns maps to `ExportError::SqlExecutionError` with a message stating that the source produces no result set, and the EXPORT statement does not run. A result-set column whose Exasol type the driver cannot map to an Arrow type maps to `ExportError::SqlExecutionError` with a message that names the column and the Exasol type. The driver attempts `close_prepared_statement` whenever the prepare succeeded, including when it then rejects the source. A close failure is returned, not discarded. The driver creates the output file only after every batch converts. When writing a batch or closing the Parquet writer fails, the driver removes the output file and returns the original error, also when the removal fails. When it maps `ParquetExportError` to `ExportError`, `Io` becomes `IoError`, `CsvParse` keeps its row (the 0-based data-row index from entry [3]), and every other variant keeps today's mapping to `CsvParseError { row: 0, .. }`.
- **Alternatives:** Add an `ExportError::SchemaError` variant. Rejected: adding a variant breaks exhaustive matches in callers, and the SQL error variant already describes a source that is not a query or whose column types the driver cannot map. Route a type lookup failure through `ParquetExportError::Schema`. Rejected: the caller would see a CSV parse error at row 0 for a column type, before any CSV exists.
- **Rationale:** The error names the real cause: the source SQL or its column types, a value and its column position, or I/O. No caller finds a truncated or empty Parquet file after a conversion, write, or close error.
- **Promotes to ADR:** no

### [8] The Exasol type-name lookup is shared from `query/results.rs`

- **Decision:** The match that turns a transport `DataType` into an `ExasolType` moves out of `ResultSet::exasol_datatype_to_arrow` into a crate-visible function in `src/query/results.rs`. `exasol_datatype_to_arrow` calls it, and the transport export calls it.
- **Alternatives:** (a) Reuse `arrow_conversion::converter::parse_exasol_type`. Rejected: `specs/architecture.md` states that `arrow_conversion` has no caller inside the crate, and `export` does not depend on it. (b) Write the match again in `export`. Rejected: the type-name match already exists in three places (`query/results.rs`, `arrow_conversion/converter.rs`, `adbc_ffi.rs`), and a fourth copy adds to that leakage.
- **Rationale:** `export` already depends on `query`, so the plan adds no module dependency. The extraction preserves `ResultSet` behavior.
- **Promotes to ADR:** no

### [9] Scope boundaries

- **Decision:** The plan leaves these unchanged: (a) `export_to_record_batches` and `export_to_arrow_ipc` keep requiring a caller-supplied schema. (b) The plan changes no exapump code. exapump's `--split` path keeps its own schema probe, and its empty split export needs an exapump change to pass the real source (entry [6]). (c) The export relies on Exasol's default session formats for typed values, which the Arrow export path relies on as well. A session with a custom `NLS_DATE_FORMAT`, `NLS_TIMESTAMP_FORMAT`, or `NLS_NUMERIC_CHARACTERS` gets a conversion error that names the column position and the value. (d) The plan adds no typed INTERVAL conversion (entry [2]).
- **Alternatives:** For (c), emit a per-column `FORMAT` in the EXPORT statement's column list. Rejected for this plan: it couples the CSV EXPORT builder to the derived schema, and issue #58 does not report the problem.
- **Rationale:** Issue #58 covers the transport Parquet path and the trim in the CSV-bytes path. The items above are separate changes.
- **Promotes to ADR:** no

### [10] Exasol's CSV text forms for typed values are verified during implementation

- **Decision:** The integration test for "Export preserves schema" covers every typed column, and includes the `PRICE` values `0.50` and `-0.50`. The implementer runs it against the Docker database first. If Exasol writes a fractional DECIMAL without the leading zero (`.5`, `-.5`), `parse_decimal_to_i128` in `src/types/conversion.rs` gains support for an empty integer part, with a unit test. If the test shows another default text form that a typed builder rejects, for example for BOOLEAN or DOUBLE, the implementer extends that builder in the same way.
- **Alternatives:** Extend the parsers without checking. Rejected: it adds code for unconfirmed inputs. Skip the check. Rejected: a typed column would then fail the export on real data.
- **Rationale:** Exasol's documentation does not state the default CSV text forms of DECIMAL, BOOLEAN, or DOUBLE values, and the planner could not start a database. The typed builders accept `TRUE`/`FALSE`/`1`/`0` in any case, any form Rust's `f64` parser accepts, `YYYY-MM-DD` dates, and `YYYY-MM-DD HH:MM:SS[.ffffff]` timestamps. This is a load-bearing assumption, so the plan states it and attaches a test.
- **Promotes to ADR:** no

### [11] Version and changelog follow the repository convention

- **Decision:** The PR adds `Fix:` entries for #58 under `## [Unreleased]` in `CHANGELOG.md`. The entries also name the behavior changes from entries [2], [4], [5], and [6]. For entry [2], they name both the Utf8 mapping of INTERVAL, GEOMETRY, and HASHTYPE and the `Timestamp(Microsecond, None)` mapping of TIMESTAMP WITH LOCAL TIME ZONE. They also state that `CHAR(n)` values now keep Exasol's space padding to length n, the same value a query returns. A version bump, if this PR carries one, follows `CONTRIBUTING.md` § Releasing. `[Unreleased]` already holds breaking entries from #60, so a bump is a minor bump.
- **Alternatives:** none
- **Rationale:** `AGENTS.md` § Changelog requires a changelog entry in the same PR as a user-facing change.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] TIMESTAMP WITH LOCAL TIME ZONE columns had no export mapping

- **Finding:** `plan-reviewer` round 1, `[COMPLETENESS_GAP]` BLOCKER. The plan never named TIMESTAMP WITH LOCAL TIME ZONE. The derived schema would type such a column `Timestamp(Microsecond, Some("UTC"))`, the timestamp builder returns `Timestamp(Microsecond, None)`, and `RecordBatch::try_new` rejects the batch, so every export of a table with this column would fail where it succeeds today. A UTC label inside the builder would shift each session-local value.
- **Direction change:** Decision [2] maps TIMESTAMP WITH LOCAL TIME ZONE to `Timestamp(Microsecond, None)` holding the session-local value, with alternatives (c) and (d). The spec delta Background names the mapping. "Export preserves schema" adds a `TS_LTZ` column, a `EUROPE/BERLIN` session, and a step that compares the value with `Connection::export_csv_to_list`. plan.md task 1.3 changes the private mapping and replaces `test_exasol_type_to_arrow_timestamp_with_tz`, task 3.4 sets the session time zone in `test_parquet_export_preserves_schema`, task 4.2 documents the mapping, and § Impact and decision [11] name the change.
- **Promotes to ADR:** no

### [plan-review] Background statements no scenario depended on

- **Finding:** `plan-reviewer` round 1, `[IMPLEMENTATION_LEAKAGE]` BLOCKER. The spec delta Background stated that the native transport returns INTERVAL columns as Utf8, gave the reason for the Utf8 mapping, and stated that a CSV-bytes field equal to `null_value` is NULL. No scenario step depended on any of the three.
- **Direction change:** The first two statements are deleted from the Background, and decision [2] Rationale carries both. The `null_value` statement stays, and the new scenario "CSV-bytes export reads only the null_value marker as NULL" depends on it: with `with_null_value("NULL")`, a `NULL` field reads back as NULL and an empty field reads back as an empty string. plan.md task 2.5 and § Scenario Coverage map it to the unit test `test_export_to_parquet_stream_reads_null_value_marker_as_null`.
- **Promotes to ADR:** no

### [plan-review] The CSV-bytes path would hold every parsed field at once

- **Finding:** `plan-reviewer` round 1, `[NFR_IGNORED]` ADVISORY. Decision [3] claimed that the CSV-bytes path's peak memory stays near its input size. `parse_csv` returns every row of the input, so the rewritten `csv_to_record_batches` would hold every parsed field at once, where today it holds one `batch_size` chunk.
- **Direction change:** plan.md task 2.1 adds a crate-visible row iterator in `export::csv`, and `parse_csv` collects it for `export_to_list`, which keeps collecting all rows. Task 2.2 makes the shared converter pull one `batch_size` chunk at a time from an iterator, and task 2.3 feeds it the row iterator. Decision [3] states the corrected memory profile and adds alternative (c). plan.md § Impact states that peak memory does not grow.
- **Promotes to ADR:** no

### [plan-review] The prepare step was assumed safe for DDL sources without a check

- **Finding:** `plan-reviewer` round 1, `[UNSTATED_ASSUMPTION]` ADVISORY. The driver now sends the raw source text to `create_prepared_statement` before any EXPORT. The plan checked only a `DELETE` source and did not state that Exasol's prepare step executes no statement class.
- **Direction change:** A probe against the Exasol 2025.2.0 Docker database prepared and closed `DELETE`, `CREATE TABLE`, `INSERT`, and `EXECUTE SCRIPT` sources on the native and WebSocket transports. No row changed and no table was created. Decision [1] Consequences states the assumption and its scope. plan.md task 3.4 extends `test_parquet_export_rejects_source_without_result_set` with the DDL source `CREATE TABLE <test schema>.T2 (X DECIMAL(1,0))` and asserts that `T2` does not exist afterwards.
- **Promotes to ADR:** no

### [plan-review] exapump's empty split export was presented as fixed

- **Finding:** `plan-reviewer` round 1, `[UNSTATED_ASSUMPTION]` ADVISORY. exapump's empty split export exports `SELECT 1 WHERE FALSE`, so after the fix it writes a file with the `SELECT 1` column, not the schema of the exported source. plan.md § Impact said that exapump picks up the fix without code changes.
- **Direction change:** plan.md § Context and § Impact state that the empty split export writes a schema-only file with the `SELECT 1` column and that exapump needs its own change. The exapump sentence moves from decision [6] Rationale to a Consequences line that states the same, and decision [9] (b) says that the plan changes no exapump code.
- **Promotes to ADR:** no

### [plan-review] Values with a lone carriage return were not covered

- **Finding:** `plan-reviewer` round 1, `[COMPLETENESS_GAP]` ADVISORY. `parse_csv` ends a row at a carriage return outside quotes. If Exasol left a value with a lone carriage return unquoted, the value would split its row.
- **Direction change:** On the Exasol 2025.2.0 Docker database, an export with `DELIMIT = AUTO` and `ROW SEPARATOR = 'LF'` writes `'a' || CHR(13) || 'b'` and `'ends' || CHR(13)` in quotes. `parse_csv` keeps a quoted carriage return as field data, so task 2.1 keeps the row-end rules and plan.md § Impact gains no limitation. The scenario "Values containing the separator, the delimiter, or a line break export intact" adds both values, and plan.md task 3.4 stores them in `test_parquet_export_keeps_values_with_separators_and_line_breaks`.
- **Promotes to ADR:** no

### [plan-review] The row number in a CsvParse error was undefined

- **Finding:** `plan-reviewer` round 1, `[AMBIGUOUS_REQUIREMENT]` ADVISORY. The scenario "CSV-bytes export keeps field whitespace" required an error "naming the row" without defining the numbering. The typed builders report the row inside the current batch, so a bad value in a later batch would report a wrong row.
- **Direction change:** The scenario step now requires the 0-based index of the data row in the whole input, header excluded. plan.md task 2.2 makes the converter add the chunk's first row to every builder-reported row and adds `test_csv_to_record_batches_reports_the_data_row_of_a_bad_value_in_a_later_batch`. Task 2.3 applies the same numbering to iterator errors. Decisions [3] and [7] state the numbering.
- **Promotes to ADR:** no

### [plan-review] The INTERVAL mapping contradicted type-mapping/exasol-to-arrow

- **Finding:** `plan-reviewer` round 1, `[REQUIREMENT_CONFLICT]` ADVISORY. The recorded scenario "Date and time types mapping" requires INTERVAL types to map to Arrow Duration or Interval types. The delta maps them to Utf8 in the Parquet export, and `native-client/type-conversion` already maps them to Utf8.
- **Direction change:** A new delta, `type-mapping/exasol-to-arrow/spec.md`, changes "Date and time types mapping" so that its INTERVAL step applies to `TypeMapper::exasol_to_arrow` and the `arrow_conversion` converters, and names the native transport's result sets and the CSV-based Parquet export as Utf8 exceptions. plan.md § Features, § Parallelization, § Scenario Coverage, § Manual Testing, and task 1.3 list the delta. Decision [2] Consequences names it.
- **Promotes to ADR:** no

### [plan-review] A write or close failure left a truncated file

- **Finding:** `plan-reviewer` round 1, `[COMPLETENESS_GAP]` ADVISORY. Decision [7] promised that no caller finds a truncated or empty Parquet file after an error, but task 3.2 covered conversion errors only. A failure in `ArrowWriter::write` or `close` would leave a truncated file.
- **Direction change:** plan.md task 3.2 removes the output file when writing or closing fails, returns the original error, and adds a unit test whose write step fails after writing bytes. Decision [7] states the removal. plan.md § Impact states that a failed export leaves no file.
- **Promotes to ADR:** no

### [plan-review] The CHAR padding change was not named

- **Finding:** `plan-reviewer` round 1, `[COMPLETENESS_GAP]` ADVISORY. Exasol pads `CHAR(n)` values to length n. The removed trim used to strip that padding, so every CHAR column in a Parquet export now keeps it. Neither plan.md § Impact nor the decision [11] CHANGELOG list named the change.
- **Direction change:** plan.md § Impact and decision [11] state that `CHAR(n)` values keep Exasol's space padding to length n, the same value a query returns. On the Docker database, a `CHAR(5)` value `ab` exports as `"ab   "`.
- **Promotes to ADR:** no

### [plan-review] Decision [3] overstated one owner for CSV-to-Arrow conversion

- **Finding:** `plan-reviewer` round 1, `[INFORMATION_LEAKAGE]` ADVISORY. Decision [3] said that one module owns how a CSV field becomes an Arrow value. `export::arrow` keeps its own builders with a different NULL rule, so two owners remain, and alternative (a) was rejected for a property the chosen design shares.
- **Direction change:** Decision [3] Rationale states that the plan unifies the two Parquet entry points only and that `export::arrow` keeps a separate builder set with a different NULL rule. Alternative (a) now names its NULL rule as the reason. plan.md § Impact states the remaining divergence and that merging the builder sets is a separate change.
- **Promotes to ADR:** no

### [plan-review] A type lookup failure surfaced as a CSV parse error

- **Finding:** `plan-reviewer` round 1, `[TACTICAL_SHORTCUT]` ADVISORY. Task 3.1 routed a type lookup failure through `ParquetExportError::Schema`, which decision [7] maps to `ExportError::CsvParseError { row: 0, .. }`. The caller would see a CSV parse error for a column type before any CSV exists.
- **Direction change:** plan.md task 3.1 returns a type lookup failure as `ExportError::SqlExecutionError` with a message that names the column and the Exasol type. Decision [7] lists the mapping and records the rejected route.
- **Promotes to ADR:** no

### [plan-review] The conversion error named the column in one place and its position in another

- **Finding:** `plan-reviewer` round 1, `[PROSE_UNCLEAR]` ADVISORY. plan.md § Impact and decision [9] (c) said that the conversion error names the column. The scenario "A value that does not match its column type fails the export" requires the column position, which is what the typed builders report.
- **Direction change:** plan.md § Impact and decision [9] (c) now say "the column position", matching the scenario.
- **Promotes to ADR:** no
