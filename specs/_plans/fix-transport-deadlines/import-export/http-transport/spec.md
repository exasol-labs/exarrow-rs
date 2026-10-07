# Feature: HTTP Transport

Specifies the HTTP transport layer for bulk data transfer using the EXA tunneling protocol, enabling firewall-friendly import and export operations through reverse-connection HTTP tunneling.

## Background

The client establishes an outbound TCP connection to Exasol and performs a magic packet handshake to create a bidirectional HTTP tunnel. All data transfer occurs through this single established connection, enabling firewall-friendly operation requiring only outbound connections. TLS encryption uses ad-hoc RSA certificates with SHA-256 fingerprints passed in SQL PUBLIC KEY clauses. The HTTP-transport TLS layer is independent of the main control-channel TLS layer (WebSocket or native TCP); applications configure HTTP-transport TLS per import/export operation via `use_tls(bool)` on the corresponding option builder.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Tunnel setup fails with the step named when the peer stops answering

* *GIVEN* an HTTP tunnel opened through `HttpTransportClient::connect_with_timeout` with a setup bound of N milliseconds
* *AND* the peer accepts the TCP connection and then stops answering, either before it sends the EXA handshake response or, with TLS enabled, after the EXA handshake and before it completes the TLS handshake
* *WHEN* the setup bound elapses
* *THEN* the client SHALL fail the tunnel setup with an error message that contains `HTTP tunnel setup timeout after <N>ms (<step>)`, where `<step>` is `TCP connect`, `EXA handshake`, or `TLS handshake`
* *AND* one deadline SHALL bound the TCP connect, the EXA handshake, and the TLS handshake together, so time spent in one step SHALL NOT be granted again to a later step
* *AND* the client SHALL close the tunnel connection
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Tunnel setup is bounded by 30 seconds by default

* *GIVEN* an import or an export that opens its HTTP tunnel
* *AND* the tunnel peer accepts the TCP connection and never sends the EXA handshake response
* *WHEN* 30 seconds pass after the tunnel setup started
* *THEN* the import or export SHALL fail with an error message that contains `HTTP tunnel setup timeout after 30000ms (EXA handshake)`
* *AND* the driver MUST NOT send the IMPORT or EXPORT statement
* *AND* the bound SHALL apply whether or not `CsvExportOptions::timeout_ms` is set, because that export timeout covers only the work after tunnel setup
<!-- /DELTA:NEW -->
