# Feature: Arrow to Exasol

Defines the mapping from Arrow data types to Exasol data types for parameter binding and DDL generation, with precision and scale handling.

## Background

Arrow Decimal128 preserves precision (1-36) and scale for Exasol DECIMAL types. Decimal256 is never used for Exasol-originated types because Exasol precision does not exceed 36.

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

### Scenario: Bound Arrow Timestamp and Date32 values are stored on both transports

* *GIVEN* an ADBC statement with a prepared `INSERT` that has one parameter per table column, over the native TCP transport or the WebSocket transport, in a session that uses the server's default time zone
* *AND* a RecordBatch whose rows hold a value after 1970, a value before 1970, and a row of nulls, with these columns: Timestamp columns of the units Second, Millisecond, Microsecond, and Nanosecond without a time zone and of the unit Microsecond with the time zone `Europe/Berlin`, each bound to a `TIMESTAMP(6)` column; a Timestamp column of the unit Microsecond with the time zone `UTC`, bound to a `TIMESTAMP(6) WITH LOCAL TIME ZONE` column; and a Date32 column, bound to a `DATE` column
* *WHEN* the RecordBatch is bound and `execute_update` is called
* *THEN* `execute_update` SHALL return the number of bound rows, and the connection SHALL remain usable for later statements
* *AND* selecting each `TIMESTAMP(6)` column back SHALL return the bound instant as microseconds since 1970-01-01 00:00:00, such as `1_704_096_000_123_456` µs for the Nanosecond value `1_704_096_000_123_456_000` and `-1_000_000` µs for the Second value `-1`
* *AND* a timestamp value with a time zone SHALL be bound as its UTC date and time, so that the `Europe/Berlin` value `1_704_096_000_000_000` µs is stored in its `TIMESTAMP(6)` column as `2024-01-01 08:00:00`
* *AND* selecting the `TIMESTAMP(6) WITH LOCAL TIME ZONE` column back in the same session SHALL return the bound UTC date and time as written, so that the `UTC` value `1_704_096_000_000_000` µs reads back as `2024-01-01 08:00:00`
* *AND* selecting the date column back SHALL return the bound Date32 values, and every null value SHALL be stored as NULL
