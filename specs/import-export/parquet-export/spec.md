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

### Scenario: Export table to Parquet file

* *GIVEN* an Exasol table contains data to export
* *WHEN* user calls export_to_parquet with table name and file path
* *THEN* system SHALL receive CSV from Exasol and convert to Parquet
* *AND* system SHALL write Parquet file

### Scenario: Export preserves schema

* *GIVEN* a session whose time zone is set to `EUROPE/BERLIN`, and a table `T` with columns `ID DECIMAL(18,0)`, `NAME VARCHAR(100)`, `PRICE DECIMAL(10,2)`, `ACTIVE BOOLEAN`, `CREATED DATE`, `UPDATED TIMESTAMP`, `RATIO DOUBLE`, and `TS_LTZ TIMESTAMP WITH LOCAL TIME ZONE`, holding rows with a value in every column, including the `PRICE` values `0.50` and `-0.50`, and one row that is NULL in every column except `ID`
* *WHEN* the caller exports `T` to a Parquet file with `Connection::export_to_parquet` on that session
* *THEN* the Parquet schema SHALL list the fields `ID`, `NAME`, `PRICE`, `ACTIVE`, `CREATED`, `UPDATED`, `RATIO`, and `TS_LTZ` in table column order, and MUST NOT contain positional placeholder names such as `col0`
* *AND* the fields SHALL have the Arrow types `Decimal128(18, 0)`, `Utf8`, `Decimal128(10, 2)`, `Boolean`, `Date32`, `Timestamp(Microsecond, None)`, `Float64`, and `Timestamp(Microsecond, None)`, in that order
* *AND* every value read back from a column other than `TS_LTZ` SHALL equal the stored value, and every stored NULL SHALL read back as NULL
* *AND* every non-NULL `TS_LTZ` value read back SHALL equal the wall-clock time in the text that `Connection::export_csv_to_list` returns for the same row on the same session, with no time-zone shift

### Scenario: Query export names fields after the select list

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)` holding at least one row
* *WHEN* the caller exports the query `SELECT ID AS ITEM_ID, UPPER(NAME) FROM T` to a Parquet file
* *THEN* the first field SHALL be named `ITEM_ID` and SHALL be typed `Decimal128(18, 0)`
* *AND* the second field SHALL carry the name Exasol assigns to the derived column, SHALL NOT be empty or a positional placeholder, and SHALL be typed `Utf8`

### Scenario: Values containing the separator, the delimiter, or a line break export intact

* *GIVEN* a table with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)`
* *AND* the `NAME` values `Smith, John`, `say "hi"`, a value with an embedded line feed, a value with an embedded carriage return and line feed, a value with an embedded carriage return that no line feed follows, and a value that ends with a carriage return, one per row
* *WHEN* the caller exports the table to a Parquet file with the default column separator and column delimiter
* *THEN* the export SHALL succeed and the file SHALL contain exactly one row per source row
* *AND* every `NAME` value read back SHALL equal its stored value exactly
* *AND* every `ID` value SHALL stay paired with the `NAME` value of its own source row

### Scenario: Exported text values keep their whitespace

* *GIVEN* a table with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)`
* *AND* the `NAME` values `  padded  `, a value of three spaces, the four-letter string `NULL`, and a NULL, one per row
* *WHEN* the caller exports the table to a Parquet file with `ParquetExportOptions::with_null_value("NULL")` set
* *THEN* the values `  padded  ` and three spaces SHALL read back unchanged, and MUST NOT read back trimmed or as NULL
* *AND* the four-letter string `NULL` SHALL read back as the string `NULL`, because the transport export does not apply `null_value`
* *AND* the stored NULL SHALL read back as NULL

### Scenario: Columns without a typed CSV conversion export as text

* *GIVEN* a table with columns `IYM INTERVAL YEAR TO MONTH`, `IDS INTERVAL DAY TO SECOND`, `G GEOMETRY`, and `H HASHTYPE` holding one row with a value in every column
* *WHEN* the caller exports the table to a Parquet file
* *THEN* the export SHALL succeed and the fields `IYM`, `IDS`, `G`, and `H` SHALL be typed `Utf8`
* *AND* each value read back SHALL equal the text that `Connection::export_csv_to_list` returns for the same row and column
* *AND* `exasol_types_to_arrow_schema` SHALL map these four Exasol types to `Utf8` instead of returning an error

### Scenario: Empty export writes a Parquet file that carries the schema

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)`
* *WHEN* the caller exports the query `SELECT ID, NAME FROM T WHERE FALSE` to a Parquet file
* *THEN* the export SHALL report zero exported rows
* *AND* the system SHALL write a Parquet file that contains zero rows
* *AND* the file's schema SHALL list the fields `ID` typed `Decimal128(18, 0)` and `NAME` typed `Utf8`

### Scenario: Export source that produces no result set is rejected before the export runs

* *GIVEN* a table `T` holding rows
* *WHEN* the caller exports the query source `DELETE FROM T` to a Parquet file
* *THEN* the system SHALL return `ExportError::SqlExecutionError` with a message stating that the export source produces no result set
* *AND* the system MUST NOT execute the EXPORT statement or the source statement, so `T` SHALL keep all of its rows
* *AND* the system MUST NOT create the output file

### Scenario: Export releases the schema prepared statement before the EXPORT statement runs

* *GIVEN* a transport whose prepared statement for the export source reports result-set columns
* *WHEN* the caller runs a transport export
* *THEN* the system SHALL prepare the source's SELECT statement, then close that prepared statement, then execute the EXPORT statement, in that order and once each
* *AND* IF the export source produces no result set, the system SHALL still close the prepared statement before it returns the error

### Scenario: A value that does not match its column type fails the export

* *GIVEN* a table with a `DATE` column holding at least one non-NULL value
* *AND* a session that sets `NLS_DATE_FORMAT` to `DD.MM.YYYY`, so Exasol writes dates in a text form the driver does not parse
* *WHEN* the caller exports the table to a Parquet file on that session
* *THEN* the system SHALL return an error whose message includes the column position and the value as Exasol wrote it
* *AND* the system MUST NOT create the output file
