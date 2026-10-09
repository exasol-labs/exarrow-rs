# Feature: Protocol

Specifies command execution, message serialization, and error handling for the Exasol WebSocket API, once a session has been established (see `websocket-client/handshake`).

## Background

The system implements the Exasol WebSocket API protocol as defined in https://github.com/exasol/websocket-api. All commands are serialized to JSON format with required fields (command type, attributes) and responses are deserialized to structured objects with validation. Error responses from Exasol are parsed into appropriate Rust error types with error codes and messages. `createPreparedStatement` request/response handling is specified separately in `websocket-client/prepared-statements`.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Set the current schema attribute

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* the driver sets the session's current schema to a name
* *THEN* the system SHALL send a `setAttributes` command whose `attributes` object holds `currentSchema` with the name
* *AND* the system SHALL return an error that contains the server's error message when the server rejects the change
* *AND* after the server accepts the change, the system SHALL send a `getAttributes` command and SHALL record the `currentSchema` member of that response's `attributes` object as the session's current schema, where an empty value means the session has no current schema
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Track the current schema attribute from responses

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* a server response holds a top-level `attributes` object with a `currentSchema` member
* *THEN* the system SHALL record the member's value as the session's current schema, where an empty value means the session has no current schema
* *AND* a response without a `currentSchema` member MUST leave the recorded current schema unchanged
<!-- /DELTA:NEW -->
