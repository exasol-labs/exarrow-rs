# Feature: CSV Export

Specifies CSV data export capabilities from Exasol tables and queries to files, streams, and callbacks.

<!-- DELTA:CHANGED -->
## Background

CSV export operations receive data from Exasol through the HTTP transport tunnel via PUT requests. The system supports exporting to file paths, async writers, in-memory collections, and callbacks. Export operations support custom CSV formatting, column headers, and compression options.

The export consumer is the callback passed to `export_to_callback`, or the file, writer, or row list that `export_to_file`, `export_to_stream`, and `export_to_list` fill. A piece is a run of at most 64 KiB (65536 bytes) of the PUT request body, in the order the tunnel delivers it. The driver passes pieces to the export consumer while the EXPORT statement runs, through a channel that holds at most 16 pieces. Piece boundaries follow the tunnel transfer, not CSV records.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Export with compression

* *GIVEN* an export operation requires compression
* *WHEN* user specifies gzip or bz2 compression option
* *THEN* EXPORT SQL SHALL use appropriate file extension
* *AND* `export_to_file`, `export_to_stream`, and `export_to_list` SHALL decompress each piece when it arrives
* *AND* the driver MUST NOT collect the complete compressed body before it starts to decompress
* *AND* IF the body completed, with its zero-length terminator or its full Content-Length, and the EXPORT statement succeeded, but the compressed stream in the body is incomplete, the export SHALL fail with `ExportError::DecompressionError`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Export to async stream

* *GIVEN* an AsyncWrite implementation is available for output
* *WHEN* user calls export_to_stream with an AsyncWrite implementation
* *THEN* system SHALL write the data of each piece to the stream when the piece arrives, before the server sends the rest of the body
* *AND* system SHALL handle backpressure from slow consumers, as the scenario "Export applies backpressure to a slow consumer" specifies
* *AND* a CSV record MAY span two writes
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Export to callback

* *GIVEN* a data-receiving callback function is available
* *WHEN* user calls export_to_callback with a data-receiving callback
* *THEN* system SHALL invoke callback with data chunks as received, one piece per chunk, while the EXPORT statement runs
* *AND* the callback SHALL receive the first piece before the server sends the rest of the body, so the system MUST NOT accumulate the complete response body first
* *AND* IF compression is configured, the pieces SHALL carry the compressed bytes as Exasol sent them
* *AND* a CSV record MAY span two pieces
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Export applies backpressure to a slow consumer

* *GIVEN* an export whose consumer stops taking pieces, either a callback that stops receiving or a writer that accepts no write
* *WHEN* 16 received pieces wait in the channel for the consumer
* *THEN* the driver SHALL read at most one further piece from the tunnel until the consumer takes the next piece
* *AND* the driver SHALL hold no more than the 16 waiting pieces, the one piece that it has read but not yet queued, and the piece that `export_to_file` or `export_to_stream` is writing, with its decoded output
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Export statement fails while data streams

* *GIVEN* an export whose consumer has received at least one piece
* *WHEN* the EXPORT statement fails before the server completes the transfer
* *THEN* the driver SHALL return `ExportError::SqlExecutionError`, also when the consumer then reports success because its data ended
* *AND* the driver SHALL return `ExportError::SqlExecutionError` also when the consumer fails because its data ended early, for example with `ExportError::DecompressionError`
* *AND* the driver MUST NOT wait for the consumer or for the pending HTTP transport task before it returns the error
* *AND* the connection SHALL remain usable for subsequent statements
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Export callback returns before the transfer completes

* *GIVEN* an export_to_callback callback that returns `Ok` before the server completes the transfer
* *WHEN* the callback returns
* *THEN* the driver SHALL read and discard the remaining body without passing it to the callback, and SHALL let the EXPORT statement finish
* *AND* the driver SHALL return the callback's value when the EXPORT statement succeeds
* *AND* the driver SHALL return `ExportError::SqlExecutionError` when the EXPORT statement fails
* *AND* the connection SHALL remain usable for subsequent statements
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Export consumer fails while data streams

* *GIVEN* an export whose callback returns an error, or whose writer fails a write, before the server completes the transfer and while the tunnel is still open
* *WHEN* the driver observes that failure
* *THEN* the driver SHALL stop reading the tunnel and SHALL close it
* *AND* the driver SHALL wait for the response to the EXPORT statement before it returns, unless a configured `CsvExportOptions::timeout_ms` elapses first
* *AND* the driver SHALL return the consumer's error, not the `ExportError::SqlExecutionError` that the closed tunnel causes
* *AND* the connection SHALL remain usable for subsequent statements
<!-- /DELTA:NEW -->
