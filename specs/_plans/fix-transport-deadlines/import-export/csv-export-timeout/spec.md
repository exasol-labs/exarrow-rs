<!-- DELTA:CHANGED -->
# Feature: CSV Export Timeout

Specifies the timeout behavior that governs a CSV export once its HTTP tunnel is set up: the absence of a client-side timer by default, how a server-enforced `query_timeout` interacts with an export, and how an explicit `CsvExportOptions::timeout_ms` stops an export. The bound on the tunnel setup itself is specified in `import-export/http-transport`.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

`import-export/csv-export` covers the mechanics of exporting Exasol tables and queries to files, streams, lists, and callbacks over the HTTP transport tunnel. This feature specifies the orthogonal concern of how long an export is allowed to run.

`CsvExportOptions::timeout_ms` is an `Option<u64>` defaulting to `None`. It covers SQL execution, data transfer, and the callback, which all follow the tunnel setup. The tunnel setup deadline specified in `import-export/http-transport` bounds the tunnel setup whether or not `timeout_ms` is set. When `timeout_ms` is unset, the driver arms no client-side timer around SQL execution, data transfer, or the callback, and the export runs until the server finishes the EXPORT statement or a server-enforced `query_timeout` aborts it. When set, the driver races the export against a `tokio::time::timeout` bound and returns `ExportError::Timeout` if that bound elapses first. `ArrowExportOptions` and `ParquetExportOptions` carry no timeout field of their own; the driver builds their underlying `CsvExportOptions` from defaults, so Arrow and Parquet exports inherit the same no-timeout-by-default behavior.

This mirrors the precedent set for query execution: `ConnectionParams::query_timeout` is `Option<Duration>` defaulting to `None`, and `Statement::timeout_ms` is `Option<u64>` defaulting to `None` (see `specs/_decision/008-remove-query-timeout.md`).
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: No client-side export timeout by default

* *GIVEN* export options built from `CsvExportOptions::default()`, which configures no export timeout
* *WHEN* the caller exports a table or query, however long its combined SQL execution and data transfer run
* *THEN* the driver MUST NOT wrap the SQL execution, the data transfer, or the callback in a client-side timer
* *AND* the export SHALL run until the server finishes the EXPORT statement, or until a server-enforced timeout aborts that statement
* *AND* only the tunnel setup deadline specified in `import-export/http-transport` SHALL bound the tunnel setup that precedes the EXPORT statement
<!-- /DELTA:CHANGED -->
