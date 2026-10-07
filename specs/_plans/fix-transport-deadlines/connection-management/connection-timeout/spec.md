# Feature: Connection Timeout

The connection timeout limits how long opening a connection to Exasol may take. A server, proxy, or load balancer that accepts the TCP connection and then stops answering fails the connection within that limit instead of blocking the caller.

## Background

The caller sets the connection timeout in seconds with the `timeout` or `connection_timeout` connection-string parameter, or with `ConnectionBuilder::connection_timeout()`. The driver hands it to the transport in milliseconds. Opening a connection runs these steps in order: the TCP connect, the TLS handshake when TLS is enabled, the WebSocket upgrade on the WebSocket transport, and the login. With TLS disabled, the native transport starts the login directly after the TCP connect.

## Scenarios

### Scenario: One deadline bounds every connection setup step

* *GIVEN* a connection timeout of N milliseconds
* *WHEN* the driver opens a connection on the native or the WebSocket transport
* *THEN* the driver SHALL start one deadline of N milliseconds when it starts the TCP connect
* *AND* that deadline SHALL bound the TCP connect, the TLS handshake, the WebSocket upgrade, and the login together, so time spent in one step SHALL NOT be granted again to a later step
* *AND* when the deadline elapses during a step, the driver SHALL fail the connection with an error message that contains `Connection timeout after <N>ms (<step>)`, where `<step>` is `TCP connect`, `TLS handshake`, `WebSocket handshake`, or `login`
* *AND* a step that fails for another reason before the deadline SHALL report its own error

### Scenario: Connection timeout default and maximum

* *GIVEN* connection parameters from a connection string or from `ConnectionBuilder`
* *WHEN* the parameters are built
* *THEN* a connection timeout that the caller does not set SHALL be 30 seconds
* *AND* a connection timeout above 300 seconds SHALL be rejected with `ConnectionError::InvalidParameter`

### Scenario: Server that never answers the TLS handshake

* *GIVEN* TLS is enabled
* *AND* a server accepts the TCP connection and never sends a byte
* *WHEN* the driver opens a connection to that server with a connection timeout of N milliseconds on the native or the WebSocket transport
* *THEN* the driver SHALL fail the connection when the deadline elapses, and not before, with an error message that contains `Connection timeout after <N>ms (TLS handshake)`
* *AND* the driver SHALL close its TCP connection to that server

### Scenario: Server that never answers the login

* *GIVEN* a server accepts the TCP connection, completes the TLS handshake when TLS is enabled, completes the WebSocket upgrade on the WebSocket transport, and then never answers
* *WHEN* the driver opens a connection to that server with a connection timeout of N milliseconds
* *THEN* the driver SHALL fail the connection when the deadline elapses, and not before, with an error message that contains `Connection timeout after <N>ms (login)`
* *AND* the transport SHALL close its socket and SHALL report itself as not connected

### Scenario: WebSocket server that never answers the upgrade

* *GIVEN* the WebSocket transport with TLS disabled
* *AND* a server accepts the TCP connection and never sends a byte
* *WHEN* the driver opens a connection to that server with a connection timeout of N milliseconds
* *THEN* the driver SHALL fail the connection when the deadline elapses, and not before, with an error message that contains `Connection timeout after <N>ms (WebSocket handshake)`
* *AND* the driver SHALL close its TCP connection to that server
