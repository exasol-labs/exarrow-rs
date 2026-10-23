# Feature: Exasol to Arrow Field Metadata

Defines the Exasol type information that the driver attaches to Arrow fields as field metadata, and the nullability of result set fields.

## Background

Every field of the result set metadata schema and of each batch schema carries the Exasol type that Exasol reports for its column as Arrow field metadata, on the native and the WebSocket transport.
The result set metadata schema is the schema that the result set metadata holds, and a batch schema is the schema of one RecordBatch.
The result set column metadata of both protocols holds only the column name and the data type, so the driver receives no column nullability from Exasol.
The type mappings themselves are specified in `type-mapping/exasol-to-arrow`.

## Scenarios

### Scenario: Column metadata

* *GIVEN* a table `M` with the columns `AMOUNT DECIMAL(10,2) NOT NULL`, `NAME VARCHAR(50)`, `CODE CHAR(3)`, `RATE DOUBLE`, `ACTIVE BOOLEAN`, `D DATE`, `TS TIMESTAMP`, and `TSL TIMESTAMP WITH LOCAL TIME ZONE` that holds one row
* *WHEN* `SELECT * FROM M` runs over the native transport or the WebSocket transport and the caller reads the result set metadata schema and the batch schema of the first RecordBatch
* *THEN* the fields `AMOUNT`, `NAME`, `CODE`, `RATE`, `ACTIVE`, `D`, and `TS` of both schemas SHALL carry the key `exasol:type_name` with the values `DECIMAL`, `VARCHAR`, `CHAR`, `DOUBLE`, `BOOLEAN`, `DATE`, and `TIMESTAMP`
* *AND* the field `TSL` of both schemas SHALL carry the key `exasol:type_name` with the value `TIMESTAMP WITH LOCAL TIME ZONE` over the WebSocket transport and the value `TIMESTAMP` over the native transport, because Exasol sends the native wire type `T_TIMESTAMP` (21) with the same column metadata for a `TIMESTAMP` column and a `TIMESTAMP WITH LOCAL TIME ZONE` column
* *AND* the field `AMOUNT` SHALL carry the key `exasol:precision` with the value `10` and the key `exasol:scale` with the value `2`, and the fields `NAME` and `CODE` SHALL carry the key `exasol:size` with the values `50` and `3`
* *AND* the fields `RATE`, `ACTIVE`, `D`, `TS`, and `TSL` SHALL NOT carry the keys `exasol:precision`, `exasol:scale`, or `exasol:size`

### Scenario: Schema documentation

* *GIVEN* the result set of `SELECT * FROM M` from the scenario "Column metadata", over the native transport or the WebSocket transport
* *WHEN* the caller passes the field metadata of a field of the result set metadata schema to `TypeMapper::from_field_metadata`
* *THEN* it SHALL return `ExasolType::Decimal` with precision `10` and scale `2` for the field `AMOUNT`
* *AND* it SHALL return `ExasolType::Varchar` with size `50` for the field `NAME`
* *AND* it SHALL return `ExasolType::Timestamp` for the field `TSL`, with `with_local_time_zone` set to true over the WebSocket transport and set to false over the native transport

### Scenario: Result set fields are nullable when Exasol reports no nullability

* *GIVEN* the table `M` from the scenario "Column metadata", whose column `AMOUNT` is declared `NOT NULL`
* *WHEN* `SELECT * FROM M` runs over the native transport or the WebSocket transport
* *THEN* every field of the result set metadata schema and of the first RecordBatch SHALL be nullable, including `AMOUNT`

### Scenario: HASHTYPE field metadata carries the byte size of the column

* *GIVEN* a native TCP or WebSocket session exists that uses the default `HASHTYPE_FORMAT` (`HEX`)
* *WHEN* the query `SELECT CAST('0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' AS HASHTYPE(32 BYTE)) AS H` returns one row
* *THEN* `TypeMapper::from_field_metadata` SHALL return `ExasolType::Hashtype` with `byte_size` `32` for the field `H` of the result set metadata schema and for the field `H` of the batch schema of the first RecordBatch
* *AND* both fields SHALL carry the key `exasol:type_name` with the value `HASHTYPE`

### Scenario: INTERVAL DAY TO SECOND field metadata carries the fractional second precision

* *GIVEN* a native TCP or WebSocket session exists
* *WHEN* the query `SELECT CAST('3 04:05:06.789123' AS INTERVAL DAY(2) TO SECOND(6)) AS IV` returns one row
* *THEN* `TypeMapper::from_field_metadata` SHALL return `ExasolType::IntervalDayToSecond` with `precision` `6` for the field `IV` of the result set metadata schema and for the field `IV` of the batch schema of the first RecordBatch
* *AND* both fields SHALL carry the key `exasol:type_name` with the value `INTERVAL DAY TO SECOND`

### Scenario: GEOMETRY field metadata carries the SRID that Exasol reports

* *GIVEN* a native TCP or WebSocket session exists
* *WHEN* the query `SELECT CAST('POINT(1 2)' AS GEOMETRY(4326)) AS G1, CAST('POINT(1 2)' AS GEOMETRY) AS G0` returns one row
* *THEN* over the WebSocket transport, the fields `G1` and `G0` of the result set metadata schema and of the batch schema of the first RecordBatch SHALL carry the key `exasol:srid` with the values `4326` and `0`, and `TypeMapper::from_field_metadata` SHALL return `ExasolType::Geometry` with `srid` `Some(4326)` and `Some(0)`
* *AND* over the native transport, neither field SHALL carry the key `exasol:srid`, and `TypeMapper::from_field_metadata` SHALL return `ExasolType::Geometry` with `srid` `None`, because the native column metadata carries no SRID
* *AND* on both transports, the fields `G1` and `G0` of both schemas SHALL carry the key `exasol:type_name` with the value `GEOMETRY`
