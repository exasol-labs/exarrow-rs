# Feature: Auto Table Creation

Specifies automatic table creation from Parquet file schemas, including Arrow schema inference, Exasol DDL generation, and the auto-create workflow for Parquet imports.

## Background

Auto table creation infers Arrow schemas from Parquet file metadata without reading the full data. When multiple files have different schemas, a union schema is computed with type widening rules: identical types remain unchanged, DECIMAL types widen to max(precision, scale), VARCHAR types widen to max(size), DECIMAL + DOUBLE widens to DOUBLE, and incompatible types fall back to VARCHAR(2000000). Column names are handled via Quoted mode (preserving names exactly with escaped double quotes) or Sanitize mode (uppercase, invalid chars replaced with underscore, digit-prefixed names get underscore prefix, reserved words get quoted).

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Auto-create table fails due to nonexistent schema

* *GIVEN* a Parquet file exists on disk
* *AND* `create_table_if_not_exists` is enabled
* *AND* the schema portion of the target table name does not exist in Exasol
* *WHEN* user calls `import_from_parquet` with the qualified table name
* *THEN* the system SHALL return an `ImportError::SqlError` containing the database error message
* *AND* the system SHALL NOT proceed to HTTP transport setup

### Scenario: Auto-create table fails due to nonexistent schema (multi-file)

* *GIVEN* multiple Parquet files exist on disk
* *AND* `create_table_if_not_exists` is enabled
* *AND* the schema portion of the target table name does not exist in Exasol
* *WHEN* user calls `import_from_parquet_files` with the qualified table name
* *THEN* the system SHALL return an `ImportError::SqlError` containing the database error message
* *AND* the system SHALL NOT proceed to HTTP transport setup

### Scenario: Auto-create table fails due to other DDL errors

* *GIVEN* a Parquet file exists on disk
* *AND* `create_table_if_not_exists` is enabled
* *AND* the CREATE TABLE DDL fails for a reason other than "table already exists"
* *WHEN* user calls `import_from_parquet` with the target table name
* *THEN* the system SHALL return an `ImportError::SqlError` containing the database error message
* *AND* the system SHALL NOT proceed to HTTP transport setup
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Auto-create with existing table

* *GIVEN* the target table already exists in Exasol
* *WHEN* importing Parquet with `create_table_if_not_exists=true` and target table already exists
* *THEN* system SHALL ignore the "table already exists" DDL error
* *AND* import SHALL proceed normally using existing table schema
<!-- /DELTA:CHANGED -->
