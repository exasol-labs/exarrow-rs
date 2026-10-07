# Feature: Session and Lifecycle

Specifies session management, connection pooling foundation, and timeout configuration for Exasol database connections.

<!-- DELTA:CHANGED -->
## Background

The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection, query, and idle operations. The connection timeout, which bounds opening a connection, is specified in `connection-management/connection-timeout`. The idle timeout SHALL apply a sensible non-zero default. The query-execution timeout SHALL be a server-enforced session setting and default to unset: when a caller configures a query timeout, the driver SHALL forward it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle, and the driver SHALL NOT wrap query execution through `Connection::execute_statement()` in a client-side timer. A client-side bound on a CSV export is a separate, export-scoped setting specified in `import-export/csv-export-timeout`. Absent an explicit query timeout, the driver SHALL set no `queryTimeout` attribute and the server-side `QUERY_TIMEOUT` setting SHALL govern. If the driver ever gives up on a running query client-side, it MUST terminate the connection rather than abandon the in-flight request and leave the session open server-side. Default-schema activation on connect is specified in the `connection-management/schema-activation` feature.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Connection timeout

* *GIVEN* timeout settings are configured
* *WHEN* establishing a connection
* *THEN* it SHALL enforce a connection timeout
* *AND* it SHALL use a sensible default (e.g., 30 seconds) if not specified
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Terminate a connection whose in-flight response is no longer trusted

* *GIVEN* the driver has given up on an in-flight request and can no longer match a response to the request that produced it
* *WHEN* the driver terminates that connection
* *THEN* the transport SHALL drop its socket and SHALL report itself as not connected, and the native transport and the WebSocket transport SHALL produce that same observable outcome
* *AND* the transport MUST NOT send a disconnect command and MUST NOT wait for any server response, because the pending response would arrive out of order and block termination for as long as the abandoned request keeps running
* *AND* every subsequent operation on that transport except `close` MUST fail instead of reading the abandoned response, and `close` SHALL succeed without a protocol round-trip
* *AND* `Connection::is_closed()` SHALL report the connection as closed once its transport is terminated, so one fact about liveness has one answer
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Operations after an export timeout name the termination

* *GIVEN* an export timeout terminated the transport before the EXPORT response was read
* *WHEN* the caller runs any operation other than `close` on that transport, including connect and authenticate, before or after closing it
* *THEN* the operation SHALL fail with an error that states the transport was terminated after an export gave up on an in-flight response and that tells the caller to reconnect
* *AND* the error MUST NOT tell the caller to authenticate
* *AND* a transport that a graceful `close` ended without a prior termination MUST NOT report this cause
<!-- /DELTA:NEW -->
