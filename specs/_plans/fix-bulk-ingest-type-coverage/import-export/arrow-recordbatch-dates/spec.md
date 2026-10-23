# Feature: Arrow RecordBatch DATE and TIMESTAMP Conversion

Specifies how RecordBatch import and export convert DATE and TIMESTAMP values, including values before 1970 and values outside Exasol's range.

<!-- DELTA:CHANGED -->
## Background

RecordBatch import converts Date32, Date64, and Timestamp values to CSV text by their year, month, and day in the proleptic Gregorian calendar, and export converts the CSV text that Exasol sends back to the same Date32 and Timestamp values. Exasol accepts DATE and TIMESTAMP values from 0001-01-01 to 9999-12-31. The general RecordBatch import and export behavior is specified in `import-export/arrow-recordbatch`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: RecordBatch import formats a pre-epoch Date64 value as the day that holds it

* *GIVEN* a RecordBatch with a Date64 column holding the millisecond values -1, -86_400_000, and -86_400_001
* *WHEN* the import converts the values to CSV text
* *THEN* the text SHALL be `1969-12-31`, `1969-12-31`, and `1969-12-30`, in that order
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range

* *GIVEN* a RecordBatch with a Date32 value 2932897 (10000-01-01), a Date32 value -719163 (0000-12-31), a Date64 value 253_402_300_800_000 (10000-01-01), a Date64 value 9_223_372_036_854_775_807, a `Timestamp(Second, None)` value 9223372036854775807, or a `Timestamp(Microsecond, None)` value 253402300800000000 (10000-01-01 00:00:00)
* *WHEN* the import converts the value to CSV text
* *THEN* the conversion SHALL return `ImportError::ConversionError` that names the value, and the import SHALL fail
<!-- /DELTA:CHANGED -->
