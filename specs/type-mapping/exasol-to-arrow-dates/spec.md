# Feature: Exasol to Arrow DATE and TIMESTAMP

Defines how Exasol DATE and TIMESTAMP values map to Arrow Date32 and Timestamp values, including the calendar rule.

## Background

Exasol DATE and TIMESTAMP values convert to Arrow Date32 and Timestamp values by the year, month, and day that Exasol reports, on the native and the WebSocket transport. The remaining type mappings are specified in `type-mapping/exasol-to-arrow`.

## Scenarios

### Scenario: Date and time types mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol temporal types
* *THEN* DATE SHALL map to Arrow Date32
* *AND* TIMESTAMP with fractional precision 0-9 SHALL map to Arrow Timestamp with appropriate TimeUnit
* *AND* parameterized TIMESTAMP variants like `TIMESTAMP(3)` SHALL be parsed correctly, ignoring the precision parameter for Arrow mapping
* *AND* INTERVAL types SHALL map to Arrow Duration or Interval types in the conversions that produce Arrow interval values, `TypeMapper::exasol_to_arrow` and the `arrow_conversion` converters, and SHALL map to Utf8 holding Exasol's text form of the value in the native transport's result sets (`native-client/type-conversion`) and in the CSV-based Parquet export (`import-export/parquet-io`)

### Scenario: Pre-1970 DATE and TIMESTAMP query results keep their calendar day

* *GIVEN* a connection over the native transport or over the WebSocket transport
* *WHEN* the application queries `SELECT DATE '1968-01-01', DATE '1900-03-01', DATE '1600-03-01', DATE '0001-01-01', DATE '9999-12-31', TIMESTAMP '1950-06-15 00:00:00', CAST(TIMESTAMP '1969-12-31 23:59:59.999999' AS TIMESTAMP(6)), TIMESTAMP '0001-01-01 00:00:00' FROM DUAL`
* *THEN* the five DATE columns SHALL hold the Date32 values -731, -25508, -135080, -719162, and 2932896, in that order
* *AND* the three TIMESTAMP columns SHALL hold the microsecond values -616896000000000, -1, and -62135596800000000, in that order

### Scenario: DATE and TIMESTAMP values count days in the proleptic Gregorian calendar

* *GIVEN* a DATE or TIMESTAMP value that Exasol reports as a year, month, and day from 0001-01-01 to 9999-12-31
* *WHEN* the driver converts the value to Arrow
* *THEN* the Date32 value SHALL equal the number of days from 1970-01-01 to that year, month, and day in the proleptic Gregorian calendar, which applies the Gregorian leap-year rule to every year, including the years before 1582
* *AND* the Timestamp value SHALL equal that day count multiplied by 86,400,000,000, plus the time of day in microseconds
* *AND* a date before 1582-10-15, which Exasol labels in the Julian calendar, SHALL convert by its year, month, and day, so that a reader that decodes Date32 in the proleptic Gregorian calendar, such as chrono or pyarrow, shows the same year, month, and day as Exasol, except for the dates of the next step
* *AND* February 29 of a year that is a leap year in the Julian calendar but not in the Gregorian calendar, such as 1500, SHALL convert to the same Date32 value as March 1 of that year
