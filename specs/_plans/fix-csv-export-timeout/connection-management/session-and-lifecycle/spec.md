# Feature: Session and Lifecycle

Specifies session management, connection pooling foundation, and timeout configuration for Exasol database connections.

## Background

<!-- DELTA:CHANGED -->
The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection, query, and idle operations. The connection and idle timeouts SHALL apply sensible non-zero defaults. The query-execution timeout SHALL be a server-enforced session setting and default to unset: when a caller configures a query timeout, the driver SHALL forward it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle, and the driver SHALL NOT wrap query execution through `Connection::execute_statement()` in a client-side timer. A client-side bound on a CSV export is a separate, export-scoped setting specified in `import-export/csv-export`. Absent an explicit query timeout, the driver SHALL set no `queryTimeout` attribute and the server-side `QUERY_TIMEOUT` setting SHALL govern. If the driver ever gives up on a running query client-side, it MUST terminate the connection rather than abandon the in-flight request and leave the session open server-side. Default-schema activation on connect is specified in the `connection-management/schema-activation` feature.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Query timeout

* *GIVEN* an explicit query timeout is configured via the `query_timeout=` connection-string parameter, the `ConnectionParams::query_timeout()` builder, or `Statement::set_timeout()`
* *WHEN* the connection is established or the statement executes
* *THEN* the driver SHALL forward the configured timeout to Exasol as the `queryTimeout` session attribute
* *AND* the server SHALL abort a query whose execution time exceeds the configured timeout and SHALL report the abort as an error through the normal response cycle
* *AND* the driver MUST NOT wrap query execution through `Connection::execute_statement()` in a client-side timer
* *AND* the connection SHALL remain usable for subsequent statements after a server-reported timeout
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Terminate a connection whose in-flight response is no longer trusted

* *GIVEN* the driver has given up on an in-flight request and can no longer match a response to the request that produced it
* *WHEN* the driver terminates that connection
* *THEN* the transport SHALL drop its socket and SHALL report itself as not connected, and the native transport and the WebSocket transport SHALL produce that same observable outcome
* *AND* the transport MUST NOT send a disconnect command and MUST NOT wait for any server response, because the pending response would arrive out of order and block termination for as long as the abandoned request keeps running
* *AND* every subsequent operation on that transport MUST fail instead of reading the abandoned response
* *AND* `Connection::is_closed()` SHALL report the connection as closed once its transport is terminated, so one fact about liveness has one answer
<!-- /DELTA:NEW -->
