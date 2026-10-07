# Feature: Arrow RecordBatch Import/Export

Specifies Arrow RecordBatch import and export capabilities, enabling direct transfer of Arrow-formatted data between applications and Exasol tables.

## Background

Arrow RecordBatch import converts RecordBatch data to CSV format for streaming through the HTTP tunnel. Export converts CSV received from Exasol into Arrow RecordBatches with schemas reflecting Exasol column types. Both single RecordBatches and streams of RecordBatches are supported, along with Arrow IPC file format for persistent storage. Streaming export supports configurable batch sizes. The HTTP-transport TLS knob is exposed as a single fluent method `use_tls(bool)` on both option builders, replacing the earlier flag-style `with_encryption()` (import) and `use_encryption(bool)` (export) names.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Pre-1970 DATE and TIMESTAMP values round-trip through RecordBatch import and export

* *GIVEN* a table `T` with columns `ID DECIMAL(18,0)`, `D DATE`, and `TS TIMESTAMP(6)`
* *AND* a RecordBatch with the fields `ID` (Int64), `D` (Date32), and `TS` (`Timestamp(Microsecond, None)`), holding the rows `(1, -731, -1)`, `(2, -719162, -43200000000)`, and `(3, -25508, -616895999500000)`
* *WHEN* the application imports the RecordBatch into `T` with `import_from_record_batch`, then exports `SELECT ID, D, TS FROM T ORDER BY ID` with `export_to_record_batches` and the RecordBatch's schema
* *THEN* `TO_CHAR(D, 'YYYY-MM-DD')` SHALL return `1968-01-01`, `0001-01-01`, and `1900-03-01` for `ID` 1, 2, and 3
* *AND* `TO_CHAR(TS, 'YYYY-MM-DD HH24:MI:SS.FF6')` SHALL return `1969-12-31 23:59:59.999999`, `1969-12-31 12:00:00.000000`, and `1950-06-15 00:00:00.500000` for `ID` 1, 2, and 3
* *AND* the exported RecordBatches SHALL hold the same `D` and `TS` values as the imported RecordBatch
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: RecordBatch import formats pre-epoch timestamps of every time unit as times before the epoch

* *GIVEN* a RecordBatch with a Timestamp column whose value is -1
* *WHEN* the import converts the value to CSV text for the time units `Second`, `Millisecond`, `Microsecond`, and `Nanosecond`
* *THEN* the text SHALL be `1969-12-31 23:59:59.000000`, `1969-12-31 23:59:59.999000`, `1969-12-31 23:59:59.999999`, and `1969-12-31 23:59:59.999999`, in that order
* *AND* the `Microsecond` value -43200000000 SHALL convert to `1969-12-31 12:00:00.000000`
* *AND* the `Microsecond` value -62135596800000000 SHALL convert to `0001-01-01 00:00:00.000000`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: RecordBatch import rejects DATE and TIMESTAMP values outside Exasol's range

* *GIVEN* a RecordBatch with a Date32 value 2932897 (10000-01-01), a Date32 value -719163 (0000-12-31), a `Timestamp(Second, None)` value 9223372036854775807, or a `Timestamp(Microsecond, None)` value 253402300800000000 (10000-01-01 00:00:00)
* *WHEN* the import converts the value to CSV text
* *THEN* the conversion SHALL return `ImportError::ConversionError` that names the value, and the import SHALL fail
<!-- /DELTA:NEW -->
