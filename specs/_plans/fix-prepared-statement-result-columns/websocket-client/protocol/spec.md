# Feature: Protocol

Specifies the WebSocket protocol implementation for the Exasol WebSocket API, including connection establishment, protocol handshake, command execution, and message serialization.

## Background

The system implements the Exasol WebSocket API protocol as defined in https://github.com/exasol/websocket-api. Connections use secure WebSocket (wss://) when TLS is enabled. All commands are serialized to JSON format with required fields (command type, attributes) and responses are deserialized to structured objects with validation. Error responses from Exasol are parsed into appropriate Rust error types with error codes and messages.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Create prepared statement response carries result-set column metadata

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* a `createPreparedStatement` response is deserialized
* *THEN* the system SHALL read `responseData.results` in addition to `responseData.parameterData`
* *AND* the system SHALL take column metadata from the `resultSet.columns` of the first entry whose `resultType` equals `"resultSet"`, preserving column order
* *AND* the system SHALL ignore any later `"resultSet"` entry, because a `createPreparedStatement` reply describes exactly one result set
* *AND* each column SHALL retain the `name` and `dataType` Exasol reported, including derived and aliased names such as `UPPER(T.NAME)`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Create prepared statement response without a result set

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* a `createPreparedStatement` response carries no `"resultSet"` entry with columns, whether because `responseData.results` is absent, because an entry's `resultType` equals `"rowCount"`, or because a `"resultSet"` entry carries no `columns`
* *THEN* the system SHALL report zero result-set columns
* *AND* the system MUST NOT read `resultSet` from a `"rowCount"` entry, because such an entry carries no `resultSet` key
* *AND* the system SHALL NOT return an error
<!-- /DELTA:NEW -->
