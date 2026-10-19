# Feature: Parallel Import

Specifies parallel import of multiple CSV or Parquet files, with one HTTP transport connection per file.

## Background

Each connection performs its own EXA tunneling handshake to obtain a unique internal address. Parquet import follows the dual-path rule of `import-export/parquet-io`: native range-request serving on Exasol 2025.1.11 and newer, CSV conversion on older servers.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Native parallel Parquet import on Exasol 2025.1.11+

* *GIVEN* multiple Parquet files exist on disk for import
* *AND* the connected Exasol server's `release_version` is at or above `2025.1.11`
* *WHEN* user calls import_parquet_from_files with a list of Parquet file paths
* *THEN* the system MUST establish N parallel HTTP transport connections and serve each file's raw Parquet bytes from that file through its corresponding connection via HTTP range requests, without any CSV conversion, reading each requested range from its file in pieces of at most 64 KiB, and MUST NOT hold any whole file in memory, so the file data it holds grows with the number of connections and not with the size of the files
* *AND* the generated IMPORT SQL MUST emit `FROM PARQUET AT 'addr1;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '001.parquet' AT 'addr2;MaxConcurrentReads=1' [PUBLIC KEY '...'] FILE '002.parquet' ...` with one `.parquet` entry per file and the `;MaxConcurrentReads=1` suffix on every URL
* *AND* the generated SQL MUST NOT contain `MULTIPLE LOCAL FILES`, `ENCODING`, `COLUMN SEPARATOR`, `COLUMN DELIMITER`, `ROW SEPARATOR`, `SKIP`, `NULL`, `TRIM`, or `REJECT LIMIT`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Fail-fast on Parquet conversion error

* *GIVEN* parallel Parquet conversion is in progress on a server below 2025.1.11
* *WHEN* any Parquet file fails to convert to CSV
* *THEN* system SHALL abort all other conversion tasks immediately and SHALL return an error indicating which file failed conversion
* *AND* on Exasol 2025.1.11+ a file that cannot be opened, or a failure while the system serves a file's byte ranges, SHALL abort the range serving of the other files in the same way, and the error SHALL name the failing file index and path
<!-- /DELTA:CHANGED -->
