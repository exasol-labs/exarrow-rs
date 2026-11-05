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
* *AND* INTERVAL types SHALL map to Arrow Interval(MonthDayNano) in `TypeMapper::exasol_to_arrow`, in the `arrow_conversion` converters, and in the result sets of the native and the WebSocket transport (`type-mapping/exasol-to-arrow`), and SHALL map to Utf8 holding Exasol's text form of the value in the CSV-based Parquet export (`import-export/parquet-io`)

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

### Scenario: TIMESTAMP WITH LOCAL TIME ZONE results have the same type on both transports

* *GIVEN* a native TCP session and a WebSocket session whose session time zones are both `UTC`, or both `EUROPE/BERLIN`
* *AND* a query returns the value `CAST(TIMESTAMP '2024-01-02 03:04:05.123456' AS TIMESTAMP(6) WITH LOCAL TIME ZONE)` in a first row and a NULL value of the same type in a second row
* *WHEN* the query runs in both sessions
* *THEN* the field of the column SHALL have the Arrow data type `Timestamp(Microsecond, "UTC")` in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`, on both transports
* *AND* the field SHALL carry the key `exasol:type_name` with the value `TIMESTAMP WITH LOCAL TIME ZONE` on both transports
* *AND* the value of the second row SHALL be NULL on both transports, and in the `UTC` sessions the value of the first row SHALL be `1_704_164_645_123_456` µs on both transports, which is `2024-01-02 03:04:05.123456` UTC

### Scenario: TIMESTAMP WITH LOCAL TIME ZONE values differ between the transports in a session outside UTC

* *GIVEN* a native TCP session and a WebSocket session whose session time zones are both `EUROPE/BERLIN`
* *AND* a query returns the value `CAST(TIMESTAMP '2024-01-02 03:04:05.123456' AS TIMESTAMP(6) WITH LOCAL TIME ZONE)`
* *WHEN* the query runs in both sessions
* *THEN* the native transport SHALL return `1_704_161_045_123_456` µs, which is the UTC instant of `2024-01-02 03:04:05.123456` in `EUROPE/BERLIN`
* *AND* the WebSocket transport SHALL return `1_704_164_645_123_456` µs, which is the date and time `2024-01-02 03:04:05.123456` that the session time zone shows, under the Arrow time zone `UTC`
* *AND* both values SHALL have the Arrow data type `Timestamp(Microsecond, "UTC")`
