# Feature: Statement and Results

Specifies statement execution, session identity, and result handling for the ADBC driver. Queries run through the Connection and return Arrow results.

## Background

A Statement is pure data and holds no transport reference. Statements created from one connection share a single database session.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: RecordBatchReader schema carries Exasol field metadata

* *GIVEN* an ADBC connection opened through an ADBC driver manager that loads the driver library
* *WHEN* a statement executes `SELECT CAST(1.5 AS DECIMAL(10,2)) AS AMOUNT, CAST('x' AS VARCHAR(20)) AS NAME` and returns a RecordBatchReader
* *THEN* the field `AMOUNT` of the reader schema SHALL carry the key `exasol:type_name` with the value `DECIMAL`, the key `exasol:precision` with the value `10`, and the key `exasol:scale` with the value `2`
* *AND* the field `NAME` of the reader schema SHALL carry the key `exasol:type_name` with the value `VARCHAR` and the key `exasol:size` with the value `20`
* *AND* each batch that the reader yields SHALL carry the same field metadata as the reader schema
<!-- /DELTA:NEW -->
