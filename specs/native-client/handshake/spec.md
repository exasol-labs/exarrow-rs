# Feature: Native TCP Handshake

The system establishes a native TCP session with Exasol before any command is exchanged: opening the transport connection, negotiating protocol version, and completing the login handshake. Malformed server input during this phase is rejected with a typed error rather than a panic.

## Background

The native TCP protocol connects to the same Exasol port (8563) as the WebSocket protocol. The server dispatches based on the first bytes received: `LOGIN_MAGIC` (0x01121201) for native TCP, `GET ` for WebSocket. Protocol version 14 is the minimum supported version, which requires ChaCha20 encryption and deprecates RC4. Once the handshake completes, ordinary command traffic is specified in `native-client/protocol`.

## Scenarios

### Scenario: Native TCP connection establishment

* *GIVEN* an Exasol database is reachable on a configured host and port
* *WHEN* connecting via the native TCP protocol
* *THEN* the system SHALL open a TCP connection to the specified host and port
* *AND* the system SHALL apply TLS when TLS is enabled (default)
* *AND* the system SHALL enforce the configured connection timeout

### Scenario: Protocol version negotiation

* *GIVEN* a TCP connection is established to Exasol
* *WHEN* initiating the native protocol handshake
* *THEN* the system SHALL send a login packet starting with `LOGIN_MAGIC` (0x01121201)
* *AND* the login packet SHALL include the client's maximum supported protocol version
* *AND* the system SHALL accept any server-negotiated version between 14 and the client's maximum
* *AND* the system SHALL reject protocol versions below 14 with a clear error

### Scenario: Login handshake

* *GIVEN* a TCP connection is established and TLS negotiation is complete
* *WHEN* performing the login handshake
* *THEN* the system SHALL send a login packet containing: magic (4 bytes), message length (4 bytes), protocol version (4 bytes), change date (4 bytes), and connection attributes
* *AND* all multi-byte integers SHALL be encoded in little-endian byte order
* *AND* the system SHALL receive the server's response containing `ATTR_PUBLIC_KEY`, `ATTR_RANDOM_PHRASE`, and `ATTR_PROTOCOL_VERSION`

### Scenario: Binary message framing

* *GIVEN* an authenticated native TCP session exists
* *WHEN* sending a command to Exasol
* *THEN* the system SHALL prefix each message with a 21-byte header containing: message length (4 bytes), command type (1 byte), serial number (4 bytes), number of attributes (4 bytes), attribute data length (4 bytes), and number of result parts (4 bytes)
* *AND* all multi-byte values in the header SHALL be little-endian

### Scenario: Malformed handshake and attribute input is rejected without panicking

* *GIVEN* the native protocol decodes server-supplied handshake bytes and binary attribute payloads
* *WHEN* the server supplies an empty random phrase
* *THEN* password encoding MUST return a `TransportError` that names the empty phrase
* *AND* password encoding MUST NOT panic with a remainder-by-zero division
* *WHEN* the server supplies a raw RSA public key shorter than 4 bytes or of odd length
* *THEN* key parsing MUST return a `TransportError`
* *WHEN* the server supplies a PKCS#1 DER key with a wrong tag, a truncated length field, or trailing bytes
* *THEN* key parsing MUST return a `TransportError` that describes the defect
* *WHEN* the server supplies an attribute payload truncated before its declared length
* *THEN* attribute parsing MUST return a `TransportError` that names the required and the available byte counts
