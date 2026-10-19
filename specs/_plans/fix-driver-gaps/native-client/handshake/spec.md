# Feature: Native TCP Handshake

The system establishes a native TCP session with Exasol before any command is exchanged: opening the transport connection, negotiating protocol version, and completing the login handshake. Malformed server input during this phase is rejected with a typed error rather than a panic.

## Background

The server selects the native protocol when the first bytes received are `LOGIN_MAGIC` (0x01121201).

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Protocol version negotiation

* *GIVEN* a TCP connection is established to Exasol
* *WHEN* initiating the native protocol handshake
* *THEN* the system SHALL send a login packet starting with `LOGIN_MAGIC` (0x01121201)
* *AND* the login packet SHALL include the client's maximum supported protocol version
* *AND* the system SHALL accept any server-negotiated version between 14 and the client's maximum
* *AND* the system SHALL accept a server-negotiated version above the client's maximum and continue the login
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Server protocol version below 14 is rejected before the password is sent

* *GIVEN* a TCP connection is established and TLS negotiation is complete
* *WHEN* the server's login response carries a protocol version below 14
* *THEN* the login MUST fail with a `TransportError` whose message names the server's protocol version and the minimum supported version 14
* *AND* the system MUST NOT send the password message to that server
<!-- /DELTA:NEW -->
