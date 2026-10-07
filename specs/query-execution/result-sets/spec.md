# Feature: Result Sets

Specifies result set retrieval, pagination, and metadata for SQL query execution.

## Background

Query results are retrieved efficiently with support for both small single-fetch result sets and large paginated result sets. All result data is converted to Arrow RecordBatch format with full schema metadata.

## Scenarios

### Scenario: Small result set retrieval

* *GIVEN* a query has been executed
* *WHEN* a query returns a small result set (< 1000 rows) whose total size fits within the server's maximum data message size
* *THEN* it SHALL receive all rows with the execute response
* *AND* it SHALL NOT send a fetch request
* *AND* it SHALL convert data to Arrow RecordBatch

### Scenario: Large result set pagination

* *GIVEN* a query has been executed over the native or the WebSocket transport
* *WHEN* a query returns a result set that needs more than one fetch request
* *THEN* it SHALL support fetching results in batches
* *AND* it SHALL maintain result set handle for pagination
* *AND* each fetch request SHALL start at the first row that no earlier response delivered
* *AND* `fetch_all` SHALL return every row of the result set exactly once

### Scenario: Result set metadata

* *GIVEN* a query has been executed
* *WHEN* retrieving result metadata
* *THEN* it SHALL provide column names and types
* *AND* it SHALL provide row count (if available)
* *AND* it SHALL provide Arrow schema
* *AND* the Arrow schema SHALL be available regardless of how many rows the result set contains, including zero rows

### Scenario: Zero-row result set preserves schema

* *GIVEN* a query has been executed
* *WHEN* the query returns a result set with zero rows
* *THEN* it SHALL yield exactly one Arrow RecordBatch
* *AND* that RecordBatch SHALL have a row count of zero
* *AND* that RecordBatch SHALL carry the full column schema (column names and Arrow data types) derived from the result set's column metadata
* *AND* it SHALL NOT return an empty batch list that drops the schema

### Scenario: Result partly delivered with the execute response

* *GIVEN* a query over the native or the WebSocket transport returns fewer than 1,000 rows whose total size exceeds the server's maximum data message size
* *AND* the execute response delivers the first rows of the result set together with a result set handle
* *WHEN* the caller reads the result set with `fetch_all` or with the result set iterator
* *THEN* the first fetch request SHALL start at the first row that the execute response did not deliver
* *AND* the system SHALL return every row of the result set exactly once

### Scenario: Result set iterator ends after the last row

* *GIVEN* a query over the native or the WebSocket transport has returned a result set with a result set handle
* *WHEN* the caller reads batches from the result set iterator until it reports no further batch
* *THEN* the iterator SHALL yield every row of the result set exactly once
* *AND* the iterator SHALL report the end of the result set once the rows it has yielded reach the result set's total row count
* *AND* the iterator SHALL NOT send a fetch request after the last row has arrived

### Scenario: Prepared statement result is paged like a query result

* *GIVEN* a prepared statement over the native or the WebSocket transport returns fewer than 1,000 rows whose total size exceeds the server's maximum data message size
* *WHEN* the caller executes the prepared statement and reads the result set with `fetch_all`
* *THEN* the first fetch request SHALL start at the first row that the execute response did not deliver
* *AND* the system SHALL return every row of the result set exactly once

### Scenario: Result set that ends before its total row count fails

* *GIVEN* a result set with a result set handle reports a total row count greater than zero
* *AND* the rows received count every row delivered so far, including the rows of the execute response
* *WHEN* a fetch request returns no rows before the rows received reach the total row count
* *THEN* `fetch_all` and the result set iterator SHALL return an error
* *AND* the error message SHALL state the number of rows received and the total row count
* *AND* `fetch_all` SHALL NOT return the rows received as a successful result

### Scenario: Result set that exceeds its total row count fails

* *GIVEN* a result set with a result set handle reports a total row count greater than zero
* *AND* the rows received count every row delivered so far, including the rows of the execute response
* *WHEN* a fetch request returns a batch that brings the rows received above the total row count
* *THEN* `fetch_all` and the result set iterator SHALL return an error
* *AND* the error message SHALL state the number of rows received including that batch and the total row count
* *AND* the result set iterator SHALL NOT yield that batch
