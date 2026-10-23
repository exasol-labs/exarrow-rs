# Feature: Arrow RecordBatch Import/Export

Specifies Arrow RecordBatch import and export between applications and Exasol tables.

<!-- DELTA:CHANGED -->
## Background

RecordBatch import converts data to CSV text with the import CSV conversion and streams it through the HTTP tunnel. Export converts the CSV from Exasol into RecordBatches whose schemas reflect Exasol column types.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: The import CSV conversion covers every Arrow type that has an Exasol column type

* *GIVEN* a RecordBatch with one column of each of these Arrow types: Boolean, Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float32, Float64, Utf8, LargeUtf8, Utf8View, `Dictionary(Int8, Utf8)`, `Dictionary(UInt64, LargeUtf8)`, Binary, LargeBinary, BinaryView, `FixedSizeBinary(3)`, Date32, Date64, Timestamp of each time unit with and without a time zone, `Decimal128(36, 2)`, `Decimal256(36, 2)`, `Interval(YearMonth)`, `Interval(DayTime)`, and `Interval(MonthDayNano)`, plus the columns `Decimal128(38, 2)`, `Decimal128(38, -5)`, and `Decimal256(76, -10)`, which have no Exasol column type
* *WHEN* the RecordBatch is converted to CSV for `import_from_record_batch`
* *THEN* every column SHALL convert without an error, a Date64 value SHALL become the `YYYY-MM-DD` text of the day that holds it, so `1_577_836_800_000` ms becomes `2020-01-01`, and a Decimal256 value SHALL become its decimal text with the column's scale, so `-123456` at scale 2 becomes `-1234.56`
* *AND* a decimal value with a negative scale SHALL become its integer digits followed by one zero per unit of negative scale, so `Decimal128(38, -5)` holding 10^37 becomes `1` followed by 42 zeros, and `Decimal256(76, -10)` holding 10^76 - 1 becomes 76 nines followed by 10 zeros
* *AND* a Dictionary value SHALL become the text of the dictionary value its key refers to, a null key or a null dictionary value SHALL become the null marker, and a Utf8View value SHALL be written as the same Utf8 value is written, and a BinaryView or FixedSizeBinary value SHALL become the lowercase hexadecimal text of its bytes, as a Binary value does
* *AND* a column whose Arrow type has no conversion, such as `Time32(Second)`, `Duration(Second)`, or `List(Int32)`, SHALL fail the import CSV conversion with `ImportError::ConversionError` that names the Arrow type
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: RecordBatch import writes Arrow interval values as Exasol INTERVAL text

* *GIVEN* a table `T` with the columns `ID DECIMAL(18,0)`, `YM INTERVAL YEAR(9) TO MONTH`, `DT INTERVAL DAY(9) TO SECOND(3)`, and `MDN INTERVAL DAY(9) TO SECOND(3)`
* *AND* a RecordBatch with the fields `ID` (Int64), `YM` (`Interval(YearMonth)`), `DT` (`Interval(DayTime)`), and `MDN` (`Interval(MonthDayNano)`), holding these rows:
  * `ID` 1: `YM` 63 months, `DT` 2 days and 46_210_123 ms, `MDN` 0 months, 2 days, and 46_210_123_000_000 ns
  * `ID` 2: `YM` -63 months, `DT` -2 days and -46_210_123 ms, `MDN` 0 months, -2 days, and -46_210_123_000_000 ns
  * `ID` 3: `YM` 2_147_483_647 months, `DT` 0 days and 86_400_000 ms, `MDN` 0 months, 1 day, and -1_000_000 ns
  * `ID` 4: NULL in `YM`, `DT`, and `MDN`
* *WHEN* the application imports the RecordBatch into `T` with `import_from_record_batch`
* *THEN* the import CSV conversion SHALL write a year-month value as its years, a `-`, and its months in two digits, and a day-time value as its days, a space, and its time as `HH:MM:SS.fff`, with a leading `-` for a negative value and no sign otherwise, after it adds the days and the time part of a `DayTime` or `MonthDayNano` value as days of 86_400_000 ms, so one sign applies to the whole value
* *AND* the import CSV conversion SHALL write `YM` as `5-03`, `-5-03`, and `178956970-07`, `DT` as `2 12:50:10.123`, `-2 12:50:10.123`, and `1 00:00:00.000`, and `MDN` as `2 12:50:10.123`, `-2 12:50:10.123`, and `0 23:59:59.999`, for `ID` 1, 2, and 3
* *AND* for `ID` 1, 2, and 3, `YM` in `T` SHALL equal `CAST('5-3' AS INTERVAL YEAR(9) TO MONTH)`, `CAST('-5-3' AS INTERVAL YEAR(9) TO MONTH)`, and `CAST('178956970-7' AS INTERVAL YEAR(9) TO MONTH)`, `DT` SHALL equal `CAST('2 12:50:10.123' AS INTERVAL DAY(9) TO SECOND(3))`, `CAST('-2 12:50:10.123' AS INTERVAL DAY(9) TO SECOND(3))`, and `CAST('1 00:00:00.000' AS INTERVAL DAY(9) TO SECOND(3))`, and `MDN` SHALL equal `DT` for `ID` 1 and 2 and `CAST('0 23:59:59.999' AS INTERVAL DAY(9) TO SECOND(3))` for `ID` 3
* *AND* `YM`, `DT`, and `MDN` SHALL be NULL for `ID` 4
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The import CSV conversion rejects interval values that INTERVAL DAY TO SECOND cannot hold

* *GIVEN* a RecordBatch with one of these values: an `Interval(MonthDayNano)` value of 1 month, 0 days, and 0 ns, an `Interval(MonthDayNano)` value of 0 months, 0 days, and 1_500_000 ns, an `Interval(MonthDayNano)` value of 0 months, 1_000_000_000 days, and 0 ns, or an `Interval(DayTime)` value of -1_000_000_000 days and 0 ms
* *WHEN* the application imports the RecordBatch with `import_from_record_batch`
* *THEN* the import CSV conversion SHALL return `ImportError::ConversionError` that names the value, and the import SHALL fail before the driver sends the IMPORT statement
* *AND* the message for the month value SHALL state that Exasol INTERVAL DAY TO SECOND has no month component
* *AND* the message for the 1_500_000 ns value SHALL state that Exasol stores INTERVAL DAY TO SECOND values in whole milliseconds
* *AND* the message for each day count SHALL state that INTERVAL DAY(9) TO SECOND holds at most 999,999,999 days
<!-- /DELTA:NEW -->
