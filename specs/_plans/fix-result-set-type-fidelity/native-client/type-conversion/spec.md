# Feature: Native Result Set Type Conversion

Specifies the per-type binary-to-Arrow conversion rules the native TCP protocol applies to each column of a result set, once `native-client/result-sets` has parsed the column-major frame and its metadata.

<!-- DELTA:CHANGED -->
## Background

Each column's data is contiguous in the frame, so conversion reads it directly into the column's Arrow array, with no intermediate array.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Direct binary to Arrow conversion for interval types

* *GIVEN* a result set contains columns of type `T_interval_year` (16) or `T_interval_day` (17), whose values Exasol sends as length-prefixed text, such as `+01-02` or `-03 04:05:06.789`
* *WHEN* parsing column data
* *THEN* the system SHALL parse each value from the wire bytes into an Arrow IntervalMonthDayNano array, with no intermediate Utf8 array
* *AND* the sign of the text SHALL apply to every component of the value
* *AND* text that is not a valid interval, or a month count outside the 32-bit signed integer range, SHALL fail the parse with an error that names the column
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Direct binary to Arrow conversion for binary and geometry types

* *GIVEN* a result set contains columns of type `T_binary` (15), `T_hashtype` (126), or `T_geometry` (123)
* *WHEN* parsing column data
* *THEN* `T_binary`, `T_hashtype`, and `T_geometry` SHALL map to Arrow Binary arrays
* *AND* each value SHALL hold the length-prefixed bytes that Exasol sends, unchanged, which for `T_geometry` are WKT text and for `T_hashtype` are the text of the session's `HASHTYPE_FORMAT`, hexadecimal by default
<!-- /DELTA:CHANGED -->
