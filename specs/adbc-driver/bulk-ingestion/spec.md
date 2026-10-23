# Feature: Bulk Ingestion

Ingests Arrow RecordBatches into Exasol tables through the ADBC Statement, in create, append, create-append, and replace modes, into a target schema of the single `EXA` catalog. The driver checks the target catalog and every bound column type before it sends the first SQL statement of an ingest, with the same column type rule in every mode.

## Background

Exasol does not support `CREATE TEMPORARY TABLE`. Exasol has one catalog, named `EXA`, and no binary column type. Exasol DECIMAL precision does not exceed 36, and Exasol INTERVAL DAY TO SECOND has no month component.

## Scenarios

### Scenario: Append to existing table

* *GIVEN* an ADBC statement with `IngestTargetTable` set to an existing table name
* *AND* `IngestMode` is set to `Append` (or not set, as append is the default)
* *AND* Arrow data is bound to the statement
* *WHEN* the statement is executed
* *THEN* the driver SHALL insert the bound data into the existing table via HTTP transport import
* *AND* the driver SHALL return the number of rows ingested

### Scenario: Create new table and ingest

* *GIVEN* an ADBC statement with `IngestTargetTable` set to a table name that does not exist
* *AND* `IngestMode` is set to `Create`
* *AND* Arrow data is bound to the statement
* *WHEN* the statement is executed
* *THEN* the driver SHALL generate a `CREATE TABLE` statement from the Arrow schema
* *AND* the driver SHALL map Arrow types to Exasol types using the existing type mapping
* *AND* the driver SHALL insert the bound data into the newly created table
* *AND* the driver SHALL return the number of rows ingested

### Scenario: Create or append (create if not exists)

* *GIVEN* an ADBC statement with `IngestTargetTable` set to a table name
* *AND* `IngestMode` is set to `CreateAppend`
* *AND* Arrow data is bound to the statement
* *WHEN* the statement is executed
* *THEN* the driver SHALL create the table if it does not exist (using `CREATE TABLE IF NOT EXISTS`)
* *AND* the driver SHALL insert the bound data into the table
* *AND* the driver SHALL return the number of rows ingested

### Scenario: Replace table contents

* *GIVEN* an ADBC statement with `IngestTargetTable` set to an existing table name
* *AND* `IngestMode` is set to `Replace`
* *AND* Arrow data is bound to the statement
* *WHEN* the statement is executed
* *THEN* the driver SHALL drop the existing table
* *AND* the driver SHALL create a new table from the Arrow schema
* *AND* the driver SHALL insert the bound data into the new table
* *AND* the driver SHALL return the number of rows ingested

### Scenario: Target schema specification

* *GIVEN* an ADBC statement with `IngestTargetTable` set to a table name
* *AND* a target schema is specified via `adbc.ingest.target_db_schema` option
* *WHEN* the statement is executed
* *THEN* the driver SHALL create or reference the table in the specified schema
* *AND* the driver SHALL use fully qualified table names (`schema.table`)

### Scenario: NOT NULL propagation from Arrow schema

* *GIVEN* an ADBC statement with `IngestMode` set to `Create` or `CreateAppend` or `Replace`
* *AND* the bound Arrow data has fields marked as non-nullable
* *WHEN* the driver generates `CREATE TABLE` DDL
* *THEN* non-nullable Arrow fields SHALL produce columns with `NOT NULL` constraints
* *AND* nullable Arrow fields SHALL produce columns without `NOT NULL` constraints

### Scenario: Dotted table name without explicit schema

* *GIVEN* an ADBC statement with `IngestTargetTable` set to a dotted name (e.g., `SCHEMA.TABLE`)
* *AND* no target schema is specified via `adbc.ingest.target_db_schema` option
* *WHEN* the statement is executed
* *THEN* the driver SHALL split the table name on the first dot
* *AND* the part before the dot SHALL be used as the schema name
* *AND* the part after the dot SHALL be used as the table name
* *AND* both parts SHALL be individually quoted as identifiers

### Scenario: Temporary table not supported

* *GIVEN* an ADBC statement with the temporary table ingestion option enabled
* *WHEN* the statement is executed
* *THEN* the driver SHALL return an error with status `NotImplemented`
* *AND* the error message SHALL indicate that Exasol does not support temporary tables

### Scenario: Target catalog EXA is accepted in any letter case

* *GIVEN* an ADBC statement with `IngestTargetTable` set to a table that does not exist, `IngestMode` `Create`, and Arrow data bound
* *AND* the option `adbc.ingest.target_catalog` set to `EXA` or to `exa`
* *WHEN* the statement is executed
* *THEN* the driver SHALL create the table and ingest the bound rows, as it does without the option

### Scenario: A target catalog other than EXA fails before any SQL statement

* *GIVEN* an ADBC statement with `IngestTargetTable` set to a table that does not exist, `IngestMode` `Create`, and Arrow data bound
* *AND* the option `adbc.ingest.target_catalog` set to `BOGUS`, to an empty string, or to a value that is not a string
* *WHEN* the statement is executed
* *THEN* the driver SHALL fail with `AdbcStatus::InvalidArguments`, and the error message SHALL state that Exasol has only the catalog `EXA`
* *AND* the driver SHALL send no SQL statement to Exasol for the ingest, so no table SHALL exist afterwards
