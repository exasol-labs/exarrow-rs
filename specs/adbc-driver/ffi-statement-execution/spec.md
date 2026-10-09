# Feature: FFI Statement Execution

Specifies how the ADBC Statement of the FFI driver executes SQL with a bound RecordBatch: how many prepared-statement executions reach Exasol, what `execute` and `execute_update` return, what a failed batch leaves in the database, and which affected-row count a driver manager reads after `execute`.

## Background

A statement returns either an affected-row count, as INSERT, UPDATE, DELETE, and MERGE do, or a result set, as SELECT does. Exasol accepts a multi-row parameter set in one prepared-statement execution for a statement that returns an affected-row count. Exasol rejects a multi-row parameter set for a statement that returns a result set. A data message is the protocol message that carries the parameter values of one execution. Exasol reports a maximum data message size at login. Over the native protocol, a bound batch whose data message would exceed that size runs as consecutive prepared-statement executions, as `prepared-statements/batch-execution` specifies.

## Scenarios

### Scenario: execute_update sends every bound row in one execution

* *GIVEN* an ADBC statement whose SQL returns an affected-row count, such as a parameterized INSERT, and a RecordBatch of one or more rows bound via `bind()` whose parameter values fit in one data message
* *WHEN* `execute_update` is called
* *THEN* the driver SHALL prepare the statement if it is not prepared yet
* *AND* the driver SHALL send the parameter values of every bound row to Exasol in a single prepared-statement execution
* *AND* the driver SHALL return the total number of rows that the execution affected
* *AND* the statement SHALL take effect for every bound row

### Scenario: execute runs a row-count statement once for the whole bound batch

* *GIVEN* an ADBC statement whose SQL returns an affected-row count, such as a parameterized INSERT, and a RecordBatch of one or more rows bound via `bind()` whose parameter values fit in one data message
* *WHEN* `execute` is called
* *THEN* the driver SHALL send the parameter values of every bound row to Exasol in a single prepared-statement execution
* *AND* the driver SHALL return a RecordBatchReader that yields no batches
* *AND* the driver MUST NOT return an error because the execution returned an affected-row count instead of a result set
* *AND* the statement SHALL take effect for every bound row

### Scenario: execute runs a result-set statement once per bound row

* *GIVEN* an ADBC statement whose SQL returns a result set, such as a parameterized SELECT, and a RecordBatch of one or more rows bound via `bind()`
* *WHEN* `execute` is called
* *THEN* the driver SHALL prepare the statement if it is not prepared yet
* *AND* the driver SHALL run one prepared-statement execution per bound row, each with the parameter values of that row
* *AND* the driver MUST NOT send two or more bound rows to Exasol as one multi-row parameter set
* *AND* the driver SHALL return the result rows of all executions as Arrow RecordBatches in one RecordBatchReader, in the order of the bound rows

### Scenario: A failed bound batch stores none of its rows

* *GIVEN* an ADBC statement with a parameterized INSERT and a bound RecordBatch of several rows whose parameter values fit in one data message, one of which Exasol rejects, such as a string longer than its VARCHAR column
* *WHEN* `execute_update` or `execute` is called
* *THEN* the driver SHALL return an error that carries the Exasol error message
* *AND* the target table SHALL contain none of the bound rows

### Scenario: A bound batch larger than one data message is stored in full over the native protocol

* *GIVEN* an ADBC connection over the native protocol, a statement whose SQL returns an affected-row count, such as a parameterized INSERT, and a bound RecordBatch whose parameter values exceed the maximum data message size that Exasol reported at login
* *WHEN* `execute_update` or `execute` is called
* *THEN* the driver SHALL run the bound rows as consecutive prepared-statement executions, as `prepared-statements/batch-execution` specifies for a batch update larger than one data message
* *AND* `execute_update` SHALL return the total number of rows that the executions affected, and `execute` SHALL return a RecordBatchReader that yields no batches
* *AND* the target table SHALL contain every bound row
* *AND* the connection SHALL remain usable for later statements

### Scenario: A failed execution of a split bound batch keeps the rows of the earlier executions

* *GIVEN* an ADBC connection over the native protocol with autocommit on, a statement with a parameterized INSERT, and a bound RecordBatch whose parameter values exceed the maximum data message size, in which Exasol rejects one row that lies after the rows of the first execution
* *WHEN* `execute_update` is called
* *THEN* the driver SHALL return an error that carries the Exasol error message
* *AND* the driver MUST NOT run an execution for the rows that follow the rows of the failing execution
* *AND* the target table SHALL contain the rows of the executions that completed before the failing execution, and no other bound row
* *AND* the connection SHALL remain usable for later statements

### Scenario: A bound value that cannot be converted fails the batch before execution

* *GIVEN* an ADBC statement with a parameterized INSERT and a bound RecordBatch of several rows, one of which holds a value that cannot be converted to an Exasol parameter, such as the Date32 value 2932897, which lies after 9999-12-31
* *WHEN* `execute_update` or `execute` is called
* *THEN* the driver SHALL return an error with status `InvalidArguments`
* *AND* the driver MUST NOT send an execution request to Exasol
* *AND* the target table SHALL contain none of the bound rows

### Scenario: A bound batch with the wrong column count fails before execution

* *GIVEN* an ADBC statement whose SQL has N parameter markers, such as a parameterized INSERT, and a bound RecordBatch of one or more rows whose column count differs from N and whose values all convert to Exasol parameters
* *WHEN* `execute_update` or `execute` is called
* *THEN* the driver SHALL return an error with status `InvalidArguments` whose message states N as the parameter count that the statement expects
* *AND* the driver MUST NOT send an execution request to Exasol
* *AND* the target table SHALL contain none of the bound rows

### Scenario: A zero-row bound batch runs no execution

* *GIVEN* an ADBC statement with SQL set and a RecordBatch of zero rows bound via `bind()`
* *WHEN* `execute_update` or `execute` is called
* *THEN* the driver MUST NOT send an execution request to Exasol
* *AND* `execute_update` SHALL report 0 affected rows
* *AND* `execute` SHALL return a RecordBatchReader that yields no batches

### Scenario: ExecuteQuery with a result stream reports an unknown affected-row count

* *GIVEN* an ADBC driver manager calls `AdbcStatementExecuteQuery` with a result stream and a `rows_affected` out-parameter that holds 42
* *WHEN* the statement executes successfully
* *THEN* the driver SHALL write -1, meaning unknown, to `rows_affected`
