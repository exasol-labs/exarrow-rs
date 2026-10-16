# Feature: HTTP Transport

Specifies the HTTP transport layer for bulk data transfer using the EXA tunneling protocol, enabling firewall-friendly import and export operations through reverse-connection HTTP tunneling.

## Background

The client establishes an outbound TCP connection to Exasol and performs a magic packet handshake to create a bidirectional HTTP tunnel. All data transfer occurs through this single established connection, enabling firewall-friendly operation requiring only outbound connections. With TLS, the IMPORT or EXPORT SQL names the tunnel certificate in a PUBLIC KEY clause.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Export data flow (Exasol sends data to client)

* *GIVEN* an HTTP tunnel is established for an export operation
* *WHEN* EXPORT SQL is executed
* *THEN* Exasol SHALL send HTTP PUT request through tunnel with data
* *AND* client SHALL receive PUT request with chunked transfer encoding, or with a Content-Length header
* *AND* client SHALL read data chunks until zero-length terminator, or read exactly the Content-Length bytes
* *AND* client SHALL send HTTP 200 OK after it has forwarded the last piece of the body
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Export body is forwarded in bounded pieces

* *GIVEN* the client reads the body of an EXPORT PUT request
* *WHEN* body bytes arrive through the tunnel
* *THEN* client SHALL forward them to the caller's bounded channel as it reads them, in pieces of at most 64 KiB (65536 bytes), and MUST NOT collect the complete body first
* *AND* while the caller's channel is full, client SHALL NOT read further body bytes from the tunnel
* *AND* IF the caller has closed its channel, client SHALL read and discard the rest of the body without forwarding it, and SHALL then send HTTP 200 OK
<!-- /DELTA:NEW -->
