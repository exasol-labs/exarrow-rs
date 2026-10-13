# Feature: Arrow RecordBatch Import/Export

Specifies Arrow RecordBatch import and export capabilities, enabling direct transfer of Arrow-formatted data between applications and Exasol tables.

## Background

Arrow RecordBatch import converts RecordBatch data to CSV format for streaming through the HTTP tunnel. Export converts CSV received from Exasol into Arrow RecordBatches with schemas reflecting Exasol column types. Both single RecordBatches and streams of RecordBatches are supported, along with Arrow IPC file format for persistent storage. Streaming export supports configurable batch sizes. The HTTP-transport TLS knob is exposed as a single fluent method `use_tls(bool)` on both option builders, replacing the earlier flag-style `with_encryption()` (import) and `use_encryption(bool)` (export) names.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: TLS for HTTP transport is configured uniformly via use_tls

* *GIVEN* an application configures Arrow RecordBatch import or export options
* *WHEN* the application enables or disables TLS on the HTTP transport tunnel
* *THEN* the option builders `ArrowImportOptions` and `ArrowExportOptions` SHALL expose a single fluent method `use_tls(bool)` to control HTTP-transport TLS
* *AND* the option builders MUST NOT expose `with_encryption()` (flag-style) or `use_encryption(bool)` aliases
* *AND* the underlying option struct field SHALL be named `use_tls` (renamed from `use_encryption`) so that public field access uses the same vocabulary as the builder
* *AND* the default value of `use_tls` SHALL remain `false`
<!-- /DELTA:CHANGED -->
