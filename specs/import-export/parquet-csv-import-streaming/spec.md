# Feature: Parquet CSV-Path Import Streaming

Specifies that the CSV conversion path of Parquet import converts and sends one RecordBatch at a time and fails the IMPORT on a conversion error.

## Background

On the CSV conversion path, the driver converts a Parquet file to CSV one RecordBatch at a time and hands each CSV buffer to the HTTP tunnel as it is produced. The tunnel sends the terminating zero-length chunk only after the conversion succeeds. The general Parquet import behavior is specified in `import-export/parquet-io`.

## Scenarios

### Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time

* *GIVEN* a Parquet file with at least 8 row groups that each hold the same number of rows
* *AND* the CSV conversion path is selected, because the connected server's `release_version` is below `2025.1.11` or `ParquetImportOptions::with_native_parquet(Some(false))` is set
* *WHEN* user calls import_from_parquet with the file path, or import_from_parquet_stream with the file's bytes, with `ParquetImportOptions::with_batch_size` set to the row count of one row group
* *THEN* the system SHALL convert the file to CSV one RecordBatch at a time, and SHALL hand the CSV of the first RecordBatch to the HTTP tunnel before it decodes the file's last row group
* *AND* the system MUST NOT hold the CSV of the whole file in memory at once
* *AND* the system SHALL import every row of the file and return the file's row count

### Scenario: CSV-path Parquet import that fails on a later RecordBatch imports no rows

* *GIVEN* a table `T` with the columns `ID DECIMAL(18,0)` and `D DATE`
* *AND* a Parquet file with the fields `ID` (Int64) and `D` (Date32) in 4 row groups of 1000 rows, whose last row holds the Date32 value 2932897 (10000-01-01)
* *WHEN* the application imports the file into `T` with `import_from_parquet`, `ParquetImportOptions::with_native_parquet(Some(false))`, and `ParquetImportOptions::with_batch_size(1000)`
* *THEN* the import SHALL fail with `ImportError::ConversionError` that names the value, and the system MUST NOT send the terminating zero-length chunk on the HTTP tunnel, so that Exasol fails the IMPORT statement
* *AND* `T` SHALL hold no rows
* *AND* the same Connection SHALL run `SELECT 1` successfully after the failed import

### Scenario: Parallel CSV-path Parquet import that fails on a value imports no rows

* *GIVEN* a table `T` with the columns `ID DECIMAL(18,0)` and `D DATE`
* *AND* two Parquet files with the fields `ID` (Int64) and `D` (Date32), of which the second holds the Date32 value 2932897 (10000-01-01) in its last row group
* *WHEN* the application imports both files into `T` with `import_parquet_from_files` and `ParquetImportOptions::with_native_parquet(Some(false))`
* *THEN* the import SHALL fail with `ImportError::ConversionError` whose message names the second file's path and the value
* *AND* `T` SHALL hold no rows
* *AND* the same Connection SHALL run `SELECT 1` successfully after the failed import
