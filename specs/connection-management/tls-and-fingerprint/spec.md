# Feature: TLS and Fingerprint

Defines TLS configuration and certificate fingerprint validation for Exasol database connections.

## Background

TLS is enabled by default for production Exasol connections.

## Scenarios

### Scenario: TLS configuration

* *GIVEN* connection parameters are configured
* *WHEN* TLS/SSL is requested
* *THEN* it SHALL enable TLS by default when the `tls` parameter (aliases `ssl`, `use_tls`) is omitted from the connection string
* *AND* it SHALL enable server-certificate validation by default when the `validate_certificate` parameter (aliases `verify_certificate`, `validateservercertificate`) is omitted
* *AND* it SHALL honor an explicit `tls=false` to disable TLS for legacy (pre-7.1) Exasol servers
* *AND* it SHALL honor an explicit `validateservercertificate=0` to disable certificate validation for self-signed certificates (e.g. Exasol Docker)
* *AND* it SHALL accept an optional certificate fingerprint for pin-based validation

### Scenario: Certificate fingerprint validation success

* *GIVEN* a TLS connection is being established
* *AND* `certificate_fingerprint` is set to the SHA-256 hex fingerprint of the server's certificate
* *WHEN* the TLS handshake completes
* *THEN* the driver SHALL compute the SHA-256 hex digest of the server's DER-encoded certificate
* *AND* it SHALL accept the connection when the fingerprints match

### Scenario: Certificate fingerprint validation failure

* *GIVEN* a TLS connection is being established
* *AND* `certificate_fingerprint` is set to an incorrect value
* *WHEN* the TLS handshake completes
* *THEN* the driver SHALL reject the connection with a certificate fingerprint mismatch error
* *AND* the error MUST include both the expected fingerprint and the actual server fingerprint

### Scenario: Certificate fingerprint bypasses hostname validation

* *GIVEN* a TLS connection is being established
* *AND* `certificate_fingerprint` is set
* *WHEN* the TLS handshake completes
* *THEN* the driver SHALL skip standard certificate chain and hostname validation
* *AND* it SHALL rely solely on the fingerprint match for trust
