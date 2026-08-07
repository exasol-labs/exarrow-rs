# Feature: Native TCP Protocol

The system implements Exasol's native binary TCP protocol as a high-performance alternative to the WebSocket JSON protocol. Once a session is established (see `native-client/handshake`), commands are serialized as binary attribute sets and responses are parsed from binary frames into structured Rust types.

## Background

Native protocol commands share a common binary message envelope: a 21-byte header followed by an attribute set. The server responds with one of a fixed set of result types (`R_ResultSet`, `R_RowCount`, `R_Empty`, `R_Exception`, `R_StillExecuting`), each parsed into its own Rust representation. Prepared-statement command handling is specified separately in `native-client/prepared-statement-protocol`.

## Scenarios

### Scenario: Execute SQL command

* *GIVEN* an authenticated native TCP session exists
* *WHEN* executing a SQL query
* *THEN* the system SHALL send a `CMD_EXECUTE` (12) command with the SQL text as an attribute
* *AND* the system SHALL parse the response as one of: `R_ResultSet`, `R_RowCount`, `R_Empty`, or `R_Exception`

### Scenario: Fetch results for large result sets

* *GIVEN* an authenticated native TCP session exists
* *AND* a query has returned a result set handle (not `SMALL_RESULTSET`)
* *WHEN* fetching additional rows
* *THEN* the system SHALL send a `CMD_FETCH2` (36) command with the result set handle
* *AND* the system SHALL parse the returned rows in column-major binary format
* *AND* the system SHALL continue fetching until all rows are received

### Scenario: Disconnect

* *GIVEN* an authenticated native TCP session exists
* *WHEN* closing the connection
* *THEN* the system SHALL send `CMD_DISCONNECT` (32)
* *AND* the system SHALL close the TCP connection after sending the disconnect command

### Scenario: Error response handling

* *GIVEN* an authenticated native TCP session exists
* *WHEN* the server responds with `R_Exception` (-1)
* *THEN* the system SHALL parse the error message length, error message text, and 5-byte SQL state
* *AND* the system SHALL convert the error into an appropriate Rust error type

### Scenario: Still-executing handling

* *GIVEN* an authenticated native TCP session exists
* *WHEN* the server responds with `R_StillExecuting` (5)
* *THEN* the system SHALL send `CMD_CONTINUE` (38) to poll for completion
* *AND* the system SHALL repeat until a final result type is received

### Scenario: Set session attributes

* *GIVEN* an authenticated native TCP session exists
* *WHEN* setting session attributes (e.g., autocommit, current schema)
* *THEN* the system SHALL send `CMD_SET_ATTRIBUTES` (35) with the attribute key-value pairs encoded in binary format
* *AND* the system SHALL validate the server response
