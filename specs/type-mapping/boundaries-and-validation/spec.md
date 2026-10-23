# Feature: Boundaries and Validation

Specifies type compatibility validation rules and Exasol data type boundary enforcement for DDL generation and type mapping.

## Background

Conversions prefer lossless transformations and warn or error on lossy conversions, except for February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow-dates` maps to March 1. Exasol type limits apply during DDL generation and type mapping. Exasol DECIMAL precision is 1 to 36, with a scale from 0 to the precision. The leading field precision of an Exasol INTERVAL type is 1 to 9.

## Scenarios

### Scenario: Lossless conversion validation

* *GIVEN* a type conversion is being performed
* *WHEN* converting between types
* *THEN* it SHALL prefer lossless conversions
* *AND* it SHALL warn or error on lossy conversions
* *AND* February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow-dates` maps to March 1, SHALL follow that scenario without a warning or an error

### Scenario: Timestamp timezone handling

* *GIVEN* a type conversion is being performed
* *WHEN* converting timestamps
* *THEN* it SHALL handle timezone-aware and timezone-naive timestamps correctly
* *AND* it SHALL preserve or convert timezones as specified
* *AND* it SHALL document timezone handling behavior

### Scenario: Integer type mappings for DDL generation

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping Arrow integer types to Exasol DDL
* *THEN* Int8, Int16, Int32 SHALL map to DECIMAL(18,0)
* *AND* Int64 SHALL map to DECIMAL(36,0)
* *AND* UInt8, UInt16, UInt32 SHALL map to DECIMAL(18,0)
* *AND* UInt64 SHALL map to DECIMAL(36,0)

### Scenario: Floating point type mappings for DDL generation

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping Arrow floating point types to Exasol DDL
* *THEN* Float32 and Float64 SHALL map to DOUBLE

### Scenario: String, binary, date, timestamp, interval, boolean, and decimal type mappings for DDL generation

* *GIVEN* DDL generation is requested for a table
* *WHEN* `TypeMapper::arrow_to_exasol` maps an Arrow type to an Exasol column type
* *THEN* Boolean SHALL map to BOOLEAN, Date32 and Date64 SHALL map to DATE, and Timestamp of any unit SHALL map to TIMESTAMP without a time zone and to TIMESTAMP WITH LOCAL TIME ZONE with a time zone
* *AND* Utf8, LargeUtf8, Utf8View, a Dictionary with any integer key type and Utf8 or LargeUtf8 values, Binary, LargeBinary, BinaryView, and FixedSizeBinary of any width SHALL map to VARCHAR(2000000)
* *AND* Decimal128(p, s) and Decimal256(p, s) with a precision p up to 36 and a scale s from 0 to p SHALL map to DECIMAL(p,s)
* *AND* Interval(YearMonth) SHALL map to INTERVAL YEAR(9) TO MONTH, and Interval(DayTime) and Interval(MonthDayNano) SHALL map to INTERVAL DAY(9) TO SECOND(3), which use the largest leading field precision, 9

### Scenario: Arrow types without an Exasol column type are rejected

* *GIVEN* DDL generation is requested for a table
* *WHEN* `TypeMapper::arrow_to_exasol` maps Decimal128 or Decimal256 with a precision above 36, a negative scale, or a scale above the precision, Time32, Time64, Duration, Null, Float16, a list, struct, map, or union type, or a Dictionary whose values are not Utf8 or LargeUtf8
* *THEN* it SHALL return `ConversionError::UnsupportedType`, and the error message SHALL name the Arrow type
