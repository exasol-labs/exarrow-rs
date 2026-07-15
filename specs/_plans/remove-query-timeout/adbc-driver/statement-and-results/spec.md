# Feature: Statement and Results

Specifies statement execution, session identity, and result set handling for the ADBC driver, ensuring queries execute through connection-mediated transport and return Arrow-formatted results.

## Background

Statements are pure data objects created via `Connection::create_statement()` and executed via `Connection::execute_statement()`. Statements do not hold transport references; all execution goes through the Connection. Multiple statements created from the same connection share a single database session. Query results are returned in Arrow format per the ADBC specification.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Statement inherits connection query timeout

* *GIVEN* a connection carrying a query timeout setting, which is either an explicit timeout or no timeout
* *WHEN* `create_statement()` is called on that connection
* *THEN* the returned Statement SHALL inherit that setting: the explicit timeout when the connection configures one, or no timeout when the connection configures none
* *AND* the Statement SHALL NOT fall back to any hardcoded non-zero timeout independent of the connection's configuration
* *AND* when the Statement carries an explicit timeout, `execute_statement()` SHALL forward that timeout to Exasol as the `queryTimeout` session attribute before executing, rather than enforce it with a client-side timer
<!-- /DELTA:CHANGED -->
