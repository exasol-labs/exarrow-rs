# Feature: Protocol

Specifies command execution, message serialization, and error handling for the Exasol WebSocket API, once a session has been established (see `websocket-client/handshake`).

## Background

The system implements the Exasol WebSocket API protocol as defined in https://github.com/exasol/websocket-api. All commands are serialized to JSON format with required fields (command type, attributes) and responses are deserialized to structured objects with validation. Error responses from Exasol are parsed into appropriate Rust error types with error codes and messages. `createPreparedStatement` request/response handling is specified separately in `websocket-client/prepared-statements`.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Fetch results command

* *GIVEN* an authenticated WebSocket session exists
* *AND* an `execute` or `executePreparedStatement` response has returned a result set handle
* *WHEN* retrieving query results
* *THEN* it SHALL send fetch commands for result data
* *AND* it SHALL handle pagination for large result sets
* *AND* each fetch command for the handle SHALL set `startPosition` to the number of rows that the execute response and every earlier fetch response for that handle delivered
<!-- /DELTA:CHANGED -->
