# Feature: Exasol to Arrow

Defines the mapping from Exasol data types to Apache Arrow data types, including complex types and metadata preservation.

## Background

The system defines a complete mapping from Exasol data types to Apache Arrow data types. Exasol DECIMAL precision does not exceed 36, so Decimal128 is always sufficient. All Arrow fields are marked nullable when the source Exasol column allows NULL values. Type metadata from Exasol (precision, scale, original type names) is preserved in Arrow field metadata for round-trip fidelity. Unsupported or complex types produce clear error messages with guidance on workarounds.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Pre-1970 DATE and TIMESTAMP query results keep their calendar day

* *GIVEN* a connection over the native transport or over the WebSocket transport
* *WHEN* the application queries `SELECT DATE '1968-01-01', DATE '1900-03-01', DATE '1600-03-01', DATE '0001-01-01', DATE '9999-12-31', TIMESTAMP '1950-06-15 00:00:00', CAST(TIMESTAMP '1969-12-31 23:59:59.999999' AS TIMESTAMP(6)), TIMESTAMP '0001-01-01 00:00:00' FROM DUAL`
* *THEN* the five DATE columns SHALL hold the Date32 values -731, -25508, -135080, -719162, and 2932896, in that order
* *AND* the three TIMESTAMP columns SHALL hold the microsecond values -616896000000000, -1, and -62135596800000000, in that order
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: DATE and TIMESTAMP values count days in the proleptic Gregorian calendar

* *GIVEN* a DATE or TIMESTAMP value that Exasol reports as a year, month, and day from 0001-01-01 to 9999-12-31
* *WHEN* the driver converts the value to Arrow
* *THEN* the Date32 value SHALL equal the number of days from 1970-01-01 to that year, month, and day in the proleptic Gregorian calendar, which applies the Gregorian leap-year rule to every year, including the years before 1582
* *AND* the Timestamp value SHALL equal that day count multiplied by 86,400,000,000, plus the time of day in microseconds
* *AND* a date before 1582-10-15, which Exasol labels in the Julian calendar, SHALL convert by its year, month, and day, so that a reader that decodes Date32 in the proleptic Gregorian calendar, such as chrono or pyarrow, shows the same year, month, and day as Exasol, except for the dates of the next step
* *AND* February 29 of a year that is a leap year in the Julian calendar but not in the Gregorian calendar, such as 1500, SHALL convert to the same Date32 value as March 1 of that year
<!-- /DELTA:NEW -->
