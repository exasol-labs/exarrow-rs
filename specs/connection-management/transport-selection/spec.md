# Feature: Transport Selection via Feature Flags

Cargo feature flags select the transport. The `native` feature (default) provides the native TCP transport, and the `websocket` feature provides the WebSocket JSON transport. The connection layer dispatches to a transport based on the enabled features and an optional connection-string override.

## Background

Each connection uses one transport. When both features are enabled, native is the default. Exasol 7.1 and later (native protocol version 18 or later, including Exasol 8) require TLS on port 8563 and use no ChaCha20 message encryption. Older servers may use the legacy non-TLS path with ChaCha20 encryption.

## Scenarios

### Scenario: Default native transport

* *GIVEN* the `native` feature flag is enabled (default)
* *AND* no explicit transport override is specified in the connection string
* *WHEN* creating a new connection
* *THEN* the system SHALL use the native TCP transport

### Scenario: WebSocket transport via feature flag

* *GIVEN* only the `websocket` feature flag is enabled
* *AND* the `native` feature flag is not enabled
* *WHEN* creating a new connection
* *THEN* the system SHALL use the WebSocket transport

### Scenario: Connection string transport override

* *GIVEN* both `native` and `websocket` feature flags are enabled
* *AND* the connection string contains `transport=websocket`
* *WHEN* creating a new connection
* *THEN* the system SHALL use the WebSocket transport regardless of the default

### Scenario: No transport feature enabled

* *GIVEN* neither `native` nor `websocket` feature flag is enabled
* *WHEN* attempting to create a connection
* *THEN* the system SHALL fail at compile time with a clear error message indicating that at least one transport feature MUST be enabled

### Scenario: Native transport uses TLS exclusively against Exasol 7.1+ and Exasol 8

* *GIVEN* the `native` feature flag is enabled
* *AND* the target Exasol server runs version 7.1 or later (native protocol version 18 or later), which requires TLS on port 8563
* *WHEN* establishing a native TCP connection with the default settings
* *THEN* the system SHALL negotiate TLS on the underlying TCP socket before the native protocol handshake
* *AND* the system MUST NOT perform the ChaCha20 key exchange during login
* *AND* all native protocol messages SHALL be transmitted as plaintext inside the TLS tunnel

### Scenario: Native transport uses ChaCha20 legacy encryption when TLS is disabled

* *GIVEN* the `native` feature flag is enabled
* *AND* the connection string sets `tls=false` (for example against a pre-7.1 Exasol server that does not support TLS)
* *WHEN* establishing a native TCP connection
* *THEN* the system MUST NOT attempt TLS negotiation on the underlying socket
* *AND* the system SHALL perform the ChaCha20 key exchange during the login handshake
* *AND* all subsequent native protocol messages SHALL be encrypted with ChaCha20
