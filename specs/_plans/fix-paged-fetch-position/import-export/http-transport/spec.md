# Feature: HTTP Transport

Specifies the HTTP transport layer for bulk data transfer using the EXA tunneling protocol, enabling firewall-friendly import and export operations through reverse-connection HTTP tunneling.

## Background

The client establishes an outbound TCP connection to Exasol and performs a magic packet handshake to create a bidirectional HTTP tunnel. All data transfer occurs through this single established connection, enabling firewall-friendly operation requiring only outbound connections. TLS encryption uses ad-hoc RSA certificates with SHA-256 fingerprints passed in SQL PUBLIC KEY clauses. The HTTP-transport TLS layer is independent of the main control-channel TLS layer (WebSocket or native TCP); applications configure HTTP-transport TLS per import/export operation via `use_tls(bool)` on the corresponding option builder.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Failed IMPORT statement returns its error without waiting for the tunnel

* *GIVEN* a single-file or multi-file CSV or Parquet import has opened its HTTP tunnel connections and sent its IMPORT statement
* *AND* Exasol rejects the IMPORT statement before it requests data through a tunnel, for example because the target table does not exist
* *WHEN* the IMPORT statement returns its error
* *THEN* the system SHALL stop serving the tunnel connections and SHALL return the Exasol error to the caller
* *AND* the system MUST NOT wait for Exasol to close a tunnel connection
* *AND* when a tunnel task has failed before the IMPORT statement returns its error, the system SHALL return that task's error
<!-- /DELTA:NEW -->
