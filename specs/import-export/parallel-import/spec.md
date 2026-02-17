# Feature: Parallel Import

Specifies parallel file import capabilities for CSV and Parquet formats, leveraging Exasol's native IMPORT parallelization with multiple HTTP transport connections.

## Background

Parallel import establishes N parallel HTTP transport connections (one per file), each with its own EXA tunneling handshake to obtain a unique internal address. The generated IMPORT SQL uses multiple AT/FILE clauses referencing each internal address. Single-file imports delegate to the existing optimized single-file path for backward compatibility. The system accepts both single paths and collections via the IntoFileSources trait. Error handling follows a fail-fast strategy, aborting all operations immediately upon first failure.

## Scenarios

### Scenario: Import multiple CSV files in parallel

* *GIVEN* multiple CSV files exist on disk for import
* *WHEN* user calls import_csv_from_files with a list of CSV file paths
* *THEN* system SHALL establish N parallel HTTP transport connections (one per file)
* *AND* system SHALL perform EXA tunneling handshake on each connection to obtain unique internal addresses
* *AND* system SHALL build IMPORT SQL with multiple FILE clauses referencing each internal address

### Scenario: Generated SQL uses multiple AT/FILE clauses

* *GIVEN* a parallel import is being prepared with N files
* *WHEN* parallel import is initiated with N files
* *THEN* generated SQL SHALL follow format: `FROM CSV AT 'addr1' FILE '001.csv' AT 'addr2' FILE '002.csv' ...`
* *AND* each file SHALL have a unique internal address

### Scenario: Import multiple Parquet files in parallel

* *GIVEN* multiple Parquet files exist on disk for import
* *WHEN* user calls import_parquet_from_files with a list of Parquet file paths
* *THEN* system SHALL convert all Parquet files to CSV format in parallel using tokio tasks
* *AND* system SHALL establish N parallel HTTP transport connections
* *AND* system SHALL stream converted CSV data through each connection

### Scenario: Parquet conversion is parallelized

* *GIVEN* multiple Parquet files need conversion to CSV
* *WHEN* multiple Parquet files are provided
* *THEN* system SHALL convert files concurrently using spawn_blocking tasks
* *AND* system SHALL NOT wait for one conversion to complete before starting another

### Scenario: Single file delegates to existing implementation

* *GIVEN* only one file is provided for parallel import
* *WHEN* user calls import_csv_from_files with a single file path
* *THEN* system SHALL delegate to existing import_from_file implementation
* *AND* behavior SHALL be identical to calling import_from_file directly

### Scenario: API accepts both single path and collection

* *GIVEN* the import API is called with file paths
* *WHEN* user provides either PathBuf or Vec<PathBuf> to import method
* *THEN* system SHALL accept both forms via IntoFileSources trait
* *AND* single path SHALL be treated as collection of one

### Scenario: Fail-fast on connection error

* *GIVEN* a parallel import with multiple HTTP connections is in progress
* *WHEN* any HTTP transport connection fails during parallel import
* *THEN* system SHALL abort all remaining connection attempts immediately
* *AND* system SHALL close all established connections gracefully
* *AND* system SHALL return error with context about which connection failed

### Scenario: Fail-fast on streaming error

* *GIVEN* parallel file streaming is in progress
* *WHEN* any file streaming fails during parallel import
* *THEN* system SHALL abort all other streaming operations immediately
* *AND* system SHALL return error indicating failed file index and path

### Scenario: Fail-fast on Parquet conversion error

* *GIVEN* parallel Parquet conversion is in progress
* *WHEN* any Parquet file fails to convert to CSV
* *THEN* system SHALL abort all other conversion tasks immediately
* *AND* system SHALL return error indicating which file failed conversion
