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

## Design Decisions

### [1] CSV-path Parquet import uses the RecordBatch CSV conversion

- **Decision:** `src/import/parquet.rs` stops formatting values itself. Its `record_batch_to_csv` formats each row with the row formatter in `src/import/arrow.rs`, and its own formatter and helpers are deleted.
- **Rationale:** `TypeMapper::arrow_to_exasol` also feeds Parquet auto-create (`src/types/infer.rs`). Once it maps the six added types, a CSV-path Parquet auto-create would create the table and then fail in the Parquet formatter, which is the #111 defect in a second entry point. One conversion removes the duplicate type knowledge. The new dependency runs from the Parquet CSV path to the RecordBatch conversion inside the same import module, so it adds no module and no cycle. The two formatters already disagree: the Parquet one drops the sign of a negative decimal below 1 (`-0.05` becomes `0.05`) and quotes on `,` instead of the configured separator.
- **Alternatives:** Add the six types to both formatters. Rejected, because the two type lists can drift again and the two existing bugs stay. Leave `arrow_to_exasol` unchanged for Parquet and keep a second mapping for ingest. Rejected, because two mappings of the same Arrow types repeat the leak this plan removes.
- **Consequences:** CSV-path Parquet import writes floats without an exponent, keeps the sign of negative decimals, and quotes values that contain the configured separator. Native Parquet import into an existing table does not change. Native Parquet import with table creation rejects Utf8View, Dictionary, BinaryView, and FixedSizeBinary fields before `CREATE TABLE`, as it does today; see `[plan-review] Native Parquet auto-create keeps rejecting the newly mapped types`.
- **Promotes to ADR:** no

### [2] Append checks CSV conversion, table creation checks the column type mapping

- **Decision:** The ingest check depends on the mode. Append needs a type that the CSV conversion converts. Create, create-append, and replace need a type that `arrow_to_exasol` maps. The two sets differ only in Decimal128 and Decimal256 with a precision above 36 or a scale outside 0 to the precision, which the conversion converts and the mapping rejects.
- **Rationale:** Exasol rejects `DECIMAL(p,s)` with p above 36, so the driver cannot create such a column. Appending a `Decimal128(38, s)` batch, a common precision in Arrow data from other engines, into an existing `DECIMAL(36,s)` column works today when the values fit, and must keep working.
- **Alternatives:** One mapping check in every mode. Rejected, because it breaks these appends. Map precision 37 and 38 to `DECIMAL(36, s)`. Rejected, because the driver would silently declare a narrower column than the bound schema.
- **Promotes to ADR:** no

### [3] Each named type covers all of its variants

- **Decision:** The interview's type list applies to every variant of each named type. Every Interval unit is rejected, not only `MonthDayNano`. A Dictionary is accepted with any integer key type and Utf8 or LargeUtf8 values. FixedSizeBinary and BinaryView map to `VARCHAR(2000000)` hex text, as Binary does. An empty or non-string `adbc.ingest.target_catalog` fails as any other non-`EXA` value does.
- **Rationale:** The either-or rule needs a verdict for every variant, and neither CSV conversion converts any Interval unit. LargeUtf8 dictionary values map to the same Exasol type as Utf8 values, and rejecting them would split one string type by its offset width.
- **Alternatives:** Map FixedSizeBinary(n) to `HASHTYPE(n BYTE)`. Rejected, because Binary already maps to hex text in VARCHAR and one rule for all binary types is simpler to document. Confirm the Interval and dictionary scope in PR review.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Native Parquet auto-create keeps rejecting the newly mapped types

- **Finding:** Parquet auto-create maps the Arrow schema with `arrow_to_exasol` before `import_from_parquet` and `import_from_parquet_files` branch on `use_native`. After task 1.1, the native path would create `VARCHAR(2000000)` columns for Utf8View, Dictionary, BinaryView, and FixedSizeBinary fields. Nothing showed what Exasol's native Parquet reader loads into those columns, so the IMPORT could fail after `CREATE TABLE` or store values that differ from the CSV path.
- **Direction change:** Native Parquet import with table creation rejects these four types with `ImportError::SchemaInferenceError` before `CREATE TABLE`, as it does today, and the message names `with_native_parquet(Some(false))`. The check sits in `src/import/parquet.rs`, which owns the path choice, so the public Parquet schema inference functions keep their signatures. The rejected alternative was to run the native import against Exasol and pin the created columns and stored values in a scenario. Planning does not run the software, and an unverified expected value would leave the failure open.
- **Promotes to ADR:** no
