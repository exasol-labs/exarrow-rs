<!-- DELTA:CHANGED -->
# Feature: Native Prepared Statement Protocol

The system implements the `CMD_CREATE_PREPARED` / `CMD_EXECUTE_PREPARED` / `CMD_CLOSE_PREPARED` command lifecycle over the native TCP protocol, including how a prepare reply's `R_HANDLE` part is classified into parameter and result-set column descriptions, how parameter column headers are encoded on the wire, and how DATE and TIMESTAMP parameter text is encoded as binary field values.
<!-- /DELTA:CHANGED -->

## Background

A `CMD_CREATE_PREPARED` reply carries an `R_HANDLE` (2) part followed by zero or more result-set sub-results: one with handle `PARAMETER_DESCRIPTION` (-5) describing the statement's parameters, and — for a result-set-producing statement — one describing the result-set columns. Both descriptions are parsed by the same column-metadata routine `native-client/result-sets` uses for ordinary result sets.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Outbound precision on a TIMESTAMP parameter column header

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared statement whose parameter data contains a column encoded with wire type `T_TIMESTAMP` (21), `T_TIMESTAMP_LOCAL_TZ` (124), or `T_TIMESTAMP_UTC` (125)
* *WHEN* building the `CMD_EXECUTE_PREPARED` (11) payload, or each payload of a parameter set that runs as consecutive executions
* *THEN* the system SHALL write a 4-byte little-endian fractional-seconds precision of 9 directly after that column's type ID, the same field that a TIMESTAMP column description carries in protocol version 19 and later
* *AND* the system SHALL encode each non-null value of that column as `[year:2 LE][month:1][day:1][hour:1][minute:1][second:1][nanoseconds:4 LE]`
* *AND* for a column with wire type `T_TIMESTAMP` (21), which is the wire type that Exasol reports for `TIMESTAMP` and `TIMESTAMP WITH LOCAL TIME ZONE` parameters, Exasol SHALL accept the parameter data and SHALL keep the connection open
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: DATE and TIMESTAMP parameter text is encoded field by field

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared statement whose parameter data contains a column encoded with wire type `T_DATE` (14), `T_TIMESTAMP` (21), `T_TIMESTAMP_LOCAL_TZ` (124), or `T_TIMESTAMP_UTC` (125) holding empty text, or text of the form `Y-M-D` optionally followed by one or more spaces and `h`, `h:m`, `h:m:s`, or `h:m:s.f`, with optional leading and trailing spaces, where `Y` has 1 to 4 digits, `M`, `D`, `h`, `m`, and `s` have 1 or 2 digits, and `f` has 0 to 9 digits
* *WHEN* building the `CMD_EXECUTE_PREPARED` (11) payload
* *THEN* the system SHALL encode empty text as NULL, with the null marker 0 and no value bytes
* *AND* the system SHALL ignore leading and trailing spaces
* *AND* the system SHALL encode the year, month, day, hour, minute, second, and fraction as written, with no calendar conversion, so that `1500-02-29` is sent as year 1500, month 2, day 29
* *AND* the system SHALL encode a missing time part as midnight and a missing minute, second, or fraction part as zero
* *AND* for a `T_DATE` value with a time part whose hour is at most 23 and whose minute and second are at most 60, which are the bounds that Exasol's text cast checks for a DATE value, the system SHALL encode only the date part, so that `2024-01-02 08:30` is sent as year 2024, month 1, day 2
* *AND* the system SHALL send a value whose fields are out of range, such as month 13, February 30, or hour 25 in a timestamp column, as written, so that Exasol rejects it with an error
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: DATE and TIMESTAMP parameter values in another form are rejected before execution

* *GIVEN* an authenticated native TCP session exists
* *AND* a parameter set with a column encoded with wire type `T_DATE` (14), `T_TIMESTAMP` (21), `T_TIMESTAMP_LOCAL_TZ` (124), or `T_TIMESTAMP_UTC` (125) whose value the scenario "DATE and TIMESTAMP parameter text is encoded field by field" does not encode, such as `2024-01-01T08:00:00`, `2024/01/02`, `garbage`, text of only spaces, the trailing colon in `2024-01-01 08:00:`, a `T_DATE` value whose time part has an hour above 23 or a minute or second above 60, or a value that is not text
* *WHEN* building the `CMD_EXECUTE_PREPARED` (11) payload
* *THEN* the system SHALL return an error whose message contains the value and the accepted form
* *AND* the system MUST NOT substitute a default value, such as `2000-01-01`, for the rejected value
* *AND* the system MUST NOT send a `CMD_EXECUTE_PREPARED` (11) request for that parameter set
<!-- /DELTA:NEW -->
