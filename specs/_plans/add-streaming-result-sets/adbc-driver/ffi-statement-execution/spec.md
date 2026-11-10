<!-- DELTA:CHANGED -->
# Feature: FFI Statement Execution

Specifies how the FFI driver's ADBC Statement executes SQL, with or without a bound RecordBatch: how many prepared-statement executions reach Exasol, what `execute` and `execute_update` return, how the RecordBatchReader that `execute` returns for SQL without a bound RecordBatch reads its batches, what a failed batch leaves in the database, and which affected-row count a driver manager reads after `execute`.
<!-- /DELTA:CHANGED -->

## Background

A statement returns either an affected-row count, as INSERT, UPDATE, DELETE, and MERGE do, or a result set, as SELECT does. Exasol accepts a multi-row parameter set in one prepared-statement execution for a statement that returns an affected-row count, and rejects it for a statement that returns a result set. A data message is the protocol message that carries the parameter values of one execution. Exasol reports a maximum data message size at login. Over the native protocol, a bound batch whose data message would exceed that size runs as consecutive prepared-statement executions, as `prepared-statements/batch-execution` specifies.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: execute streams a query result one batch per read

* *GIVEN* an ADBC statement whose SQL returns a result set, such as a SELECT, with no bound RecordBatch, and a result whose rows need more than one fetch request
* *WHEN* `execute` is called and the caller reads the returned RecordBatchReader until it reports no further batch
* *THEN* `execute` SHALL return the RecordBatchReader without sending a fetch request
* *AND* each read from the reader SHALL send at most one fetch request and SHALL return one batch or report the end of the result
* *AND* the reader SHALL NOT keep a reference to a column array of any batch that it has returned
* *AND* the reader SHALL return every row of the result exactly once
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A streaming reader closes the result set handle at its end

* *GIVEN* an ADBC statement whose SQL returns a result set with no bound RecordBatch, and a result with a result set handle
* *WHEN* the caller reads the RecordBatchReader that `execute` returned until it reports no further batch
* *THEN* the driver SHALL close the result set handle on the server
* *AND* releasing the reader afterwards SHALL NOT send a request
* *AND* the connection SHALL remain usable for later statements
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Releasing a streaming reader before its end closes the result set handle

* *GIVEN* an ADBC statement whose SQL returns a result set with no bound RecordBatch, and a result whose rows need more than one fetch request
* *AND* the caller has read some, but not all, batches from the RecordBatchReader that `execute` returned
* *WHEN* the caller releases the reader
* *THEN* the driver SHALL close the result set handle on the server before the release returns
* *AND* the driver SHALL NOT send a fetch request for the remaining rows
* *AND* the connection SHALL remain usable for later statements
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A failed fetch fails the read of a streaming reader

* *GIVEN* an ADBC statement whose SQL returns a result set with no bound RecordBatch, and a result whose rows need more than one fetch request
* *AND* `execute` has returned a RecordBatchReader
* *WHEN* the fetch request of a read fails, or the rows received fall short of or exceed the total row count of the result
* *THEN* that read SHALL return an error whose message carries the message of the failure
* *AND* the read SHALL NOT return a batch
<!-- /DELTA:NEW -->
