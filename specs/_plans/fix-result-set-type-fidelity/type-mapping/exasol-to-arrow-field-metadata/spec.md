# Feature: Exasol to Arrow Field Metadata

Defines the Exasol type information that the driver attaches to Arrow fields as field metadata, and the nullability of result set fields.

## Background

Every field of the result set metadata schema and of each batch schema carries the Exasol type that Exasol reports for its column as Arrow field metadata, on the native and the WebSocket transport.
The result set metadata schema is the schema that the result set metadata holds, and a batch schema is the schema of one RecordBatch.
The result set column metadata of both protocols holds only the column name and the data type, so the driver receives no column nullability from Exasol.
The type mappings themselves are specified in `type-mapping/exasol-to-arrow`.

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Field metadata on the native transport keeps the decoded Arrow data type

* *GIVEN* an authenticated native TCP session exists
* *WHEN* the query `SELECT CAST('3 04:05:06.789' AS INTERVAL DAY TO SECOND) AS IV` returns one row
* *THEN* the field `IV` of the batch schema of the first RecordBatch SHALL have the Arrow data type `Utf8`, which `native-client/type-conversion` specifies for `T_interval_day`
* *AND* the field `IV` SHALL carry the key `exasol:type_name` with the value `INTERVAL DAY TO SECOND`
* *AND* the column `IV` SHALL hold one non-null string value
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: HASHTYPE field metadata carries the byte size of the column

* *GIVEN* a native TCP or WebSocket session exists that uses the default `HASHTYPE_FORMAT` (`HEX`)
* *WHEN* the query `SELECT CAST('0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' AS HASHTYPE(32 BYTE)) AS H` returns one row
* *THEN* `TypeMapper::from_field_metadata` SHALL return `ExasolType::Hashtype` with `byte_size` `32` for the field `H` of the result set metadata schema and for the field `H` of the batch schema of the first RecordBatch
* *AND* both fields SHALL carry the key `exasol:type_name` with the value `HASHTYPE`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: INTERVAL DAY TO SECOND field metadata carries the fractional second precision

* *GIVEN* a native TCP or WebSocket session exists
* *WHEN* the query `SELECT CAST('3 04:05:06.789123' AS INTERVAL DAY(2) TO SECOND(6)) AS IV` returns one row
* *THEN* `TypeMapper::from_field_metadata` SHALL return `ExasolType::IntervalDayToSecond` with `precision` `6` for the field `IV` of the result set metadata schema and for the field `IV` of the batch schema of the first RecordBatch
* *AND* both fields SHALL carry the key `exasol:type_name` with the value `INTERVAL DAY TO SECOND`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: GEOMETRY field metadata carries the SRID that Exasol reports

* *GIVEN* a native TCP or WebSocket session exists
* *WHEN* the query `SELECT CAST('POINT(1 2)' AS GEOMETRY(4326)) AS G1, CAST('POINT(1 2)' AS GEOMETRY) AS G0` returns one row
* *THEN* over the WebSocket transport, the fields `G1` and `G0` of the result set metadata schema and of the batch schema of the first RecordBatch SHALL carry the key `exasol:srid` with the values `4326` and `0`, and `TypeMapper::from_field_metadata` SHALL return `ExasolType::Geometry` with `srid` `Some(4326)` and `Some(0)`
* *AND* over the native transport, neither field SHALL carry the key `exasol:srid`, and `TypeMapper::from_field_metadata` SHALL return `ExasolType::Geometry` with `srid` `None`, because the native column metadata carries no SRID
* *AND* on both transports, the fields `G1` and `G0` of both schemas SHALL carry the key `exasol:type_name` with the value `GEOMETRY`
<!-- /DELTA:NEW -->
