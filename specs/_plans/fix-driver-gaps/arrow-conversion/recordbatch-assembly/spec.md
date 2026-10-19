# Feature: RecordBatch Assembly

Specifies how row-major JSON result data becomes Arrow RecordBatches, including null handling, schema consistency, and error reporting.

## Background

JSON result data is type-converted to Arrow arrays before assembly into RecordBatches. A schema is available for each result set, either provided upfront or inferred from JSON metadata.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Conversion error reporting

* *GIVEN* row-major JSON result data and a schema that names each result column
* *WHEN* converting a JSON value to the Arrow type of its column fails
* *THEN* the error MUST name the column by its schema name and by its zero-based column index
* *AND* the error MUST give the zero-based index of the row within the converted batch
* *AND* the error MUST include the JSON value that failed to convert
* *AND* the error MUST name the Arrow data type of the column and the reason the conversion failed
<!-- /DELTA:CHANGED -->
