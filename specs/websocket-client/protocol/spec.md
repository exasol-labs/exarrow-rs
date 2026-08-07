# Feature: Protocol

Specifies command execution, message serialization, and error handling for the Exasol WebSocket API, once a session has been established (see `websocket-client/handshake`).

## Background

The system implements the Exasol WebSocket API protocol as defined in https://github.com/exasol/websocket-api. All commands are serialized to JSON format with required fields (command type, attributes) and responses are deserialized to structured objects with validation. Error responses from Exasol are parsed into appropriate Rust error types with error codes and messages. `createPreparedStatement` request/response handling is specified separately in `websocket-client/prepared-statements`.

## Scenarios

### Scenario: Large result set transfer via WebSocket

* *GIVEN* an authenticated WebSocket session exists
* *AND* a table contains enough data to produce a response exceeding 16 MiB
* *WHEN* executing a SELECT query that returns the full result set
* *THEN* the system SHALL receive the complete response without frame size errors
* *AND* the system SHALL return all rows as Arrow RecordBatches

### Scenario: Execute SQL command

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* executing a SQL query
* *THEN* it SHALL send an execute command with the SQL text
* *AND* it SHALL include execution parameters (e.g., result set handle)

### Scenario: Fetch results command

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* retrieving query results
* *THEN* it SHALL send fetch commands for result data
* *AND* it SHALL handle pagination for large result sets

### Scenario: Disconnect command

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* closing a connection
* *THEN* it SHALL send a disconnect command before closing the WebSocket
* *AND* it SHALL wait for acknowledgment or timeout

### Scenario: JSON request serialization

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* sending a command to Exasol
* *THEN* it SHALL serialize the command to JSON format
* *AND* it SHALL include required fields (command type, attributes)

### Scenario: JSON response deserialization

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* receiving a response from Exasol
* *THEN* it SHALL deserialize JSON to structured response objects
* *AND* it SHALL validate response structure and required fields

### Scenario: Error response handling

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* Exasol returns an error response
* *THEN* it SHALL parse error codes and messages
* *AND* it SHALL convert them to appropriate Rust error types

### Scenario: Set session attributes command

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* setting session attributes (e.g., autocommit mode)
* *THEN* the system SHALL send a `setAttributes` command with the attributes to modify
* *AND* the system SHALL validate the server response status
* *AND* the system SHALL return an error if the server rejects the attribute change
