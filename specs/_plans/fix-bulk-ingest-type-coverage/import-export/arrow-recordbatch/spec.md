# Feature: Arrow RecordBatch Import/Export

Specifies Arrow RecordBatch import and export between applications and Exasol tables.

## Background

RecordBatch import converts data to CSV and streams it through the HTTP tunnel. Export converts the CSV from Exasol into RecordBatches whose schemas reflect Exasol column types.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: RecordBatch CSV conversion covers every Arrow type that has an Exasol column type

* *GIVEN* a RecordBatch with one column of each Arrow type that `TypeMapper::arrow_to_exasol` maps to an Exasol column type, plus a `Decimal128(38, 2)` column
* *WHEN* the RecordBatch is converted to CSV for `import_from_record_batch`
* *THEN* every column SHALL convert without an error, a Date64 value SHALL become the `YYYY-MM-DD` text of the day that holds it, so `1_577_836_800_000` ms becomes `2020-01-01`, and a Decimal256 value SHALL become its decimal text with the column's scale, so `-123456` at scale 2 becomes `-1234.56`
* *AND* a Dictionary value SHALL become the text of the dictionary value its key refers to, a null key or a null dictionary value SHALL become the null marker, and a Utf8View value SHALL be written as the same Utf8 value is written
* *AND* a BinaryView or FixedSizeBinary value SHALL become the lowercase hexadecimal text of its bytes, as a Binary value does
* *AND* a column whose Arrow type has no conversion, such as `Time32(Second)` or `Interval(MonthDayNano)`, SHALL fail the conversion with `ImportError::ConversionError` that names the Arrow type
<!-- /DELTA:NEW -->
