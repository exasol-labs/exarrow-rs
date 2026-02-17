# Feature: Statement and Results

Specifies statement execution, session identity, and result set handling for the ADBC driver, ensuring queries execute through connection-mediated transport and return Arrow-formatted results.

## Background

Statements are pure data objects created via `Connection::create_statement()` and executed via `Connection::execute_statement()`. Statements do not hold transport references; all execution goes through the Connection. Multiple statements created from the same connection share a single database session. Query results are returned in Arrow format per the ADBC specification.

## Scenarios

### Scenario: Query preparation

* *GIVEN* an active database connection exists
* *WHEN* a SQL query is prepared via `Connection::create_statement()`
* *THEN* it SHALL return a Statement containing SQL text and parameters
* *AND* the Statement SHALL NOT hold transport references

### Scenario: Query execution

* *GIVEN* an active database connection exists
* *WHEN* a Statement is executed via `Connection::execute_statement()`
* *THEN* it SHALL send the query via WebSocket protocol
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
* *THEN* it SHALL use the connection's existing WebSocket transport
* *AND* it SHALL NOT open a new WebSocket connection
* *AND* it SHALL NOT perform a new TLS handshake or authentication

### Scenario: Arrow RecordBatch results

* *GIVEN* an active database connection exists
* *WHEN* a query returns data
* *THEN* results SHALL be provided as Arrow RecordBatch
* *AND* the schema SHALL accurately reflect Exasol column types

### Scenario: Empty result sets

* *GIVEN* an active database connection exists
* *WHEN* a query returns no rows
* *THEN* it SHALL return an empty RecordBatch with correct schema

### Scenario: Result set metadata

* *GIVEN* an active database connection exists
* *WHEN* result metadata is requested
* *THEN* it SHALL provide row count, column count, and schema information
