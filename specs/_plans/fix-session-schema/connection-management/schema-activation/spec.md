<!-- DELTA:CHANGED -->
# Feature: Schema Activation

Specifies how the session's current schema is set from the connection URI or `ConnectionParams` during connect, how a caller sets and reads it through the Rust API and the ADBC `adbc.connection.db_schema` connection option, and how the driver keeps the reported value in step with schema changes that SQL statements make.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

The current schema is an Exasol session attribute. The driver sets it with the protocol's set-attributes command and never with an `OPEN SCHEMA` statement, so the driver passes the schema name as a plain string and does not quote it. The server resolves the name in two steps: it opens the schema whose name matches exactly, and otherwise the schema whose name matches the upper-case form of the name. When neither schema exists, the server rejects the change with a `schema ... not found` error and the session keeps its previous current schema. The server reports the current schema in the response to a get-attributes request and in the response to any statement that changes it, such as `OPEN SCHEMA`, `CREATE SCHEMA`, `CLOSE SCHEMA`, or `DROP SCHEMA` of the current schema. An empty reported value means the session has no current schema. A schema named in the connection URI or in `ConnectionParams` must exist on the server, so a rejected schema fails the connect.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Schema in connection params is opened on connect

* *GIVEN* a `Database` configured with a connection string of the form `exasol://user:pass@host/SCHEMA_NAME`
* *AND* the schema `SCHEMA_NAME` exists on the server
* *WHEN* the application calls `Database::connect()` (or the equivalent ADBC FFI path)
* *THEN* the driver SHALL set `SCHEMA_NAME` as the session's current schema with the set-attributes command after the login and before returning the `Connection`, and MUST NOT send an `OPEN SCHEMA` statement
* *AND* the returned `Connection` MUST report `SCHEMA_NAME` from `current_schema()`
* *AND* a subsequent unqualified `SELECT * FROM TABLE_X` MUST resolve against `SCHEMA_NAME` without the caller invoking `set_schema()`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Schema activation failure surfaces during connect

* *GIVEN* a `Database` configured with a schema in the connection params
* *AND* the server rejects that schema, because the schema does not exist or for another reason such as missing privileges
* *WHEN* the application calls `Database::connect()` (or the equivalent ADBC FFI path)
* *THEN* `connect()` MUST return a `ConnectionError` whose message names the schema and contains the server's error message
* *AND* the underlying transport session MUST be closed before the error is returned so that no leaked session remains on the server
* *AND* the driver MUST fail the connect for every rejection and MUST NOT inspect the server's error text to decide whether the connect fails
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Non-existent URI schema is a best-effort default

* *GIVEN* a `Database` configured with a connection string of the form `exasol://user:pass@host/SCHEMA_NAME` where `SCHEMA_NAME` does not yet exist on the server
* *WHEN* the application calls `Database::connect()` (or the equivalent ADBC FFI path)
* *THEN* `connect()` MUST succeed and return an open `Connection` rather than returning an error
* *AND* the driver MUST swallow the "schema not found" failure from the implicit `OPEN SCHEMA` and leave the session with no active schema, so the returned `Connection` MUST NOT report `SCHEMA_NAME` from `current_schema()`
* *AND* a subsequent fully-qualified `SELECT * FROM SCHEMA_NAME.TABLE_X` (once the schema and table exist) MUST resolve normally, and the caller MAY activate `SCHEMA_NAME` later via `set_schema()` once it has been created
* *AND* the driver SHALL classify the failure as a missing-schema error by a message-based check (it inspects the server error text for a "not found" indication), which is a known limitation if the server changes its error wording
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: URI schema name follows the server's case rule

* *GIVEN* the server holds the schemas `"ZZ_MixedCase"`, `"zz-hyphen"`, `ZZ_UPPER`, `"zz_both"`, and `ZZ_BOTH`, and no schema named `zz_mixedcase` or `ZZ_MIXEDCASE`
* *WHEN* the application connects with the URI schema `ZZ_MixedCase`, `zz-hyphen`, or `zz_upper`
* *THEN* the session's current schema SHALL be `ZZ_MixedCase`, `zz-hyphen`, or `ZZ_UPPER` respectively
* *AND* `current_schema()` MUST report the name of the schema the server opened, not the name in the URI
* *AND* a connect with the URI schema `zz_both` SHALL open the schema `zz_both`, not `ZZ_BOTH`
* *AND* a connect with the URI schema `zz_mixedcase` MUST fail as "Schema activation failure surfaces during connect" describes
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Set the current schema at runtime

* *GIVEN* an open `Connection`
* *WHEN* the application calls `set_schema(name)`
* *THEN* the driver SHALL send `name` with the set-attributes command and MUST NOT send an `OPEN SCHEMA` statement
* *AND* after the server accepts the change, `current_schema()` MUST report the name of the schema the server opened
* *AND* when the server rejects the change, `set_schema()` MUST return an error that contains the server's error message, and `current_schema()` MUST keep reporting the previous current schema
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Current schema follows schema changes made in SQL

* *GIVEN* an open `Connection`
* *WHEN* a statement on that connection changes the current schema, such as `OPEN SCHEMA S`, `CREATE SCHEMA S`, `CLOSE SCHEMA` while a schema is open, or `DROP SCHEMA` of the current schema
* *THEN* `current_schema()` MUST report the current schema that the server reported in the response to that statement, without sending another request to the server
* *AND* after the current schema is closed or dropped, `current_schema()` MUST return `None`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: ADBC db_schema option sets the server's current schema

* *GIVEN* an ADBC connection created through a driver manager
* *WHEN* the application sets the `adbc.connection.db_schema` connection option to a schema name, at connection creation or later
* *THEN* the driver SHALL establish the session if it does not exist yet, and SHALL set the schema with the set-attributes command before the option call returns
* *AND* when the server accepts the change, the option call SHALL succeed and the session's current schema SHALL be the schema the server opened
* *AND* when the server rejects the change, the option call MUST fail with an error that contains the server's error message, and the session's current schema MUST stay unchanged
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: ADBC db_schema option reads the server's current schema

* *GIVEN* an ADBC connection whose session is established
* *WHEN* the application reads the `adbc.connection.db_schema` connection option
* *THEN* the driver SHALL request the current schema from the server with the get-attributes command and SHALL return the server's value
* *AND* the value SHALL reflect a schema set by the connection URI, by the option, or by SQL such as `OPEN SCHEMA` or `CREATE SCHEMA`
* *AND* when the session has no current schema, the read MUST fail with status `NOT_FOUND`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: ADBC db_schema option before the session exists

* *GIVEN* an ADBC connection whose session the driver has not established yet
* *WHEN* the application reads the `adbc.connection.db_schema` connection option
* *THEN* the driver MUST NOT connect to the server for the read
* *AND* the driver SHALL return the schema named in the connection URI
* *AND* when the connection URI names no schema, the read MUST fail with status `NOT_FOUND`
<!-- /DELTA:NEW -->
