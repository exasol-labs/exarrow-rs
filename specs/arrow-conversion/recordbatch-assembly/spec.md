# Feature: RecordBatch Assembly

Specifies the assembly of Arrow RecordBatches from converted data, including JSON-to-Arrow conversion, null handling, schema consistency, and error recovery.

## Background

JSON result data is type-converted to Arrow arrays before assembly into RecordBatches. A schema is available for each result set, either provided upfront or inferred from JSON metadata. The system SHALL convert Exasol WebSocket API JSON responses to Apache Arrow RecordBatch format using row-major input data, construct valid RecordBatch instances, correctly represent NULL values, ensure schema consistency throughout conversion, and provide detailed error information when conversion fails.

## Scenarios

### Scenario: Column data conversion

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* converting JSON result data to Arrow
* *THEN* it SHALL receive data in row-major format (`data[row_idx][col_idx]`)
* *AND* it SHALL apply correct type conversions based on schema
* *AND* it SHALL build Arrow arrays for each column

### Scenario: Row-oriented to columnar conversion

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* JSON data arrives in row-major format from the deserializer
* *THEN* it SHALL iterate rows and distribute values to column builders
* *AND* it SHALL maintain data ordering and alignment

### Scenario: Column-oriented conversion

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* JSON data is already in columnar format
* *THEN* it SHALL efficiently map to Arrow columnar structure
* *AND* it SHALL minimize data copying

### Scenario: Single batch creation

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* creating a RecordBatch from result data
* *THEN* it SHALL validate all columns have equal length
* *AND* it SHALL attach the correct schema
* *AND* it SHALL produce a valid RecordBatch

### Scenario: Empty RecordBatch

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* result set contains no rows
* *THEN* it SHALL create an empty RecordBatch with correct schema
* *AND* it SHALL have zero rows but valid column definitions

### Scenario: Large batch handling

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* result sets exceed batch size limits
* *THEN* it SHALL split data into multiple RecordBatches
* *AND* it SHALL maintain consistent schema across batches

### Scenario: NULL bitmap construction

* *GIVEN* a schema is available for the result set
* *WHEN* converting columns containing NULLs
* *THEN* it SHALL build Arrow null bitmaps correctly
* *AND* it SHALL mark NULL positions as invalid in the bitmap

### Scenario: JSON null representation

* *GIVEN* a schema is available for the result set
* *WHEN* JSON contains null values
* *THEN* it SHALL detect JSON nulls regardless of how they're encoded
* *AND* it SHALL convert them to Arrow nulls

### Scenario: All-null columns

* *GIVEN* a schema is available for the result set
* *WHEN* a column contains only NULL values
* *THEN* it SHALL create a valid Arrow array with all nulls
* *AND* it SHALL handle this efficiently without unnecessary allocations

### Scenario: Schema validation

* *GIVEN* a schema is available for the result set
* *WHEN* converting data with known schema
* *THEN* it SHALL validate data types match schema expectations
* *AND* it SHALL return errors for type mismatches

### Scenario: Dynamic schema inference

* *GIVEN* a schema is available for the result set
* *WHEN* schema is not provided upfront
* *THEN* it SHALL infer Arrow schema from JSON metadata
* *AND* it SHALL apply correct type mappings

### Scenario: Conversion error reporting

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* a data conversion fails
* *THEN* it SHALL report the column name and row number
* *AND* it SHALL include the problematic value
* *AND* it SHALL explain what conversion was attempted

### Scenario: Partial conversion handling

* *GIVEN* JSON result data has been type-converted to Arrow arrays
* *WHEN* some rows fail conversion in a batch
* *THEN* it SHALL have configurable behavior (fail-fast or collect errors)
* *AND* it SHALL provide all error details for debugging
