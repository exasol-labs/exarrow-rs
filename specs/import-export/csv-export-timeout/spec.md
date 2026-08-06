# Feature: CSV Export Timeout

Specifies the timeout behavior that governs a CSV export: the absence of a client-side timer by default, how a server-enforced `query_timeout` interacts with an export, and how an explicit `CsvExportOptions::timeout_ms` stops an export.

## Background

`import-export/csv-export` covers the mechanics of exporting Exasol tables and queries to files, streams, lists, and callbacks over the HTTP transport tunnel. This feature specifies the orthogonal concern of how long an export is allowed to run.

`CsvExportOptions::timeout_ms` is an `Option<u64>` defaulting to `None`. When unset, the driver arms no client-side timer and the export runs until the server finishes the EXPORT statement or a server-enforced `query_timeout` aborts it. When set, the driver races the export against a `tokio::time::timeout` bound and returns `ExportError::Timeout` if that bound elapses first. `ArrowExportOptions` and `ParquetExportOptions` carry no timeout field of their own; the driver builds their underlying `CsvExportOptions` from defaults, so Arrow and Parquet exports inherit the same no-timeout-by-default behavior.

This mirrors the precedent set for query execution: `ConnectionParams::query_timeout` is `Option<Duration>` defaulting to `None`, and `Statement::timeout_ms` is `Option<u64>` defaulting to `None` (see `specs/_decision/008-remove-query-timeout.md`).

## Scenarios

### Scenario: No client-side export timeout by default

* *GIVEN* export options built from `CsvExportOptions::default()`, which configures no export timeout
* *WHEN* the caller exports a table or query, however long its combined SQL execution and data transfer run
* *THEN* the driver MUST NOT wrap the export in a client-side timer
* *AND* the export SHALL run until the server finishes the EXPORT statement, or until a server-enforced timeout aborts that statement

### Scenario: Arrow and Parquet exports inherit the CSV export defaults

* *GIVEN* an Arrow or Parquet export configured through `ArrowExportOptions` or `ParquetExportOptions`, neither of which exposes an export timeout
* *WHEN* the driver builds the CSV export options that carry the request
* *THEN* the driver SHALL construct them from the `CsvExportOptions` defaults
* *AND* the constructed options SHALL configure no export timeout

### Scenario: Server-enforced timeout governs an export

* *GIVEN* a connection that configures a `query_timeout`, and export options that configure no export timeout
* *WHEN* the EXPORT statement runs longer than the configured server-enforced timeout
* *THEN* the server SHALL abort the EXPORT statement and report the abort through the normal response cycle
* *AND* the driver SHALL surface the abort as `ExportError::SqlExecutionError` and MUST NOT surface it as `ExportError::Timeout`
* *AND* the driver MUST return that SQL error without waiting for the pending HTTP transport task, because the tunnel read has no timeout of its own and no client-side bound is armed
* *AND* the connection SHALL remain usable for subsequent statements

### Scenario: Explicit export timeout stops the export

* *GIVEN* an explicit export timeout configured through `CsvExportOptions::timeout_ms`
* *WHEN* the combined SQL execution, data transfer, and callback processing exceed the configured timeout
* *THEN* the driver SHALL stop waiting and SHALL return `ExportError::Timeout`, which SHALL report both the configured timeout in milliseconds and whether the driver terminated the transport
* *AND* the driver MUST abort the pending HTTP transport task rather than detach it
* *AND* IF the timeout elapses before the EXPORT response has been consumed, the driver MUST terminate the transport before returning, because that response can no longer be matched to a request, and a subsequent operation on the same connection MUST fail instead of returning data from the abandoned response
* *AND* IF the timeout elapses after the EXPORT response has been consumed, for example while the callback is still draining received data, the driver MUST NOT terminate the transport and the connection SHALL remain usable for subsequent statements
* *AND* the driver MAKES no guarantee that data already written to the caller's sink forms a complete CSV document, so the caller MUST discard partially written output after `ExportError::Timeout`

## See Also

Export mechanics — file, stream, list, and callback destinations; formatting and compression options — are specified in `import-export/csv-export`.
