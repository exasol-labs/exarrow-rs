# Feature: Session and Lifecycle

Specifies session management, connection pooling foundation, and timeout configuration for Exasol database connections.

<!-- DELTA:CHANGED -->
## Background

The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection and query operations. The connection timeout, which bounds opening a connection, is specified in `connection-management/connection-timeout`. The driver has no idle timeout and never closes a connection because the connection is idle, so the `idle_timeout` connection-string parameter is deprecated and has no effect. The query-execution timeout SHALL be a server-enforced session setting and default to unset: when a caller configures a query timeout, the driver SHALL forward it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle, and the driver SHALL NOT wrap query execution through `Connection::execute_statement()` in a client-side timer. A client-side bound on a CSV export is a separate, export-scoped setting specified in `import-export/csv-export-timeout`. Absent an explicit query timeout, the driver SHALL set no `queryTimeout` attribute and the server-side `QUERY_TIMEOUT` setting SHALL govern. If the driver ever gives up on a running query client-side, it MUST terminate the connection rather than abandon the in-flight request and leave the session open server-side. Default-schema activation on connect is specified in the `connection-management/schema-activation` feature. The client-side session states and the rules for operations in each state are specified in `connection-management/session-state`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Idle timeout

* *GIVEN* the connection URI `exasol://alice:Secret1@db.example.com:8563?idle_timeout=120`
* *WHEN* the URI is parsed into connection parameters
* *THEN* parsing SHALL succeed
* *AND* the driver SHALL emit a warning through the `log` crate at level `Warn` that names `idle_timeout` and states that the parameter is deprecated and has no effect
* *AND* the warning MUST NOT contain the value `120`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Idle timeout with an invalid value

* *GIVEN* the connection URI `exasol://alice:Secret1@db.example.com:8563?idle_timeout=Qx7`
* *WHEN* the URI is parsed into connection parameters
* *THEN* parsing SHALL fail with `ConnectionError::InvalidParameter` whose parameter is `idle_timeout`
* *AND* the error message MUST NOT contain `Qx7`
<!-- /DELTA:NEW -->
