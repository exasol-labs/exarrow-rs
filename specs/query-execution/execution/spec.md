# Feature: Execution

Specifies SQL query execution capabilities including direct queries, prepared statements, batch execution, and query cancellation through connection-owned transport.

## Background

All SQL execution occurs through Connection-owned transport via the WebSocket protocol. The system supports direct query execution (SELECT, DDL, DML), prepared statements with parameter binding, batch execution of multiple queries, and cancellation of in-flight queries. Results are returned as Arrow RecordBatch. Execution goes through Connection methods such as `execute_statement()`, `execute_statement_update()`, `prepare()`, and `execute_prepared()`.

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

### Scenario: Sequential query execution

* *GIVEN* multiple queries are ready for execution
* *WHEN* multiple queries are submitted for execution
* *THEN* it SHALL execute them in order
* *AND* it SHALL return results for each query separately
* *AND* it SHALL stop on first error if specified

### Scenario: Independent query execution

* *GIVEN* multiple queries are ready for execution
* *WHEN* queries are marked as independent
* *THEN* it SHALL execute all queries regardless of individual failures
* *AND* it SHALL collect results and errors for each query

### Scenario: Cancel running query

* *GIVEN* a long-running query is executing
* *WHEN* a query is cancelled during execution
* *THEN* it SHALL send a cancellation request to Exasol
* *AND* it SHALL wait for cancellation acknowledgment or timeout
* *AND* it SHALL return a cancellation error

### Scenario: Cancel with timeout

* *GIVEN* a long-running query is executing
* *WHEN* cancellation takes longer than timeout
* *THEN* it SHALL forcefully abort the local query execution
* *AND* it SHALL close the connection if necessary

### Scenario: Literal question mark inside a single-quoted string

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement whose SQL text contains `?` inside a single-quoted string literal (for example `SELECT 'a?b' AS v`) and no parameters are bound
* *THEN* the system MUST send the SQL unmodified to Exasol
* *AND* the system MUST NOT raise a parameter-binding error for the embedded `?`
* *AND* the system SHALL return the literal value `a?b` in the result set

### Scenario: Escaped single quote inside a string literal containing a question mark

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement whose SQL text contains the SQL standard `''` escape inside a single-quoted string literal that also contains `?` (for example `SELECT 'It''s ?' AS v`)
* *THEN* the system MUST treat the doubled `'` as part of the string literal
* *AND* the system MUST NOT treat the `?` as a positional placeholder
* *AND* the system SHALL send the SQL unmodified to Exasol

### Scenario: Question mark inside a line comment

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement that contains a line comment with `?` (for example `SELECT 1 -- is this ?\n`)
* *THEN* the system MUST treat the entire comment, including `?`, as non-placeholder text up to the next newline
* *AND* the system MUST NOT raise a parameter-binding error

### Scenario: Question mark inside a block comment

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement that contains a block comment with `?` (for example `SELECT /* what? */ 1`)
* *THEN* the system MUST treat the entire `/* ... */` region as non-placeholder text
* *AND* the system MUST NOT raise a parameter-binding error

### Scenario: Question mark inside a double-quoted identifier

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement that contains a `?` inside a double-quoted identifier (for example `SELECT 1 AS "col?name"`)
* *THEN* the system MUST treat the `?` as part of the identifier
* *AND* the system MUST NOT treat the `?` as a positional placeholder

### Scenario: Mixed placeholders and literal question marks

* *GIVEN* an authenticated connection exists to Exasol
* *AND* a statement with SQL `SELECT 'a?b' AS lit, ? AS bound` and a single parameter bound to integer `42`
* *WHEN* the statement is executed
* *THEN* the system SHALL substitute only the unquoted `?` with the bound value
* *AND* the system MUST leave the `?` inside the string literal intact
* *AND* the result SHALL contain the literal `a?b` and the value `42`
