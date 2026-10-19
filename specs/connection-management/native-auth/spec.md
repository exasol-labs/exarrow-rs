# Feature: Native Auth

Defines password encryption and the ChaCha20 key exchange in the native TCP login handshake.

## Background

The native login encrypts the password with Exasol's raw RSA modular exponentiation (no PKCS#1 padding). The driver generates and sends ChaCha20 session keys only when TLS is not active (legacy mode for pre-7.1 servers). With TLS active, messages travel in plaintext inside the TLS tunnel.

## Scenarios

### Scenario: ChaCha20 key exchange during native protocol login (legacy mode)

* *GIVEN* a native TCP connection has been established WITHOUT TLS (e.g. `tls=false` against a pre-7.1 Exasol server)
* *AND* the RSA-encrypted password authentication has succeeded
* *WHEN* completing the native protocol login phase
* *THEN* the system SHALL generate two 32-byte ChaCha20 keys, RSA-encrypt them, and send them via `CMD_SET_ATTRIBUTES`
* *AND* all subsequent native protocol messages SHALL be encrypted with ChaCha20

### Scenario: Native protocol password encryption

* *GIVEN* a native TCP login handshake is in progress
* *AND* the server has responded with `ATTR_PUBLIC_KEY` and `ATTR_RANDOM_PHRASE`
* *WHEN* authenticating with username and password
* *THEN* the system SHALL encrypt the password using Exasol's raw RSA modular exponentiation scheme (no PKCS#1 padding) with the server's public key and random phrase
* *AND* the system SHALL send the encrypted password via `ATTR_ENCODED_PASSWORD` (34)

### Scenario: ChaCha20 key exchange skipped when TLS is active

* *GIVEN* a native TCP connection has completed TLS negotiation successfully
* *AND* the RSA-encrypted password authentication has succeeded
* *WHEN* completing the native protocol login phase
* *THEN* the system MUST NOT generate ChaCha20 keys
* *AND* the `CMD_SET_ATTRIBUTES` message MUST NOT include `ATTR_CLIENT_SEND_KEY`, `ATTR_CLIENT_RECEIVE_KEY`, or `ATTR_CLIENT_KEYS_LEN`
* *AND* all subsequent native protocol messages SHALL be transmitted in plaintext inside the TLS tunnel
