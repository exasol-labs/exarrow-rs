# type-mapping Specification

## Purpose

Defines the bidirectional type mapping between Exasol data types and Apache Arrow data types, ensuring accurate data
representation and type compatibility validation.

## Requirements

### Requirement: Exasol to Arrow Type Mapping

The system SHALL define a complete mapping from Exasol data types to Apache Arrow data types.

#### Scenario: Numeric types mapping

- **WHEN** mapping Exasol numeric types
- **THEN** DECIMAL(p, s) with p in range 1-36 SHALL map to Arrow Decimal128(p, s)
- **AND** Exasol DECIMAL precision SHALL NOT exceed 36 (Exasol's documented maximum)
- **AND** DOUBLE SHALL map to Arrow Float64
- **AND** INTEGER SHALL map to Arrow Int64
- **AND** SMALLINT SHALL map to Arrow Int32 or Int16

#### Scenario: String types mapping

- **WHEN** mapping Exasol string types
- **THEN** VARCHAR(n) SHALL map to Arrow Utf8
- **AND** CHAR(n) SHALL map to Arrow Utf8 with padding handling
- **AND** CLOB SHALL map to Arrow LargeUtf8

#### Scenario: Date and time types mapping

- **WHEN** mapping Exasol temporal types
- **THEN** DATE SHALL map to Arrow Date32
- **AND** TIMESTAMP with fractional precision 0-9 SHALL map to Arrow Timestamp with appropriate TimeUnit
- **AND** TIMESTAMP precision 0-3 SHOULD map to Millisecond, 4-6 to Microsecond, 7-9 to Nanosecond
- **AND** INTERVAL types SHALL map to Arrow Duration or Interval types

#### Scenario: Boolean type mapping

- **WHEN** mapping Exasol BOOLEAN type
- **THEN** it SHALL map to Arrow Boolean

#### Scenario: Binary types mapping

- **WHEN** mapping Exasol binary types
- **THEN** VARBINARY SHALL map to Arrow Binary
- **AND** BLOB SHALL map to Arrow LargeBinary

#### Scenario: NULL handling

- **WHEN** a column allows NULL values
- **THEN** the Arrow field SHALL be marked as nullable
- **AND** NULL values SHALL be represented in Arrow null bitmaps

### Requirement: Arrow to Exasol Type Mapping

The system SHALL define mappings from Arrow types to Exasol types for parameter binding.

#### Scenario: Arrow numeric to Exasol

- **WHEN** binding Arrow numeric types
- **THEN** Arrow Int64 SHALL convert to Exasol INTEGER or DECIMAL
- **AND** Arrow Float64 SHALL convert to Exasol DOUBLE
- **AND** Arrow Decimal128 SHALL convert to Exasol DECIMAL

#### Scenario: Arrow string to Exasol

- **WHEN** binding Arrow string types
- **THEN** Arrow Utf8 SHALL convert to Exasol VARCHAR
- **AND** Arrow LargeUtf8 SHALL convert to Exasol VARCHAR or CLOB

#### Scenario: Arrow temporal to Exasol

- **WHEN** binding Arrow temporal types
- **THEN** Arrow Date32 SHALL convert to Exasol DATE
- **AND** Arrow Timestamp SHALL convert to Exasol TIMESTAMP

### Requirement: Type Precision and Scale Handling

The system SHALL preserve precision and scale information for numeric types.

#### Scenario: DECIMAL precision mapping

- **WHEN** mapping Exasol DECIMAL(p, s) type
- **THEN** Arrow Decimal128 SHALL preserve precision p (1-36) and scale s
- **AND** it SHALL always use Decimal128 since Exasol precision never exceeds 36
- **AND** Decimal256 SHALL NOT be used for Exasol-originated types

#### Scenario: Overflow detection

- **WHEN** a value exceeds Arrow type capacity
- **THEN** it SHALL return a type conversion error
- **AND** it SHALL specify which column and value caused the overflow

### Requirement: Complex Type Mapping

The system SHALL handle Exasol's complex types appropriately.

#### Scenario: GEOMETRY type handling

- **WHEN** encountering Exasol GEOMETRY types
- **THEN** it SHALL map to Arrow Binary or Utf8 (WKT/WKB format)
- **AND** it SHALL document the encoding used

#### Scenario: Array type handling

- **WHEN** Exasol array types are encountered (if supported)
- **THEN** it SHALL map to Arrow List types
- **AND** it SHALL handle nested type mappings recursively

#### Scenario: Unsupported types

- **WHEN** an unsupported Exasol type is encountered
- **THEN** it SHALL return a clear error message
- **AND** it SHALL specify the unsupported type name
- **AND** it SHALL provide guidance on workarounds if available

### Requirement: Type Compatibility Validation

The system SHALL validate type compatibility during conversions.

#### Scenario: Lossless conversion validation

- **WHEN** converting between types
- **THEN** it SHALL prefer lossless conversions
- **AND** it SHALL warn or error on lossy conversions

#### Scenario: String length validation

- **WHEN** binding string values
- **THEN** it SHALL validate length against Exasol VARCHAR limits
- **AND** it SHALL truncate or error as appropriate based on configuration

#### Scenario: Timestamp timezone handling

- **WHEN** converting timestamps
- **THEN** it SHALL handle timezone-aware and timezone-naive timestamps correctly
- **AND** it SHALL preserve or convert timezones as specified
- **AND** it SHALL document timezone handling behavior

### Requirement: Type Metadata Preservation

The system SHALL preserve type metadata in Arrow schemas.

#### Scenario: Column metadata

- **WHEN** creating Arrow schemas from Exasol result sets
- **THEN** it SHALL include Exasol type information in Arrow field metadata
- **AND** it SHALL preserve nullability information
- **AND** it SHALL include precision/scale for applicable types

#### Scenario: Schema documentation

- **WHEN** schema is inspected
- **THEN** it SHALL provide access to original Exasol type names
- **AND** it SHALL expose type mapping used for each column

### Requirement: Exasol Data Type Boundaries

The system SHALL enforce Exasol's documented data type limits when generating DDL or validating type mappings.

#### Scenario: VARCHAR type boundaries

- **WHEN** mapping to Exasol VARCHAR
- **THEN** the maximum length SHALL be 2,000,000 characters
- **AND** values exceeding this limit SHALL be truncated or rejected based on configuration

#### Scenario: CHAR type boundaries

- **WHEN** mapping to Exasol CHAR
- **THEN** the maximum length SHALL be 2,000 characters
- **AND** CHAR is fixed-width with space padding

#### Scenario: DECIMAL type boundaries

- **WHEN** mapping to Exasol DECIMAL(p, s)
- **THEN** precision SHALL be in range 1-36
- **AND** scale SHALL be in range 0-36
- **AND** scale SHALL NOT exceed precision

#### Scenario: TIMESTAMP type boundaries

- **WHEN** mapping to Exasol TIMESTAMP
- **THEN** fractional seconds precision SHALL be in range 0-9
- **AND** TIMESTAMP WITH LOCAL TIME ZONE SHALL be used for timezone-aware timestamps

#### Scenario: Integer type mappings for DDL generation

- **WHEN** mapping Arrow integer types to Exasol DDL
- **THEN** Int8, Int16, Int32 SHALL map to DECIMAL(18,0)
- **AND** Int64 SHALL map to DECIMAL(36,0)
- **AND** UInt8, UInt16, UInt32 SHALL map to DECIMAL(18,0)
- **AND** UInt64 SHALL map to DECIMAL(36,0)

#### Scenario: Floating point type mappings for DDL generation

- **WHEN** mapping Arrow floating point types to Exasol DDL
- **THEN** Float32 and Float64 SHALL map to DOUBLE

#### Scenario: INTERVAL type boundaries

- **WHEN** mapping to Exasol INTERVAL types
- **THEN** INTERVAL DAY TO SECOND fractional precision SHALL be in range 0-9
- **AND** both INTERVAL types use fixed 8-byte storage
