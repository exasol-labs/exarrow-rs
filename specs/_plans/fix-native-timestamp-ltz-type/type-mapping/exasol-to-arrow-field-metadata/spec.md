# Feature: Exasol to Arrow Field Metadata

Defines the Exasol type information that the driver attaches to Arrow fields as field metadata, and the nullability of result set fields.

## Background

Every field of the result set metadata schema and of each batch schema carries the Exasol type that Exasol reports for its column as Arrow field metadata, on the native and the WebSocket transport.
The result set metadata schema is the schema that the result set metadata holds, and a batch schema is the schema of one RecordBatch.
The result set column metadata of both protocols holds only the column name and the data type, so the driver receives no column nullability from Exasol.
The type mappings themselves are specified in `type-mapping/exasol-to-arrow`.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Column metadata

* *GIVEN* a table `M` with the columns `AMOUNT DECIMAL(10,2) NOT NULL`, `NAME VARCHAR(50)`, `CODE CHAR(3)`, `RATE DOUBLE`, `ACTIVE BOOLEAN`, `D DATE`, `TS TIMESTAMP`, and `TSL TIMESTAMP WITH LOCAL TIME ZONE` that holds one row
* *WHEN* `SELECT * FROM M` runs over the native transport or the WebSocket transport and the caller reads the result set metadata schema and the batch schema of the first RecordBatch
* *THEN* the fields `AMOUNT`, `NAME`, `CODE`, `RATE`, `ACTIVE`, `D`, `TS`, and `TSL` of both schemas SHALL carry the key `exasol:type_name` with the values `DECIMAL`, `VARCHAR`, `CHAR`, `DOUBLE`, `BOOLEAN`, `DATE`, `TIMESTAMP`, and `TIMESTAMP WITH LOCAL TIME ZONE` on both transports
* *AND* the field `AMOUNT` SHALL carry the key `exasol:precision` with the value `10` and the key `exasol:scale` with the value `2`, and the fields `NAME` and `CODE` SHALL carry the key `exasol:size` with the values `50` and `3`
* *AND* the fields `RATE`, `ACTIVE`, `D`, `TS`, and `TSL` SHALL NOT carry the keys `exasol:precision`, `exasol:scale`, or `exasol:size`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Schema documentation

* *GIVEN* the result set of `SELECT * FROM M` from the scenario "Column metadata", over the native transport or the WebSocket transport
* *WHEN* the caller passes the field metadata of a field of the result set metadata schema to `TypeMapper::from_field_metadata`
* *THEN* it SHALL return `ExasolType::Decimal` with precision `10` and scale `2` for the field `AMOUNT`
* *AND* it SHALL return `ExasolType::Varchar` with size `50` for the field `NAME`
* *AND* it SHALL return `ExasolType::Timestamp` with `with_local_time_zone` set to true for the field `TSL`, on both transports
<!-- /DELTA:CHANGED -->
