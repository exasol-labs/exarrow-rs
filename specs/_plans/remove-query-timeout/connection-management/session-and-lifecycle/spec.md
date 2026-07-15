# Feature: Session and Lifecycle

## Background

<!-- DELTA:CHANGED -->
The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection, query, and idle operations. The connection and idle timeouts SHALL apply sensible non-zero defaults. The query-execution timeout SHALL be opt-in and default to unbounded: absent an explicit query timeout, the driver SHALL NOT impose any client-side query-execution limit, and a query SHALL run until the server returns a result or the server-side session `QUERY_TIMEOUT` setting terminates it. When an explicit query timeout is configured, the driver SHALL cancel a query that exceeds it. When the connection URI or `ConnectionParams` carries a schema, the driver SHALL treat that schema as a best-effort default: it SHALL attempt to activate the schema server-side during `connect()` so unqualified statements resolve immediately, but a schema that does not yet exist SHALL NOT abort the connection. This matches tools such as dbt, which connect first and create their target schema afterwards while fully qualifying every relation, so a not-yet-existing default schema is a normal state rather than a fatal error.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Query timeout

* *GIVEN* an explicit query timeout is configured via the `query_timeout=` connection-string parameter, the `ConnectionParams::query_timeout()` builder, or `Statement::set_timeout()`
* *WHEN* a query executes and its elapsed execution time exceeds the configured timeout
* *THEN* the driver SHALL cancel the query and return a timeout error
* *AND* the driver SHALL enforce a query timeout only when a caller explicitly configures one
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: No query timeout by default

* *GIVEN* a connection with no query timeout configured
* *WHEN* a query is executed via `Connection::execute_statement()`
* *THEN* the driver MUST NOT wrap query execution in a client-side timer
* *AND* the driver MUST NOT abort or cancel the query based on any client-side elapsed duration
* *AND* the query SHALL run until the server returns a result or the server-side session `QUERY_TIMEOUT` setting terminates it
<!-- /DELTA:NEW -->
