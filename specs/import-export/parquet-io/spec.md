# Feature: Parquet I/O

Specifies Parquet file import through the HTTP tunnel, natively on Exasol 2025.1.11 and newer or after CSV conversion on older servers. Parquet export is specified in `import-export/parquet-export` and `import-export/parquet-csv-bytes`.

## Background

- The driver selects the path from the connected server's `release_version`. On 2025.1.11 and newer, the server requests the Parquet file with HTTP range requests (footer first, then row groups). On older servers, the driver converts each `RecordBatch` to CSV with `record_batch_to_csv` and streams the CSV body.
- The Parquet IMPORT statement omits all CSV format options and the `MULTIPLE LOCAL FILES` tag, because Exasol opens one HTTP server per file for native Parquet import.

## Scenarios

### Scenario: Import Parquet file into table

* *GIVEN* a Parquet file exists on disk
* *AND* the connected Exasol server's `release_version` is below `2025.1.11`
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* system SHALL read the Parquet file, convert each RecordBatch to CSV, and stream the CSV body through the HTTP tunnel
* *AND* the generated IMPORT SQL SHALL use the `FROM CSV ... FILE 'NNN.csv'` clause

### Scenario: Native Parquet import on Exasol 2025.1.11+

* *GIVEN* a Parquet file exists on disk
* *AND* the connected Exasol server's `release_version` is at or above `2025.1.11`
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* the system MUST serve the raw Parquet bytes from the open file through the range-request handler, MUST read only the requested bytes of each range from the file, in pieces of at most 64 KiB, without holding the whole file or a whole requested range in memory at once, and MUST NOT invoke any CSV conversion path
* *AND* the range-request handler MUST take the file size from the file once, before it answers the first request, and MUST reply to `HEAD /<file>` with `200 OK` plus `Content-Length: <file size>` and to `GET /<file>` with `Range: bytes=<start>-<end>` headers using `206 Partial Content` carrying the requested byte slice, looping until Exasol closes the connection
* *AND* the generated IMPORT SQL MUST use the `FROM PARQUET AT 'http(s)://addr;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE 'NNN.parquet'` form, with the `.parquet` extension and without any CSV format option clauses or the `MULTIPLE LOCAL FILES` tag

### Scenario: Import Parquet from stream

* *GIVEN* Parquet data is available as an async stream
* *WHEN* user provides AsyncRead for Parquet data
* *THEN* the system SHALL buffer the stream into a `Vec<u8>` in memory before serving it because Parquet requires footer random access
* *AND* on Exasol 2025.1.11+ the system SHALL serve the buffered Parquet bytes via HTTP range requests (no CSV conversion) and SHALL emit `FROM PARQUET AT '...;MaxConcurrentReads=1' ... FILE '...parquet'` SQL
* *AND* on servers below 2025.1.11 the system SHALL parse the Parquet, convert to CSV, and emit `FROM CSV` SQL as before

### Scenario: Import Parquet preserves data types

* *GIVEN* a Parquet file contains typed columns
* *WHEN* the file is imported via the native (2025.1.11+) path
* *THEN* the Parquet bytes SHALL reach Exasol unchanged so type fidelity SHALL be governed by the server's Parquet reader
* *AND* on the legacy CSV-conversion path the system SHALL convert types to CSV-compatible representations and SHALL preserve NULL values per the existing CSV mapping

### Scenario: Native Parquet import option overrides the server-version probe

* *GIVEN* an application that wants to force-test a specific import path
* *WHEN* `ParquetImportOptions::with_native_parquet(Some(true|false))` is set explicitly
* *THEN* the driver MUST honor that override regardless of the server's reported `release_version`
* *AND* when the override is `None` (the default) the driver SHALL probe the session's `release_version` and pick the path automatically
* *AND* if the override forces native streaming against an unsupported server, the resulting Exasol error MUST be surfaced to the caller verbatim

### Scenario: Native Parquet IMPORT SQL has no CSV format clauses and appends MaxConcurrentReads=1

* *GIVEN* the driver builds an IMPORT statement for the native Parquet path
* *WHEN* `ImportQuery::with_format(ImportFormat::Parquet)` is in effect
* *THEN* every `AT '...'` URL MUST end with `;MaxConcurrentReads=1` appended directly to the URL string, before the closing quote
* *AND* the SQL MUST NOT contain any `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, or `REJECT LIMIT` clause
* *AND* the SQL MUST NOT contain `MULTIPLE LOCAL FILES`, regardless of how many files are listed
* *AND* when TLS is enabled, every `AT '...'` clause MUST be followed by a matching `PUBLIC KEY '<sha256-fingerprint>'` clause exactly as for the CSV path

### Scenario: TLS for HTTP transport is configured uniformly via use_tls

* *GIVEN* an application configures Parquet import or export options
* *WHEN* the application enables or disables TLS on the HTTP transport tunnel
* *THEN* the option builders `ParquetImportOptions` and `ParquetExportOptions` SHALL expose a single fluent method `use_tls(bool)` to control HTTP-transport TLS
* *AND* the option builders MUST NOT expose `with_encryption(bool)` or `use_encryption(bool)` aliases
* *AND* the underlying option struct field SHALL be named `use_tls` (renamed from `use_encryption`) so that public field access uses the same vocabulary as the builder
* *AND* the default value of `use_tls` SHALL remain `false`

### Scenario: CSV-path Parquet import keeps pre-epoch timestamps with fractional seconds

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)` and `TS TIMESTAMP(6)`
* *AND* a Parquet file with the fields `ID` (Int64) and `TS` (`Timestamp(Microsecond, None)`), holding the rows `(1, -500000)` and `(2, -1)`
* *WHEN* the application imports the file into `T` with `import_from_parquet` and `ParquetImportOptions::with_native_parquet(Some(false))`
* *THEN* `TO_CHAR(TS, 'YYYY-MM-DD HH24:MI:SS.FF6')` SHALL return `1969-12-31 23:59:59.500000` for `ID` 1 and `1969-12-31 23:59:59.999999` for `ID` 2

### Scenario: CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch

* *GIVEN* a Parquet RecordBatch with a Timestamp column whose value is -1
* *WHEN* the CSV path converts the value to CSV text for the time units `Second`, `Millisecond`, `Microsecond`, and `Nanosecond`
* *THEN* the text SHALL be `1969-12-31 23:59:59.000000`, `1969-12-31 23:59:59.999000`, `1969-12-31 23:59:59.999999`, and `1969-12-31 23:59:59.999999`, in that order
* *AND* the `Microsecond` value -500000 SHALL convert to `1969-12-31 23:59:59.500000`

### Scenario: CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range

* *GIVEN* a Parquet RecordBatch with a Date32 value 2932897 (10000-01-01), a Date32 value -719163 (0000-12-31), a `Timestamp(Second, None)` value 9223372036854775807, or a `Timestamp(Microsecond, None)` value 253402300800000000 (10000-01-01 00:00:00)
* *WHEN* the CSV path converts the value to CSV text
* *THEN* the conversion SHALL return `ImportError::ConversionError` that names the value, and the import SHALL fail
