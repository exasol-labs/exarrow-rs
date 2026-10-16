# Feature: Parallel Import

Specifies parallel file import capabilities for CSV and Parquet formats, leveraging Exasol's native IMPORT parallelization with multiple HTTP transport connections.

## Background

Parallel import establishes N parallel HTTP transport connections (one per file), each with its own EXA tunneling handshake to obtain a unique internal address. The generated IMPORT SQL uses multiple `AT '...' FILE '...'` clauses referencing each internal address. For Parquet, the wire model follows the same dual-path rule as single-file Parquet import: on Exasol 2025.1.11+ the driver serves each file's raw Parquet bytes from memory via HTTP range requests on its own connection and emits `FROM PARQUET AT 'addr;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE 'NNN.parquet'` (no CSV format clauses, no `MULTIPLE LOCAL FILES` tag); on older servers the driver converts each file to CSV in parallel and emits `FROM CSV ... FILE 'NNN.csv'` with the usual format options. Single-file imports delegate to the existing optimized single-file path for backward compatibility. The system accepts both single paths and collections via the IntoFileSources trait. Error handling follows a fail-fast strategy, aborting all operations immediately upon first failure.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Parallel CSV import streams each file in pieces

* *GIVEN* multiple uncompressed CSV files exist on disk for import
* *WHEN* user calls import_csv_from_files with a list of those file paths and no compression set
* *THEN* the system SHALL read each file in pieces of at most 64 KiB and SHALL send each piece through that file's HTTP transport connection before it reads the next piece of that file
* *AND* the system MUST NOT read a whole file into memory before it sends the file
* *AND* the system SHALL import every row of every file and return the total row count
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Parallel CSV import compresses each file while streaming it

* *GIVEN* multiple uncompressed CSV files with the `.csv` extension exist on disk for import
* *WHEN* user calls import_csv_from_files with those file paths and gzip or bzip2 compression set in `CsvImportOptions`
* *THEN* the system SHALL compress each file piece by piece while it streams the file
* *AND* the body that each HTTP transport connection carries SHALL decompress to the exact bytes of its file
* *AND* the system SHALL import every row of every file and return the total row count
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Parallel CSV import fails before running IMPORT when a file cannot be opened

* *GIVEN* a list of two CSV file paths, of which the second does not exist
* *WHEN* user calls import_csv_from_files with that list
* *THEN* the import SHALL fail with `ImportError::IoError`
* *AND* the system MUST NOT open an HTTP transport connection or run IMPORT SQL
* *AND* the target table SHALL keep its rows unchanged
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Import multiple Parquet files in parallel

* *GIVEN* multiple Parquet files exist on disk for import
* *AND* the CSV conversion path is selected, because the connected Exasol server's `release_version` is below `2025.1.11` or `ParquetImportOptions::with_native_parquet(Some(false))` is set
* *WHEN* user calls import_parquet_from_files with a list of Parquet file paths
* *THEN* system SHALL convert each Parquet file to CSV one RecordBatch at a time in its own blocking task, and SHALL stream each file's CSV through its own HTTP transport connection while the conversion runs, with N parallel connections for N files, and MUST NOT hold the CSV of a whole file in memory at once
* *AND* when every file holds zero rows, the system SHALL return 0 without opening an HTTP transport connection or running IMPORT SQL
* *AND* the generated IMPORT SQL SHALL use `FROM CSV` with `.csv` file names and the existing CSV format option clauses
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Parallel CSV-path Parquet import that fails on a value imports no rows

* *GIVEN* a table `T` with the columns `ID DECIMAL(18,0)` and `D DATE`
* *AND* two Parquet files with the fields `ID` (Int64) and `D` (Date32), of which the second holds the Date32 value 2932897 (10000-01-01) in its last row group
* *WHEN* the application imports both files into `T` with `import_parquet_from_files` and `ParquetImportOptions::with_native_parquet(Some(false))`
* *THEN* the import SHALL fail with `ImportError::ConversionError` whose message names the second file's path and the value
* *AND* `T` SHALL hold no rows
* *AND* the same Connection SHALL run `SELECT 1` successfully after the failed import
<!-- /DELTA:NEW -->
