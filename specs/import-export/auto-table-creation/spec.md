# Feature: Auto Table Creation

Specifies automatic table creation from Parquet file schemas, including Arrow schema inference, Exasol DDL generation, and the auto-create workflow for Parquet imports.

## Background

Auto table creation infers Arrow schemas from Parquet file metadata without reading the full data. When multiple files have different schemas, a union schema is computed with type widening rules: identical types remain unchanged, DECIMAL types widen to max(precision, scale), VARCHAR types widen to max(size), DECIMAL + DOUBLE widens to DOUBLE, and incompatible types fall back to VARCHAR(2000000). Column names are handled via Quoted mode (preserving names exactly with escaped double quotes) or Sanitize mode (uppercase, invalid chars replaced with underscore, digit-prefixed names get underscore prefix, reserved words get quoted).

## Scenarios

### Scenario: Infer schema from single Parquet file

* *GIVEN* a Parquet file exists with metadata
* *WHEN* user requests schema inference from a single Parquet file
* *THEN* system SHALL read only the Parquet metadata (not data)
* *AND* system SHALL return the Arrow schema with field names and types
* *AND* system SHALL include nullability information for each field

### Scenario: Infer union schema from multiple Parquet files

* *GIVEN* multiple Parquet files exist with potentially different schemas
* *WHEN* user requests schema inference from multiple Parquet files
* *THEN* system SHALL read metadata from all files
* *AND* system SHALL compute a union schema that accommodates all files
* *AND* system SHALL widen types when fields have different types across files

### Scenario: Schema inference error handling

* *GIVEN* Parquet files are being processed for schema inference
* *WHEN* schema inference encounters an error
* *THEN* system SHALL return SchemaInferenceError with file path context
* *AND* system SHALL indicate whether the error was in reading metadata or type conversion

### Scenario: Column name handling with Quoted mode

* *GIVEN* a schema with column names is ready for DDL generation
* *WHEN* generating DDL with Quoted column name mode
* *THEN* column names SHALL be wrapped in double quotes
* *AND* internal double quotes in names SHALL be escaped by doubling
* *AND* original column names SHALL be preserved exactly

### Scenario: Column name handling with Sanitize mode

* *GIVEN* a schema with column names is ready for DDL generation
* *WHEN* generating DDL with Sanitize column name mode
* *THEN* column names SHALL be converted to uppercase
* *AND* invalid identifier characters SHALL be replaced with underscore
* *AND* names starting with digits SHALL be prefixed with underscore

### Scenario: DDL type generation

* *GIVEN* an Arrow schema has been mapped to Exasol types
* *WHEN* generating DDL column types
* *THEN* ExasolType SHALL be converted to valid DDL syntax
* *AND* BOOLEAN SHALL generate "BOOLEAN"
* *AND* VARCHAR(n) SHALL generate "VARCHAR(n)"

### Scenario: Complete DDL statement generation

* *GIVEN* all column definitions have been generated
* *WHEN* generating CREATE TABLE DDL
* *THEN* output SHALL include "CREATE TABLE schema.table (" prefix
* *AND* output SHALL include column definitions separated by commas
* *AND* output SHALL include closing ");"

### Scenario: Auto-create table option enabled

* *GIVEN* the target table does not exist in Exasol
* *WHEN* importing Parquet with create_table_if_not_exists=true
* *THEN* system SHALL infer schema from Parquet file(s)
* *AND* system SHALL generate CREATE TABLE DDL
* *AND* system SHALL execute DDL before IMPORT statement

### Scenario: Auto-create with existing table

* *GIVEN* the target table already exists in Exasol
* *WHEN* importing Parquet with create_table_if_not_exists=true and target table already exists
* *THEN* system SHALL skip DDL execution
* *AND* import SHALL proceed normally using existing table schema

### Scenario: Auto-create option disabled (default)

* *GIVEN* the auto-create option is not enabled
* *WHEN* importing Parquet with create_table_if_not_exists=false (default)
* *THEN* system SHALL NOT attempt schema inference
* *AND* system SHALL NOT execute any CREATE TABLE DDL

### Scenario: Multi-file auto-create

* *GIVEN* multiple Parquet files need to be imported with auto-create
* *WHEN* importing multiple Parquet files with create_table_if_not_exists=true
* *THEN* system SHALL compute union schema from all files
* *AND* system SHALL create table with widened types
* *AND* all files SHALL be importable into the created table
