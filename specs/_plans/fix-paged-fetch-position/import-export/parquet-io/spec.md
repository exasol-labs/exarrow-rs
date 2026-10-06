<!-- DELTA:CHANGED -->
# Feature: Parquet I/O

Specifies Parquet file import. Import streams Parquet bytes through the HTTP transport tunnel either natively, on a server for which `supports_native_parquet_import()` returns true (`connection-management/version-capability`), or after CSV conversion on every other server. Parquet export is specified in `import-export/parquet-export` and `import-export/parquet-csv-bytes`.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

Parquet import operates over the same HTTP tunnel as CSV import but selects between two on-the-wire transport models based on the connected server's `release_version`, through `supports_native_parquet_import()` (`connection-management/version-capability`):

- On a server for which `supports_native_parquet_import()` returns true (Exasol 2025.1.11 and newer, except 2025.2.0) the server requests the Parquet file from the driver using **HTTP range requests**. The driver responds to `HEAD` with `200 OK` plus `Content-Length` and to `GET` with `Range: bytes=X-Y` using `206 Partial Content` carrying the requested byte slice. Multiple sequential HEAD/GET-Range requests typically arrive on the same connection (footer first, then row groups) until the server closes it. The generated SQL is `IMPORT INTO ... FROM PARQUET AT '...;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '...parquet'`.
- On every other server, including Exasol 2025.2.0 and releases below 2025.1.11, the driver reads each Parquet `RecordBatch`, converts to CSV via `record_batch_to_csv`, streams the CSV body through the existing chunked-encoding response, and emits `FROM CSV ... FILE '...csv'`.

The Parquet variant of the IMPORT statement omits all CSV format options (no `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, `REJECT LIMIT`) and never emits the `MULTIPLE LOCAL FILES` tag (Exasol opens one HTTP server per file for native Parquet import). The `;MaxConcurrentReads=1` suffix is appended inside the `AT '...'` URL of every file entry, matching the JDBC reference behavior. Path selection is automatic by default and can be overridden via `ParquetImportOptions::with_native_parquet(Some(true|false))`.

The HTTP-transport TLS knob is exposed as `use_tls(bool)` on both option builders.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Import Parquet file into table

* *GIVEN* a Parquet file exists on disk
* *AND* `supports_native_parquet_import()` returns false for the connected Exasol server
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* system SHALL read the Parquet file, convert each RecordBatch to CSV, and stream the CSV body through the HTTP tunnel
* *AND* the generated IMPORT SQL SHALL use the `FROM CSV ... FILE 'NNN.csv'` clause
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Native Parquet import on Exasol 2025.1.11+

* *GIVEN* a Parquet file exists on disk
* *AND* the connected Exasol server's `release_version` is at or above `2025.1.11`
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* the system MUST buffer the entire file's raw Parquet bytes in memory and MUST NOT invoke any CSV conversion path
* *AND* the system MUST serve the bytes via the range-request handler: replying to `HEAD /<file>` with `200 OK` plus `Content-Length: <total bytes>` and to `GET /<file>` with `Range: bytes=<start>-<end>` headers using `206 Partial Content` carrying the requested byte slice, looping until Exasol closes the connection
* *AND* the generated IMPORT SQL MUST use the `FROM PARQUET AT 'http(s)://addr;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE 'NNN.parquet'` form, with the `.parquet` extension and without any CSV format option clauses or the `MULTIPLE LOCAL FILES` tag
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Native Parquet import on a server that supports it

* *GIVEN* a Parquet file exists on disk
* *AND* `supports_native_parquet_import()` returns true for the connected Exasol server
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* the system MUST buffer the entire file's raw Parquet bytes in memory and MUST NOT invoke any CSV conversion path
* *AND* the system MUST serve the bytes via the range-request handler: replying to `HEAD /<file>` with `200 OK` plus `Content-Length: <total bytes>` and to `GET /<file>` with `Range: bytes=<start>-<end>` headers using `206 Partial Content` carrying the requested byte slice, looping until Exasol closes the connection
* *AND* the generated IMPORT SQL MUST use the `FROM PARQUET AT 'http(s)://addr;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE 'NNN.parquet'` form, with the `.parquet` extension and without any CSV format option clauses or the `MULTIPLE LOCAL FILES` tag
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Exasol 2025.2.0 receives Parquet converted to CSV

* *GIVEN* a Parquet file exists on disk
* *AND* the connected Exasol server's `release_version` is `2025.2.0`
* *AND* `ParquetImportOptions::with_native_parquet` is not set
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* the system SHALL convert the file to CSV and SHALL emit `FROM CSV ... FILE 'NNN.csv'`
* *AND* the system MUST NOT send an IMPORT statement with `FROM PARQUET`, which Exasol 2025.2.0 rejects with `ETL-2210`
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Import Parquet from stream

* *GIVEN* Parquet data is available as an async stream
* *WHEN* user provides AsyncRead for Parquet data
* *THEN* the system SHALL buffer the stream into a `Vec<u8>` in memory before serving it because Parquet requires footer random access
* *AND* on a server for which `supports_native_parquet_import()` returns true the system SHALL serve the buffered Parquet bytes via HTTP range requests (no CSV conversion) and SHALL emit `FROM PARQUET AT '...;MaxConcurrentReads=1' ... FILE '...parquet'` SQL
* *AND* on every other server the system SHALL parse the Parquet, convert to CSV, and emit `FROM CSV` SQL as before
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Import Parquet preserves data types

* *GIVEN* a Parquet file contains typed columns
* *WHEN* the file is imported via the native path
* *THEN* the Parquet bytes SHALL reach Exasol unchanged so type fidelity SHALL be governed by the server's Parquet reader
* *AND* on the legacy CSV-conversion path the system SHALL convert types to CSV-compatible representations and SHALL preserve NULL values per the existing CSV mapping
<!-- /DELTA:CHANGED -->
