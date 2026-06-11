# Feature: Session and Lifecycle

Specifies session management, connection pooling foundation, and timeout configuration for Exasol database connections.

## Background

The system SHALL manage database session lifecycle from establishment through termination. Connections SHALL support clean state reset to enable future pooling implementations. Configurable timeouts SHALL govern connection, query, and idle operations with sensible defaults. When the connection URI or `ConnectionParams` carries a schema, the driver SHALL treat that schema as a best-effort default: it SHALL attempt to activate the schema server-side during `connect()` so unqualified statements resolve immediately, but a schema that does not yet exist SHALL NOT abort the connection. This matches tools such as dbt, which connect first and create their target schema afterwards while fully qualifying every relation, so a not-yet-existing default schema is a normal state rather than a fatal error.

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

* *GIVEN* timeout settings are configured
* *WHEN* executing a query
* *THEN* it SHALL support optional query timeout configuration
* *AND* it SHALL cancel queries that exceed the timeout

### Scenario: Idle timeout

* *GIVEN* timeout settings are configured
* *WHEN* a connection is idle
* *THEN* it SHALL support optional idle timeout configuration
* *AND* it SHALL close connections that exceed idle timeout

### Scenario: Schema in connection params is opened on connect

* *GIVEN* a `Database` configured with a connection string of the form `exasol://user:pass@host/SCHEMA_NAME`
* *WHEN* the application calls `Database::connect()` (or the equivalent ADBC FFI path)
* *THEN* the driver SHALL execute `OPEN SCHEMA SCHEMA_NAME` against the established session before returning the `Connection`
* *AND* the returned `Connection` MUST report `SCHEMA_NAME` from `current_schema()`
* *AND* a subsequent unqualified `SELECT * FROM TABLE_X` MUST resolve against `SCHEMA_NAME` without the caller invoking `set_schema()`

### Scenario: Schema activation failure surfaces during connect

* *GIVEN* a `Database` configured with a schema in the connection params whose activation fails for a reason OTHER than the schema not existing (for example insufficient privileges)
* *WHEN* the application calls `Database::connect()`
* *THEN* `connect()` MUST return a `ConnectionError` whose source identifies the schema activation failure
* *AND* the underlying transport session MUST be closed before the error is returned so that no leaked session remains on the server

### Scenario: Non-existent URI schema is a best-effort default

* *GIVEN* a `Database` configured with a connection string of the form `exasol://user:pass@host/SCHEMA_NAME` where `SCHEMA_NAME` does not yet exist on the server
* *WHEN* the application calls `Database::connect()` (or the equivalent ADBC FFI path)
* *THEN* `connect()` MUST succeed and return an open `Connection` rather than returning an error
* *AND* the driver MUST swallow the "schema not found" failure from the implicit `OPEN SCHEMA` and leave the session with no active schema, so the returned `Connection` MUST NOT report `SCHEMA_NAME` from `current_schema()`
* *AND* a subsequent fully-qualified `SELECT * FROM SCHEMA_NAME.TABLE_X` (once the schema and table exist) MUST resolve normally, and the caller MAY activate `SCHEMA_NAME` later via `set_schema()` once it has been created
* *AND* the driver SHALL classify the failure as a missing-schema error by a message-based check (it inspects the server error text for a "not found" indication), which is a known limitation if the server changes its error wording
