# Feature: Boundaries and Validation

Specifies type compatibility validation rules and Exasol data type boundary enforcement for DDL generation and type mapping.

<!-- DELTA:CHANGED -->
## Background

The system enforces Exasol's documented data type limits during DDL generation and type mapping. Conversions prefer lossless transformations and warn or error on lossy conversions, except for February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow` maps to March 1. Exasol type boundaries include: VARCHAR max 2,000,000 characters, CHAR max 2,000 characters (fixed-width with space padding), DECIMAL precision 1-36 with scale 0-36 (scale must not exceed precision), TIMESTAMP fractional seconds precision 0-9, and INTERVAL types with fixed 8-byte storage. Timezone handling for timestamps is explicitly documented and configurable.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Lossless conversion validation

* *GIVEN* a type conversion is being performed
* *WHEN* converting between types
* *THEN* it SHALL prefer lossless conversions
* *AND* it SHALL warn or error on lossy conversions
* *AND* February 29 of a Julian-only leap year, which `type-mapping/exasol-to-arrow` maps to March 1, SHALL follow that scenario without a warning or an error
<!-- /DELTA:CHANGED -->
