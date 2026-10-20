# Feature: Statement Schema Lookup

Implements the ADBC Statement methods `get_parameter_schema` and `execute_schema`, which return the Arrow Schema of a statement's parameters and of its result set from the metadata Exasol reports at prepare, without executing the statement. Setting different SQL discards an earlier prepare for every Statement method, including `execute` and `execute_update` with bound parameters.

## Background

Exasol reports the type of each parameter at prepare and, for a statement that returns a result set, the name and type of each result-set column, as `prepared-statements/result-columns` specifies. A statement that returns an affected-row count, such as INSERT, UPDATE, or DELETE, has no result-set columns. The metadata of a prepare belongs to the SQL text that was set when the prepare ran.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Execute schema returns the result columns of a query without executing it

* *GIVEN* an ADBC statement, prepared or not, whose SQL is a query that returns a result set, such as a SELECT
* *WHEN* `execute_schema` is called
* *THEN* the driver SHALL prepare the statement if it is not prepared yet
* *AND* the driver SHALL return an Arrow `Schema` with one nullable field per result-set column, in select-list order, named with the column name that Exasol reports, which is the alias of an aliased column and the name Exasol assigned to an unaliased derived column, and each field SHALL carry the Arrow field metadata that `type-mapping/exasol-to-arrow` specifies for its column
* *AND* the returned schema SHALL equal the schema of the RecordBatchReader that `execute` returns for the same query when that query returns zero rows, including the field metadata
* *AND* the driver MUST NOT send an execution request for the query, so a query that fails at run time, such as a cast of non-numeric text to DECIMAL, SHALL still return its schema
<!-- /DELTA:CHANGED -->
