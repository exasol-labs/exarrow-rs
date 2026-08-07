# Feature: Binding and Execution

Specifies the native prepared statement protocol, type-safe parameter binding, and execution behavior for Exasol, enabling secure and efficient parameterized query execution with SQL injection prevention by protocol design.

## Background

The system implements Exasol's native prepared statement protocol for secure parameterized query execution. Parameters are sent separately from SQL text via the wire protocol, preventing SQL injection by design. Parameter binding is positional (1-indexed) and type-safe, with validation against expected parameter types. Prepared statements can be reused across multiple executions with different parameter values without re-parsing SQL.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Prepared statement creation

* *GIVEN* a connection to Exasol is established
* *WHEN* a SQL statement with parameters is prepared
* *THEN* it SHALL send a createPreparedStatement request to Exasol over the active transport
* *AND* it SHALL receive a statement handle, parameter metadata, and result-set column metadata from that single response
* *AND* it SHALL store the parameter type information and the result-set column metadata on the prepared statement handle
* *AND* it SHALL NOT issue an additional round-trip, catalog lookup, or probe query to obtain the result-set column metadata
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Result-set column metadata for a parameterized SELECT

* *GIVEN* a table `T` exists with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(50)`
* *WHEN* preparing `SELECT ID, NAME FROM T WHERE ID = ? AND NAME = ?`
* *THEN* the prepared statement handle SHALL report two parameters and two result-set columns in select-list order
* *AND* the result-set columns SHALL be `ID` with Exasol type `DECIMAL(18,0)` followed by `NAME` with Exasol type `VARCHAR(50)`
* *AND* the reported result-set columns MUST be identical over the native TCP transport and the WebSocket transport for the identical SQL text
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Result-set column metadata for a derived select list

* *GIVEN* a table `T` exists with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(50)`
* *WHEN* preparing `SELECT ID*2 AS DOUBLED, UPPER(NAME) FROM T`
* *THEN* the prepared statement handle SHALL report zero parameters and two result-set columns
* *AND* the aliased column SHALL be named `DOUBLED`
* *AND* the unaliased derived column SHALL carry the name Exasol assigned it rather than an empty name or a positional placeholder
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Row-count-producing statements report no result-set columns

* *GIVEN* a connection to Exasol is established
* *WHEN* preparing a statement that Exasol classifies as row-count-producing rather than result-set-producing, such as `INSERT INTO T VALUES (?,?)` or `DELETE FROM T WHERE ID = ?`
* *THEN* preparation SHALL succeed and the prepared statement handle SHALL report the statement's parameters
* *AND* the prepared statement handle SHALL report zero result-set columns
* *AND* the system SHALL NOT report an error for the absent result-set description, because Exasol classifies every statement other than `SELECT` and `DESCRIBE` as row-count-producing, computes no result-set description for it, and therefore sends none
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Non-ASCII VARCHAR parameter round-trip

* *GIVEN* a connection to Exasol is established over the native TCP transport
* *AND* a table `T` exists with a column `NAME VARCHAR(50)`
* *WHEN* binding a parameter value containing non-ASCII UTF-8 characters and executing the prepared statement
* *THEN* Exasol SHALL accept the parameter data
* *AND* selecting the stored value back SHALL return the bound string unchanged, byte for byte
<!-- /DELTA:NEW -->
