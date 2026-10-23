# Feature: Boundaries and Validation

Specifies type compatibility validation rules and Exasol data type boundary enforcement for DDL generation and type mapping.

<!-- DELTA:CHANGED -->
## Background

Conversions prefer lossless transformations and warn or error on lossy conversions, except for February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow-dates` maps to March 1. Exasol type limits apply during DDL generation and type mapping. Exasol DECIMAL precision is 1 to 36, with a scale from 0 to the precision.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: String, binary, date, timestamp, boolean, and decimal type mappings for DDL generation

* *GIVEN* DDL generation is requested for a table
* *WHEN* `TypeMapper::arrow_to_exasol` maps an Arrow type to an Exasol column type
* *THEN* Boolean SHALL map to BOOLEAN, and Date32 and Date64 SHALL map to DATE
* *AND* Utf8, LargeUtf8, Utf8View, a Dictionary with any integer key type and Utf8 or LargeUtf8 values, Binary, LargeBinary, BinaryView, and FixedSizeBinary of any width SHALL map to VARCHAR(2000000)
* *AND* Timestamp of any unit SHALL map to TIMESTAMP without a time zone and to TIMESTAMP WITH LOCAL TIME ZONE with a time zone
* *AND* Decimal128(p, s) and Decimal256(p, s) with a precision p up to 36 and a scale s from 0 to p SHALL map to DECIMAL(p,s)
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Arrow types without an Exasol column type are rejected

* *GIVEN* DDL generation is requested for a table
* *WHEN* `TypeMapper::arrow_to_exasol` maps Decimal128 or Decimal256 with a precision above 36, a negative scale, or a scale above the precision, an Interval of any unit, Time32, Time64, Duration, Null, Float16, a list, struct, map, or union type, or a Dictionary whose values are not Utf8 or LargeUtf8
* *THEN* it SHALL return `ConversionError::UnsupportedType`, and the error message SHALL name the Arrow type
<!-- /DELTA:NEW -->
