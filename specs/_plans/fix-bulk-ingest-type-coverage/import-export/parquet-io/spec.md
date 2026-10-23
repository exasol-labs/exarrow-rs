# Feature: Parquet I/O

Specifies Parquet file import through the HTTP tunnel, natively on Exasol 2025.1.11 and newer or after CSV conversion on older servers. Parquet export is specified in `import-export/parquet-export` and `import-export/parquet-csv-bytes`.

## Background

- The driver selects the path from the connected server's `release_version`. On 2025.1.11 and newer, the server requests the Parquet file with HTTP range requests (footer first, then row groups). On older servers, the driver converts each `RecordBatch` to CSV with `record_batch_to_csv` and streams the CSV body.
- The Parquet IMPORT statement omits all CSV format options and the `MULTIPLE LOCAL FILES` tag, because Exasol opens one HTTP server per file for native Parquet import.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: CSV-path Parquet import converts values as RecordBatch import does

* *GIVEN* a Parquet file, written with its Arrow schema, with the fields `ID` (Int64), `N` (`Decimal128(5, 2)`), `S` (`Dictionary(Int32, Utf8)`), and `B` (`FixedSizeBinary(2)`), holding the row `(1, -0.05, 'a;b', 0xABCD)`
* *AND* no table `T` exists
* *WHEN* the application imports the file into `T` with `import_from_parquet` and `ParquetImportOptions` set with `with_native_parquet(Some(false))`, `with_create_table_if_not_exists(true)`, and `with_column_separator(';')`
* *THEN* the driver SHALL create `T` with the columns `N DECIMAL(5,2)`, `S VARCHAR(2000000)`, and `B VARCHAR(2000000)`
* *AND* `T` SHALL hold one row with `N` `-0.05`, `S` `a;b`, and `B` `abcd`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Native Parquet import with table creation rejects Utf8View, dictionary, BinaryView, FixedSizeBinary, and interval columns

* *GIVEN* six Parquet files, each written with its Arrow schema, each with the field `ID` (Int64) and one field `X` of the Arrow type Utf8View, `Dictionary(Int32, Utf8)`, BinaryView, `FixedSizeBinary(16)`, `Interval(YearMonth)`, or `Interval(DayTime)`
* *AND* no table `T` exists
* *WHEN* the application imports each file into `T` with `import_from_parquet` and `ParquetImportOptions` set with `with_native_parquet(Some(true))` and `with_create_table_if_not_exists(true)`
* *THEN* each import SHALL fail with `ImportError::SchemaInferenceError` before the driver sends `CREATE TABLE` or `IMPORT`
* *AND* each error message SHALL name the column `X` and its Arrow type, and SHALL name `with_native_parquet(Some(false))` as the way to import the file
* *AND* `T` SHALL NOT exist after the imports
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: CSV-path Parquet import rejects an INTERVAL column from a file without an Arrow schema

* *GIVEN* a Parquet file written without Arrow schema metadata, with the fields `ID` (Int64) and `X` (`Interval(YearMonth)`) holding 14 months
* *AND* no table `T` exists
* *WHEN* the application imports the file into `T` with `import_from_parquet` and `ParquetImportOptions` set with `with_native_parquet(Some(false))` and `with_create_table_if_not_exists(true)`
* *THEN* the import SHALL fail with `ImportError::SchemaInferenceError` that names the column `X`, before the driver sends `CREATE TABLE` or `IMPORT`
* *AND* the error message SHALL state that the column would lose its month part
* *AND* `T` SHALL NOT exist
<!-- /DELTA:NEW -->
