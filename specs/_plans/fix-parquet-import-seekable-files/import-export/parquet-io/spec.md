# Feature: Parquet I/O

Specifies Parquet file import through the HTTP tunnel, natively on Exasol 2025.1.11 and newer or after CSV conversion on older servers. Parquet export is specified in `import-export/parquet-export` and `import-export/parquet-csv-bytes`.

## Background

- The driver selects the path from the connected server's `release_version`. On 2025.1.11 and newer, the server requests the Parquet file with HTTP range requests (footer first, then row groups). On older servers, the driver converts each `RecordBatch` to CSV with `record_batch_to_csv` and streams the CSV body.
- The Parquet IMPORT statement omits all CSV format options and the `MULTIPLE LOCAL FILES` tag, because Exasol opens one HTTP server per file for native Parquet import.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Native Parquet import on Exasol 2025.1.11+

* *GIVEN* a Parquet file exists on disk
* *AND* the connected Exasol server's `release_version` is at or above `2025.1.11`
* *WHEN* user calls import_from_parquet with table name and file path
* *THEN* the system MUST serve the raw Parquet bytes from the open file through the range-request handler, MUST read only the requested bytes of each range from the file, in pieces of at most 64 KiB, without holding the whole file or a whole requested range in memory at once, and MUST NOT invoke any CSV conversion path
* *AND* the range-request handler MUST take the file size from the file once, before it answers the first request, and MUST reply to `HEAD /<file>` with `200 OK` plus `Content-Length: <file size>` and to `GET /<file>` with `Range: bytes=<start>-<end>` headers using `206 Partial Content` carrying the requested byte slice, looping until Exasol closes the connection
* *AND* the generated IMPORT SQL MUST use the `FROM PARQUET AT 'http(s)://addr;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE 'NNN.parquet'` form, with the `.parquet` extension and without any CSV format option clauses or the `MULTIPLE LOCAL FILES` tag
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Native Parquet import fails when the file becomes shorter during the import

* *GIVEN* the native Parquet path serves a Parquet file from disk
* *AND* the system has answered Exasol's `HEAD /<file>` with the file size
* *AND* the file becomes shorter afterwards
* *WHEN* Exasol requests a byte range that ends past the file's new end
* *THEN* the range-request handler MUST stop the response body at the first short read and MUST fail with an I/O error, so that the import fails with that error and Exasol receives fewer body bytes than the declared `Content-Length`
* *AND* the system MUST close the tunnel connection instead of waiting for another request
<!-- /DELTA:NEW -->
