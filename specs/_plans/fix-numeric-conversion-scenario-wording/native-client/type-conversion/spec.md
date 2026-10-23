# Feature: Native Result Set Type Conversion

Specifies the per-type binary-to-Arrow conversion rules the native TCP protocol applies to each column of a result set, once `native-client/result-sets` has parsed the column-major frame and its metadata.

## Background

Each column's data is contiguous in the frame, so conversion reads it directly into the column's Arrow array, with no intermediate array.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Direct binary to Arrow conversion for numeric types

* *GIVEN* a result set contains columns of type `T_double` (8), `T_decimal` (6), `T_smalldecimal` (63), `T_bigdecimal` (64), `T_integer` (5), `T_smallint` (4), or `T_real` (7)
* *WHEN* parsing column data
* *THEN* the system SHALL read the binary values directly into Arrow numeric arrays (`T_double`/`T_real` → Float64, `T_decimal` → Decimal128 with the precision and scale of the column description, `T_smalldecimal` → Decimal128 read from a 4-byte value and `T_bigdecimal` → Decimal128 read from a 16-byte value, both with the precision and scale of the column description, `T_integer` → Decimal128(18, 0), `T_smallint` → Decimal128(9, 0)), converting each little-endian value to native byte order
* *AND* the system SHALL read each `T_integer` value as an 8-byte integer and each `T_smallint` value as a 4-byte integer into the Decimal128 array
* *AND* the Arrow data type of a `T_integer` or `T_smallint` field SHALL equal the Arrow data type of the DECIMAL(18,0) or DECIMAL(9,0) that the driver reports for the column in the result set metadata, because the column description of these wire types carries no precision or scale
* *AND* decimal values SHALL preserve precision and scale
<!-- /DELTA:CHANGED -->
