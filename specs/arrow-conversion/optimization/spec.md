# Feature: Arrow Conversion Optimization

Specifies performance optimizations for arrow conversion including streaming deserialization, null-free fast paths, string buffer optimization, and validation gates.

## Background

Exasol returns column-major JSON data that must be transposed to row-major format for Arrow conversion. Columns may or may not contain NULL values, and null statistics are available to guide optimization. Large VARCHAR columns require buffer pre-allocation for efficient conversion. ALL changes to arrow conversion MUST pass comprehensive testing, including DECIMAL overflow tests, null handling fast path tests, clippy with zero warnings, and performance benchmarks demonstrating no regression.

## Scenarios

### Scenario: Custom deserializer application

* *GIVEN* Exasol returns column-major JSON data
* *WHEN* deserializing `FetchResponseData` or `ResultSetData` from JSON
* *THEN* the `data` field SHALL use a custom deserializer that transposes during parsing
* *AND* the resulting `Vec<Vec<Value>>` SHALL be in row-major format: `data[row_idx][col_idx]`

### Scenario: Transposition during deserialization

* *GIVEN* Exasol returns column-major JSON data
* *WHEN* Exasol returns column-major JSON data (columns as outer array)
* *THEN* the custom deserializer SHALL iterate columns and distribute values to rows
* *AND* it SHALL use `DeserializeSeed` pattern to accumulate across columns
* *AND* no post-hoc transposition step SHALL be required

### Scenario: Empty result handling

* *GIVEN* Exasol returns column-major JSON data
* *WHEN* deserializing an empty result set
* *THEN* it SHALL return an empty `Vec<Vec<Value>>`
* *AND* it SHALL handle zero columns gracefully

### Scenario: Single column handling

* *GIVEN* Exasol returns column-major JSON data
* *WHEN* deserializing a result with one column
* *THEN* each row SHALL contain exactly one value
* *AND* the format SHALL be consistent with multi-column results

### Scenario: All-values-present detection

* *GIVEN* a column's null statistics are available
* *WHEN* a column contains no NULL values
* *THEN* it SHALL detect this condition before building the array
* *AND* it SHALL skip null bitmap construction for the column
* *AND* it SHALL use direct array construction without validity buffer

### Scenario: Null-presence hint

* *GIVEN* a column's null statistics are available
* *WHEN* result metadata indicates null presence or absence
* *THEN* it SHALL use this hint to select optimal conversion path
* *AND* it SHALL fall back to null-safe path when hint is unavailable

### Scenario: Large string pre-allocation

* *GIVEN* large VARCHAR columns are being converted
* *WHEN* converting large VARCHAR columns
* *THEN* it SHALL estimate buffer size based on average string length
* *AND* it SHALL pre-allocate string buffers to reduce reallocations

### Scenario: String array builder hints

* *GIVEN* large VARCHAR columns are being converted
* *WHEN* building Arrow string arrays
* *THEN* it SHALL provide capacity hints to StringBuilder
* *AND* it SHALL minimize intermediate string copies

### Scenario: Unit test requirement

* *GIVEN* arrow conversion changes are complete
* *WHEN* arrow conversion optimization is complete
* *THEN* `cargo test` MUST pass with zero failures
* *AND* DECIMAL overflow tests MUST be present and passing
* *AND* null handling fast path tests MUST be present and passing

### Scenario: Performance validation

* *GIVEN* arrow conversion changes are complete
* *WHEN* arrow conversion optimization is complete
* *THEN* benchmarks MUST demonstrate performance improvement
* *AND* no performance regression MUST occur for existing workloads

### Scenario: Code quality requirement

* *GIVEN* arrow conversion changes are complete
* *WHEN* arrow conversion optimization is complete
* *THEN* `cargo clippy --all-targets --all-features -- -W clippy::all` MUST produce zero warnings
* *AND* all new code MUST follow project conventions
