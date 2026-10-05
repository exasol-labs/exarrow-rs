# Feature: Exasol to Arrow

Defines the mapping from Exasol data types to Apache Arrow data types, including complex types and metadata preservation.

## Background

The system defines a complete mapping from Exasol data types to Apache Arrow data types. Exasol DECIMAL precision does not exceed 36, so Decimal128 is always sufficient. All Arrow fields are marked nullable when the source Exasol column allows NULL values. Type metadata from Exasol (precision, scale, original type names) is preserved in Arrow field metadata for round-trip fidelity. Unsupported or complex types produce clear error messages with guidance on workarounds.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Date and time types mapping

* *GIVEN* an Exasol schema is provided with column type information
* *WHEN* mapping Exasol temporal types
* *THEN* DATE SHALL map to Arrow Date32
* *AND* TIMESTAMP with fractional precision 0-9 SHALL map to Arrow Timestamp with appropriate TimeUnit
* *AND* parameterized TIMESTAMP variants like `TIMESTAMP(3)` SHALL be parsed correctly, ignoring the precision parameter for Arrow mapping
* *AND* INTERVAL types SHALL map to Arrow Duration or Interval types in the conversions that produce Arrow interval values, `TypeMapper::exasol_to_arrow` and the `arrow_conversion` converters, and SHALL map to Utf8 holding Exasol's text form of the value in the native transport's result sets (`native-client/type-conversion`) and in the CSV-based Parquet export (`import-export/parquet-io`)
<!-- /DELTA:CHANGED -->
