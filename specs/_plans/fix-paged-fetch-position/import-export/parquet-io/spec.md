# Feature: Parquet I/O

Specifies Parquet file import. Import streams Parquet bytes through the HTTP transport tunnel either natively (Exasol 2025.1.11+) or after CSV conversion (older servers). Parquet export is specified in `import-export/parquet-export` and `import-export/parquet-csv-bytes`.

<!-- DELTA:CHANGED -->
## Background

Parquet import operates over the same HTTP tunnel as CSV import but selects between two on-the-wire transport models based on the connected server's `release_version`:

- On Exasol 2025.1.11 and newer the driver selects native Parquet import, and the server requests the Parquet file from the driver using **HTTP range requests**. The driver responds to `HEAD` with `200 OK` plus `Content-Length` and to `GET` with `Range: bytes=X-Y` using `206 Partial Content` carrying the requested byte slice. Multiple sequential HEAD/GET-Range requests typically arrive on the same connection (footer first, then row groups) until the server closes it. The generated SQL is `IMPORT INTO ... FROM PARQUET AT '...;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '...parquet'`.
- On older servers the driver reads each Parquet `RecordBatch`, converts to CSV via `record_batch_to_csv`, streams the CSV body through the existing chunked-encoding response, and emits `FROM CSV ... FILE '...csv'`.

The Parquet variant of the IMPORT statement omits all CSV format options (no `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, `REJECT LIMIT`) and never emits the `MULTIPLE LOCAL FILES` tag (Exasol opens one HTTP server per file for native Parquet import). The `;MaxConcurrentReads=1` suffix is appended inside the `AT '...'` URL of every file entry, matching the JDBC reference behavior. Path selection is automatic by default and can be overridden via `ParquetImportOptions::with_native_parquet(Some(true|false))`.

The HTTP-transport TLS knob is exposed as `use_tls(bool)` on both option builders.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Import Parquet file into table

* *GIVEN* a Parquet file exists on disk
* *AND* the connected Exasol server's `release_version` is below `2025.1.11`
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* system SHALL read the Parquet file, convert each RecordBatch to CSV, and stream the CSV body through the HTTP tunnel
* *AND* the generated IMPORT SQL SHALL use the `FROM CSV ... FILE 'NNN.csv'` clause
