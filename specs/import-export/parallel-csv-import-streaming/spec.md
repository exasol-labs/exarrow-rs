# Feature: Parallel CSV Import Streaming

Specifies that multi-file CSV import streams each file in pieces instead of holding whole files in memory.

## Background

Parallel CSV import sends each file through its own HTTP transport connection. Exasol requests all files of one IMPORT at the same time, so the driver reads each file in pieces of at most 64 KiB. The general parallel import behavior is specified in `import-export/parallel-import`.

## Scenarios

### Scenario: Parallel CSV import streams each file in pieces

* *GIVEN* multiple uncompressed CSV files exist on disk for import
* *WHEN* user calls import_csv_from_files with a list of those file paths and no compression set
* *THEN* the system SHALL read each file in pieces of at most 64 KiB and SHALL send each piece through that file's HTTP transport connection before it reads the next piece of that file
* *AND* the system MUST NOT read a whole file into memory before it sends the file
* *AND* the system SHALL import every row of every file and return the total row count

### Scenario: Parallel CSV import compresses each file while streaming it

* *GIVEN* multiple uncompressed CSV files with the `.csv` extension exist on disk for import
* *WHEN* user calls import_csv_from_files with those file paths and gzip or bzip2 compression set in `CsvImportOptions`
* *THEN* the system SHALL compress each file piece by piece while it streams the file
* *AND* the body that each HTTP transport connection carries SHALL decompress to the exact bytes of its file
* *AND* the system SHALL import every row of every file and return the total row count

### Scenario: Parallel CSV import fails before running IMPORT when a file cannot be opened

* *GIVEN* a list of two CSV file paths, of which the second does not exist
* *WHEN* user calls import_csv_from_files with that list
* *THEN* the import SHALL fail with `ImportError::IoError`
* *AND* the system MUST NOT open an HTTP transport connection or run IMPORT SQL
* *AND* the target table SHALL keep its rows unchanged
