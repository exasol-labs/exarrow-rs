# Feature: Statement and Results

Specifies statement execution, session identity, and result handling for the ADBC driver. Queries run through the Connection and return Arrow results.

## Background

A Statement is pure data and holds no transport reference. Statements created from one connection share a single database session.

## Scenarios

### Scenario: Query preparation

* *GIVEN* an active database connection exists
* *WHEN* a SQL query is prepared via `Connection::create_statement()`
* *THEN* it SHALL return a Statement containing SQL text and parameters
* *AND* the Statement SHALL NOT hold transport references

### Scenario: Query execution

* *GIVEN* an active database connection exists
* *WHEN* a Statement is executed via `Connection::execute_statement()`
* *THEN* it SHALL send the query over the connection's transport
* *AND* it SHALL return results as Arrow RecordBatch
* *AND* it SHALL include result metadata (schema, row count)

### Scenario: Parameterized queries

* *GIVEN* an active database connection exists
* *WHEN* a Statement with parameters is executed via `Connection::execute_statement()`
* *THEN* it SHALL bind parameters safely to prevent SQL injection
* *AND* it SHALL support Exasol's parameter binding syntax

### Scenario: Session reuse across statements

* *GIVEN* two statements are created from the same connection
* *WHEN* two statements are created from the same connection
* *AND* each executes `SELECT CURRENT_SESSION`
* *THEN* both SHALL return the same Exasol session identifier

### Scenario: No connection-per-statement

* *GIVEN* an active database connection exists
* *WHEN* a statement is executed
* *THEN* it SHALL use the connection's existing transport
* *AND* it SHALL NOT open a new transport connection
* *AND* it SHALL NOT perform a new TLS handshake or authentication

### Scenario: Arrow RecordBatch results

* *GIVEN* an active database connection exists
* *WHEN* a query returns data
* *THEN* results SHALL be provided as Arrow RecordBatch
* *AND* the schema SHALL accurately reflect Exasol column types

### Scenario: Empty result sets

* *GIVEN* an active database connection exists
* *WHEN* a query returns no rows
* *THEN* it SHALL return exactly one Arrow RecordBatch with zero rows
* *AND* that RecordBatch SHALL carry the correct column schema (column names and Arrow data types)

### Scenario: Result set metadata

* *GIVEN* an active database connection exists
* *WHEN* result metadata is requested
* *THEN* it SHALL provide row count, column count, and schema information

### Scenario: Statement inherits connection query timeout

* *GIVEN* a connection carrying a query timeout setting, which is either an explicit timeout or no timeout
* *WHEN* `create_statement()` is called on that connection
* *THEN* the returned Statement SHALL inherit that setting: the explicit timeout when the connection configures one, or no timeout when the connection configures none
* *AND* the Statement SHALL NOT fall back to any hardcoded non-zero timeout independent of the connection's configuration
* *AND* when the Statement carries an explicit timeout, `execute_statement()` SHALL forward that timeout to Exasol as the `queryTimeout` session attribute before executing, rather than enforce it with a client-side timer
