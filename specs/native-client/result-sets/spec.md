# Feature: Binary Result Set to Arrow Conversion

The native TCP protocol returns query results as column-major binary data, which maps directly to Arrow's columnar memory format. The system parses binary result set frames and their column metadata; per-type conversion into Arrow arrays is specified in `native-client/type-conversion`.

## Background

Native protocol result sets contain: a result type marker (1 byte), result set handle (4 bytes), column count (4 bytes), total rows (8 bytes), rows in this message (8 bytes), column metadata (variable), and column-major row data with per-column null masks. Each column's data is contiguous in the binary stream, matching Arrow's memory layout.

## Scenarios

### Scenario: Column metadata parsing

* *GIVEN* an authenticated native TCP session exists
* *AND* a query has returned an `R_ResultSet` (1)
* *WHEN* parsing the result set header
* *THEN* the system SHALL extract the result set handle, column count, total rows, and rows received
* *AND* for each column the system SHALL parse: column name length, column name, type ID, and type-specific metadata (precision, scale, the vcFlag byte whose varchar indicator is bit `0x01`, max length)
* *AND* the system SHALL build an Arrow Schema from the parsed column metadata

### Scenario: VARCHAR is distinguished from CHAR by the vcFlag varchar bit

* *GIVEN* a result set contains a column of type `T_char` (10)
* *WHEN* parsing that column's metadata
* *THEN* a vcFlag byte of `0x11`, which Exasol sends for `VARCHAR(n)` with character set UTF8, SHALL be reported as `VARCHAR(n)`
* *AND* a vcFlag byte of `0x10`, which Exasol sends for `CHAR(n)` with character set UTF8, SHALL be reported as `CHAR(n)`
* *AND* a vcFlag byte of `0x00`, for a `T_char` column with the varchar bit clear, SHALL NOT be reported as `VARCHAR`
* *AND* the reported Exasol type name MUST match the type name the WebSocket transport reports for the identical column

### Scenario: NULL handling in column data

* *GIVEN* a result set contains columns with NULL values
* *WHEN* parsing column data
* *THEN* the system SHALL read the null marker byte (0 = NULL, 1 = NOT NULL) before each value
* *AND* NULL positions SHALL be recorded in the Arrow validity bitmap
* *AND* no data bytes SHALL be read for NULL values

### Scenario: Small result set (complete in one message)

* *GIVEN* a query returns a result set with handle `SMALL_RESULTSET` (-3)
* *WHEN* parsing the result
* *THEN* the system SHALL parse all rows from the single response message
* *AND* the system SHALL NOT issue a `CMD_FETCH2` command
* *AND* the system SHALL return a complete Arrow RecordBatch

### Scenario: Large result set (multi-fetch)

* *GIVEN* a query returns a result set with a positive handle
* *AND* the initial message contains fewer rows than total_rows
* *WHEN* retrieving the full result
* *THEN* the system SHALL issue `CMD_FETCH2` commands to retrieve remaining rows
* *AND* each fetch response SHALL be converted to an Arrow RecordBatch
* *AND* the system SHALL close the result set handle after all rows are fetched

### Scenario: Row count result

* *GIVEN* a DML statement (INSERT, UPDATE, DELETE) is executed
* *WHEN* the server responds with `R_RowCount` (0)
* *THEN* the system SHALL extract the 8-byte row count
* *AND* the system SHALL return the count without building a RecordBatch

### Scenario: Empty result

* *GIVEN* a statement that produces no result is executed
* *WHEN* the server responds with `R_Empty` (-2)
* *THEN* the system SHALL return an empty result without error

### Scenario: Multi-value IN-list predicate returns correct row count

* *GIVEN* an authenticated native TCP session exists
* *AND* a table `T` exists with a single `VARCHAR` column `k` populated with the rows `'apple'`, `'banana'`, and `'cherry'`
* *WHEN* executing the SQL text `SELECT COUNT(*) FROM T WHERE k IN ('apple','banana')` over the native TCP transport
* *THEN* the system MUST parse the returned `R_RESULTSET` such that the single row containing the COUNT value is included in the resulting `RecordBatch`
* *AND* the system SHALL surface the COUNT value as `2` to the caller
* *AND* the behavior of the native transport SHALL match the behavior of the WebSocket transport for the identical SQL text

### Scenario: Multi-value IN-list predicate yields the matching rows

* *GIVEN* an authenticated native TCP session exists
* *AND* a table `T` exists with a single `VARCHAR` column `k` populated with the rows `'apple'`, `'banana'`, and `'cherry'`
* *WHEN* executing the SQL text `SELECT k FROM T WHERE k IN ('apple','banana') ORDER BY k` over the native TCP transport
* *THEN* the system MUST return a `RecordBatch` whose row count equals `2`
* *AND* the returned `RecordBatch` SHALL contain the values `'apple'` and `'banana'` in column `k`
* *AND* the result MUST match the rows returned by the WebSocket transport for the identical SQL text

### Scenario: Single-value IN-list predicate continues to behave correctly

* *GIVEN* an authenticated native TCP session exists
* *AND* a table `T` exists with a single `VARCHAR` column `k` populated with the rows `'apple'`, `'banana'`, and `'cherry'`
* *WHEN* executing the SQL text `SELECT COUNT(*) FROM T WHERE k IN ('apple')` over the native TCP transport
* *THEN* the system SHALL return a `RecordBatch` containing the COUNT value `1`
* *AND* the existing single-value IN-list path MUST remain unaffected by the multi-value fix
