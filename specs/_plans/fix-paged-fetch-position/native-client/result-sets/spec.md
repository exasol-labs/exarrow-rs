# Feature: Binary Result Set to Arrow Conversion

The native TCP protocol returns query results as column-major binary data, which maps directly to Arrow's columnar memory format. The system parses binary result set frames and their column metadata; per-type conversion into Arrow arrays is specified in `native-client/type-conversion`.

## Background

Native protocol result sets contain: a result type marker (1 byte), result set handle (4 bytes), column count (4 bytes), total rows (8 bytes), rows in this message (8 bytes), column metadata (variable), and column-major row data with per-column null masks. Each column's data is contiguous in the binary stream, matching Arrow's memory layout.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Large result set (multi-fetch)

* *GIVEN* a query or a prepared statement returns a result set with a positive handle whose initial message contains fewer rows than total_rows
* *WHEN* retrieving the full result
* *THEN* the system SHALL issue `CMD_FETCH2` commands to retrieve remaining rows
* *AND* each `CMD_FETCH2` for the handle SHALL start at the rows received in the initial message plus the rows received by every earlier `CMD_FETCH2` for that handle
* *AND* each fetch response SHALL be converted to an Arrow RecordBatch
* *AND* the system SHALL close the result set handle after all rows are fetched
<!-- /DELTA:CHANGED -->
