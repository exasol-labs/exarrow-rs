# Feature: Batch Execution

Specifies multi-row batch execution of prepared statements, including column-major parameter assembly, batch update and query methods, arity validation, and edge-case behavior for empty batches and closed statements.

## Background

A batch of rows (each row a list of positional parameters) can be assembled into column-major wire data and executed in a single prepared-statement call. This enables efficient bulk DML and parameterized queries without per-row round trips. The transport layer accepts N-row column-major parameter data natively; batch execution is therefore a pure API-surface addition over the single-row execution path.

## Scenarios

### Scenario: Batch parameter data assembly in column-major form

* *GIVEN* a prepared statement with a known parameter count
* *WHEN* batch parameter data is built from a slice of rows, where each row is a list of `Parameter` values
* *THEN* the system SHALL produce one column per parameter position
* *AND* each column SHALL contain one wire-protocol value per input row, in input row order
* *AND* the value at column `c` row `r` SHALL equal the wire-protocol conversion of the parameter at position `c` in input row `r`
* *AND* binary parameters SHALL be hex-encoded identically to single-row execution

### Scenario: Batch update execution with affected row count

* *GIVEN* a prepared INSERT/UPDATE/DELETE statement and multiple rows of bound parameter values
* *WHEN* the batch is executed for an affected-row-count result
* *THEN* the system SHALL send all rows to Exasol in a single prepared-statement execution
* *AND* the system SHALL return the total number of affected rows
* *AND* the system SHALL reuse the server-side prepared statement without re-parsing the SQL

### Scenario: Batch query execution returning a result set

* *GIVEN* a prepared statement and multiple rows of bound parameter values
* *WHEN* the batch is executed for a result set
* *THEN* the system SHALL send all rows to Exasol in a single prepared-statement execution
* *AND* the system SHALL return the result set in Arrow form

### Scenario: Batch execution with parameter arity mismatch

* *GIVEN* a prepared statement with a known parameter count
* *WHEN* a batch is supplied in which any row's parameter count differs from the statement's parameter count
* *THEN* the system MUST reject the batch with a parameter binding error
* *AND* the system MUST NOT send any data to Exasol

### Scenario: Empty batch execution

* *GIVEN* a prepared statement
* *WHEN* a batch is supplied with zero rows
* *THEN* the system SHALL treat the batch as carrying no parameter data
* *AND* the system SHALL NOT report a parameter binding error for the empty batch

### Scenario: Batch execution on a closed statement

* *GIVEN* a prepared statement that has been closed
* *WHEN* a batch execution is attempted on it
* *THEN* the system MUST reject the execution with a statement-closed error

### Scenario: Batch update rejects a result-set response

* *GIVEN* a prepared statement executed as a batch update
* *WHEN* Exasol returns a result set instead of an affected-row count
* *THEN* the system MUST reject the outcome with an unexpected-result-set error
