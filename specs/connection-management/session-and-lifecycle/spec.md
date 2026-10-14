# Feature: Session and Lifecycle

Specifies session management, connection pooling foundation, and timeout configuration for Exasol database connections.

## Background

The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection and query operations, and the query timeout and the deprecated idle timeout are specified in `connection-management/query-timeout`. The connection timeout, which bounds opening a connection, is specified in `connection-management/connection-timeout`. The driver has no idle timeout and never closes a connection because the connection is idle, so the `idle_timeout` connection-string parameter is deprecated and has no effect. The query-execution timeout SHALL be a server-enforced session setting and default to unset: when a caller configures a query timeout, the driver SHALL forward it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle, and the driver SHALL NOT wrap query execution through `Connection::execute_statement()` in a client-side timer. A client-side bound on a CSV export is a separate, export-scoped setting specified in `import-export/csv-export-timeout`. Absent an explicit query timeout, the driver SHALL set no `queryTimeout` attribute and the server-side `QUERY_TIMEOUT` setting SHALL govern. If the driver ever gives up on a running query client-side, it MUST terminate the connection rather than abandon the in-flight request and leave the session open server-side. Default-schema activation on connect is specified in the `connection-management/schema-activation` feature. The client-side session states and the rules for operations in each state are specified in `connection-management/session-state`.

## Scenarios

### Scenario: Session establishment

* *GIVEN* a session is active with Exasol
* *WHEN* a connection is authenticated
* *THEN* it SHALL create a session with Exasol
* *AND* it SHALL track session identifiers

### Scenario: Session attributes

* *GIVEN* a session is active with Exasol
* *WHEN* session attributes are requested
* *THEN* it SHALL provide current schema, session ID, and other metadata
* *AND* it SHALL allow setting session attributes (e.g., current schema)
* *AND* when a schema is supplied via the connection URI or `ConnectionParams.schema`, it SHALL apply that schema server-side during `connect()` so that subsequent statements resolve unqualified identifiers against it without an additional client call
* *AND* if the server rejects that schema for any reason, including a schema that does not exist, `connect()` MUST return a `ConnectionError` and MUST NOT leave a half-open connection visible to the caller

### Scenario: Session termination

* *GIVEN* a session is active with Exasol
* *WHEN* a session is closed
* *THEN* the server SHALL be notified of session termination
* *AND* it SHALL release server-side resources

### Scenario: Connection reusability

* *GIVEN* a connection has been used and released
* *WHEN* a connection is closed by the application
* *THEN* its implementation SHALL support clean state reset
* *AND* it SHALL be designed to allow reuse in future pooling implementations

### Scenario: Connection health checking

* *GIVEN* a connection has been used and released
* *WHEN* checking if a connection is usable
* *THEN* it SHALL provide a health check method
* *AND* it SHALL return connection validity status

### Scenario: Terminate a connection whose in-flight response is no longer trusted

* *GIVEN* the driver has given up on an in-flight request and can no longer match a response to the request that produced it
* *WHEN* the driver terminates that connection
* *THEN* the transport SHALL drop its socket and SHALL report itself as not connected, and the native transport and the WebSocket transport SHALL produce that same observable outcome
* *AND* the transport MUST NOT send a disconnect command and MUST NOT wait for any server response, because the pending response would arrive out of order and block termination for as long as the abandoned request keeps running
* *AND* every subsequent operation on that transport except `close` MUST fail instead of reading the abandoned response, and `close` SHALL succeed without a protocol round-trip
* *AND* `Connection::is_closed()` SHALL report the connection as closed once its transport is terminated, so one fact about liveness has one answer

### Scenario: Operations after an export timeout name the termination

* *GIVEN* an export timeout terminated the transport before the EXPORT response was read
* *WHEN* the caller runs any operation other than `close` on that transport, including connect and authenticate, before or after closing it
* *THEN* the operation SHALL fail with an error that states the transport was terminated after an export gave up on an in-flight response and that tells the caller to reconnect
* *AND* the error MUST NOT tell the caller to authenticate
* *AND* a transport that a graceful `close` ended without a prior termination MUST NOT report this cause
