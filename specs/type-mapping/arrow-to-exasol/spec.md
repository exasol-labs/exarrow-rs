# Feature: Arrow to Exasol

Defines the mapping from Apache Arrow data types to Exasol data types for parameter binding, with precision and scale handling.

## Background

The system defines mappings from Arrow types to Exasol types for parameter binding and DDL generation. Arrow Decimal128 preserves precision (1-36) and scale for Exasol DECIMAL types; Decimal256 is never used for Exasol-originated types since Exasol precision does not exceed 36. Values that exceed Arrow type capacity during conversion produce type conversion errors with column and value details.

## Scenarios

### Scenario: Arrow numeric to Exasol

* *GIVEN* Arrow data needs conversion to Exasol types
* *WHEN* binding Arrow numeric types
* *THEN* Arrow Int64 SHALL convert to Exasol INTEGER or DECIMAL
* *AND* Arrow Float64 SHALL convert to Exasol DOUBLE
* *AND* Arrow Decimal128 SHALL convert to Exasol DECIMAL

### Scenario: Arrow string to Exasol

* *GIVEN* Arrow data needs conversion to Exasol types
* *WHEN* binding Arrow string types
* *THEN* Arrow Utf8 SHALL convert to Exasol VARCHAR
* *AND* Arrow LargeUtf8 SHALL convert to Exasol VARCHAR or CLOB

### Scenario: Arrow temporal to Exasol

* *GIVEN* Arrow data needs conversion to Exasol types
* *WHEN* binding Arrow temporal types
* *THEN* Arrow Date32 SHALL convert to Exasol DATE
* *AND* Arrow Timestamp SHALL convert to Exasol TIMESTAMP

### Scenario: DECIMAL precision mapping

* *GIVEN* Arrow data needs conversion to Exasol types
* *WHEN* mapping Exasol DECIMAL(p, s) type
* *THEN* Arrow Decimal128 SHALL preserve precision p (1-36) and scale s
* *AND* it SHALL always use Decimal128 since Exasol precision never exceeds 36
* *AND* Decimal256 SHALL NOT be used for Exasol-originated types

### Scenario: Overflow detection

* *GIVEN* Arrow data needs conversion to Exasol types
* *WHEN* a value exceeds Arrow type capacity
* *THEN* it SHALL return a type conversion error
* *AND* it SHALL specify which column and value caused the overflow

### Scenario: Parameter binding formats pre-epoch DATE and TIMESTAMP values and rejects values outside Exasol's range

* *GIVEN* a Date32 parameter value -731 and a `Timestamp(Microsecond, None)` parameter value -1
* *WHEN* the driver binds the values as prepared-statement parameters
* *THEN* the bound text SHALL be `1968-01-01` and `1969-12-31 23:59:59.999999`
* *AND* a Date32 value outside 0001-01-01 to 9999-12-31, such as 2932897, SHALL fail with `AdbcStatus::InvalidArguments`
* *AND* a Timestamp value outside 0001-01-01 00:00:00 to 9999-12-31 23:59:59.999999, or one that overflows when converted to microseconds, SHALL fail with `AdbcStatus::InvalidArguments`
