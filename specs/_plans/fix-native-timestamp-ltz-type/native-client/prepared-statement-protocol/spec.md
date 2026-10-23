# Feature: Native Prepared Statement Protocol

The system implements the `CMD_CREATE_PREPARED`, `CMD_EXECUTE_PREPARED`, and `CMD_CLOSE_PREPARED` lifecycle over native TCP: how a prepare reply's `R_HANDLE` part is classified into parameter and result-set column descriptions, how parameter column headers are encoded, and how DATE and TIMESTAMP parameter text is encoded as binary field values.

## Background

A `CMD_CREATE_PREPARED` reply carries an `R_HANDLE` (2) part followed by zero or more result-set sub-results: one with handle `PARAMETER_DESCRIPTION` (-5) describing the parameters and, for a statement that returns a result set, one describing the result-set columns.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Outbound precision on a TIMESTAMP parameter column header

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared statement with a parameter that the prepare reply reports with wire type `T_TIMESTAMP` (21), `T_TIMESTAMP_LOCAL_TZ` (124), or `T_TIMESTAMP_UTC` (125)
* *WHEN* building the `CMD_EXECUTE_PREPARED` (11) payload, or each payload of a parameter set that runs as consecutive executions
* *THEN* the system SHALL encode that parameter's column as `T_TIMESTAMP` (21), so that Exasol reads the value in the session time zone, and Exasol SHALL accept the parameter data and SHALL keep the connection open
* *AND* the system SHALL write a 4-byte little-endian fractional-seconds precision of 9 directly after that column's type ID, the same field that a TIMESTAMP column description carries in protocol version 19 and later
* *AND* the system SHALL encode each non-null value of that column as `[year:2 LE][month:1][day:1][hour:1][minute:1][second:1][nanoseconds:4 LE]`
<!-- /DELTA:CHANGED -->
