# Feature: Exasol to Arrow

Defines the mapping from Exasol data types to Apache Arrow data types, including complex types and NULL handling.

## Background

Exasol DECIMAL precision does not exceed 36, so Decimal128 is always sufficient. The field metadata that carries the Exasol type is specified in `type-mapping/exasol-to-arrow-field-metadata`.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Non-decimal numeric type mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol non-decimal numeric types
* *THEN* DOUBLE SHALL map to Arrow Float64
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: INTEGER and SMALLINT type mapping

* *GIVEN* a query over the native or the WebSocket transport returns an INTEGER column, such as `CAST(-999999999999999999 AS INTEGER) AS I`, or a SMALLINT column, such as `CAST(999999999 AS SMALLINT) AS S`, which Exasol reports as DECIMAL(18,0) and DECIMAL(9,0), because INTEGER and SMALLINT are aliases of these DECIMAL types
* *WHEN* the driver builds the result set metadata schema and the RecordBatches of the result set
* *THEN* an INTEGER field, such as `I`, SHALL have the Arrow data type Decimal128(18, 0), and a SMALLINT field, such as `S`, SHALL have the Arrow data type Decimal128(9, 0), in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`
* *AND* each value SHALL be the Decimal128 value of the integer that Exasol returns, such as -999999999999999999 for `I` and 999999999 for `S`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: WebSocket INTEGER and SMALLINT results equal native results

* *GIVEN* a query returns an INTEGER or SMALLINT value in a first row and a NULL value of the same type in a second row
* *WHEN* the query runs over the native transport and over the WebSocket transport
* *THEN* each field of the WebSocket batch schema SHALL have the same Arrow data type as the native field of the same column
* *AND* each WebSocket value SHALL equal the native value of the same row and column
* *AND* each value of the second row SHALL be NULL on both transports
<!-- /DELTA:NEW -->
