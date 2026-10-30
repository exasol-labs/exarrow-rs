# Feature: Native Result Set Type Conversion

Specifies the per-type binary-to-Arrow conversion rules the native TCP protocol applies to each column of a result set, once `native-client/result-sets` has parsed the column-major frame and its metadata.

## Background

Each column's data is contiguous in the frame, so conversion reads it directly into the column's Arrow array, with no intermediate array.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Direct binary to Arrow conversion for temporal types

* *GIVEN* a result set contains columns of type `T_date` (14), `T_timestamp` (21), `T_timestamp_local_tz` (124), or `T_timestamp_utc` (125)
* *WHEN* parsing column data
* *THEN* the system SHALL convert dates to Arrow Date32 arrays
* *AND* the system SHALL convert `T_timestamp` columns to Arrow `Timestamp(Microsecond, None)` arrays
* *AND* the system SHALL convert `T_timestamp_local_tz` and `T_timestamp_utc` columns to Arrow `Timestamp(Microsecond, "UTC")` arrays, which is the Arrow type that `TypeMapper` maps for `TIMESTAMP WITH LOCAL TIME ZONE`
* *AND* the system SHALL convert the date and time of each timestamp value to microseconds since 1970-01-01 00:00:00 without a time zone shift, so that a `T_timestamp_utc` value, which Exasol sends as the UTC date and time of the stored instant, holds that UTC instant
<!-- /DELTA:CHANGED -->
