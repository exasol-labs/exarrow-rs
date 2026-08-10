# Feature: Native Result Set Type Conversion

Specifies the per-type binary-to-Arrow conversion rules the native TCP protocol applies to each column of a result set, once `native-client/result-sets` has parsed the column-major frame and its metadata.

## Background

Each column's data is contiguous in the binary stream, matching Arrow's memory layout, so conversion is a direct read into the matching Arrow array type rather than an intermediate representation. Null handling is specified in `native-client/result-sets`.

## Scenarios

### Scenario: Direct binary to Arrow conversion for numeric types

* *GIVEN* a result set contains columns of type `T_double` (8), `T_decimal` (6), `T_integer` (5), `T_smallint` (4), or `T_real` (7)
* *WHEN* parsing column data
* *THEN* the system SHALL read the binary values directly into Arrow numeric arrays (`T_double`/`T_real` → Float64, `T_decimal` → Decimal128, `T_integer` → Int64, `T_smallint` → Int32)
* *AND* the system SHALL handle little-endian to native byte order conversion
* *AND* decimal values SHALL preserve precision and scale

### Scenario: Direct binary to Arrow conversion for string types

* *GIVEN* a result set contains columns of type `T_char` (10) with or without the vcFlag varchar bit `0x01` set
* *WHEN* parsing column data
* *THEN* the system SHALL read length-prefixed UTF-8 strings into Arrow Utf8 or LargeUtf8 arrays
* *AND* the system SHALL decode every string payload as UTF-8 regardless of the vcFlag UTF-8 bit `0x10`, because Exasol transmits native-protocol string payloads as UTF-8
* *AND* `VARCHAR(n)` and `CHAR(n)` SHALL map to the same Arrow string type, so the varchar bit SHALL affect only the reported Exasol type name

### Scenario: Direct binary to Arrow conversion for temporal types

* *GIVEN* a result set contains columns of type `T_date` (14), `T_timestamp` (21), or `T_timestamp_utc` (125)
* *WHEN* parsing column data
* *THEN* the system SHALL convert dates to Arrow Date32 arrays
* *AND* the system SHALL convert timestamps to Arrow Timestamp arrays with appropriate time unit
* *AND* `T_timestamp_utc` SHALL map to Arrow Timestamp with UTC timezone

### Scenario: Direct binary to Arrow conversion for interval types

* *GIVEN* a result set contains columns of type `T_interval_year` (16) or `T_interval_day` (17)
* *WHEN* parsing column data
* *THEN* the system SHALL convert interval values to Arrow Utf8 arrays (string representation)

### Scenario: Direct binary to Arrow conversion for binary and geometry types

* *GIVEN* a result set contains columns of type `T_binary` (15), `T_hashtype` (126), or `T_geometry` (123)
* *WHEN* parsing column data
* *THEN* `T_binary` SHALL map to Arrow Binary arrays
* *AND* `T_hashtype` and `T_geometry` SHALL map to Arrow Utf8 arrays

### Scenario: Boolean type conversion

* *GIVEN* a result set contains columns of type `T_boolean`
* *WHEN* parsing column data
* *THEN* the system SHALL convert boolean values to Arrow Boolean arrays
