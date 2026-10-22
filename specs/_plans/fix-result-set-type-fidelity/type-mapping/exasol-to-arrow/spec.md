# Feature: Exasol to Arrow

Defines the mapping from Exasol data types to Apache Arrow data types, including complex types and NULL handling.

## Background

Exasol DECIMAL precision does not exceed 36, so Decimal128 is always sufficient. The field metadata that carries the Exasol type is specified in `type-mapping/exasol-to-arrow-field-metadata`.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: GEOMETRY type handling

* *GIVEN* a query over the native or the WebSocket transport returns a GEOMETRY column, such as `SELECT CAST('POINT(1 2)' AS GEOMETRY) AS G`, whose values Exasol sends as WKT text, such as `POINT (1 2)`
* *WHEN* the driver builds the result set metadata schema and the RecordBatches of the result set
* *THEN* the field `G` SHALL have the Arrow data type Binary in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`
* *AND* each value SHALL hold the bytes of the WKT text unchanged, such as the bytes of `POINT (1 2)`
* *AND* `TypeMapper::exasol_to_arrow` SHALL map GEOMETRY to Arrow Binary
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: INTERVAL type mapping

* *GIVEN* a query over the native or the WebSocket transport returns the columns `CAST('1-2' AS INTERVAL YEAR(2) TO MONTH) AS YM` and `CAST('-3 04:05:06.789' AS INTERVAL DAY(2) TO SECOND(3)) AS DS`, whose values Exasol sends as the text `+01-02` and `-03 04:05:06.789`
* *WHEN* the driver builds the result set metadata schema and the RecordBatches of the result set
* *THEN* the fields `YM` and `DS` SHALL have the Arrow data type Interval(MonthDayNano) in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`
* *AND* the value of `YM` SHALL hold 14 months, 0 days, and 0 nanoseconds
* *AND* the value of `DS` SHALL hold 0 months, -3 days, and -14706789000000 nanoseconds, because the sign of the text applies to every component
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: INTERVAL YEAR TO MONTH value outside the Arrow month range

* *GIVEN* a query over the native or the WebSocket transport returns `CAST('999999999-11' AS INTERVAL YEAR(9) TO MONTH) AS BIG`, whose month count exceeds the 32-bit signed integer range of an Arrow Interval(MonthDayNano) value
* *WHEN* the driver builds the RecordBatch of the result set
* *THEN* the query SHALL fail with an error that names the column `BIG`
* *AND* the driver MUST NOT return a truncated or wrapped month count
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: HASHTYPE type mapping

* *GIVEN* a session with the default `HASHTYPE_FORMAT` (`HEX`) runs a query over the native or the WebSocket transport that returns `CAST('550e8400-e29b-11d4-a716-446655440000' AS HASHTYPE(16 BYTE)) AS H`, whose value Exasol sends as the text `550e8400e29b11d4a716446655440000`
* *WHEN* the driver builds the result set metadata schema and the RecordBatches of the result set
* *THEN* the field `H` SHALL have the Arrow data type Binary in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`
* *AND* the value SHALL hold the bytes of the text that Exasol sends, unchanged, such as the 32 bytes of `550e8400e29b11d4a716446655440000`
* *AND* `TypeMapper::exasol_to_arrow` SHALL map HASHTYPE to Arrow Binary
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: WebSocket INTERVAL, GEOMETRY, and HASHTYPE results equal native results

* *GIVEN* a query returns an INTERVAL YEAR TO MONTH, INTERVAL DAY TO SECOND, GEOMETRY, or HASHTYPE value in a first row and a NULL value of the same type in a second row
* *WHEN* the query runs over the native transport and over the WebSocket transport
* *THEN* each field of the WebSocket batch schema SHALL have the same Arrow data type as the native field of the same column
* *AND* each WebSocket value SHALL equal the native value of the same row and column
* *AND* each value of the second row SHALL be NULL on both transports
<!-- /DELTA:NEW -->
