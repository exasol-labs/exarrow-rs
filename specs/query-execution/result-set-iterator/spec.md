# Feature: Result Set Iterator

Specifies how the result set iterator reads a result set one batch per call, keeps no yielded batch, and releases the result set handle.

## Background

* A result set holds the rows and the column metadata of one query, converted to Arrow RecordBatches with full schema metadata.
* A caller reads a result set all at once with `fetch_all`, or one batch at a time with the result set iterator that `ResultSet::into_iterator` returns.
* A result set whose rows the execute response does not fully deliver carries a result set handle, and fetch requests deliver the remaining rows.

## Scenarios

### Scenario: Result set iterator fetches one batch per call

* *GIVEN* a query over the native or the WebSocket transport has returned a result set with a result set handle whose rows need more than one fetch request
* *WHEN* the caller awaits `next_batch` on the result set iterator until it reports the end of the result set
* *THEN* the first call SHALL return the batch of the execute response without sending a fetch request
* *AND* each later call SHALL send at most one fetch request and SHALL return the batch of that fetch or report the end of the result set
* *AND* the iterator SHALL NOT send a fetch request while no call to `next_batch` is in progress
* *AND* the iterator SHALL yield every row of the result set exactly once

### Scenario: Result set iterator keeps no batch that it has yielded

* *GIVEN* a query over the native or the WebSocket transport has returned a result set whose rows need more than one fetch request
* *WHEN* the caller reads every batch from the result set iterator and drops each batch before it awaits the next one
* *THEN* the iterator SHALL NOT keep a reference to a column array of any batch that it has yielded, including the batch of the execute response
* *AND* the iterator SHALL yield every row of the result set exactly once

### Scenario: Result set iterator reads inside an async runtime

* *GIVEN* a caller runs on a Tokio runtime with a single thread, or on a Tokio runtime with several worker threads
* *AND* a query over the native or the WebSocket transport has returned a result set whose rows need more than one fetch request
* *WHEN* the caller awaits `next_batch` on the result set iterator until it reports the end of the result set
* *THEN* `next_batch` MUST NOT panic
* *AND* the iterator SHALL yield every row of the result set exactly once

### Scenario: Result set iterator closes the result set handle when it ends

* *GIVEN* a query over the native or the WebSocket transport has returned a result set with a result set handle
* *WHEN* the result set iterator reports the end of the result set, or returns an error because the rows received fall short of or exceed the total row count
* *THEN* the iterator SHALL close the result set handle on the server
* *AND* a failed close SHALL NOT change the end or the error that the iterator reports
* *AND* later calls to `next_batch` and `close` SHALL send no request, and `next_batch` SHALL report the end of the result set
* *AND* the connection SHALL remain usable for later statements

### Scenario: Result set iterator closed before its end releases the result set handle

* *GIVEN* a query over the native or the WebSocket transport has returned a result set whose rows need more than one fetch request
* *AND* the caller has read some, but not all, batches from the result set iterator
* *WHEN* the caller awaits `close` on the iterator
* *THEN* the driver SHALL close the result set handle on the server
* *AND* the driver SHALL NOT send a fetch request for the remaining rows
* *AND* the connection SHALL remain usable for later statements

### Scenario: Result set request dropped before its response terminates the connection

* *GIVEN* a query over the native or the WebSocket transport has returned a result set with a result set handle
* *AND* a call to `next_batch` or `close` on the result set iterator, or a call to `fetch_all`, has started a fetch request or a close request for that handle and has not read its response
* *WHEN* the caller drops the future of that call, for example because a `tokio::time::timeout` around it elapsed
* *THEN* the driver SHALL terminate the connection and SHALL NOT send another request on it
* *AND* every later operation on that connection other than `close` SHALL fail with an error that states that the transport was terminated because an in-flight response was abandoned and that tells the caller to reconnect
* *AND* a future that the caller drops while its call waits for the transport, before its request starts, MUST NOT terminate the connection
