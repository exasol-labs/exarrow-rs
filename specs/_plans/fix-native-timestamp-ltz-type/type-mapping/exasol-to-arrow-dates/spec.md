# Feature: Exasol to Arrow DATE and TIMESTAMP

Defines how Exasol DATE and TIMESTAMP values map to Arrow Date32 and Timestamp values, including the calendar rule.

## Background

Exasol DATE and TIMESTAMP values convert to Arrow Date32 and Timestamp values by the year, month, and day that Exasol reports, on the native and the WebSocket transport. The remaining type mappings are specified in `type-mapping/exasol-to-arrow`.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: TIMESTAMP WITH LOCAL TIME ZONE results have the same type on both transports

* *GIVEN* a native TCP session and a WebSocket session whose session time zones are both `UTC`, or both `EUROPE/BERLIN`
* *AND* a query returns the value `CAST(TIMESTAMP '2024-01-02 03:04:05.123456' AS TIMESTAMP(6) WITH LOCAL TIME ZONE)` in a first row and a NULL value of the same type in a second row
* *WHEN* the query runs in both sessions
* *THEN* the field of the column SHALL have the Arrow data type `Timestamp(Microsecond, "UTC")` in the result set metadata schema, in the batch schema of every RecordBatch, and in the zero-row RecordBatch of the same query with `WHERE FALSE`, on both transports
* *AND* the field SHALL carry the key `exasol:type_name` with the value `TIMESTAMP WITH LOCAL TIME ZONE` on both transports
* *AND* the value of the second row SHALL be NULL on both transports, and in the `UTC` sessions the value of the first row SHALL be `1_704_164_645_123_456` µs on both transports, which is `2024-01-02 03:04:05.123456` UTC
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: TIMESTAMP WITH LOCAL TIME ZONE values differ between the transports in a session outside UTC

* *GIVEN* a native TCP session and a WebSocket session whose session time zones are both `EUROPE/BERLIN`
* *AND* a query returns the value `CAST(TIMESTAMP '2024-01-02 03:04:05.123456' AS TIMESTAMP(6) WITH LOCAL TIME ZONE)`
* *WHEN* the query runs in both sessions
* *THEN* the native transport SHALL return `1_704_161_045_123_456` µs, which is the UTC instant of `2024-01-02 03:04:05.123456` in `EUROPE/BERLIN`
* *AND* the WebSocket transport SHALL return `1_704_164_645_123_456` µs, which is the date and time `2024-01-02 03:04:05.123456` that the session time zone shows, under the Arrow time zone `UTC`
* *AND* both values SHALL have the Arrow data type `Timestamp(Microsecond, "UTC")`
<!-- /DELTA:NEW -->
