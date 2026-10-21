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
* *AND* INTEGER SHALL map to Arrow Int64
* *AND* SMALLINT SHALL map to Arrow Int32 or Int16

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

* *GIVEN* Exasol returns a result set with typed columns
* *WHEN* encountering Exasol GEOMETRY types
* *THEN* it SHALL map to Arrow Binary or Utf8 (WKT/WKB format)
* *AND* it SHALL document the encoding used

### Scenario: Unsupported types

* *GIVEN* Exasol returns a result set with typed columns
* *WHEN* an unsupported Exasol type is encountered
* *THEN* it SHALL return a clear error message
* *AND* it SHALL specify the unsupported type name
* *AND* it SHALL provide guidance on workarounds if available
