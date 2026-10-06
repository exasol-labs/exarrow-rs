# Feature: Parquet I/O

Specifies Parquet file import. Import streams Parquet bytes through the HTTP transport tunnel either natively (Exasol 2025.1.11+) or after CSV conversion (older servers). Parquet export is specified in `import-export/parquet-export` and `import-export/parquet-csv-bytes`.

## Background

Parquet import operates over the same HTTP tunnel as CSV import but selects between two on-the-wire transport models based on the connected server's `release_version`:

- On Exasol 2025.1.11 and newer the server requests the Parquet file from the driver using **HTTP range requests**. The driver responds to `HEAD` with `200 OK` plus `Content-Length` and to `GET` with `Range: bytes=X-Y` using `206 Partial Content` carrying the requested byte slice. Multiple sequential HEAD/GET-Range requests typically arrive on the same connection (footer first, then row groups) until the server closes it. The generated SQL is `IMPORT INTO ... FROM PARQUET AT '...;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '...parquet'`.
- On older servers the driver reads each Parquet `RecordBatch`, converts to CSV via `record_batch_to_csv`, streams the CSV body through the existing chunked-encoding response, and emits `FROM CSV ... FILE '...csv'`.

The Parquet variant of the IMPORT statement omits all CSV format options (no `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, `REJECT LIMIT`) and never emits the `MULTIPLE LOCAL FILES` tag (Exasol opens one HTTP server per file for native Parquet import). The `;MaxConcurrentReads=1` suffix is appended inside the `AT '...'` URL of every file entry, matching the JDBC reference behavior. Path selection is automatic by default and can be overridden via `ParquetImportOptions::with_native_parquet(Some(true|false))`.

The HTTP-transport TLS knob is exposed as `use_tls(bool)` on both option builders.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: CSV-path Parquet import keeps pre-epoch timestamps with fractional seconds

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)` and `TS TIMESTAMP(6)`
* *AND* a Parquet file with the fields `ID` (Int64) and `TS` (`Timestamp(Microsecond, None)`), holding the rows `(1, -500000)` and `(2, -1)`
* *WHEN* the application imports the file into `T` with `import_from_parquet` and `ParquetImportOptions::with_native_parquet(Some(false))`
* *THEN* `TO_CHAR(TS, 'YYYY-MM-DD HH24:MI:SS.FF6')` SHALL return `1969-12-31 23:59:59.500000` for `ID` 1 and `1969-12-31 23:59:59.999999` for `ID` 2
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch

* *GIVEN* a Parquet RecordBatch with a Timestamp column whose value is -1
* *WHEN* the CSV path converts the value to CSV text for the time units `Second`, `Millisecond`, `Microsecond`, and `Nanosecond`
* *THEN* the text SHALL be `1969-12-31 23:59:59.000000`, `1969-12-31 23:59:59.999000`, `1969-12-31 23:59:59.999999`, and `1969-12-31 23:59:59.999999`, in that order
* *AND* the `Microsecond` value -500000 SHALL convert to `1969-12-31 23:59:59.500000`
<!-- /DELTA:NEW -->
