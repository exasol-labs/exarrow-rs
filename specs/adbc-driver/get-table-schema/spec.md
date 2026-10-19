# Feature: Get Table Schema

Implements the ADBC `Connection::get_table_schema` method, which returns the Arrow Schema of an Exasol table.

## Background

Column types and nullability come from `SYS.EXA_ALL_COLUMNS`.

## Scenarios

### Scenario: Get schema of existing table

* *GIVEN* an active ADBC connection to Exasol
* *AND* a table exists with known columns and types
* *WHEN* `get_table_schema` is called with the table name
* *THEN* the driver SHALL return an Arrow `Schema` with one field per column
* *AND* each field SHALL have the correct Arrow data type mapped from the Exasol type
* *AND* each field SHALL have the correct nullability based on the column's `IS_NULLABLE` attribute

### Scenario: Get schema with schema filter

* *GIVEN* an active ADBC connection to Exasol
* *AND* a table exists in a specific schema
* *WHEN* `get_table_schema` is called with both `db_schema` and `table_name`
* *THEN* the driver SHALL restrict the query to the specified schema
* *AND* the driver SHALL return the schema for the matching table

### Scenario: Table not found

* *GIVEN* an active ADBC connection to Exasol
* *WHEN* `get_table_schema` is called with a table name that does not exist
* *THEN* the driver SHALL return an error with status `NotFound`
* *AND* the error message SHALL indicate the table was not found
