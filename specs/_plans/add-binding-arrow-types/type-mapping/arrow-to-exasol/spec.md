# Feature: Arrow to Exasol

Defines the mapping from Arrow data types to Exasol data types for parameter binding and DDL generation, with precision and scale handling.

## Background

Arrow Decimal128 preserves precision (1-36) and scale for Exasol DECIMAL types. Decimal256 is never used for Exasol-originated types because Exasol precision does not exceed 36.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Bound integer, Date64, and decimal values are stored on both transports

* *GIVEN* an ADBC statement with a prepared `INSERT` that has one parameter per table column, over the native TCP transport or the WebSocket transport
* *AND* a RecordBatch with a row of values and a row of nulls in these columns: `Int8` holding `-128`, `UInt8` holding `255`, `UInt16` holding `65535`, `UInt32` holding `4294967295`, `UInt64` holding `18446744073709551615`, `Date64` holding `1_577_836_800_000` ms, `Decimal128(36, 2)` holding `-1234567890123456789012345678901234.56`, and `Decimal256(36, 2)` holding `1234567890123456789012345678901234.56`
* *AND* the table columns `DECIMAL(3,0)`, `DECIMAL(3,0)`, `DECIMAL(5,0)`, `DECIMAL(10,0)`, `DECIMAL(20,0)`, `DATE`, `DECIMAL(36,2)`, and `DECIMAL(36,2)`, in that order
* *WHEN* the RecordBatch is bound and `execute_update` is called
* *THEN* `execute_update` SHALL return 2
* *AND* selecting the first row back SHALL return `-128`, `255`, `65535`, `4294967295`, `18446744073709551615`, `2020-01-01`, `-1234567890123456789012345678901234.56`, and `1234567890123456789012345678901234.56`, with no digit lost or rounded
* *AND* every value of the second row SHALL be NULL
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Bound string view, string dictionary, binary, and Null values are stored on both transports

* *GIVEN* an ADBC statement with a prepared `INSERT` that has one parameter per table column, over the native TCP transport or the WebSocket transport, into a table whose columns are all `VARCHAR(100)`
* *AND* a RecordBatch with a row of values and a row of nulls in these columns: `Utf8View` holding `a,b "c"`, `Dictionary(Int32, Utf8)` holding `alpha`, `Dictionary(UInt32, LargeUtf8)` holding `beta`, `BinaryView` holding the bytes `0x00 0xAB 0xFF`, `FixedSizeBinary(3)` holding the bytes `0x00 0xAB 0xFF`, and `Null`
* *WHEN* the RecordBatch is bound and `execute_update` is called
* *THEN* `execute_update` SHALL return 2
* *AND* selecting the first row back SHALL return `a,b "c"` unchanged, with no quoting or escaping added, then `alpha`, `beta`, `00abff`, `00abff`, and NULL
* *AND* every value of the second row SHALL be NULL
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Bound Interval values are stored on both transports

* *GIVEN* an ADBC statement with a prepared `INSERT` that has one parameter per table column, over the native TCP transport or the WebSocket transport
* *AND* a RecordBatch with a row of values and a row of nulls in these columns: `Interval(YearMonth)` holding 14 months, `Interval(DayTime)` holding 3 days and 3_723_456 ms, and `Interval(MonthDayNano)` holding 0 months, 3 days, and 3_723_456_000_000 ns
* *AND* the table columns `INTERVAL YEAR(9) TO MONTH`, `INTERVAL DAY(9) TO SECOND(3)`, and `INTERVAL DAY(9) TO SECOND(3)`, in that order
* *WHEN* the RecordBatch is bound and `execute_update` is called
* *THEN* `execute_update` SHALL return 2
* *AND* selecting the first row back SHALL return 1 year and 2 months, and twice 3 days, 1 hour, 2 minutes, and 3.456 seconds
* *AND* every value of the second row SHALL be NULL
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A bound value that Exasol cannot store without loss fails before execution

* *GIVEN* an ADBC statement with a prepared `INSERT` and a bound RecordBatch of several rows, one of which holds one of these values: a `Decimal256(40, 0)` value with 37 digits, a `Decimal256(38, 37)` value with 37 digits after the decimal point, an `Interval(MonthDayNano)` value with a month component, or an `Interval(MonthDayNano)` value with a fraction of a millisecond
* *WHEN* `execute_update` or `execute` is called
* *THEN* the driver SHALL return an error with status `InvalidArguments` whose message names the value's Arrow type
* *AND* the driver MUST NOT send an execution request to Exasol
* *AND* the target table SHALL contain none of the bound rows
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A bound Arrow type without an Exasol type fails with NotImplemented

* *GIVEN* an ADBC statement with a prepared `INSERT` and a bound RecordBatch with a column of one of the Arrow types `Time32(Second)`, `Time64(Microsecond)`, `Duration(Second)`, `List(Int32)`, or `Dictionary(Int32, Int64)`, holding a value or only nulls
* *WHEN* `execute_update` or `execute` is called
* *THEN* the driver SHALL return an error with status `NotImplemented` whose message names the Arrow type
* *AND* the driver MUST NOT send an execution request to Exasol
<!-- /DELTA:NEW -->
