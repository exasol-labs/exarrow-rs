# Feature: Result Sets

Specifies result set retrieval, pagination, and metadata for SQL query execution.

## Background

Query results are retrieved efficiently with support for both small single-fetch result sets and large paginated result sets. All result data is converted to Arrow RecordBatch format with full schema metadata.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Zero-row result set preserves schema

* *GIVEN* a query has been executed
* *WHEN* the query returns a result set with zero rows
* *THEN* it SHALL yield exactly one Arrow RecordBatch
* *AND* that RecordBatch SHALL have a row count of zero
* *AND* that RecordBatch SHALL carry the full column schema (column names, Arrow data types, and the Arrow field metadata that `type-mapping/exasol-to-arrow` specifies) derived from the result set's column metadata
* *AND* it SHALL NOT return an empty batch list that drops the schema
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Every batch carries the field metadata of the result set schema

* *GIVEN* a query over the native or the WebSocket transport returns a result set that needs more than one fetch request
* *WHEN* the caller reads every batch with `fetch_all` or with the result set iterator
* *THEN* each field of each batch SHALL carry the same Arrow field metadata as the field at the same position in the schema of the result set metadata
* *AND* each field of each batch SHALL carry the key `exasol:type_name`
<!-- /DELTA:NEW -->
