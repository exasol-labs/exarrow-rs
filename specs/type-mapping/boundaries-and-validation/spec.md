# Feature: Boundaries and Validation

Specifies type compatibility validation rules and Exasol data type boundary enforcement for DDL generation and type mapping.

## Background

The system enforces Exasol's documented data type limits during DDL generation and type mapping. Conversions prefer lossless transformations and warn or error on lossy conversions. Exasol type boundaries include: VARCHAR max 2,000,000 characters, CHAR max 2,000 characters (fixed-width with space padding), DECIMAL precision 1-36 with scale 0-36 (scale must not exceed precision), TIMESTAMP fractional seconds precision 0-9, and INTERVAL types with fixed 8-byte storage. Timezone handling for timestamps is explicitly documented and configurable.

## Scenarios

### Scenario: Lossless conversion validation

* *GIVEN* a type conversion is being performed
* *WHEN* converting between types
* *THEN* it SHALL prefer lossless conversions
* *AND* it SHALL warn or error on lossy conversions

### Scenario: String length validation

* *GIVEN* a type conversion is being performed
* *WHEN* binding string values
* *THEN* it SHALL validate length against Exasol VARCHAR limits
* *AND* it SHALL truncate or error as appropriate based on configuration

### Scenario: Timestamp timezone handling

* *GIVEN* a type conversion is being performed
* *WHEN* converting timestamps
* *THEN* it SHALL handle timezone-aware and timezone-naive timestamps correctly
* *AND* it SHALL preserve or convert timezones as specified
* *AND* it SHALL document timezone handling behavior

### Scenario: VARCHAR type boundaries

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping to Exasol VARCHAR
* *THEN* the maximum length SHALL be 2,000,000 characters
* *AND* values exceeding this limit SHALL be truncated or rejected based on configuration

### Scenario: CHAR type boundaries

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping to Exasol CHAR
* *THEN* the maximum length SHALL be 2,000 characters
* *AND* CHAR SHALL be fixed-width with space padding

### Scenario: DECIMAL type boundaries

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping to Exasol DECIMAL(p, s)
* *THEN* precision SHALL be in range 1-36
* *AND* scale SHALL be in range 0-36
* *AND* scale SHALL NOT exceed precision

### Scenario: TIMESTAMP type boundaries

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping to Exasol TIMESTAMP
* *THEN* fractional seconds precision SHALL be in range 0-9
* *AND* TIMESTAMP WITH LOCAL TIME ZONE SHALL be used for timezone-aware timestamps

### Scenario: Integer type mappings for DDL generation

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping Arrow integer types to Exasol DDL
* *THEN* Int8, Int16, Int32 SHALL map to DECIMAL(18,0)
* *AND* Int64 SHALL map to DECIMAL(36,0)
* *AND* UInt8, UInt16, UInt32 SHALL map to DECIMAL(18,0)
* *AND* UInt64 SHALL map to DECIMAL(36,0)

### Scenario: Floating point type mappings for DDL generation

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping Arrow floating point types to Exasol DDL
* *THEN* Float32 and Float64 SHALL map to DOUBLE

### Scenario: INTERVAL type boundaries

* *GIVEN* DDL generation is requested for a table
* *WHEN* mapping to Exasol INTERVAL types
* *THEN* INTERVAL DAY TO SECOND fractional precision SHALL be in range 0-9
* *AND* both INTERVAL types SHALL use fixed 8-byte storage
