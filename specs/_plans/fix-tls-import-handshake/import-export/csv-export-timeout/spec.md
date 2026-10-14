<!-- DELTA:CHANGED -->
# Feature: CSV Export Timeout

Specifies the timeout behavior that governs a CSV export once its HTTP tunnel has completed the TCP connect and the EXA handshake: the absence of a client-side timer by default, how a server-enforced `query_timeout` interacts with an export, and how an explicit `CsvExportOptions::timeout_ms` stops an export, including while the TLS handshake of a TLS tunnel is pending. The bounds on the tunnel setup itself are specified in `import-export/http-transport`.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

`CsvExportOptions::timeout_ms` is an `Option<u64>` defaulting to `None`. It covers SQL execution, data transfer, and the callback, which all follow the TCP connect and the EXA handshake of the tunnel. With TLS, it also covers the TLS handshake of the tunnel, which runs concurrently with the EXPORT statement. The tunnel setup bounds specified in `import-export/http-transport` apply whether or not `timeout_ms` is set: one deadline bounds the TCP connect and the EXA handshake, and with TLS a separate bound covers the TLS handshake. A `timeout_ms` shorter than the TLS handshake bound can therefore elapse first while the TLS handshake is pending. When `timeout_ms` is unset, the driver arms no client-side timer around SQL execution, data transfer, or the callback, and the export runs until the server finishes the EXPORT statement or a server-enforced `query_timeout` aborts it. When set, the driver returns `ExportError::Timeout` if that bound elapses first. `ArrowExportOptions` and `ParquetExportOptions` carry no timeout field of their own; the driver builds their underlying `CsvExportOptions` from defaults, so Arrow and Parquet exports inherit the same no-timeout-by-default behavior.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: No client-side export timeout by default

* *GIVEN* export options built from `CsvExportOptions::default()`, which configures no export timeout
* *WHEN* the caller exports a table or query, however long its combined SQL execution and data transfer run
* *THEN* the driver MUST NOT wrap the SQL execution, the data transfer, or the callback in a client-side timer
* *AND* the export SHALL run until the server finishes the EXPORT statement, or until a server-enforced timeout aborts that statement
* *AND* only the setup deadline specified in `import-export/http-transport` SHALL bound the TCP connect and the EXA handshake of the tunnel, which precede the EXPORT statement
* *AND* with TLS, only the separate TLS handshake bound specified in `import-export/http-transport` SHALL bound the TLS handshake of the tunnel, which starts at the first tunnel read or write and can overlap the EXPORT statement
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Server-enforced timeout governs an export

* *GIVEN* a connection that configures a `query_timeout`, and export options that configure no export timeout
* *WHEN* the EXPORT statement runs longer than the configured server-enforced timeout
* *THEN* the server SHALL abort the EXPORT statement and report the abort through the normal response cycle
* *AND* the driver SHALL surface the abort as `ExportError::SqlExecutionError` and MUST NOT surface it as `ExportError::Timeout`
* *AND* the driver MUST return that SQL error without waiting for the pending HTTP transport task
* *AND* the connection SHALL remain usable for subsequent statements
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Explicit export timeout covers a pending TLS handshake

* *GIVEN* export options with `use_tls(true)` and an explicit export timeout configured through `CsvExportOptions::timeout_ms` that is shorter than the TLS handshake bound specified in `import-export/http-transport`
* *AND* the tunnel has completed the EXA handshake, and Exasol has not completed the TLS handshake
* *WHEN* the export timeout elapses
* *THEN* the driver SHALL return `ExportError::Timeout` with the configured timeout in milliseconds before the TLS handshake bound elapses
* *AND* `ExportError::Timeout` SHALL report that the driver terminated the transport, because the EXPORT response has not been consumed
* *AND* the driver MUST abort the pending HTTP transport task rather than detach it
<!-- /DELTA:NEW -->
