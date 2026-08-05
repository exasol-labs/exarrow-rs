# Feature: CSV Export

Specifies CSV data export capabilities from Exasol tables and queries to files, streams, and callbacks.

## Background

CSV export operations receive data from Exasol through the HTTP transport tunnel via PUT requests. The system supports exporting to file paths, async writers, in-memory collections, and callbacks. Export operations support custom CSV formatting, column headers, and compression options.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: No client-side export timeout by default

* *GIVEN* export options built from `CsvExportOptions::default()`, which configures no export timeout
* *WHEN* the caller exports a table or query, however long its combined SQL execution and data transfer run
* *THEN* the driver MUST NOT wrap the export in a client-side timer
* *AND* the export SHALL run until the server finishes the EXPORT statement, or until a server-enforced timeout aborts that statement
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Arrow and Parquet exports inherit the CSV export defaults

* *GIVEN* an Arrow or Parquet export configured through `ArrowExportOptions` or `ParquetExportOptions`, neither of which exposes an export timeout
* *WHEN* the driver builds the CSV export options that carry the request
* *THEN* the driver SHALL construct them from the `CsvExportOptions` defaults
* *AND* the constructed options SHALL configure no export timeout
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Server-enforced timeout governs an export

* *GIVEN* a connection that configures a `query_timeout`, and export options that configure no export timeout
* *WHEN* the EXPORT statement runs longer than the configured server-enforced timeout
* *THEN* the server SHALL abort the EXPORT statement and report the abort through the normal response cycle
* *AND* the driver SHALL surface the abort as `ExportError::SqlExecutionError` and MUST NOT surface it as `ExportError::Timeout`
* *AND* the driver MUST return that SQL error without waiting for the pending HTTP transport task, because the tunnel read has no timeout of its own and no client-side bound is armed
* *AND* the connection SHALL remain usable for subsequent statements
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Explicit export timeout stops the export

* *GIVEN* an explicit export timeout configured through `CsvExportOptions::timeout_ms`
* *WHEN* the combined SQL execution, data transfer, and callback processing exceed the configured timeout
* *THEN* the driver SHALL stop waiting and SHALL return `ExportError::Timeout`, which SHALL report both the configured timeout in milliseconds and whether the driver terminated the transport
* *AND* the driver MUST abort the pending HTTP transport task rather than detach it
* *AND* IF the timeout elapses before the EXPORT response has been consumed, the driver MUST terminate the transport before returning, because that response can no longer be matched to a request, and a subsequent operation on the same connection MUST fail instead of returning data from the abandoned response
* *AND* IF the timeout elapses after the EXPORT response has been consumed, for example while the callback is still draining received data, the driver MUST NOT terminate the transport and the connection SHALL remain usable for subsequent statements
* *AND* the driver MAKES no guarantee that data already written to the caller's sink forms a complete CSV document, so the caller MUST discard partially written output after `ExportError::Timeout`
<!-- /DELTA:NEW -->
