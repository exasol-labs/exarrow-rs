# Feature: Execution

Specifies SQL query execution capabilities including direct queries, prepared statements and batch execution through connection-owned transport.

## Background

All SQL execution occurs through Connection-owned transport via the WebSocket protocol. The system supports direct query execution (SELECT, DDL, DML), prepared statements with parameter binding, and batch execution of multiple queries. Results are returned as Arrow RecordBatch. Execution goes through Connection methods such as `execute_statement()`, `execute_statement_update()`, `prepare()`, and `execute_prepared()`.

## Scenarios

### Scenario: Simple SELECT query

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a simple SELECT statement via `Connection::execute_statement()`
* *THEN* it SHALL send the query to Exasol via WebSocket
* *AND* it SHALL retrieve the complete result set
* *AND* it SHALL return results as Arrow RecordBatch

### Scenario: DDL statement execution

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing DDL statements (CREATE, ALTER, DROP) via `Connection::execute_statement()`
* *THEN* it SHALL execute the statement
* *AND* it SHALL return success or error status
* *AND* it SHALL return affected object information

### Scenario: DML statement execution

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing DML statements (INSERT, UPDATE, DELETE) via `Connection::execute_statement_update()`
* *THEN* it SHALL execute the statement
* *AND* it SHALL return the number of affected rows

### Scenario: Statement preparation

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* preparing a SQL statement via `Connection::prepare()`
* *THEN* it SHALL validate the SQL syntax
* *AND* it SHALL identify parameter placeholders
* *AND* it SHALL return a PreparedStatement handle

### Scenario: Parameter binding

* *GIVEN* a prepared statement has been created
* *WHEN* binding parameters to a `PreparedStatement`
* *THEN* it SHALL validate parameter types against expected types
* *AND* it SHALL convert Rust types to Exasol-compatible values
* *AND* it SHALL prevent SQL injection through proper escaping
* *AND* it MUST treat `?` characters inside single-quoted string literals, double-quoted identifiers, line comments (`-- ...`), and block comments (`/* ... */`) as literal text rather than positional placeholders

### Scenario: Prepared statement execution

* *GIVEN* a prepared statement has been created
* *WHEN* executing a PreparedStatement via `Connection::execute_prepared()`
* *THEN* it SHALL substitute parameters safely
* *AND* it SHALL execute the query
* *AND* it SHALL return results in Arrow format
