# Feature: Statement Schema Lookup

Implements the ADBC Statement methods `get_parameter_schema` and `execute_schema`, which return the Arrow Schema of a statement's parameters and of its result set from the metadata Exasol reports at prepare, without executing the statement. Setting different SQL discards an earlier prepare for every Statement method, including `execute` and `execute_update` with bound parameters.

## Background

Exasol reports the type of each parameter at prepare and, for a statement that returns a result set, the name and type of each result-set column, as `prepared-statements/result-columns` specifies. A statement that returns an affected-row count, such as INSERT, UPDATE, or DELETE, has no result-set columns. The metadata of a prepare belongs to the SQL text that was set when the prepare ran.

## Scenarios

### Scenario: Get parameter schema after prepare

* *GIVEN* an ADBC statement has been prepared with a SQL query containing parameters
* *WHEN* `get_parameter_schema` is called
* *THEN* the driver SHALL return an Arrow `Schema` with one field per parameter
* *AND* each field SHALL use the name from the Exasol prepared statement metadata
* *AND* each field SHALL have the Arrow data type corresponding to the Exasol parameter type

### Scenario: Get parameter schema without prepare

* *GIVEN* an ADBC statement has NOT been prepared
* *WHEN* `get_parameter_schema` is called
* *THEN* the driver SHALL return an error with status `InvalidState`
* *AND* the error message SHALL indicate that the statement must be prepared first

### Scenario: Execute schema returns the result columns of a query without executing it

* *GIVEN* an ADBC statement, prepared or not, whose SQL is a query that returns a result set, such as a SELECT
* *WHEN* `execute_schema` is called
* *THEN* the driver SHALL prepare the statement if it is not prepared yet
* *AND* the driver SHALL return an Arrow `Schema` with one nullable field per result-set column, in select-list order, named with the column name that Exasol reports, which is the alias of an aliased column and the name Exasol assigned to an unaliased derived column
* *AND* the returned schema SHALL equal the schema of the RecordBatchReader that `execute` returns for the same query when that query returns zero rows
* *AND* the driver MUST NOT send an execution request for the query, so a query that fails at run time, such as a cast of non-numeric text to DECIMAL, SHALL still return its schema

### Scenario: Execute schema of a parameterized query needs no bound parameters

* *GIVEN* an ADBC statement whose SQL is a SELECT with parameter markers, such as `SELECT ID, NAME FROM T WHERE ID = ?`, and no bound parameters
* *WHEN* `execute_schema` is called
* *THEN* the driver SHALL return an Arrow `Schema` with the fields `ID` and `NAME`
* *AND* the driver MUST NOT return an error because no parameters are bound

### Scenario: Execute schema of a statement without a result set returns an empty schema

* *GIVEN* a table `T` that holds rows and an ADBC statement whose SQL returns an affected-row count, such as `INSERT INTO T VALUES (2)` or `DELETE FROM T`
* *WHEN* `execute_schema` is called
* *THEN* the driver SHALL return an Arrow `Schema` with zero fields
* *AND* the driver MUST NOT return an error because the statement has no result set
* *AND* the driver MUST NOT send an execution request for the statement
* *AND* table `T` SHALL hold the same rows as before the call

### Scenario: Setting new SQL discards the earlier prepare

* *GIVEN* an ADBC statement prepared for a SQL query A
* *WHEN* `set_sql_query` is called with a different SQL query B
* *THEN* `execute_schema` SHALL return the result-set columns of B and MUST NOT return those of A
* *AND* `get_parameter_schema` SHALL return an error with status `InvalidState` as long as the statement has not been prepared for B
* *AND* `execute` and `execute_update` with a bound RecordBatch SHALL prepare and run B and MUST NOT run A

### Scenario: Execute schema without SQL fails

* *GIVEN* an ADBC statement with no SQL set, such as a statement configured only for bulk ingestion through the `adbc.ingest.target_table` option
* *WHEN* `execute_schema` is called
* *THEN* the driver SHALL return an error with status `InvalidState`
* *AND* the driver MUST NOT send a prepare or execution request to Exasol

### Scenario: Execute schema reports a result-set column type that has no Arrow mapping

* *GIVEN* a prepared ADBC statement whose result-set metadata holds a column with an Exasol type that the driver cannot map to an Arrow type
* *WHEN* `execute_schema` is called
* *THEN* the driver SHALL return an error with status `Internal` whose message names that Exasol type
* *AND* the driver MUST NOT send an execution request for the statement
