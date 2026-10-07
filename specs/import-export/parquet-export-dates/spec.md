# Feature: Parquet Export DATE and TIMESTAMP Conversion

Specifies how the transport Parquet export converts DATE and TIMESTAMP values.

## Background

The transport Parquet export parses the DATE and TIMESTAMP text that Exasol writes into Date32 and Timestamp(Microsecond, None) values by the year, month, and day, counted in the proleptic Gregorian calendar. The general export behavior is specified in `import-export/parquet-export`.

## Scenarios

### Scenario: Export keeps pre-1970 DATE and TIMESTAMP values

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)`, `D DATE`, and `TS TIMESTAMP(6)`, holding the rows `(1, DATE '1968-01-01', TIMESTAMP '1950-06-15 00:00:00')` and `(2, DATE '0001-01-01', TIMESTAMP '1969-12-31 23:59:59.999999')`
* *WHEN* the application calls `Connection::export_to_parquet` for table `T`
* *THEN* the `D` field of the Parquet file SHALL hold the Date32 value -731 for `ID` 1 and -719162 for `ID` 2
* *AND* the `TS` field of the Parquet file SHALL hold the microsecond value -616896000000000 for `ID` 1 and -1 for `ID` 2
