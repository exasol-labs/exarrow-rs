# Feature: Parallel Import

Specifies parallel file import capabilities for CSV and Parquet formats, leveraging Exasol's native IMPORT parallelization with multiple HTTP transport connections.

<!-- DELTA:CHANGED -->
## Background

Parallel import establishes N parallel HTTP transport connections (one per file), each with its own EXA tunneling handshake to obtain a unique internal address. The generated IMPORT SQL uses multiple `AT '...' FILE '...'` clauses referencing each internal address. For Parquet, the wire model follows the same dual-path rule as single-file Parquet import (`import-export/parquet-io`): on a server for which `supports_native_parquet_import()` returns true the driver serves each file's raw Parquet bytes from memory via HTTP range requests on its own connection and emits `FROM PARQUET AT 'addr;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE 'NNN.parquet'` (no CSV format clauses, no `MULTIPLE LOCAL FILES` tag); on every other server the driver converts each file to CSV in parallel and emits `FROM CSV ... FILE 'NNN.csv'` with the usual format options. Single-file imports delegate to the existing optimized single-file path for backward compatibility. The system accepts both single paths and collections via the IntoFileSources trait. Error handling follows a fail-fast strategy, aborting all operations immediately upon first failure.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Import multiple Parquet files in parallel

* *GIVEN* multiple Parquet files exist on disk for import
* *AND* `supports_native_parquet_import()` returns false for the connected Exasol server
* *WHEN* user calls import_parquet_from_files with a list of Parquet file paths
* *THEN* system SHALL convert all Parquet files to CSV format in parallel using tokio tasks and stream the converted CSV through N parallel HTTP transport connections
* *AND* the generated IMPORT SQL SHALL use `FROM CSV` with `.csv` file names and the existing CSV format option clauses
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Native parallel Parquet import on Exasol 2025.1.11+

* *GIVEN* multiple Parquet files exist on disk for import
* *AND* the connected Exasol server's `release_version` is at or above `2025.1.11`
* *WHEN* user calls import_parquet_from_files with a list of Parquet file paths
* *THEN* the system MUST establish N parallel HTTP transport connections and serve each file's raw Parquet bytes through its corresponding connection via HTTP range requests, without any CSV conversion
* *AND* the generated IMPORT SQL MUST emit `FROM PARQUET AT 'addr1;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '001.parquet' AT 'addr2;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '002.parquet' ...` with one `.parquet` entry per file and the `;MaxConcurrentReads=1` suffix on every URL
* *AND* the generated SQL MUST NOT contain `MULTIPLE LOCAL FILES`, `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, or `REJECT LIMIT`
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Native parallel Parquet import on a server that supports it

* *GIVEN* multiple Parquet files exist on disk for import
* *AND* `supports_native_parquet_import()` returns true for the connected Exasol server
* *WHEN* user calls import_parquet_from_files with a list of Parquet file paths
* *THEN* the system MUST establish N parallel HTTP transport connections and serve each file's raw Parquet bytes through its corresponding connection via HTTP range requests, without any CSV conversion
* *AND* the generated IMPORT SQL MUST emit `FROM PARQUET AT 'addr1;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '001.parquet' AT 'addr2;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '002.parquet' ...` with one `.parquet` entry per file and the `;MaxConcurrentReads=1` suffix on every URL
* *AND* the generated SQL MUST NOT contain `MULTIPLE LOCAL FILES`, `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, or `REJECT LIMIT`
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Parquet conversion is parallelized

* *GIVEN* multiple Parquet files need conversion to CSV
* *AND* `supports_native_parquet_import()` returns false for the connected server, so the CSV-conversion path is selected
* *WHEN* multiple Parquet files are provided
* *THEN* system SHALL convert files concurrently using spawn_blocking tasks
* *AND* system SHALL NOT wait for one conversion to complete before starting another
* *AND* on a server for which `supports_native_parquet_import()` returns true no client-side conversion SHALL run because the native path serves Parquet bytes directly via range requests
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Fail-fast on Parquet conversion error

* *GIVEN* parallel Parquet conversion is in progress on a server for which `supports_native_parquet_import()` returns false
* *WHEN* any Parquet file fails to convert to CSV
* *THEN* system SHALL abort all other conversion tasks immediately and SHALL return an error indicating which file failed conversion
* *AND* on a server for which `supports_native_parquet_import()` returns true the equivalent fail-fast behavior SHALL apply to file-read errors during native range-request serving and SHALL identify the failing file index and path
<!-- /DELTA:CHANGED -->
