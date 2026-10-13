# Feature: HTTP Transport

Specifies the HTTP transport layer for bulk data transfer using the EXA tunneling protocol, enabling firewall-friendly import and export operations through reverse-connection HTTP tunneling.

<!-- DELTA:CHANGED -->
## Background

The client establishes an outbound TCP connection to Exasol and performs a magic packet handshake to create a bidirectional HTTP tunnel. All data transfer occurs through this single established connection, enabling firewall-friendly operation requiring only outbound connections. With TLS, the IMPORT or EXPORT SQL names the tunnel certificate in a PUBLIC KEY clause.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Tunnel setup fails with the step named when the peer stops answering

* *GIVEN* an HTTP tunnel opened through `HttpTransportClient::connect_with_timeout` with a setup bound of N milliseconds
* *AND* the peer accepts the TCP connection and then stops answering, either before it sends the EXA handshake response or, with TLS enabled, after the EXA handshake and before it completes the TLS handshake
* *WHEN* the bound for the stalled step elapses
* *THEN* the client SHALL fail with an error message that contains `HTTP tunnel setup timeout after <N>ms (<step>)`, where `<step>` is `TCP connect`, `EXA handshake`, or `TLS handshake`
* *AND* a stall in the TCP connect or the EXA handshake SHALL fail `connect_with_timeout`, a stall in the TLS handshake SHALL fail the first tunnel operation that reads or writes data, and in both cases the client SHALL close the tunnel connection
* *AND* one deadline of N milliseconds SHALL bound the TCP connect and the EXA handshake together, so time spent in one step SHALL NOT be granted again to the next
* *AND* a separate bound of N milliseconds SHALL apply to the TLS handshake, SHALL start when the first tunnel read or write begins, and SHALL include the wait for the first TLS byte from the peer
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: TLS tunnel fails with the step named when Exasol does not start the TLS handshake

* *GIVEN* an HTTP tunnel opened with TLS enabled and a setup bound of N milliseconds
* *AND* the IMPORT or EXPORT statement that names the tunnel has been sent, and the first tunnel read or write has begun
* *WHEN* Exasol sends no TLS byte on the tunnel within N milliseconds after that read or write began
* *THEN* that read or write SHALL fail with an error message that contains `HTTP tunnel setup timeout after <N>ms (TLS handshake)`
* *AND* the client SHALL close the tunnel connection
* *AND* when Exasol starts the TLS handshake within the bound, the client SHALL complete it and serve the HTTP request
* *AND* when the peer closes the tunnel connection before it sends a TLS byte, that read or write SHALL fail with an error before the bound elapses
* *AND* when the IMPORT statement returns an error during the wait, the system SHALL stop serving the tunnel and SHALL return the Exasol error to the caller
<!-- /DELTA:NEW -->
