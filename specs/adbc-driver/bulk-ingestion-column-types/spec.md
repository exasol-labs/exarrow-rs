# Feature: Bulk Ingestion Column Types

Specifies which Arrow column types ADBC bulk ingest accepts, how it stores them in Exasol, and how it rejects the types it cannot ingest before it sends the first SQL statement. The ingest modes and the target catalog are specified in `adbc-driver/bulk-ingestion`.

## Background

Exasol has no binary column type and no TIME or DURATION column type. Exasol DECIMAL precision does not exceed 36, and Exasol INTERVAL DAY TO SECOND has no month component. The driver applies one column type rule in every ingest mode.

## Scenarios

### Scenario: A bound column type that the driver cannot ingest fails before any SQL statement

* *GIVEN* a table `KEEP` that holds one row, and no table named `NEW_T`
* *AND* a RecordBatch with an `Int32` column `ID` and a column `C` of one of the Arrow types `Time32(Second)`, `Time64(Microsecond)`, `Duration(Second)`, `List(Int32)`, `Null`, `Decimal128(38, 2)`, or `Decimal128(10, -2)`
* *WHEN* the RecordBatch is ingested into `KEEP` with `IngestMode` `Append` or `Replace`, and into `NEW_T` with `IngestMode` `Create` or `CreateAppend`
* *THEN* each ingest SHALL fail with `AdbcStatus::InvalidArguments`, and the error message SHALL name the column `C` and its Arrow type
* *AND* the driver SHALL send no SQL statement to Exasol for the ingest
* *AND* `KEEP` SHALL still hold its one row, and no table named `NEW_T` SHALL exist

### Scenario: Date64, Decimal256, string dictionary, and Utf8View columns are ingested

* *GIVEN* a RecordBatch with a row of values and a row of nulls in these columns: `D` `Date64` holding `1_577_836_800_000` ms (2020-01-01), `N` `Decimal256(36, 2)` holding `-1234567890123456789012345678901234.56`, `S` `Dictionary(Int32, Utf8)` holding `alpha`, `L` `Dictionary(UInt32, LargeUtf8)` holding `beta`, and `V` `Utf8View` holding `gamma`
* *WHEN* the RecordBatch is ingested with `IngestMode` `Create` into a table that does not exist
* *THEN* the ingest SHALL return 2 rows
* *AND* the created table SHALL have the columns `D DATE`, `N DECIMAL(36,2)`, and `S`, `L`, and `V` of type `VARCHAR(2000000)`
* *AND* selecting the first row back SHALL return `2020-01-01`, `-1234567890123456789012345678901234.56`, `alpha`, `beta`, and `gamma`, and every value of the second row SHALL be NULL

### Scenario: Binary column types are stored as lowercase hexadecimal text

* *GIVEN* a RecordBatch with a row of values and a row of nulls in the columns `B` `Binary`, `LB` `LargeBinary`, `BV` `BinaryView`, and `FB` `FixedSizeBinary(3)`, each holding the bytes `0x00 0xAB 0xFF`
* *WHEN* the RecordBatch is ingested with `IngestMode` `Create` into a table that does not exist
* *THEN* the created table SHALL have the columns `B`, `LB`, `BV`, and `FB` of type `VARCHAR(2000000)`
* *AND* selecting the first row back SHALL return the text `00abff` in every column, and every value of the second row SHALL be NULL

### Scenario: Interval columns are ingested

* *GIVEN* a RecordBatch with a row of values and a row of nulls in these columns: `YM` `Interval(YearMonth)` holding -63 months, `DT` `Interval(DayTime)` holding 2 days and 46_210_123 ms, and `MDN` `Interval(MonthDayNano)` holding 0 months, -2 days, and -46_210_123_000_000 ns
* *WHEN* the RecordBatch is ingested with `IngestMode` `Create` into a table that does not exist
* *THEN* the ingest SHALL return 2 rows
* *AND* the created table SHALL have the columns `YM INTERVAL YEAR(9) TO MONTH`, `DT INTERVAL DAY(9) TO SECOND(3)`, and `MDN INTERVAL DAY(9) TO SECOND(3)`
* *AND* in the first row, `YM` SHALL equal `CAST('-5-3' AS INTERVAL YEAR(9) TO MONTH)`, `DT` SHALL equal `CAST('2 12:50:10.123' AS INTERVAL DAY(9) TO SECOND(3))`, and `MDN` SHALL equal `CAST('-2 12:50:10.123' AS INTERVAL DAY(9) TO SECOND(3))`
* *AND* every value of the second row SHALL be NULL

### Scenario: An Interval(MonthDayNano) value with a month component fails the ingest

* *GIVEN* a table `T` with a column `I INTERVAL DAY(9) TO SECOND(3)` that holds one row
* *AND* a RecordBatch with an `Interval(MonthDayNano)` column `I` holding 1 month, 0 days, and 0 ns
* *WHEN* the RecordBatch is ingested into `T` with `IngestMode` `Append`
* *THEN* the ingest SHALL fail with `AdbcStatus::Internal`, the status of every import error, and the error message SHALL name the month count and state that Exasol INTERVAL DAY TO SECOND has no month component
* *AND* `T` SHALL still hold only its one row
