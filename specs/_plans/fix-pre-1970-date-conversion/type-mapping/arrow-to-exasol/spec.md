# Feature: Arrow to Exasol

Defines the mapping from Apache Arrow data types to Exasol data types for parameter binding, with precision and scale handling.

## Background

The system defines mappings from Arrow types to Exasol types for parameter binding and DDL generation. Arrow Decimal128 preserves precision (1-36) and scale for Exasol DECIMAL types; Decimal256 is never used for Exasol-originated types since Exasol precision does not exceed 36. Values that exceed Arrow type capacity during conversion produce type conversion errors with column and value details.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Parameter binding formats pre-epoch DATE and TIMESTAMP values and rejects values outside Exasol's range

* *GIVEN* a Date32 parameter value -731 and a `Timestamp(Microsecond, None)` parameter value -1
* *WHEN* the driver binds the values as prepared-statement parameters
* *THEN* the bound text SHALL be `1968-01-01` and `1969-12-31 23:59:59.999999`
* *AND* a Date32 value outside 0001-01-01 to 9999-12-31, such as 2932897, SHALL fail with `AdbcStatus::InvalidArguments`
* *AND* a Timestamp value outside 0001-01-01 00:00:00 to 9999-12-31 23:59:59.999999, or one that overflows when converted to microseconds, SHALL fail with `AdbcStatus::InvalidArguments`
<!-- /DELTA:NEW -->
