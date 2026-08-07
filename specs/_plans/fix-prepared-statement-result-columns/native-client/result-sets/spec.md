# Feature: Binary Result Set to Arrow Conversion

The native TCP protocol returns query results as column-major binary data, which maps directly to Arrow's columnar memory format. The system parses binary result set frames into Arrow RecordBatches without intermediate JSON representation, achieving zero-copy columnar transfer for supported types.

## Background

Native protocol result sets contain: a result type marker (1 byte), result set handle (4 bytes), column count (4 bytes), total rows (8 bytes), rows in this message (8 bytes), column metadata (variable), and column-major row data with per-column null masks. Each column's data is contiguous in the binary stream, matching Arrow's memory layout.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Column metadata parsing

* *GIVEN* an authenticated native TCP session exists
* *AND* a query has returned an `R_ResultSet` (1)
* *WHEN* parsing the result set header
* *THEN* the system SHALL extract the result set handle, column count, total rows, and rows received
* *AND* for each column the system SHALL parse: column name length, column name, type ID, and type-specific metadata (precision, scale, the vcFlag byte whose varchar indicator is bit `0x01`, max length)
* *AND* the system SHALL build an Arrow Schema from the parsed column metadata
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: VARCHAR is distinguished from CHAR by the vcFlag varchar bit

* *GIVEN* a result set contains a column of type `T_char` (10)
* *WHEN* parsing that column's metadata
* *THEN* a vcFlag byte of `0x11`, which Exasol sends for `VARCHAR(n)` with character set UTF8, SHALL be reported as `VARCHAR(n)`
* *AND* a vcFlag byte of `0x10`, which Exasol sends for `CHAR(n)` with character set UTF8, SHALL be reported as `CHAR(n)`
* *AND* a vcFlag byte of `0x00`, for a `T_char` column with the varchar bit clear, SHALL NOT be reported as `VARCHAR`
* *AND* the reported Exasol type name MUST match the type name the WebSocket transport reports for the identical column
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Direct binary to Arrow conversion for string types

* *GIVEN* a result set contains columns of type `T_char` (10) with or without the vcFlag varchar bit `0x01` set
* *WHEN* parsing column data
* *THEN* the system SHALL read length-prefixed UTF-8 strings into Arrow Utf8 or LargeUtf8 arrays
* *AND* the system SHALL decode every string payload as UTF-8 regardless of the vcFlag UTF-8 bit `0x10`, because Exasol transmits native-protocol string payloads as UTF-8
* *AND* `VARCHAR(n)` and `CHAR(n)` SHALL map to the same Arrow string type, so the varchar bit SHALL affect only the reported Exasol type name
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Prepared-statement sub-results carry both descriptions

* *GIVEN* a prepared-statement reply whose `R_HANDLE` (2) part is followed by a `PARAMETER_DESCRIPTION` (-5) sub-result and a second result-set sub-result
* *WHEN* parsing that reply
* *THEN* the parser SHALL return a response shape carrying the statement handle, the parameter column metadata, and the result-set column metadata as three distinct values, parsed by the same column-metadata routine ordinary result sets use
* *AND* the parser SHALL NOT encode a sub-result handle in the `total_rows` field of a result-set response
* *AND* a reply whose `R_HANDLE` part carries no result-set sub-result SHALL yield empty parameter column metadata and empty result-set column metadata
* *AND* a caller that receives this response shape where a query result was expected SHALL be rejected with a protocol error
* *AND* a sub-result reporting a server exception SHALL be rejected with a protocol error naming the server message and SQL state, and SHALL NOT be reported as a successful preparation with empty metadata
* *AND* a warning carried by a sub-result SHALL be propagated to the caller rather than discarded
<!-- /DELTA:NEW -->
