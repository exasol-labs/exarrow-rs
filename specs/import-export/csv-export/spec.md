# Feature: CSV Export

Specifies CSV data export capabilities from Exasol tables and queries to files, streams, and callbacks.

## Background

CSV export operations receive data from Exasol through the HTTP transport tunnel via PUT requests. The system supports exporting to file paths, async writers, in-memory collections, and callbacks. Export operations support custom CSV formatting, column headers, and compression options.

## Scenarios

### Scenario: Export table to CSV file

* *GIVEN* an Exasol table contains data to export
* *WHEN* user calls export_to_file with table name, file path, and options
* *THEN* system SHALL establish HTTP tunnel and execute EXPORT SQL via WebSocket
* *AND* system SHALL receive CSV data from HTTP PUT request body
* *AND* system SHALL write data to file

### Scenario: Export query to CSV file

* *GIVEN* a SQL query is ready to produce export data
* *WHEN* user calls export_to_file with SQL query instead of table
* *THEN* system SHALL generate EXPORT query with embedded SELECT
* *AND* system SHALL export query results to file

### Scenario: Export with column headers

* *GIVEN* an export operation is being configured
* *WHEN* user enables column headers option
* *THEN* EXPORT SQL SHALL include WITH COLUMN NAMES clause
* *AND* CSV output SHALL begin with header row

### Scenario: Export with compression

* *GIVEN* an export operation requires compression
* *WHEN* user specifies gzip or bz2 compression option
* *THEN* system SHALL decompress received data
* *AND* EXPORT SQL SHALL use appropriate file extension

### Scenario: Export to async stream

* *GIVEN* an AsyncWrite implementation is available for output
* *WHEN* user calls export_to_stream with an AsyncWrite implementation
* *THEN* system SHALL write received data to stream
* *AND* system SHALL handle backpressure from slow consumers

### Scenario: Export to in-memory list

* *GIVEN* export data should be collected in memory
* *WHEN* user calls export_to_list
* *THEN* system SHALL collect all CSV rows into a Vec
* *AND* system SHALL return parsed rows

### Scenario: Export to callback

* *GIVEN* a data-receiving callback function is available
* *WHEN* user calls export_to_callback with a data-receiving callback
* *THEN* system SHALL invoke callback with data chunks as received
* *AND* callback SHALL process data in streaming fashion
