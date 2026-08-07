# Feature: Binding and Execution

Specifies prepared statement creation and execution behavior for Exasol: creating a statement over the active transport, running it once or repeatedly, and executing it through the ADBC FFI bind/execute paths. Parameter counting, type validation, and wire encoding are specified in `prepared-statements/parameter-binding`; result-set column metadata is specified in `prepared-statements/result-columns`.

## Background

The system implements Exasol's native prepared statement protocol for secure parameterized query execution. Parameters are sent separately from SQL text via the wire protocol, preventing SQL injection by design. Prepared statements can be reused across multiple executions with different parameter values without re-parsing SQL.

## Scenarios

### Scenario: Prepared statement creation

* *GIVEN* a connection to Exasol is established
* *WHEN* a SQL statement with parameters is prepared
* *THEN* it SHALL send a createPreparedStatement request to Exasol over the active transport
* *AND* it SHALL receive a statement handle, parameter metadata, and result-set column metadata from that single response
* *AND* it SHALL store the parameter type information and the result-set column metadata on the prepared statement handle
* *AND* it SHALL NOT issue an additional round-trip, catalog lookup, or probe query to obtain the result-set column metadata

### Scenario: Single execution with parameters

* *GIVEN* a prepared statement exists with bound parameters
* *WHEN* executing a prepared statement with bound parameters via the ADBC FFI interface
* *THEN* the driver SHALL use the prepared statement handle and bound Arrow RecordBatch data
* *AND* the driver SHALL convert Arrow column data to Exasol wire-protocol parameters
* *AND* the driver SHALL return results in Arrow format

### Scenario: Re-execution with different parameters

* *GIVEN* a prepared statement exists with bound parameters
* *WHEN* executing a prepared statement multiple times with different parameter values
* *THEN* it SHALL reuse the server-side prepared statement
* *AND* it SHALL allow re-binding parameters between executions
* *AND* it SHALL avoid re-parsing the SQL statement

### Scenario: Prepared statement with no parameters

* *GIVEN* a connection to Exasol is established
* *WHEN* preparing and executing a statement with no parameters
* *THEN* it SHALL handle the statement through the prepared statement protocol
* *AND* it SHALL NOT require parameter binding

### Scenario: FFI bind and execute_update with parameters

* *GIVEN* an ADBC statement with SQL set and a RecordBatch bound via `bind()`
* *WHEN* `execute_update` is called
* *THEN* the driver SHALL prepare the statement if not already prepared
* *AND* the driver SHALL extract parameter values from the bound RecordBatch
* *AND* the driver SHALL execute the prepared statement with the bound parameters

### Scenario: FFI bind and execute_query with parameters

* *GIVEN* an ADBC statement with SQL set and a RecordBatch bound via `bind()`
* *WHEN* `execute` (execute_query) is called
* *THEN* the driver SHALL prepare the statement if not already prepared
* *AND* the driver SHALL extract parameter values from the bound RecordBatch
* *AND* the driver SHALL return the result set as Arrow RecordBatches
