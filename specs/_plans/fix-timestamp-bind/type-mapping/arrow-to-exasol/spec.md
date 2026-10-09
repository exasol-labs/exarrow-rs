# Feature: Arrow to Exasol

Defines the mapping from Apache Arrow data types to Exasol data types for parameter binding, with precision and scale handling.

## Background

The system defines mappings from Arrow types to Exasol types for parameter binding and DDL generation. Arrow Decimal128 preserves precision (1-36) and scale for Exasol DECIMAL types; Decimal256 is never used for Exasol-originated types since Exasol precision does not exceed 36. Values that exceed Arrow type capacity during conversion produce type conversion errors with column and value details.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Bound Arrow Timestamp and Date32 values are stored on both transports

* *GIVEN* an ADBC statement with a prepared `INSERT` that has one parameter per table column, over the native TCP transport or the WebSocket transport, in a session that uses the server's default time zone
* *AND* a RecordBatch whose rows hold a value after 1970, a value before 1970, and a row of nulls, with these columns: Timestamp columns of the units Second, Millisecond, Microsecond, and Nanosecond without a time zone and of the unit Microsecond with the time zone `Europe/Berlin`, each bound to a `TIMESTAMP(6)` column; a Timestamp column of the unit Microsecond with the time zone `UTC`, bound to a `TIMESTAMP(6) WITH LOCAL TIME ZONE` column; and a Date32 column, bound to a `DATE` column
* *WHEN* the RecordBatch is bound and `execute_update` is called
* *THEN* `execute_update` SHALL return the number of bound rows, and the connection SHALL remain usable for later statements
* *AND* selecting each `TIMESTAMP(6)` column back SHALL return the bound instant as microseconds since 1970-01-01 00:00:00, such as `1_704_096_000_123_456` µs for the Nanosecond value `1_704_096_000_123_456_000` and `-1_000_000` µs for the Second value `-1`
* *AND* a timestamp value with a time zone SHALL be bound as its UTC date and time, so that the `Europe/Berlin` value `1_704_096_000_000_000` µs is stored in its `TIMESTAMP(6)` column as `2024-01-01 08:00:00`
* *AND* selecting the `TIMESTAMP(6) WITH LOCAL TIME ZONE` column back in the same session SHALL return the bound UTC date and time as written, so that the `UTC` value `1_704_096_000_000_000` µs reads back as `2024-01-01 08:00:00`
* *AND* selecting the date column back SHALL return the bound Date32 values, and every null value SHALL be stored as NULL
<!-- /DELTA:NEW -->
