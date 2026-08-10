# Feature: Native Prepared Statement Protocol

The system implements the `CMD_CREATE_PREPARED` / `CMD_EXECUTE_PREPARED` / `CMD_CLOSE_PREPARED` command lifecycle over the native TCP protocol, including how a prepare reply's `R_HANDLE` part is classified into parameter and result-set column descriptions, and how parameter column headers are encoded on the wire.

## Background

A `CMD_CREATE_PREPARED` reply carries an `R_HANDLE` (2) part followed by zero or more result-set sub-results: one with handle `PARAMETER_DESCRIPTION` (-5) describing the statement's parameters, and — for a result-set-producing statement — one describing the result-set columns. Both descriptions are parsed by the same column-metadata routine `native-client/result-sets` uses for ordinary result sets.

## Scenarios

### Scenario: Prepared statement lifecycle

* *GIVEN* an authenticated native TCP session exists
* *WHEN* creating a prepared statement
* *THEN* the system SHALL send `CMD_CREATE_PREPARED` (10) with the SQL text
* *AND* the system SHALL parse the returned statement handle, parameter metadata, and result-set column metadata
* *WHEN* executing the prepared statement with parameters
* *THEN* the system SHALL send `CMD_EXECUTE_PREPARED` (11) with the handle and binary parameter data
* *WHEN* closing the prepared statement
* *THEN* the system SHALL send `CMD_CLOSE_PREPARED` (18) with the handle

### Scenario: Sub-result classification in a prepared statement reply

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared-statement reply whose `R_HANDLE` (2) part is followed by one or more result-set sub-results
* *WHEN* parsing that part
* *THEN* the system SHALL classify the sub-result whose handle equals `PARAMETER_DESCRIPTION` (-5) as the parameter description
* *AND* the system SHALL classify every other result-set sub-result as the result-set column description, retaining the last one when the part carries more than one
* *AND* the system SHALL retain both descriptions, and SHALL NOT discard either one in favour of the other

### Scenario: Prepared statement reply for a row-count-producing statement

* *GIVEN* an authenticated native TCP session exists
* *WHEN* preparing a statement that Exasol classifies as row-count-producing rather than result-set-producing, such as `INSERT INTO T VALUES (?,?)` or `DELETE FROM T WHERE ID = ?`
* *THEN* the system SHALL parse the statement handle and the parameter description as for any other prepared statement
* *AND* the system SHALL report zero result-set columns
* *AND* the system SHALL NOT report an error for the absent result-set description, because Exasol computes no result-set description for a row-count-producing statement and therefore sends none

### Scenario: Outbound vcFlag on a CHAR parameter column header

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared statement whose parameter data contains a column encoded with wire type `T_CHAR` (10)
* *WHEN* building the `CMD_EXECUTE_PREPARED` (11) payload
* *THEN* the system SHALL write a vcFlag byte of `0x11` before that column's `maxLen` and `octetLen`
* *AND* `0x11` SHALL be the varchar bit `0x01` combined with the UTF-8 bit `0x10`, matching the encoding Exasol itself sends for a `VARCHAR` column with character set UTF8

### Scenario: Prepared-statement sub-results carry both descriptions

* *GIVEN* a prepared-statement reply whose `R_HANDLE` (2) part is followed by a `PARAMETER_DESCRIPTION` (-5) sub-result and a second result-set sub-result
* *WHEN* parsing that reply
* *THEN* the parser SHALL return a response shape carrying the statement handle, the parameter column metadata, and the result-set column metadata as three distinct values, parsed by the same column-metadata routine ordinary result sets use
* *AND* the parser SHALL NOT encode a sub-result handle in the `total_rows` field of a result-set response
* *AND* a reply whose `R_HANDLE` part carries no result-set sub-result SHALL yield empty parameter column metadata and empty result-set column metadata
* *AND* a caller that receives this response shape where a query result was expected SHALL be rejected with a protocol error
* *AND* a sub-result reporting a server exception SHALL be rejected with a protocol error naming the server message and SQL state, and SHALL NOT be reported as a successful preparation with empty metadata
* *AND* a warning carried by a sub-result SHALL be propagated to the caller rather than discarded
