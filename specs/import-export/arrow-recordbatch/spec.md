# Feature: Arrow RecordBatch Import/Export

Specifies Arrow RecordBatch import and export between applications and Exasol tables.

## Background

RecordBatch import converts data to CSV text with the import CSV conversion and streams it through the HTTP tunnel. Export converts the CSV from Exasol into RecordBatches whose schemas reflect Exasol column types.

## Scenarios

### Scenario: Import single RecordBatch into table

* *GIVEN* an Arrow RecordBatch is available in memory
* *WHEN* user calls import_from_record_batch with RecordBatch
* *THEN* system SHALL convert RecordBatch to CSV format
* *AND* system SHALL stream CSV through HTTP tunnel

### Scenario: Import stream of RecordBatches

* *GIVEN* a Stream of RecordBatches is available
* *WHEN* user provides Stream of RecordBatches
* *THEN* system SHALL convert and stream each batch sequentially

### Scenario: Import from Arrow IPC file

* *GIVEN* an Arrow IPC file exists on disk
* *WHEN* user calls import_from_arrow_ipc with file path
* *THEN* system SHALL read Arrow IPC file and convert to CSV
* *AND* system SHALL stream through HTTP tunnel

### Scenario: Arrow type conversion preserves data

* *GIVEN* a RecordBatch contains various Arrow types
* *WHEN* RecordBatch is converted to CSV representation
* *THEN* system SHALL convert to appropriate CSV representation
* *AND* NULL values and special characters SHALL be escaped correctly

### Scenario: Export to RecordBatch stream

* *GIVEN* an Exasol table or query has data to export
* *WHEN* user calls export_to_record_batches
* *THEN* system SHALL return Stream of RecordBatches
* *AND* batches SHALL be created with configurable size

### Scenario: Export to Arrow IPC file

* *GIVEN* an output file path is specified for Arrow IPC
* *WHEN* user calls export_to_arrow_ipc with file path
* *THEN* system SHALL write Arrow IPC format file

### Scenario: Export preserves schema from Exasol metadata

* *GIVEN* an export operation has completed
* *WHEN* export completes
* *THEN* Arrow schema SHALL reflect Exasol column types

### Scenario: Configurable batch size for streaming

* *GIVEN* streaming export is configured with batch size
* *WHEN* user specifies batch size option
* *THEN* RecordBatches SHALL contain approximately specified number of rows

### Scenario: TLS for HTTP transport is configured uniformly via use_tls

* *GIVEN* an application configures Arrow RecordBatch import or export options
* *WHEN* the application enables or disables TLS on the HTTP transport tunnel
* *THEN* the option builders `ArrowImportOptions` and `ArrowExportOptions` SHALL expose a single fluent method `use_tls(bool)` to control HTTP-transport TLS
* *AND* the option builders MUST NOT expose `with_encryption()` (flag-style) or `use_encryption(bool)` aliases
* *AND* the underlying option struct field SHALL be named `use_tls` (renamed from `use_encryption`) so that public field access uses the same vocabulary as the builder
* *AND* the default value of `use_tls` SHALL remain `false`

### Scenario: The import CSV conversion covers every Arrow type that has an Exasol column type

* *GIVEN* a RecordBatch with one column of each of these Arrow types: Boolean, Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float32, Float64, Utf8, LargeUtf8, Utf8View, `Dictionary(Int8, Utf8)`, `Dictionary(UInt64, LargeUtf8)`, Binary, LargeBinary, BinaryView, `FixedSizeBinary(3)`, Date32, Date64, Timestamp of each time unit with and without a time zone, `Decimal128(36, 2)`, `Decimal256(36, 2)`, `Interval(YearMonth)`, `Interval(DayTime)`, and `Interval(MonthDayNano)`, plus the columns `Decimal128(38, 2)`, `Decimal128(38, -5)`, and `Decimal256(76, -10)`, which have no Exasol column type
* *WHEN* the RecordBatch is converted to CSV for `import_from_record_batch`
* *THEN* every column SHALL convert without an error, a Date64 value SHALL become the `YYYY-MM-DD` text of the day that holds it, so `1_577_836_800_000` ms becomes `2020-01-01`, and a Decimal256 value SHALL become its decimal text with the column's scale, so `-123456` at scale 2 becomes `-1234.56`
* *AND* a decimal value with a negative scale SHALL become its integer digits followed by one zero per unit of negative scale, so `Decimal128(38, -5)` holding 10^37 becomes `1` followed by 42 zeros, and `Decimal256(76, -10)` holding 10^76 - 1 becomes 76 nines followed by 10 zeros
* *AND* a Dictionary value SHALL become the text of the dictionary value its key refers to, a null key or a null dictionary value SHALL become the null marker, and a Utf8View value SHALL be written as the same Utf8 value is written, and a BinaryView or FixedSizeBinary value SHALL become the lowercase hexadecimal text of its bytes, as a Binary value does
* *AND* a column whose Arrow type has no conversion, such as `Time32(Second)`, `Duration(Second)`, or `List(Int32)`, SHALL fail the import CSV conversion with `ImportError::ConversionError` that names the Arrow type
