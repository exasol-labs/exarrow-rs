# Feature: HTTP Transport TLS

Specifies TLS behavior of the HTTP transport used for bulk data transfer.

## Background

The HTTP transport encrypts the tunnel with ad-hoc certificates, independently of the WebSocket connection TLS.

## Scenarios

### Scenario: TLS encryption with ad-hoc certificates

* *GIVEN* TLS encryption is configured for the connection
* *WHEN* TLS encryption is enabled
* *THEN* client SHALL generate ad-hoc RSA certificate
* *AND* client SHALL wrap connection with TLS after magic packet exchange
* *AND* client SHALL compute SHA-256 fingerprint of DER-encoded public key
* *AND* fingerprint SHALL be formatted as `sha256//<base64>` for SQL PUBLIC KEY clause

### Scenario: WebSocket TLS and HTTP transport TLS are independent

* *GIVEN* a connection has been established to Exasol with the main control channel (WebSocket or native TCP) using its own TLS configuration
* *WHEN* the application initiates an import or export operation that uses the HTTP transport tunnel
* *THEN* the HTTP-transport TLS setting (`use_tls(bool)` on the import/export options) SHALL be evaluated independently of the main control channel's TLS state
* *AND* the driver MUST NOT infer the HTTP-transport TLS setting from the control channel's TLS configuration
* *AND* the documented default for the HTTP-transport `use_tls` SHALL remain `false` because Exasol generates ad-hoc certificates for the tunnel that fail standard certificate validation in many client environments

### Scenario: HTTP transport TLS against Exasol Docker (self-signed certificate)

* *GIVEN* the application connects to a local `exasol/docker-db` instance with a control-channel connection string of the form `exasol://sys:exasol@localhost:8563/?validateservercertificate=0`
* *WHEN* the application performs an import or export through the HTTP transport tunnel
* *THEN* the application SHOULD set `use_tls(false)` on the import/export options
* *AND* the driver SHALL use a plain HTTP tunnel (no rustls wrap) so that the Exasol-side SQLProcess can connect back without certificate-validation failures from the ad-hoc cert

### Scenario: HTTP transport TLS against Exasol SaaS / production

* *GIVEN* the application connects to a managed or production Exasol cluster with TLS termination on the control channel and a trusted certificate chain
* *WHEN* the application performs an import or export through the HTTP transport tunnel
* *THEN* the application SHOULD set `use_tls(true)` on the import/export options
* *AND* the driver SHALL generate an ad-hoc RSA certificate, wrap the tunnel with rustls, and pass the SHA-256 fingerprint as `PUBLIC KEY 'sha256//<base64>'` in the IMPORT/EXPORT SQL `AT` clause
