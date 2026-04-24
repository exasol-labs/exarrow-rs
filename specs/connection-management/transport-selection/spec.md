# Feature: Transport Selection via Feature Flags

The system uses Cargo feature flags to select the transport implementation. The `native` feature (enabled by default) provides the high-performance native TCP transport. The `websocket` feature provides the existing WebSocket JSON transport as a fallback. The connection layer dispatches to the appropriate transport based on enabled features and optional connection string overrides.

## Background

Both transports connect to the same Exasol port (8563). The server auto-detects the protocol based on the first bytes received. Only one transport is used per connection. When both features are enabled, native is preferred unless explicitly overridden.

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
