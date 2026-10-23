# Feature: Arrow RecordBatch Interval Import

Specifies how RecordBatch import writes Arrow interval values as Exasol INTERVAL text, and which interval values it rejects. The conversion of the other Arrow types is specified in `import-export/arrow-recordbatch`.

## Background

RecordBatch import converts data to CSV text with the import CSV conversion and streams it through the HTTP tunnel. Exasol INTERVAL DAY TO SECOND has no month component, stores whole milliseconds, and holds at most 999,999,999 days.

## Scenarios

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

### Scenario: The import CSV conversion rejects interval values that INTERVAL DAY TO SECOND cannot hold

* *GIVEN* a RecordBatch with one of these values: an `Interval(MonthDayNano)` value of 1 month, 0 days, and 0 ns, an `Interval(MonthDayNano)` value of 0 months, 0 days, and 1_500_000 ns, an `Interval(MonthDayNano)` value of 0 months, 1_000_000_000 days, and 0 ns, or an `Interval(DayTime)` value of -1_000_000_000 days and 0 ms
* *WHEN* the application imports the RecordBatch with `import_from_record_batch`
* *THEN* the import CSV conversion SHALL return `ImportError::ConversionError` that names the value, and the import SHALL fail before the driver sends the IMPORT statement
* *AND* the message for the month value SHALL state that Exasol INTERVAL DAY TO SECOND has no month component
* *AND* the message for the 1_500_000 ns value SHALL state that Exasol stores INTERVAL DAY TO SECOND values in whole milliseconds
* *AND* the message for each day count SHALL state that INTERVAL DAY(9) TO SECOND holds at most 999,999,999 days
