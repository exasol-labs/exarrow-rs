# Feature: ChaCha20 Session Encryption

The native TCP protocol uses ChaCha20 stream encryption for all messages after the initial login handshake. The client generates random send and receive keys, encrypts them with the server's RSA public key, and transmits them during login. All subsequent message payloads (after the handshake) are encrypted with ChaCha20, providing an additional encryption layer on top of TLS.

## Background

Protocol version 14+ requires ChaCha20 encryption (RC4 is deprecated). ChaCha20 keys are 32 bytes. The key exchange happens during the login phase via `CMD_SET_ATTRIBUTES` with `ATTR_CLIENT_SEND_KEY` and `ATTR_CLIENT_RECEIVE_KEY`. TLS provides the outer encryption layer; ChaCha20 provides inner message-level encryption.

## Scenarios

### Scenario: ChaCha20 key generation

* *GIVEN* the native protocol login handshake has received the server's RSA public key
* *WHEN* preparing encryption keys
* *THEN* the system SHALL generate two cryptographically random 32-byte keys (send key and receive key)
* *AND* the system SHALL use a cryptographically secure random number generator

### Scenario: RSA-encrypted key exchange

* *GIVEN* ChaCha20 send and receive keys have been generated
* *AND* the server's RSA public key and random phrase are available
* *WHEN* exchanging encryption keys with the server
* *THEN* the system SHALL RSA-encrypt each key using the server's public key via Exasol's raw RSA modular exponentiation scheme (no PKCS#1 padding)
* *AND* the system SHALL send `ATTR_CLIENT_SEND_KEY`, `ATTR_CLIENT_RECEIVE_KEY`, and `ATTR_CLIENT_KEYS_LEN` via `CMD_SET_ATTRIBUTES`

### Scenario: Encrypted message sending

* *GIVEN* ChaCha20 key exchange has completed successfully
* *WHEN* sending any message after the handshake
* *THEN* the system SHALL encrypt the message payload using ChaCha20 with the send key
* *AND* the message header SHALL remain unencrypted for framing purposes

### Scenario: Encrypted message receiving

* *GIVEN* ChaCha20 key exchange has completed successfully
* *WHEN* receiving any message after the handshake
* *THEN* the system SHALL decrypt the message payload using ChaCha20 with the receive key
* *AND* the system SHALL verify that decrypted data is valid before processing

### Scenario: TLS and ChaCha20 layering

* *GIVEN* a TLS connection is established to Exasol
* *WHEN* the native protocol session is fully initialized
* *THEN* TLS SHALL provide the outer transport encryption layer
* *AND* ChaCha20 SHALL provide the inner message-level encryption layer
* *AND* the system SHALL support both layers simultaneously
