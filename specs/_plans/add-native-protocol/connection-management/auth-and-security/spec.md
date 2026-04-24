# Feature: Auth and Security

Connection parameters are required for Exasol connectivity and SHALL be validated before use. Authentication mechanisms SHALL operate securely, with credentials protected in memory and never exposed through logging. TLS is enabled by default for production Exasol connections.

## Background

All authentication credentials are protected in memory and zeroed on drop. The native TCP protocol adds ChaCha20 session encryption on top of the existing RSA password encryption.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: ChaCha20 key exchange during native protocol login

* *GIVEN* a native TCP connection is established to Exasol
* *AND* the RSA-encrypted password authentication has succeeded
* *WHEN* completing the native protocol login phase
* *THEN* the system SHALL generate two 32-byte ChaCha20 keys, RSA-encrypt them, and send via `CMD_SET_ATTRIBUTES`
* *AND* all subsequent native protocol messages SHALL be encrypted with ChaCha20

### Scenario: Native protocol password encryption

* *GIVEN* a native TCP login handshake is in progress
* *AND* the server has responded with `ATTR_PUBLIC_KEY` and `ATTR_RANDOM_PHRASE`
* *WHEN* authenticating with username and password
* *THEN* the system SHALL encrypt the password using RSA PKCS#1 v1.5 with the server's public key and random phrase
* *AND* the system SHALL send the encrypted password via `ATTR_ENCODED_PASSWORD` (34)
<!-- /DELTA:NEW -->
