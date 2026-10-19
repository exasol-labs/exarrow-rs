# Feature: Boundaries and Validation

Specifies type compatibility validation rules and Exasol data type boundary enforcement for DDL generation and type mapping.

## Background

Conversions prefer lossless transformations and warn or error on lossy conversions, except for February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow-dates` maps to March 1. Exasol type limits apply during DDL generation and type mapping.

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
