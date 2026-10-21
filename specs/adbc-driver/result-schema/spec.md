# Feature: Result Schema

Specifies the schema that a RecordBatchReader surfaces for an ADBC statement result, including empty results and Exasol field metadata.

## Background

A RecordBatchReader returned by an ADBC statement exposes the schema of the result set.
Statement execution is specified in `adbc-driver/statement-and-results`.

## Scenarios

### Scenario: Empty result schema surfaced via RecordBatchReader

* *GIVEN* an ADBC statement is executed whose query returns zero rows (for example a `SELECT ... WHERE FALSE LIMIT 0` schema probe)
* *WHEN* the resulting RecordBatchReader's schema is read from its first batch
* *THEN* the RecordBatchReader SHALL surface the full column schema (column names and Arrow data types)
* *AND* it SHALL NOT fall back to an empty schema with zero columns

### Scenario: RecordBatchReader schema carries Exasol field metadata

* *GIVEN* an ADBC connection opened through an ADBC driver manager that loads the driver library
* *WHEN* a statement executes `SELECT CAST(1.5 AS DECIMAL(10,2)) AS AMOUNT, CAST('x' AS VARCHAR(20)) AS NAME` and returns a RecordBatchReader
* *THEN* the field `AMOUNT` of the reader schema SHALL carry the key `exasol:type_name` with the value `DECIMAL`, the key `exasol:precision` with the value `10`, and the key `exasol:scale` with the value `2`
* *AND* the field `NAME` of the reader schema SHALL carry the key `exasol:type_name` with the value `VARCHAR` and the key `exasol:size` with the value `20`
* *AND* each batch that the reader yields SHALL carry the same field metadata as the reader schema
