# Feature: Schema Activation

Specifies how a default schema supplied via the connection URI or `ConnectionParams` is activated server-side on connect, and how activation failures are handled.

## Background

When the connection URI or `ConnectionParams` carries a schema, the driver SHALL treat that schema as a best-effort default: it SHALL attempt to activate the schema server-side during `connect()` so unqualified statements resolve immediately, but a schema that does not yet exist SHALL NOT abort the connection. This matches tools such as dbt, which connect first and create their target schema afterwards while fully qualifying every relation, so a not-yet-existing default schema is a normal state rather than a fatal error.

## Scenarios

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
