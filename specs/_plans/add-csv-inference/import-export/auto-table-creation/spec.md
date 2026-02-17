# Feature: Auto Table Creation

<!-- CHANGED -->
Specifies automatic table creation from Parquet and CSV file schemas, including Arrow schema inference, Exasol DDL generation, and the auto-create workflow for imports.
<!-- /CHANGED -->

## Background

<!-- CHANGED -->
Auto table creation infers Arrow schemas from source file metadata. For Parquet files, schema inference reads only metadata without loading data. For CSV files, schema inference samples data rows using arrow-csv to detect column types, controlled by `CsvInferenceOptions` (delimiter, quote, escape, header, sample limit). When multiple files have different schemas, a union schema is computed with type widening rules: identical types remain unchanged, DECIMAL types widen to max(precision, scale), VARCHAR types widen to max(size), DECIMAL + DOUBLE widens to DOUBLE, and incompatible types fall back to VARCHAR(2000000). Column names are handled via Quoted mode (preserving names exactly with escaped double quotes) or Sanitize mode (uppercase, invalid chars replaced with underscore, digit-prefixed names get underscore prefix, reserved words get quoted). For CSV files without a header row, column names are generated as col_1, col_2, ..., col_N.
<!-- /CHANGED -->

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

<!-- NEW -->
### Scenario: Infer schema from single CSV file

* *GIVEN* a CSV file exists on disk
* *WHEN* user calls infer_schema_from_csv with file path and CsvInferenceOptions
* *THEN* system SHALL use arrow-csv Format to infer Arrow types from sampled rows
* *AND* system SHALL map Arrow types to Exasol types via TypeMapper
* *AND* system SHALL return InferredTableSchema with column definitions and source file

### Scenario: Infer schema from multiple CSV files with type widening

* *GIVEN* multiple CSV files exist with potentially different inferred types
* *WHEN* user calls infer_schema_from_csv_files with file paths and CsvInferenceOptions
* *THEN* system SHALL infer schema from each file independently
* *AND* system SHALL merge column types using widen_type()
* *AND* system SHALL merge nullable flags (true if any file has nullable)
* *AND* system SHALL return SchemaMismatchError if files have different column counts

### Scenario: CSV inference with custom format options

* *GIVEN* a CSV file uses non-default formatting
* *WHEN* user configures CsvInferenceOptions with custom delimiter, quote, or escape
* *THEN* system SHALL apply configured options to arrow-csv Format
* *AND* system SHALL correctly parse fields using the specified format

### Scenario: CSV inference without header row

* *GIVEN* a CSV file has no header row
* *WHEN* user sets has_header to false in CsvInferenceOptions
* *THEN* system SHALL generate column names as col_1, col_2, ..., col_N
* *AND* system SHALL apply column_name_mode to generated names

### Scenario: CSV inference empty file error

* *GIVEN* a CSV file contains only a header row with no data
* *WHEN* user requests schema inference
* *THEN* system SHALL return SchemaInferenceError indicating no data rows
* *AND* error message SHALL include the file path

### Scenario: CSV inference unrecognized type fallback

* *GIVEN* arrow-csv infers an Arrow type not mapped in TypeMapper
* *WHEN* mapping Arrow types to Exasol types during CSV inference
* *THEN* system SHALL fall back to VARCHAR(2000000) for unrecognized types
* *AND* system SHALL NOT return an error for unmapped types
<!-- /NEW -->
