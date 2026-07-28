# Feature: Native TCP Protocol

The system implements Exasol's native binary TCP protocol as a high-performance alternative to the WebSocket JSON protocol. The native protocol uses binary message framing with 21-byte headers, little-endian byte ordering, and protocol version negotiation starting at v14. All commands are serialized as binary attribute sets, and responses are parsed from binary frames into structured Rust types.

## Background

The native TCP protocol connects to the same Exasol port (8563) as the WebSocket protocol. The server dispatches based on the first bytes received: `LOGIN_MAGIC` (0x01121201) for native TCP, `GET ` for WebSocket. Protocol version 14 is the minimum supported version, which requires ChaCha20 encryption and deprecates RC4. Every decoder that parses server-supplied bytes MUST report malformed input as an error and MUST NOT panic.

## Scenarios

<!-- DELTA:NEW -->
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
<!-- /DELTA:NEW -->
