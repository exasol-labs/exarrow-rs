# Feature: Binding and Execution

Specifies prepared statement creation over the active transport and execution, once or repeatedly.

## Background

Parameters travel separately from the SQL text. A prepared statement is reused across executions without re-parsing the SQL.

## Scenarios

### Scenario: Prepared statement creation

* *GIVEN* a connection to Exasol is established
* *WHEN* a SQL statement with parameters is prepared
* *THEN* it SHALL send a createPreparedStatement request to Exasol over the active transport
* *AND* it SHALL receive a statement handle, parameter metadata, and result-set column metadata from that single response
* *AND* it SHALL store the parameter type information and the result-set column metadata on the prepared statement handle
* *AND* it SHALL NOT issue an additional round-trip, catalog lookup, or probe query to obtain the result-set column metadata

### Scenario: Re-execution with different parameters

* *GIVEN* a prepared statement exists with bound parameters
* *WHEN* executing a prepared statement multiple times with different parameter values
* *THEN* it SHALL reuse the server-side prepared statement
* *AND* it SHALL allow re-binding parameters between executions
* *AND* it SHALL avoid re-parsing the SQL statement

### Scenario: Prepared statement with no parameters

* *GIVEN* a connection to Exasol is established
* *WHEN* preparing and executing a statement with no parameters
* *THEN* it SHALL handle the statement through the prepared statement protocol
* *AND* it SHALL NOT require parameter binding
