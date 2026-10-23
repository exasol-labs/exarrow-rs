# Feature: Parquet Import Type Coverage

Specifies which Arrow field types Parquet import accepts when it creates the target table, and how CSV-path Parquet import converts values. The path choice and the Parquet IMPORT statement are specified in `import-export/parquet-io`.

## Background

- The driver selects the native path on Exasol 2025.1.11 and newer, and the CSV path on older servers or with `with_native_parquet(Some(false))`.
- CSV-path Parquet import converts each `RecordBatch` with the same import CSV conversion as RecordBatch import.
- Native auto-create accepts only the Arrow types that native Parquet import reads, and rejects every other field before `CREATE TABLE`.

## Scenarios

### Scenario: CSV-path Parquet import converts values as RecordBatch import does

* *GIVEN* a Parquet file, written with its Arrow schema, with the fields `ID` (Int64), `N` (`Decimal128(5, 2)`), `S` (`Dictionary(Int32, Utf8)`), and `B` (`FixedSizeBinary(2)`), holding the row `(1, -0.05, 'a;b', 0xABCD)`
* *AND* no table `T` exists
* *WHEN* the application imports the file into `T` with `import_from_parquet` and `ParquetImportOptions` set with `with_native_parquet(Some(false))`, `with_create_table_if_not_exists(true)`, and `with_column_separator(';')`
* *THEN* the driver SHALL create `T` with the columns `N DECIMAL(5,2)`, `S VARCHAR(2000000)`, and `B VARCHAR(2000000)`
* *AND* `T` SHALL hold one row with `N` `-0.05`, `S` `a;b`, and `B` `abcd`

### Scenario: Native Parquet import with table creation rejects Utf8View, dictionary, BinaryView, FixedSizeBinary, and interval columns

* *GIVEN* six Parquet files, each written with its Arrow schema, each with the field `ID` (Int64) and one field `X` of the Arrow type Utf8View, `Dictionary(Int32, Utf8)`, BinaryView, `FixedSizeBinary(16)`, `Interval(YearMonth)`, or `Interval(DayTime)`
* *AND* no table `T` exists
* *WHEN* the application imports each file into `T` with `import_from_parquet` and `ParquetImportOptions` set with `with_native_parquet(Some(true))` and `with_create_table_if_not_exists(true)`
* *THEN* each import SHALL fail with `ImportError::SchemaInferenceError` before the driver sends `CREATE TABLE` or `IMPORT`
* *AND* each error message SHALL name the column `X` and its Arrow type, and SHALL name `with_native_parquet(Some(false))` as the way to import the file
* *AND* `T` SHALL NOT exist after the imports

### Scenario: CSV-path Parquet import rejects an INTERVAL column from a file without an Arrow schema

* *GIVEN* a Parquet file written without Arrow schema metadata, with the fields `ID` (Int64) and `X` (`Interval(YearMonth)`) holding 14 months
* *AND* no table `T` exists
* *WHEN* the application imports the file into `T` with `import_from_parquet` and `ParquetImportOptions` set with `with_native_parquet(Some(false))` and `with_create_table_if_not_exists(true)`
* *THEN* the import SHALL fail with `ImportError::SchemaInferenceError` that names the column `X`, before the driver sends `CREATE TABLE` or `IMPORT`
* *AND* the error message SHALL state that the column would lose its month part
* *AND* `T` SHALL NOT exist
