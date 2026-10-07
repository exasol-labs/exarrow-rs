# Feature: Parquet Export

Specifies the transport Parquet export (`Connection::export_to_parquet` and `export_to_parquet_via_transport`). It takes column names and types from the export source and writes a Parquet file from the CSV that Exasol sends.

## Background

Export receives CSV from Exasol, converts it to Parquet locally, and writes to files or streams. It has two kinds of entry point. The CSV-bytes entry points are specified in `import-export/parquet-csv-bytes`:

- The transport export (`Connection::export_to_parquet` and `export_to_parquet_via_transport`) reads the column names and Exasol types of the export source before it runs the EXPORT statement. It prepares the SELECT statement that the source describes (`SELECT <columns or *> FROM [schema.]table` for a table source, the query text for a query source), reads the result-set column metadata specified in `prepared-statements/result-columns`, and closes the prepared statement.
- The CSV-bytes entry points (`export_to_parquet`, `export_to_parquet_stream`, and `csv_to_record_batches` in `export::parquet`) parse caller-supplied CSV against a caller-supplied Arrow schema. A quoted field can contain the column separator, the column delimiter, and line breaks.

The transport export maps BOOLEAN, CHAR, VARCHAR, DECIMAL, DOUBLE, DATE, and TIMESTAMP columns as `type-mapping/exasol-to-arrow` specifies. It maps TIMESTAMP WITH LOCAL TIME ZONE columns to `Timestamp(Microsecond, None)`, a timestamp without a time zone that holds the wall-clock value in the session time zone. It maps INTERVAL YEAR TO MONTH, INTERVAL DAY TO SECOND, GEOMETRY, and HASHTYPE columns to Utf8 holding the CSV text Exasol writes for the value. `exasol_types_to_arrow_schema` applies the same mapping.

Exasol writes NULL as an empty field, and Exasol stores an empty string as NULL. The transport export therefore reads an empty field as NULL and keeps every other field verbatim. It does not apply `ParquetExportOptions::null_value`, because the driver passes no NULL clause to the EXPORT statement. The CSV-bytes entry points keep every field verbatim as well: an empty field is NULL when `null_value` is unset, and a field equal to `null_value` is NULL. Neither kind of entry point trims whitespace. Typed values are parsed in the text form of Exasol's default session formats, for example `YYYY-MM-DD` for `NLS_DATE_FORMAT`.

The HTTP-transport TLS knob is exposed as `use_tls(bool)` on both option builders.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Export keeps pre-1970 DATE and TIMESTAMP values

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)`, `D DATE`, and `TS TIMESTAMP(6)`, holding the rows `(1, DATE '1968-01-01', TIMESTAMP '1950-06-15 00:00:00')` and `(2, DATE '0001-01-01', TIMESTAMP '1969-12-31 23:59:59.999999')`
* *WHEN* the application calls `Connection::export_to_parquet` for table `T`
* *THEN* the `D` field of the Parquet file SHALL hold the Date32 value -731 for `ID` 1 and -719162 for `ID` 2
* *AND* the `TS` field of the Parquet file SHALL hold the microsecond value -616896000000000 for `ID` 1 and -1 for `ID` 2
<!-- /DELTA:NEW -->
