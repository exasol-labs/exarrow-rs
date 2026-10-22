# Feature: Exasol to Arrow DATE and TIMESTAMP

Defines how Exasol DATE and TIMESTAMP values map to Arrow Date32 and Timestamp values, including the calendar rule.

## Background

Exasol DATE and TIMESTAMP values convert to Arrow Date32 and Timestamp values by the year, month, and day that Exasol reports, on the native and the WebSocket transport. The remaining type mappings are specified in `type-mapping/exasol-to-arrow`.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Date and time types mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol temporal types
* *THEN* DATE SHALL map to Arrow Date32
* *AND* TIMESTAMP with fractional precision 0-9 SHALL map to Arrow Timestamp with appropriate TimeUnit
* *AND* parameterized TIMESTAMP variants like `TIMESTAMP(3)` SHALL be parsed correctly, ignoring the precision parameter for Arrow mapping
* *AND* INTERVAL types SHALL map to Arrow Interval(MonthDayNano) in `TypeMapper::exasol_to_arrow`, in the `arrow_conversion` converters, and in the result sets of the native and the WebSocket transport (`type-mapping/exasol-to-arrow`), and SHALL map to Utf8 holding Exasol's text form of the value in the CSV-based Parquet export (`import-export/parquet-io`)
<!-- /DELTA:CHANGED -->
