# Feature: Parquet CSV-Bytes Export

Specifies the CSV-bytes Parquet entry points (`export_to_parquet`, `export_to_parquet_stream`, and `csv_to_record_batches` in `export::parquet`). They parse caller-supplied CSV against a caller-supplied Arrow schema.

## Background

The CSV-bytes entry points (`export_to_parquet`, `export_to_parquet_stream`, and `csv_to_record_batches` in `export::parquet`) parse caller-supplied CSV against a caller-supplied Arrow schema. A quoted field can contain the column separator, the column delimiter, and line breaks.

The CSV-bytes entry points keep every field verbatim: an empty field is NULL when `null_value` is unset, and a field equal to `null_value` is NULL. They do not trim whitespace. The transport export is specified in `import-export/parquet-export`.

## Scenarios

### Scenario: Export to Parquet stream

* *GIVEN* a synchronous `std::io::Write + Send` implementation is available for Parquet output
* *WHEN* user provides that `Write` implementation to `export_to_parquet_stream`
* *THEN* system SHALL stream Parquet data to writer

### Scenario: CSV-bytes export keeps field whitespace

* *GIVEN* an Arrow schema with an `Int64` field `ID` and a `Utf8` field `NAME`
* *AND* CSV bytes whose `NAME` fields are `  padded  ` in quotes, three unquoted spaces, and an empty field, one per row
* *WHEN* the caller converts the bytes with `export_to_parquet_stream` and no `null_value` set
* *THEN* the `NAME` values `  padded  ` and three spaces SHALL read back unchanged and MUST NOT read back as NULL
* *AND* the empty field SHALL read back as NULL
* *AND* IF an `ID` field carries leading or trailing whitespace, such as ` 7`, the conversion SHALL fail with `ParquetExportError::CsvParse` whose row is the 0-based index of that data row in the whole input, header excluded, instead of trimming the field

### Scenario: CSV-bytes export reads only the null_value marker as NULL

* *GIVEN* an Arrow schema with an `Int64` field `ID` and a `Utf8` field `NAME`
* *AND* CSV bytes whose `NAME` fields are the four-letter string `NULL` and an empty field, one per row
* *WHEN* the caller converts the bytes with `export_to_parquet_stream` and `ParquetExportOptions::with_null_value("NULL")` set
* *THEN* the `NAME` field `NULL` SHALL read back as NULL
* *AND* the empty `NAME` field SHALL read back as an empty string, and MUST NOT read back as NULL

### Scenario: CSV-bytes export accepts line breaks inside quoted fields

* *GIVEN* an Arrow schema with an `Int64` field `ID` and a `Utf8` field `NAME`
* *AND* CSV bytes holding two records, the first of which has a quoted `NAME` field that contains a line feed, and the second of which has a quoted `NAME` field that contains the column separator
* *WHEN* the caller converts the bytes with `csv_to_record_batches` with `with_column_names(false)`
* *THEN* the result SHALL hold exactly two rows
* *AND* each `NAME` value SHALL equal the quoted field's content, including the line feed and the column separator
