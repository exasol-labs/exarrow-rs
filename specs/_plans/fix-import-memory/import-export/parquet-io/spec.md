# Feature: Parquet I/O

Specifies Parquet file import. Import streams Parquet bytes through the HTTP transport tunnel either natively (Exasol 2025.1.11+) or after CSV conversion (older servers). Parquet export is specified in `import-export/parquet-export` and `import-export/parquet-csv-bytes`.

## Background

Parquet import operates over the same HTTP tunnel as CSV import but selects between two on-the-wire transport models based on the connected server's `release_version`:

- On Exasol 2025.1.11 and newer the driver selects native Parquet import, and the server requests the Parquet file from the driver using **HTTP range requests**. The driver responds to `HEAD` with `200 OK` plus `Content-Length` and to `GET` with `Range: bytes=X-Y` using `206 Partial Content` carrying the requested byte slice. Multiple sequential HEAD/GET-Range requests typically arrive on the same connection (footer first, then row groups) until the server closes it. The generated SQL is `IMPORT INTO ... FROM PARQUET AT '...;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '...parquet'`.
- On older servers the driver reads each Parquet `RecordBatch`, converts to CSV via `record_batch_to_csv`, streams the CSV body through the existing chunked-encoding response, and emits `FROM CSV ... FILE '...csv'`.

The Parquet variant of the IMPORT statement omits all CSV format options (no `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, `REJECT LIMIT`) and never emits the `MULTIPLE LOCAL FILES` tag (Exasol opens one HTTP server per file for native Parquet import). The `;MaxConcurrentReads=1` suffix is appended inside the `AT '...'` URL of every file entry, matching the JDBC reference behavior. Path selection is automatic by default and can be overridden via `ParquetImportOptions::with_native_parquet(Some(true|false))`.

The HTTP-transport TLS knob is exposed as `use_tls(bool)` on both option builders.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time

* *GIVEN* a Parquet file with at least 8 row groups that each hold the same number of rows
* *AND* the CSV conversion path is selected, because the connected server's `release_version` is below `2025.1.11` or `ParquetImportOptions::with_native_parquet(Some(false))` is set
* *WHEN* user calls import_from_parquet with the file path, or import_from_parquet_stream with the file's bytes, with `ParquetImportOptions::with_batch_size` set to the row count of one row group
* *THEN* the system SHALL convert the file to CSV one RecordBatch at a time, and SHALL hand the CSV of the first RecordBatch to the HTTP tunnel before it decodes the file's last row group
* *AND* the system MUST NOT hold the CSV of the whole file in memory at once
* *AND* the system SHALL import every row of the file and return the file's row count
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: CSV-path Parquet import that fails on a later RecordBatch imports no rows

* *GIVEN* a table `T` with the columns `ID DECIMAL(18,0)` and `D DATE`
* *AND* a Parquet file with the fields `ID` (Int64) and `D` (Date32) in 4 row groups of 1000 rows, whose last row holds the Date32 value 2932897 (10000-01-01)
* *WHEN* the application imports the file into `T` with `import_from_parquet`, `ParquetImportOptions::with_native_parquet(Some(false))`, and `ParquetImportOptions::with_batch_size(1000)`
* *THEN* the import SHALL fail with `ImportError::ConversionError` that names the value, and the system MUST NOT send the terminating zero-length chunk on the HTTP tunnel, so that Exasol fails the IMPORT statement
* *AND* `T` SHALL hold no rows
* *AND* the same Connection SHALL run `SELECT 1` successfully after the failed import
<!-- /DELTA:NEW -->
