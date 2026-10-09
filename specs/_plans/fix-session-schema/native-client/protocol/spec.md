# Feature: Native TCP Protocol

The system implements Exasol's native binary TCP protocol as a high-performance alternative to the WebSocket JSON protocol. Once a session is established (see `native-client/handshake`), commands are serialized as binary attribute sets and responses are parsed from binary frames into structured Rust types.

## Background

Native protocol commands share a common binary message envelope: a 21-byte header followed by an attribute set. The server responds with one of a fixed set of result types (`R_ResultSet`, `R_RowCount`, `R_Empty`, `R_Exception`, `R_StillExecuting`), each parsed into its own Rust representation. Prepared-statement command handling is specified separately in `native-client/prepared-statement-protocol`.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Set the current schema attribute

* *GIVEN* an authenticated native TCP session exists
* *WHEN* the driver sets the session's current schema to a name
* *THEN* the system SHALL send `CMD_SET_ATTRIBUTES` (35) with attribute 22 (current schema) holding the name as a string
* *AND* when the response holds an exception, the system SHALL return an error that contains the server's error message and MUST leave the recorded current schema unchanged
* *AND* after the server accepts the change, the system SHALL send `CMD_GET_ATTRIBUTES` (34) and SHALL record attribute 22 of that response as the session's current schema, where an absent or empty attribute 22 means the session has no current schema
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Track the current schema attribute from responses

* *GIVEN* an authenticated native TCP session exists
* *WHEN* the attribute block of a server response holds attribute 22 (current schema)
* *THEN* the system SHALL record its value as the session's current schema, where an empty value means the session has no current schema
* *AND* a response to any command other than `CMD_GET_ATTRIBUTES` whose attribute block lacks attribute 22 MUST leave the recorded current schema unchanged
* *AND* a response to `CMD_GET_ATTRIBUTES`, including the one the login sends, that lacks attribute 22 SHALL set the recorded current schema to none, because that response reports every session attribute
<!-- /DELTA:NEW -->
