# Feature: FFI Streaming Reader

Specifies how the RecordBatchReader that the FFI driver's `execute` returns for SQL without a bound RecordBatch reads its batches and releases the result set handle.

## Background

A statement that returns a result set and has no bound RecordBatch returns a streaming reader. Each read of the reader blocks the calling thread for one fetch of the result set, so the reader keeps no batch that it has yielded. `adbc-driver/ffi-statement-execution` specifies the executions that reach Exasol and the affected-row counts.

## Scenarios

### Scenario: execute streams a query result one batch per read

* *GIVEN* an ADBC statement whose SQL returns a result set, such as a SELECT, with no bound RecordBatch, and a result whose rows need more than one fetch request
* *WHEN* `execute` is called and the caller reads the returned RecordBatchReader until it reports no further batch
* *THEN* `execute` SHALL return the RecordBatchReader without sending a fetch request
* *AND* each read from the reader SHALL send at most one fetch request and SHALL return one batch or report the end of the result
* *AND* the reader SHALL NOT keep a reference to a column array of any batch that it has returned
* *AND* the reader SHALL return every row of the result exactly once

### Scenario: A streaming reader closes the result set handle at its end

* *GIVEN* an ADBC statement whose SQL returns a result set with no bound RecordBatch, and a result with a result set handle
* *WHEN* the caller reads the RecordBatchReader that `execute` returned until it reports no further batch
* *THEN* the driver SHALL close the result set handle on the server
* *AND* releasing the reader afterwards SHALL NOT send a request
* *AND* the connection SHALL remain usable for later statements

### Scenario: Releasing a streaming reader before its end closes the result set handle

* *GIVEN* an ADBC statement whose SQL returns a result set with no bound RecordBatch, and a result whose rows need more than one fetch request
* *AND* the caller has read some, but not all, batches from the RecordBatchReader that `execute` returned
* *WHEN* the caller releases the reader
* *THEN* the driver SHALL close the result set handle on the server before the release returns
* *AND* the driver SHALL NOT send a fetch request for the remaining rows
* *AND* the connection SHALL remain usable for later statements

### Scenario: A failed fetch fails the read of a streaming reader

* *GIVEN* an ADBC statement whose SQL returns a result set with no bound RecordBatch, and a result whose rows need more than one fetch request
* *AND* `execute` has returned a RecordBatchReader
* *WHEN* the fetch request of a read fails, or the rows received fall short of or exceed the total row count of the result
* *THEN* that read SHALL return an error whose message carries the message of the failure
* *AND* the read SHALL NOT return a batch
