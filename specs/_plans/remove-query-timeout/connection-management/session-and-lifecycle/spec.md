# Feature: Session and Lifecycle

## Background

<!-- DELTA:CHANGED -->
The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection, query, and idle operations. The connection and idle timeouts SHALL apply sensible non-zero defaults. The query-execution timeout SHALL be a server-enforced session setting and default to unset: when a caller configures a query timeout, the driver SHALL forward it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle, and the driver SHALL NOT wrap query execution in a client-side timer. Absent an explicit query timeout, the driver SHALL set no `queryTimeout` attribute and the server-side `QUERY_TIMEOUT` setting SHALL govern. If the driver ever gives up on a running query client-side, it MUST terminate the connection rather than abandon the in-flight request and leave the session open server-side. When the connection URI or `ConnectionParams` carries a schema, the driver SHALL treat that schema as a best-effort default: it SHALL attempt to activate the schema server-side during `connect()` so unqualified statements resolve immediately, but a schema that does not yet exist SHALL NOT abort the connection. This matches tools such as dbt, which connect first and create their target schema afterwards while fully qualifying every relation, so a not-yet-existing default schema is a normal state rather than a fatal error.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Query timeout

* *GIVEN* an explicit query timeout is configured via the `query_timeout=` connection-string parameter, the `ConnectionParams::query_timeout()` builder, or `Statement::set_timeout()`
* *WHEN* the connection is established or the statement executes
* *THEN* the driver SHALL forward the configured timeout to Exasol as the `queryTimeout` session attribute
* *AND* the server SHALL abort a query whose execution time exceeds the configured timeout and SHALL report the abort as an error through the normal response cycle
* *AND* the driver MUST NOT wrap query execution in a client-side timer
* *AND* the connection SHALL remain usable for subsequent statements after a server-reported timeout
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: No query timeout by default

* *GIVEN* a connection with no query timeout configured
* *WHEN* the connection is established and a query is executed via `Connection::execute_statement()`
* *THEN* the driver MUST NOT set the `queryTimeout` session attribute
* *AND* the driver MUST NOT wrap query execution in a client-side timer
* *AND* IF a prior statement on the same connection set the `queryTimeout` session attribute, the driver MUST reset it to `0` (unlimited) before executing so no stale server timeout is inherited
* *AND* the query SHALL run until the server returns a result or the server-side `QUERY_TIMEOUT` setting terminates it
<!-- /DELTA:NEW -->
