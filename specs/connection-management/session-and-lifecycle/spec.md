# Feature: Session and Lifecycle

Specifies session management, connection pooling foundation, and timeout configuration for Exasol database connections.

## Background

The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection, query, and idle operations. The connection and idle timeouts SHALL apply sensible non-zero defaults. The query-execution timeout SHALL be a server-enforced session setting and default to unset: when a caller configures a query timeout, the driver SHALL forward it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle, and the driver SHALL NOT wrap query execution in a client-side timer. Absent an explicit query timeout, the driver SHALL set no `queryTimeout` attribute and the server-side `QUERY_TIMEOUT` setting SHALL govern. If the driver ever gives up on a running query client-side, it MUST terminate the connection rather than abandon the in-flight request and leave the session open server-side. Default-schema activation on connect is specified in the `connection-management/schema-activation` feature.

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
* *AND* when a schema is supplied via the connection URI or `ConnectionParams.schema`, it SHALL attempt to apply that schema server-side during `connect()` so that subsequent statements resolve unqualified identifiers against it without an additional client call
* *AND* if the server-side schema activation fails for any reason OTHER than the schema not existing (for example authentication, permissions, or transport errors), `connect()` MUST return a `ConnectionError` and MUST NOT leave a half-open connection visible to the caller

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

### Scenario: Connection timeout

* *GIVEN* timeout settings are configured
* *WHEN* establishing a connection
* *THEN* it SHALL enforce a connection timeout
* *AND* it SHALL use a sensible default (e.g., 30 seconds) if not specified

### Scenario: Query timeout

* *GIVEN* an explicit query timeout is configured via the `query_timeout=` connection-string parameter, the `ConnectionParams::query_timeout()` builder, or `Statement::set_timeout()`
* *WHEN* the connection is established or the statement executes
* *THEN* the driver SHALL forward the configured timeout to Exasol as the `queryTimeout` session attribute
* *AND* the server SHALL abort a query whose execution time exceeds the configured timeout and SHALL report the abort as an error through the normal response cycle
* *AND* the driver MUST NOT wrap query execution in a client-side timer
* *AND* the connection SHALL remain usable for subsequent statements after a server-reported timeout

### Scenario: No query timeout by default

* *GIVEN* a connection with no query timeout configured
* *WHEN* the connection is established and a query is executed via `Connection::execute_statement()`
* *THEN* the driver MUST NOT set the `queryTimeout` session attribute
* *AND* the driver MUST NOT wrap query execution in a client-side timer
* *AND* IF a prior statement on the same connection set the `queryTimeout` session attribute, the driver MUST reset it to `0` (unlimited) before executing so no stale server timeout is inherited
* *AND* the query SHALL run until the server returns a result or the server-side `QUERY_TIMEOUT` setting terminates it

### Scenario: Idle timeout

* *GIVEN* timeout settings are configured
* *WHEN* a connection is idle
* *THEN* it SHALL support optional idle timeout configuration
* *AND* it SHALL close connections that exceed idle timeout
