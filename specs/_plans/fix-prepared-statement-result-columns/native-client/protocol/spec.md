# Feature: Native TCP Protocol

The system implements Exasol's native binary TCP protocol as a high-performance alternative to the WebSocket JSON protocol. The native protocol uses binary message framing with 21-byte headers, little-endian byte ordering, and protocol version negotiation starting at v14. All commands are serialized as binary attribute sets, and responses are parsed from binary frames into structured Rust types.

## Background

The native TCP protocol connects to the same Exasol port (8563) as the WebSocket protocol. The server dispatches based on the first bytes received: `LOGIN_MAGIC` (0x01121201) for native TCP, `GET ` for WebSocket. Protocol version 14 is the minimum supported version, which requires ChaCha20 encryption and deprecates RC4.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Prepared statement lifecycle

* *GIVEN* an authenticated native TCP session exists
* *WHEN* creating a prepared statement
* *THEN* the system SHALL send `CMD_CREATE_PREPARED` (10) with the SQL text
* *AND* the system SHALL parse the returned statement handle, parameter metadata, and result-set column metadata
* *WHEN* executing the prepared statement with parameters
* *THEN* the system SHALL send `CMD_EXECUTE_PREPARED` (11) with the handle and binary parameter data
* *WHEN* closing the prepared statement
* *THEN* the system SHALL send `CMD_CLOSE_PREPARED` (18) with the handle
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Sub-result classification in a prepared statement reply

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared-statement reply whose `R_HANDLE` (2) part is followed by one or more result-set sub-results
* *WHEN* parsing that part
* *THEN* the system SHALL classify the sub-result whose handle equals `PARAMETER_DESCRIPTION` (-5) as the parameter description
* *AND* the system SHALL classify every other result-set sub-result as the result-set column description, retaining the last one when the part carries more than one
* *AND* the system SHALL retain both descriptions, and SHALL NOT discard either one in favour of the other
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Prepared statement reply for a row-count-producing statement

* *GIVEN* an authenticated native TCP session exists
* *WHEN* preparing a statement that Exasol classifies as row-count-producing rather than result-set-producing, such as `INSERT INTO T VALUES (?,?)` or `DELETE FROM T WHERE ID = ?`
* *THEN* the system SHALL parse the statement handle and the parameter description as for any other prepared statement
* *AND* the system SHALL report zero result-set columns
* *AND* the system SHALL NOT report an error for the absent result-set description, because Exasol computes no result-set description for a row-count-producing statement and therefore sends none
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Outbound vcFlag on a CHAR parameter column header

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared statement whose parameter data contains a column encoded with wire type `T_CHAR` (10)
* *WHEN* building the `CMD_EXECUTE_PREPARED` (11) payload
* *THEN* the system SHALL write a vcFlag byte of `0x11` before that column's `maxLen` and `octetLen`
* *AND* `0x11` SHALL be the varchar bit `0x01` combined with the UTF-8 bit `0x10`, matching the encoding Exasol itself sends for a `VARCHAR` column with character set UTF8
<!-- /DELTA:NEW -->
