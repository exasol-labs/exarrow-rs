# Feature: HTTP Transport TLS

Specifies TLS behavior of the HTTP transport used for bulk data transfer.

## Background

The HTTP transport encrypts the tunnel with ad-hoc certificates, independently of the WebSocket connection TLS.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: TLS encryption with ad-hoc certificates

* *GIVEN* an import or export with `use_tls(true)` on its options
* *WHEN* the client opens the HTTP tunnel
* *THEN* client SHALL generate an ad-hoc self-signed certificate
* *AND* client SHALL take the TLS server role on the tunnel connection, SHALL accept the TLS handshake that Exasol starts as the TLS client, and SHALL present the ad-hoc certificate in that handshake
* *AND* client SHALL compute SHA-256 fingerprint of DER-encoded public key
* *AND* fingerprint SHALL be formatted as `sha256//<base64>` for SQL PUBLIC KEY clause
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: TLS handshake starts after the IMPORT or EXPORT statement is sent

* *GIVEN* an import or export with `use_tls(true)` whose HTTP tunnel has completed the EXA handshake
* *AND* Exasol starts the TLS handshake on a tunnel only after it receives the IMPORT or EXPORT statement that names the tunnel
* *WHEN* the client sends that statement
* *THEN* the client MUST NOT send TLS data on the tunnel before Exasol starts the TLS handshake
* *AND* the client MUST NOT wait for the TLS handshake before it sends the statement
* *AND* the client MAY begin its first tunnel read before it dispatches the statement, because as the TLS server it sends no TLS byte until Exasol's first TLS handshake message arrives
* *AND* the client SHALL complete the TLS handshake when Exasol starts it and SHALL serve Exasol's HTTP requests over the TLS connection
* *AND* when the TLS handshake fails during an import, the system SHALL return the tunnel error to the caller only after the IMPORT statement has returned
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: WebSocket TLS and HTTP transport TLS are independent

* *GIVEN* a connection has been established to Exasol with the main control channel (WebSocket or native TCP) using its own TLS configuration
* *WHEN* the application initiates an import or export operation that uses the HTTP transport tunnel
* *THEN* the HTTP-transport TLS setting (`use_tls(bool)` on the import/export options) SHALL be evaluated independently of the main control channel's TLS state
* *AND* the driver MUST NOT infer the HTTP-transport TLS setting from the control channel's TLS configuration
* *AND* the documented default for the HTTP-transport `use_tls` SHALL remain `false`, so an application that does not set it keeps a plain HTTP tunnel
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: HTTP transport TLS against Exasol Docker (self-signed certificate)

* *GIVEN* the application connects to a local `exasol/docker-db` instance with a control-channel connection string of the form `exasol://sys:exasol@localhost:8563/?validateservercertificate=0`
* *WHEN* the application runs a single-file CSV import, a multi-file CSV import, a native Parquet import, or a CSV export with `use_tls(true)` on the import or export options
* *THEN* the operation SHALL complete and SHALL transfer every row
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: HTTP transport TLS against Exasol SaaS / production

* *GIVEN* the application connects to a managed or production Exasol cluster with TLS termination on the control channel and a trusted certificate chain
* *WHEN* the application performs an import or export through the HTTP transport tunnel
* *THEN* the application SHOULD set `use_tls(true)` on the import/export options
* *AND* the driver SHALL generate an ad-hoc self-signed certificate, accept the TLS handshake that Exasol starts on the tunnel, and pass the SHA-256 fingerprint as `PUBLIC KEY 'sha256//<base64>'` in the IMPORT/EXPORT SQL `AT` clause
<!-- /DELTA:CHANGED -->
