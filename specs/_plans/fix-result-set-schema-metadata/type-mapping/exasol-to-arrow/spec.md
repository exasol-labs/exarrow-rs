# Feature: Exasol to Arrow

Defines the mapping from Exasol data types to Apache Arrow data types, including complex types and metadata preservation.

<!-- DELTA:CHANGED -->
## Background

Exasol DECIMAL precision does not exceed 36, so Decimal128 is always sufficient. Every field of the result set metadata schema and of each batch schema carries the Exasol type of its column as Arrow field metadata, on the native and the WebSocket transport. The result set metadata schema is the schema that the result set metadata holds, and a batch schema is the schema of one RecordBatch. The result set column metadata of both protocols holds only the column name and the data type, so the driver receives no column nullability from Exasol.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Column metadata

* *GIVEN* a table `M` with the columns `AMOUNT DECIMAL(10,2) NOT NULL`, `NAME VARCHAR(50)`, `CODE CHAR(3)`, `RATE DOUBLE`, `ACTIVE BOOLEAN`, `D DATE`, `TS TIMESTAMP`, and `TSL TIMESTAMP WITH LOCAL TIME ZONE` that holds one row
* *WHEN* `SELECT * FROM M` runs over the native transport or the WebSocket transport and the caller reads the result set metadata schema and the batch schema of the first RecordBatch
* *THEN* each field of both schemas SHALL carry the key `exasol:type_name` with the Exasol type name of its column: `DECIMAL`, `VARCHAR`, `CHAR`, `DOUBLE`, `BOOLEAN`, `DATE`, `TIMESTAMP`, and `TIMESTAMP WITH LOCAL TIME ZONE`
* *AND* the field `AMOUNT` SHALL carry the key `exasol:precision` with the value `10` and the key `exasol:scale` with the value `2`
* *AND* the fields `NAME` and `CODE` SHALL carry the key `exasol:size` with the values `50` and `3`
* *AND* the fields `RATE`, `ACTIVE`, `D`, `TS`, and `TSL` SHALL NOT carry the keys `exasol:precision`, `exasol:scale`, or `exasol:size`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Result set fields are nullable when Exasol reports no nullability

* *GIVEN* the table `M` from the scenario "Column metadata", whose column `AMOUNT` is declared `NOT NULL`
* *WHEN* `SELECT * FROM M` runs over the native transport or the WebSocket transport
* *THEN* every field of the result set metadata schema and of the first RecordBatch SHALL be nullable, including `AMOUNT`
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Schema documentation

* *GIVEN* the result set of `SELECT * FROM M` from the scenario "Column metadata", over the native transport or the WebSocket transport
* *WHEN* the caller passes the field metadata of a field of the result set metadata schema to `TypeMapper::from_field_metadata`
* *THEN* it SHALL return `ExasolType::Decimal` with precision `10` and scale `2` for the field `AMOUNT`
* *AND* it SHALL return `ExasolType::Varchar` with size `50` for the field `NAME`
* *AND* it SHALL return `ExasolType::Timestamp` with `with_local_time_zone` set to true for the field `TSL`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Field metadata on the native transport keeps the decoded Arrow data type

* *GIVEN* an authenticated native TCP session exists
* *WHEN* the query `SELECT CAST('3 04:05:06.789' AS INTERVAL DAY TO SECOND) AS IV` returns one row
* *THEN* the field `IV` of the batch schema of the first RecordBatch SHALL have the Arrow data type `Utf8`, which `native-client/type-conversion` specifies for `T_interval_day`
* *AND* the field `IV` SHALL carry the key `exasol:type_name` with the value `INTERVAL DAY TO SECOND`
* *AND* the column `IV` SHALL hold one non-null string value
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: HASHTYPE field metadata carries the byte size of the column

* *GIVEN* an authenticated native TCP session exists
* *WHEN* the query `SELECT CAST('0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' AS HASHTYPE(32 BYTE)) AS H` returns one row
* *THEN* `TypeMapper::from_field_metadata` SHALL return `ExasolType::Hashtype` with `byte_size` `32` for the field `H` of the result set metadata schema and for the field `H` of the batch schema of the first RecordBatch
* *AND* both fields SHALL carry the key `exasol:type_name` with the value `HASHTYPE`
<!-- /DELTA:NEW -->
