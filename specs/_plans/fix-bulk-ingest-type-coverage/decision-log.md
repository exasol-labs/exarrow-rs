# Decision Log: fix-bulk-ingest-type-coverage

## Interview

- Q: Which open bugs go in one plan? → A: #111 (type coverage) and #109 (target catalog). Both touch bulk ingest. #103, #66, #69 excluded after triage.
- Q: Rule for Arrow types in ingest? → A: A type is either rejected with InvalidArguments before any SQL, or ingested end to end (per #111).
- Q: Which types are added? → A: Date64, Decimal256 up to 36 digits, Dictionary of Utf8, Utf8View, BinaryView, FixedSizeBinary (per #111).
- Q: Which types stay rejected? → A: Time32, Time64, Duration, List, and Interval(MonthDayNano), with a clear message. Interval is not named in #111; rejecting it follows the either-or rule. Confirm in PR review.
- Q: When is the schema validated? → A: Before any DROP or CREATE, so replace never drops the table and create never leaves an empty table (per #111).
- Q: Binary mapping? → A: Keep Binary as hex text in VARCHAR and document it (per #111).
- Q: Target catalog? → A: Accept `adbc.ingest.target_catalog` only when it equals `EXA`, case-insensitive. Any other value fails with InvalidArguments (per #109).
- Q: Tests? → A: Driver manager test for both catalog cases; tests for each added and rejected type, including no table left behind.
- Q: Interval: reject or support (PR #112 review)? → A: Support it. Interval(YearMonth) maps to INTERVAL YEAR TO MONTH, Interval(DayTime) to INTERVAL DAY TO SECOND, and Interval(MonthDayNano) to INTERVAL DAY TO SECOND, where a value with a month component fails the ingest with a clear error. Exasol INTERVAL YEAR(p) TO MONTH takes p from 1 to 9, and INTERVAL DAY(lfp) TO SECOND(fsp) takes lfp from 1 to 9 and fsp from 0 to 9, with accuracy limited to milliseconds (https://docs.exasol.com/db/latest/sql_references/data_types/datatypedetails.htm). Handle sub-millisecond values without silent data loss unless the docs say otherwise. The docs do not describe the CSV text of interval values, so an integration test against the Docker database pins the text format and the stored value. This answer replaces the Interval part of the previous answer.
- Q: Decimal limit in append mode (PR #112 review)? → A: The same limit in every mode. Append rejects a Decimal precision above 36, a negative scale, or a scale above the precision with InvalidArguments before any SQL. This replaces the earlier rule, in which append mode accepted any decimal that the import CSV conversion converts.
- Q: Should ADBC ingest convert the bound values before its first SQL statement, so that a value error leaves replace mode's table in place (PR #112 review round 2)? → A: No, keep today's order: the type and catalog checks run before any SQL, and value conversion runs after the DDL. A value that the conversion rejects can fail replace mode after the old table is dropped; the docs warn about it. Replace mode exists so that the new data alone defines the column types of the new table, with no influence from the old table.
- Q: How does CSV-path Parquet import handle an INTERVAL column from a file without an Arrow schema, which the Parquet reader reads as Interval(DayTime) without its month part (PR #112 review round 2)? → A: Reject the column only when the Arrow schema is missing.

## Design Decisions

### [1] CSV-path Parquet import uses the import CSV conversion

- **Decision:** `src/import/parquet.rs` stops formatting values itself. Its `record_batch_to_csv` formats each row with the row formatter of the import CSV conversion in `src/import/arrow.rs`, and its own formatter and helpers are deleted.
- **Rationale:** `TypeMapper::arrow_to_exasol` also feeds Parquet auto-create (`src/types/infer.rs`). Once it maps the added types, a CSV-path Parquet auto-create would create the table and then fail in the Parquet formatter, which is the #111 defect in a second entry point. One conversion removes the duplicate type knowledge. The new dependency runs from the Parquet CSV path to the import CSV conversion inside the same import module, so it adds no module and no cycle. The two formatters already disagree: the Parquet one drops the sign of a negative decimal below 1 (`-0.05` becomes `0.05`) and quotes on `,` instead of the configured separator.
- **Alternatives:** Add the new types to both formatters. Rejected, because the two type lists can drift again and the two existing bugs stay. Leave `arrow_to_exasol` unchanged for Parquet and keep a second mapping for ingest. Rejected, because two mappings of the same Arrow types repeat the leak this plan removes.
- **Consequences:** CSV-path Parquet import writes floats without an exponent, keeps the sign of negative decimals, and quotes values that contain the configured separator. Native Parquet import into an existing table does not change. Native Parquet import with table creation rejects Utf8View, Dictionary, BinaryView, FixedSizeBinary, and Interval fields before `CREATE TABLE`; see `[plan-review] Native Parquet auto-create keeps rejecting the newly mapped types` and decision [4].
- **Promotes to ADR:** no

### [2] One ingest column type rule in every mode

- **Decision:** Every ingest mode accepts a bound column type only when `TypeMapper::arrow_to_exasol` maps it and the import CSV conversion converts it. Append checks the same rule as create, create-append, and replace.
- **Rationale:** The user chose one limit for every mode in the PR review. Exasol rejects `DECIMAL(p,s)` with p above 36, so the create modes need the mapping check, and one rule lets the documented type table apply to every mode.
- **Alternatives:** A mode-dependent check, in which append accepts any type that the import CSV conversion converts, so a `Decimal128(38, s)` batch still appends into an existing `DECIMAL(36,s)` column. Rejected by the user in the PR review.
- **Consequences:** Appending a Decimal128 column with a precision above 36 now fails with `InvalidArguments` before any SQL. The import CSV conversion still converts decimals with a precision above 36 or a negative scale for RecordBatch and Parquet imports into existing tables, so its negative-scale rule (task 1.3b) stays reachable.
- **Promotes to ADR:** no

### [3] Each named type covers all of its variants

- **Decision:** The interview's type list applies to every variant of each named type. A Dictionary is accepted with any integer key type and Utf8 or LargeUtf8 values. FixedSizeBinary and BinaryView map to `VARCHAR(2000000)` hex text, as Binary does. An empty or non-string `adbc.ingest.target_catalog` fails as any other non-`EXA` value does.
- **Rationale:** The either-or rule needs a verdict for every variant. LargeUtf8 dictionary values map to the same Exasol type as Utf8 values, and rejecting them would split one string type by its offset width.
- **Alternatives:** Map FixedSizeBinary(n) to `HASHTYPE(n BYTE)`. Rejected, because Binary already maps to hex text in VARCHAR and one rule for all binary types is simpler to document. Confirm the dictionary scope in PR review.
- **Promotes to ADR:** no

### [4] Interval values convert without loss, into columns of leading precision 9

- **Decision:** `arrow_to_exasol` maps Interval(YearMonth) to INTERVAL YEAR TO MONTH and Interval(DayTime) and Interval(MonthDayNano) to INTERVAL DAY TO SECOND(3), and `ExasolType::to_ddl_type` writes these as `INTERVAL YEAR(9) TO MONTH` and `INTERVAL DAY(9) TO SECOND(p)`. The import CSV conversion rejects a day-time value with a fraction of a millisecond or more than 999,999,999 days, and a MonthDayNano value with a month component. The INTERVAL text writer sits in `src/types/conversion.rs` next to `parse_interval`.
- **Rationale:** Exasol stores INTERVAL DAY TO SECOND values in whole milliseconds, so truncating or rounding a sub-millisecond part would lose data without an error. The default leading precision 2 holds at most 99 years or days. Precision 9 holds every Interval(YearMonth) value, because an `i32` month count spans less than 179,000,000 years, but it holds fewer days than the `i32` range, so larger day counts fail. One module then owns the INTERVAL text format in both directions, as it owns the DATE and TIMESTAMP text. The written text follows the form that Exasol sends and `parse_interval` reads, without zero padding. The Exasol docs do not describe the CSV text of interval values, so the integration test of the `import-export/arrow-recordbatch` interval scenario pins it.
- **Alternatives:** Truncate the sub-millisecond part. Rejected, because the user asked for no silent data loss. Add a leading-precision field to the public `ExasolType::IntervalYearToMonth` and `IntervalDayToSecond` variants. Rejected, because it breaks the public enum and its match sites in 9 source files, while only DDL generation needs the leading precision.
- **Consequences:** A value read from an INTERVAL YEAR TO MONTH column arrives as Interval(MonthDayNano) with a month component, so writing it back needs a cast to Interval(YearMonth) first. Native Parquet import with table creation rejects Interval fields, as it rejects the four types of the review finding below, because nothing verifies what Exasol's native Parquet reader loads from a Parquet INTERVAL column. arrow-rs cannot write Interval(MonthDayNano) to Parquet, so a Parquet file holds only the other two units. CSV-path Parquet import rejects an Interval field from a file without an Arrow schema; see `[plan-review] CSV-path Parquet import rejects an Interval field from a file without an Arrow schema`. A value that the import CSV conversion rejects fails ADBC ingest after the DDL; see decision [5].
- **Promotes to ADR:** no

### [5] Value conversion stays after the DDL

- **Decision:** ADBC bulk ingest runs the target catalog check and the column type check before its first SQL statement, and converts the bound values with the import CSV conversion after the DDL of create, create-append, and replace mode, as it does today.
- **Rationale:** The user kept today's order in the PR review. The column type check removes the #111 defect, in which a column type that the import CSV conversion could not convert failed after the DDL. Replace mode exists so that the new data alone defines the column types of the new table, with no influence from the old table.
- **Alternatives:** Convert every bound value to CSV before the first SQL statement, so a value error leaves every table unchanged. The bound input is already one in-memory RecordBatch, so this would add no memory. Rejected by the user in the PR review.
- **Consequences:** A value that the import CSV conversion rejects, such as an Interval(MonthDayNano) value with a month component or a date outside 0001-01-01 to 9999-12-31, fails a replace-mode ingest after the old table is dropped, and leaves an empty table in create mode. A value read from an INTERVAL YEAR TO MONTH column and ingested back without a cast to Interval(YearMonth) hits this case. The user accepted this, and `docs/driver-manager.md` warns about it (task 3.2). Only the type and catalog checks guarantee that a rejected ingest changes no table.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Native Parquet auto-create keeps rejecting the newly mapped types

- **Finding:** Parquet auto-create maps the Arrow schema with `arrow_to_exasol` before `import_from_parquet` and `import_from_parquet_files` branch on `use_native`. After task 1.1, the native path would create `VARCHAR(2000000)` columns for Utf8View, Dictionary, BinaryView, and FixedSizeBinary fields. Nothing showed what Exasol's native Parquet reader loads into those columns, so the IMPORT could fail after `CREATE TABLE` or store values that differ from the CSV path.
- **Direction change:** Native Parquet import with table creation rejects these four types with `ImportError::SchemaInferenceError` before `CREATE TABLE`, as it does today, and the message names `with_native_parquet(Some(false))`. The check sits in `src/import/parquet.rs`, which owns the path choice, so the public Parquet schema inference functions keep their signatures. The rejected alternative was to run the native import against Exasol and pin the created columns and stored values in a scenario. Planning does not run the software, and an unverified expected value would leave the failure open.
- **Promotes to ADR:** no

### [plan-review] One formatter dispatch decides which Arrow types convert

- **Finding:** The `match` in `format_value`, the new schema check, and `TypeMapper::arrow_to_exasol` each listed the convertible Arrow types by hand, and only a unit test with a hand-written sample list kept them in agreement.
- **Direction change:** The import CSV conversion resolves the value formatter of each column from its `DataType` once, in one function of `src/import/arrow.rs`. The schema check calls that function for every field, and the row formatter uses the formatters it returns, so the check cannot disagree with the import CSV conversion. The unit test compares only `arrow_to_exasol` with that function.
- **Promotes to ADR:** no

### [plan-review] CSV-path Parquet import rejects an Interval field from a file without an Arrow schema

- **Finding:** A Parquet INTERVAL column holds months, days, and milliseconds. Without an `ARROW:schema` footer entry, the Parquet reader of arrow-rs reads it as Interval(DayTime) and drops the month part, so after this plan the CSV path would import a 1-month value as `0 00:00:00.000` without an error.
- **Direction change:** CSV-path Parquet import rejects an Interval field with `ImportError::SchemaInferenceError` when the footer holds no Arrow schema, in `open_parquet_csv_reader` and, with table creation, before `CREATE TABLE` (task 1.9). A file written with its Arrow schema still imports its Interval columns on the CSV path. The user chose this over the rejected alternative of rejecting Interval fields in every Parquet import.
- **Promotes to ADR:** no

### [plan-review] The native Parquet auto-create check is an allow-list

- **Finding:** The native Parquet auto-create check listed by hand the types it rejects. That deny-list repeated knowledge that `src/types/mapping.rs` owns, and a type that a later change maps in `arrow_to_exasol` would pass native auto-create unverified.
- **Direction change:** The check accepts only an allow-list named once in `src/import/parquet.rs`: the Arrow types that native auto-create accepted before this plan, minus Interval. Every other field fails with `ImportError::SchemaInferenceError` before `CREATE TABLE` (task 1.7).
- **Promotes to ADR:** no
