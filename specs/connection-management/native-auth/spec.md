# Feature: Native Auth

Defines the authentication mechanisms specific to the Exasol native TCP protocol, including key exchange and password encryption for the native login handshake.

## Background

The native TCP protocol uses a distinct authentication flow from the WebSocket protocol. Passwords are encrypted using Exasol's raw RSA modular exponentiation scheme (without PKCS#1 padding), and a ChaCha20 session key is established for encrypting all subsequent messages.

## Scenarios

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
* *THEN* the system SHALL encrypt the password using Exasol's raw RSA modular exponentiation scheme (no PKCS#1 padding) with the server's public key and random phrase
* *AND* the system SHALL send the encrypted password via `ATTR_ENCODED_PASSWORD` (34)
