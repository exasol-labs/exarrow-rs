# Feature: Session and Lifecycle

Specifies session management, connection pooling foundation, and timeout configuration for Exasol database connections.

<!-- DELTA:CHANGED -->
## Background

The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection, query, and idle operations. The connection timeout, which bounds opening a connection, is specified in `connection-management/connection-timeout`. The idle timeout SHALL apply a sensible non-zero default. The query-execution timeout SHALL be a server-enforced session setting and default to unset: when a caller configures a query timeout, the driver SHALL forward it to Exasol as the `queryTimeout` session attribute so the server aborts an over-running query and reports the abort through the normal response cycle, and the driver SHALL NOT wrap query execution through `Connection::execute_statement()` in a client-side timer. A client-side bound on a CSV export is a separate, export-scoped setting specified in `import-export/csv-export-timeout`. Absent an explicit query timeout, the driver SHALL set no `queryTimeout` attribute and the server-side `QUERY_TIMEOUT` setting SHALL govern. If the driver ever gives up on a running query client-side, it MUST terminate the connection rather than abandon the in-flight request and leave the session open server-side. Default-schema activation on connect is specified in the `connection-management/schema-activation` feature. The client-side session state SHALL be one of `Ready`, `InTransaction`, `Closing`, or `Closed`. A statement execution, a prepare, a prepared-statement execution, or a transaction start SHALL begin only in `Ready` or `InTransaction`, and the driver SHALL keep the session in that state while the operation runs, so that no exit path of an operation, including an error or an abandoned execution, leaves a state that blocks the next operation.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Session attributes

* *GIVEN* a session is active with Exasol
* *WHEN* session attributes are requested
* *THEN* it SHALL provide current schema, session ID, and other metadata
* *AND* it SHALL allow setting session attributes (e.g., current schema)
* *AND* when a schema is supplied via the connection URI or `ConnectionParams.schema`, it SHALL apply that schema server-side during `connect()` so that subsequent statements resolve unqualified identifiers against it without an additional client call
* *AND* if the server rejects that schema for any reason, including a schema that does not exist, `connect()` MUST return a `ConnectionError` and MUST NOT leave a half-open connection visible to the caller
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A failed statement leaves the session usable

* *GIVEN* an open connection with no active transaction
* *WHEN* a statement or prepared-statement execution on that connection fails, because the server reports an error, a parameter value cannot be converted, or the server rejects the `queryTimeout` attribute
* *THEN* the session state SHALL be `Ready`
* *AND* the next statement on the connection SHALL execute
* *AND* `begin_transaction()` and setting the ADBC `AutoCommit` connection option to `false` SHALL succeed and MUST NOT report the connection as closed
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A failed statement inside a transaction keeps the transaction

* *GIVEN* an open connection with an active transaction
* *WHEN* a statement or prepared-statement execution on that connection fails
* *THEN* the session state SHALL be `InTransaction`
* *AND* `commit()` and `rollback()` SHALL end the transaction and return the session to `Ready`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: An abandoned execution leaves the session state unchanged

* *GIVEN* an open connection in state `Ready` or `InTransaction`
* *WHEN* the caller drops a statement execution before it completes
* *THEN* the session state SHALL be the state the session had before the execution started
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A closed session rejects operations as closed

* *GIVEN* a connection whose session state is `Closing` or `Closed`
* *WHEN* the caller starts a statement execution, a prepare, a prepared-statement execution, or a transaction
* *THEN* the operation MUST fail with an error that reports the connection as closed
* *AND* the driver MUST NOT send any request for the operation to the server
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Starting a second transaction reports the active transaction

* *GIVEN* an open connection with an active transaction
* *WHEN* the caller calls `begin_transaction()` again
* *THEN* the call MUST fail with an error that states a transaction is already active
* *AND* the error MUST NOT report the connection as closed
* *AND* the driver MUST NOT send a request to the server for the rejected call
<!-- /DELTA:NEW -->
