# Feature: Parquet Import File Streaming

Specifies how the native Parquet import path serves a Parquet file from disk and fails the import when the file shrinks during the import.

## Background

- The native Parquet path serves the file to Exasol through HTTP range requests, as described in `import-export/parquet-io`.
- The system answers `HEAD /<file>` with the file size, and range responses declare their length up front.

## Scenarios

### Scenario: Native Parquet import fails when the file becomes shorter during the import

* *GIVEN* the native Parquet path serves a Parquet file from disk
* *AND* the system has answered Exasol's `HEAD /<file>` with the file size
* *AND* the file becomes shorter afterwards
* *WHEN* Exasol requests a byte range that ends past the file's new end
* *THEN* the range-request handler MUST stop the response body at the first short read and MUST fail with an I/O error, so that the import fails with that error and Exasol receives fewer body bytes than the declared `Content-Length`
* *AND* the system MUST close the tunnel connection instead of waiting for another request
