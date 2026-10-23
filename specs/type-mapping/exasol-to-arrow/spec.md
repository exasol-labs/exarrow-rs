# Feature: Exasol to Arrow

Defines the mapping from Exasol data types to Apache Arrow data types, including complex types and NULL handling.

## Background

Exasol DECIMAL precision does not exceed 36, so Decimal128 is always sufficient. The field metadata that carries the Exasol type is specified in `type-mapping/exasol-to-arrow-field-metadata`.

## Scenarios

### Scenario: Decimal type mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol DECIMAL types
* *THEN* DECIMAL(p, s) with p in range 1-36 SHALL map to Arrow Decimal128(p, s)
* *AND* Exasol DECIMAL precision SHALL NOT exceed 36 (Exasol's documented maximum)

### Scenario: Non-decimal numeric type mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol non-decimal numeric types
* *THEN* DOUBLE SHALL map to Arrow Float64

### Scenario: String types mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol string types
* *THEN* VARCHAR(n) SHALL map to Arrow Utf8
* *AND* CHAR(n) SHALL map to Arrow Utf8 with padding handling
* *AND* CLOB SHALL map to Arrow Utf8

### Scenario: Boolean type mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol BOOLEAN type
* *THEN* it SHALL map to Arrow Boolean

### Scenario: Binary types mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol binary types
* *THEN* VARBINARY SHALL map to Arrow Binary
* *AND* BLOB SHALL map to Arrow LargeBinary

### Scenario: NULL handling

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* a column allows NULL values
* *THEN* the Arrow field SHALL be marked as nullable
* *AND* NULL values SHALL be represented in Arrow null bitmaps

### Scenario: GEOMETRY type handling

* *GIVEN* a query over the native or the WebSocket transport returns a GEOMETRY column, such as `SELECT CAST('POINT(1 2)' AS GEOMETRY) AS G`, whose values Exasol sends as WKT text, such as `POINT (1 2)`
* *WHEN* the driver builds the result set metadata schema and the RecordBatches of the result set
* *THEN* the field `G` SHALL have the Arrow data type Binary in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`
* *AND* each value SHALL hold the bytes of the WKT text unchanged, such as the bytes of `POINT (1 2)`
* *AND* `TypeMapper::exasol_to_arrow` SHALL map GEOMETRY to Arrow Binary

### Scenario: Unsupported types

* *GIVEN* Exasol returns a result set with typed columns
* *WHEN* an unsupported Exasol type is encountered
* *THEN* it SHALL return a clear error message
* *AND* it SHALL specify the unsupported type name
* *AND* it SHALL provide guidance on workarounds if available

### Scenario: INTEGER and SMALLINT type mapping

* *GIVEN* a query over the native or the WebSocket transport returns an INTEGER column, such as `CAST(-999999999999999999 AS INTEGER) AS I`, or a SMALLINT column, such as `CAST(999999999 AS SMALLINT) AS S`, which Exasol reports as DECIMAL(18,0) and DECIMAL(9,0), because INTEGER and SMALLINT are aliases of these DECIMAL types
* *WHEN* the driver builds the result set metadata schema and the RecordBatches of the result set
* *THEN* an INTEGER field, such as `I`, SHALL have the Arrow data type Decimal128(18, 0), and a SMALLINT field, such as `S`, SHALL have the Arrow data type Decimal128(9, 0), in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`
* *AND* each value SHALL be the Decimal128 value of the integer that Exasol returns, such as -999999999999999999 for `I` and 999999999 for `S`

### Scenario: WebSocket INTEGER and SMALLINT results equal native results

* *GIVEN* a query returns an INTEGER or SMALLINT value in a first row and a NULL value of the same type in a second row
* *WHEN* the query runs over the native transport and over the WebSocket transport
* *THEN* each field of the WebSocket batch schema SHALL have the same Arrow data type as the native field of the same column
* *AND* each WebSocket value SHALL equal the native value of the same row and column
* *AND* each value of the second row SHALL be NULL on both transports
