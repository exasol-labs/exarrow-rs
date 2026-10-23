# Feature: Native TCP Handshake

The system establishes a native TCP session with Exasol before any command is exchanged: opening the transport connection, negotiating protocol version, and completing the login handshake. Malformed server input during this phase is rejected with a typed error rather than a panic.

## Background

The server selects the native protocol when the first bytes received are `LOGIN_MAGIC` (0x01121201).

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Login makes Exasol send TIMESTAMP WITH LOCAL TIME ZONE columns with their own wire type

* *GIVEN* a TCP connection is established and TLS negotiation is complete
* *WHEN* performing the login handshake
* *THEN* the system SHALL set the session attribute `ATTR_TSUTC_ENABLED` (51) to the boolean value true during the login, before it sends the first statement
* *AND* Exasol SHALL then send each `TIMESTAMP WITH LOCAL TIME ZONE` result column of the session with the wire type `T_TIMESTAMP_UTC` (125) instead of `T_TIMESTAMP` (21), so that the column reaches the driver as `TIMESTAMP WITH LOCAL TIME ZONE` and not as `TIMESTAMP`
<!-- /DELTA:NEW -->
