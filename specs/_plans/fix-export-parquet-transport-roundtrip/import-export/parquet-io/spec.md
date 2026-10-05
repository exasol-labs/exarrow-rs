# Feature: Parquet I/O

Specifies Parquet file import and export capabilities. Import streams Parquet bytes through the HTTP transport tunnel either natively (Exasol 2025.1.11+) or after CSV conversion (older servers). Export continues to receive CSV from Exasol and convert to Parquet locally.

<!-- DELTA:CHANGED -->
## Background

Parquet import operates over the same HTTP tunnel as CSV import but selects between two on-the-wire transport models based on the connected server's `release_version`:

- On Exasol 2025.1.11 and newer the server requests the Parquet file from the driver using **HTTP range requests**. The driver responds to `HEAD` with `200 OK` plus `Content-Length` and to `GET` with `Range: bytes=X-Y` using `206 Partial Content` carrying the requested byte slice. Multiple sequential HEAD/GET-Range requests typically arrive on the same connection (footer first, then row groups) until the server closes it. The generated SQL is `IMPORT INTO ... FROM PARQUET AT '...;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '...parquet'`.
- On older servers the driver reads each Parquet `RecordBatch`, converts to CSV via `record_batch_to_csv`, streams the CSV body through the existing chunked-encoding response, and emits `FROM CSV ... FILE '...csv'`.

The Parquet variant of the IMPORT statement omits all CSV format options (no `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, `REJECT LIMIT`) and never emits the `MULTIPLE LOCAL FILES` tag (Exasol opens one HTTP server per file for native Parquet import). The `;MaxConcurrentReads=1` suffix is appended inside the `AT '...'` URL of every file entry, matching the JDBC reference behavior. Path selection is automatic by default and can be overridden via `ParquetImportOptions::with_native_parquet(Some(true|false))`.

Export receives CSV from Exasol, converts it to Parquet locally, and writes to files or streams. It has two kinds of entry point:

- The transport export (`Connection::export_to_parquet` and `export_to_parquet_via_transport`) reads the column names and Exasol types of the export source before it runs the EXPORT statement. It prepares the SELECT statement that the source describes (`SELECT <columns or *> FROM [schema.]table` for a table source, the query text for a query source), reads the result-set column metadata specified in `prepared-statements/result-columns`, and closes the prepared statement.
- The CSV-bytes entry points (`export_to_parquet`, `export_to_parquet_stream`, and `csv_to_record_batches` in `export::parquet`) parse caller-supplied CSV against a caller-supplied Arrow schema. A quoted field can contain the column separator, the column delimiter, and line breaks.

The transport export maps BOOLEAN, CHAR, VARCHAR, DECIMAL, DOUBLE, DATE, and TIMESTAMP columns as `type-mapping/exasol-to-arrow` specifies. It maps TIMESTAMP WITH LOCAL TIME ZONE columns to `Timestamp(Microsecond, None)`, a timestamp without a time zone that holds the wall-clock value in the session time zone. It maps INTERVAL YEAR TO MONTH, INTERVAL DAY TO SECOND, GEOMETRY, and HASHTYPE columns to Utf8 holding the CSV text Exasol writes for the value. `exasol_types_to_arrow_schema` applies the same mapping.

Exasol writes NULL as an empty field, and Exasol stores an empty string as NULL. The transport export therefore reads an empty field as NULL and keeps every other field verbatim. It does not apply `ParquetExportOptions::null_value`, because the driver passes no NULL clause to the EXPORT statement. The CSV-bytes entry points keep every field verbatim as well: an empty field is NULL when `null_value` is unset, and a field equal to `null_value` is NULL. Neither kind of entry point trims whitespace. Typed values are parsed in the text form of Exasol's default session formats, for example `YYYY-MM-DD` for `NLS_DATE_FORMAT`.

The HTTP-transport TLS knob is exposed as `use_tls(bool)` on both option builders.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Export preserves schema

* *GIVEN* a session whose time zone is set to `EUROPE/BERLIN`, and a table `T` with columns `ID DECIMAL(18,0)`, `NAME VARCHAR(100)`, `PRICE DECIMAL(10,2)`, `ACTIVE BOOLEAN`, `CREATED DATE`, `UPDATED TIMESTAMP`, `RATIO DOUBLE`, and `TS_LTZ TIMESTAMP WITH LOCAL TIME ZONE`, holding rows with a value in every column, including the `PRICE` values `0.50` and `-0.50`, and one row that is NULL in every column except `ID`
* *WHEN* the caller exports `T` to a Parquet file with `Connection::export_to_parquet` on that session
* *THEN* the Parquet schema SHALL list the fields `ID`, `NAME`, `PRICE`, `ACTIVE`, `CREATED`, `UPDATED`, `RATIO`, and `TS_LTZ` in table column order, and MUST NOT contain positional placeholder names such as `col0`
* *AND* the fields SHALL have the Arrow types `Decimal128(18, 0)`, `Utf8`, `Decimal128(10, 2)`, `Boolean`, `Date32`, `Timestamp(Microsecond, None)`, `Float64`, and `Timestamp(Microsecond, None)`, in that order
* *AND* every value read back from a column other than `TS_LTZ` SHALL equal the stored value, and every stored NULL SHALL read back as NULL
* *AND* every non-NULL `TS_LTZ` value read back SHALL equal the wall-clock time in the text that `Connection::export_csv_to_list` returns for the same row on the same session, with no time-zone shift
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Query export names fields after the select list

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)` holding at least one row
* *WHEN* the caller exports the query `SELECT ID AS ITEM_ID, UPPER(NAME) FROM T` to a Parquet file
* *THEN* the first field SHALL be named `ITEM_ID` and SHALL be typed `Decimal128(18, 0)`
* *AND* the second field SHALL carry the name Exasol assigns to the derived column, SHALL NOT be empty or a positional placeholder, and SHALL be typed `Utf8`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Values containing the separator, the delimiter, or a line break export intact

* *GIVEN* a table with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)`
* *AND* the `NAME` values `Smith, John`, `say "hi"`, a value with an embedded line feed, a value with an embedded carriage return and line feed, a value with an embedded carriage return that no line feed follows, and a value that ends with a carriage return, one per row
* *WHEN* the caller exports the table to a Parquet file with the default column separator and column delimiter
* *THEN* the export SHALL succeed and the file SHALL contain exactly one row per source row
* *AND* every `NAME` value read back SHALL equal its stored value exactly
* *AND* every `ID` value SHALL stay paired with the `NAME` value of its own source row
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Exported text values keep their whitespace

* *GIVEN* a table with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)`
* *AND* the `NAME` values `  padded  `, a value of three spaces, the four-letter string `NULL`, and a NULL, one per row
* *WHEN* the caller exports the table to a Parquet file with `ParquetExportOptions::with_null_value("NULL")` set
* *THEN* the values `  padded  ` and three spaces SHALL read back unchanged, and MUST NOT read back trimmed or as NULL
* *AND* the four-letter string `NULL` SHALL read back as the string `NULL`, because the transport export does not apply `null_value`
* *AND* the stored NULL SHALL read back as NULL
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Columns without a typed CSV conversion export as text

* *GIVEN* a table with columns `IYM INTERVAL YEAR TO MONTH`, `IDS INTERVAL DAY TO SECOND`, `G GEOMETRY`, and `H HASHTYPE` holding one row with a value in every column
* *WHEN* the caller exports the table to a Parquet file
* *THEN* the export SHALL succeed and the fields `IYM`, `IDS`, `G`, and `H` SHALL be typed `Utf8`
* *AND* each value read back SHALL equal the text that `Connection::export_csv_to_list` returns for the same row and column
* *AND* `exasol_types_to_arrow_schema` SHALL map these four Exasol types to `Utf8` instead of returning an error
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Empty export writes a Parquet file that carries the schema

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(100)`
* *WHEN* the caller exports the query `SELECT ID, NAME FROM T WHERE FALSE` to a Parquet file
* *THEN* the export SHALL report zero exported rows
* *AND* the system SHALL write a Parquet file that contains zero rows
* *AND* the file's schema SHALL list the fields `ID` typed `Decimal128(18, 0)` and `NAME` typed `Utf8`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Export source that produces no result set is rejected before the export runs

* *GIVEN* a table `T` holding rows
* *WHEN* the caller exports the query source `DELETE FROM T` to a Parquet file
* *THEN* the system SHALL return `ExportError::SqlExecutionError` with a message stating that the export source produces no result set
* *AND* the system MUST NOT execute the EXPORT statement or the source statement, so `T` SHALL keep all of its rows
* *AND* the system MUST NOT create the output file
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Export releases the schema prepared statement before the EXPORT statement runs

* *GIVEN* a transport whose prepared statement for the export source reports result-set columns
* *WHEN* the caller runs a transport export
* *THEN* the system SHALL prepare the source's SELECT statement, then close that prepared statement, then execute the EXPORT statement, in that order and once each
* *AND* IF the export source produces no result set, the system SHALL still close the prepared statement before it returns the error
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A value that does not match its column type fails the export

* *GIVEN* a table with a `DATE` column holding at least one non-NULL value
* *AND* a session that sets `NLS_DATE_FORMAT` to `DD.MM.YYYY`, so Exasol writes dates in a text form the driver does not parse
* *WHEN* the caller exports the table to a Parquet file on that session
* *THEN* the system SHALL return an error whose message includes the column position and the value as Exasol wrote it
* *AND* the system MUST NOT create the output file
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: CSV-bytes export keeps field whitespace

* *GIVEN* an Arrow schema with an `Int64` field `ID` and a `Utf8` field `NAME`
* *AND* CSV bytes whose `NAME` fields are `  padded  ` in quotes, three unquoted spaces, and an empty field, one per row
* *WHEN* the caller converts the bytes with `export_to_parquet_stream` and no `null_value` set
* *THEN* the `NAME` values `  padded  ` and three spaces SHALL read back unchanged and MUST NOT read back as NULL
* *AND* the empty field SHALL read back as NULL
* *AND* IF an `ID` field carries leading or trailing whitespace, such as ` 7`, the conversion SHALL fail with `ParquetExportError::CsvParse` whose row is the 0-based index of that data row in the whole input, header excluded, instead of trimming the field
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: CSV-bytes export reads only the null_value marker as NULL

* *GIVEN* an Arrow schema with an `Int64` field `ID` and a `Utf8` field `NAME`
* *AND* CSV bytes whose `NAME` fields are the four-letter string `NULL` and an empty field, one per row
* *WHEN* the caller converts the bytes with `export_to_parquet_stream` and `ParquetExportOptions::with_null_value("NULL")` set
* *THEN* the `NAME` field `NULL` SHALL read back as NULL
* *AND* the empty `NAME` field SHALL read back as an empty string, and MUST NOT read back as NULL
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: CSV-bytes export accepts line breaks inside quoted fields

* *GIVEN* an Arrow schema with an `Int64` field `ID` and a `Utf8` field `NAME`
* *AND* CSV bytes holding two records, the first of which has a quoted `NAME` field that contains a line feed, and the second of which has a quoted `NAME` field that contains the column separator
* *WHEN* the caller converts the bytes with `csv_to_record_batches` with `with_column_names(false)`
* *THEN* the result SHALL hold exactly two rows
* *AND* each `NAME` value SHALL equal the quoted field's content, including the line feed and the column separator
<!-- /DELTA:NEW -->
