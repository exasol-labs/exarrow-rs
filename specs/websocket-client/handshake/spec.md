# Feature: WebSocket Handshake

Specifies WebSocket connection establishment, the Exasol-specific protocol handshake, and login authentication, before any ordinary command is exchanged.

## Background

The system implements the Exasol WebSocket API protocol as defined in https://github.com/exasol/websocket-api. Connections use secure WebSocket (wss://) when TLS is enabled. Once the handshake and login complete, ordinary command traffic is specified in `websocket-client/protocol`.

## Scenarios

### Scenario: WebSocket connection establishment

* *GIVEN* a WebSocket endpoint is reachable
* *WHEN* connecting to an Exasol database
* *THEN* it SHALL establish a WebSocket connection to the specified host and port
* *AND* it SHALL use secure WebSocket (wss://) when TLS is enabled
* *AND* it SHALL handle connection timeouts gracefully
* *AND* it SHALL configure the WebSocket with no frame size limit and no message size limit

### Scenario: Protocol handshake

* *GIVEN* a WebSocket endpoint is reachable
* *WHEN* WebSocket connection is established
* *THEN* it SHALL perform the Exasol-specific protocol handshake
* *AND* it SHALL negotiate protocol version compatibility

### Scenario: Login command

* *GIVEN* a WebSocket endpoint is reachable
* *WHEN* authenticating with the database
* *THEN* it SHALL send a login command with credentials
* *AND* it SHALL handle authentication success and failure responses
