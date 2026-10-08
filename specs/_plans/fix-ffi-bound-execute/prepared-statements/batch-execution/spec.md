# Feature: Batch Execution

Specifies multi-row batch execution of prepared statements, including column-major parameter assembly, batch update and query methods, arity validation, and edge-case behavior for empty batches and closed statements.

<!-- DELTA:CHANGED -->
## Background

A batch of rows (each row a list of positional parameters) is assembled into column-major parameter data and executed as one prepared-statement call, which avoids one round trip per row for bulk DML. Exasol accepts a multi-row parameter set only for a statement that returns an affected-row count. For a statement that returns a result set, Exasol rejects a parameter set of two or more rows. A data message is the protocol message that carries the parameter values of one execution. Exasol reports a maximum data message size at login. Over the native protocol, the transport runs a batch update whose data message would exceed that size as consecutive executions.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Batch update execution with affected row count

* *GIVEN* a prepared INSERT/UPDATE/DELETE statement and multiple rows of bound parameter values whose data message fits the maximum data message size that Exasol reported at login
* *WHEN* the batch is executed for an affected-row-count result
* *THEN* the system SHALL send all rows to Exasol in a single prepared-statement execution
* *AND* the system SHALL return the total number of affected rows
* *AND* the system SHALL reuse the server-side prepared statement without re-parsing the SQL
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Batch update larger than one data message over the native protocol

* *GIVEN* a connection over the native protocol, a prepared statement that returns an affected-row count, and multiple rows of bound parameter values whose data message would exceed the maximum data message size that Exasol reported at login
* *WHEN* the batch is executed for an affected-row-count result
* *THEN* the system SHALL split the rows into consecutive ranges in input row order and run one prepared-statement execution per range
* *AND* the data message of each execution MUST NOT exceed the maximum data message size, unless its range holds a single row
* *AND* the system SHALL return the sum of the affected-row counts of all executions, or, when an execution fails, return its error and run no later range
* *AND* the connection SHALL remain usable after the batch
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Batch query execution returning a result set

* *GIVEN* a prepared statement that returns a result set and one row of bound parameter values
* *WHEN* the batch is executed for a result set
* *THEN* the system SHALL send the row to Exasol in a single prepared-statement execution
* *AND* the system SHALL return the result set in Arrow form
<!-- /DELTA:CHANGED -->
