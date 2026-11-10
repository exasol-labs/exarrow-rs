# Feature: Result Schema

Specifies the schema that a RecordBatchReader surfaces for an ADBC statement result, including empty results and Exasol field metadata.

## Background

A RecordBatchReader returned by an ADBC statement exposes the schema of the result set.
Statement execution is specified in `adbc-driver/statement-and-results`.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Reader schema is available before the first read

* *GIVEN* an ADBC connection opened through an ADBC driver manager that loads the driver library
* *WHEN* a statement with no bound RecordBatch executes a query that returns zero rows, or a query whose rows need more than one fetch request, and the caller reads the RecordBatchReader schema before it reads a batch
* *THEN* the reader schema SHALL carry the column names, the Arrow data types, and the `exasol:` field metadata of the result columns
* *AND* reading the schema SHALL NOT send a fetch request
* *AND* every batch that the reader returns SHALL have the field names, the Arrow data types, and the field metadata of the reader schema
<!-- /DELTA:NEW -->
