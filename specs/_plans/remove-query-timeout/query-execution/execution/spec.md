# Feature: Execution

Specifies SQL query execution capabilities including direct queries, prepared statements, batch execution, and query cancellation through connection-owned transport.

## Background

All SQL execution occurs through Connection-owned transport via the WebSocket protocol. The system supports direct query execution (SELECT, DDL, DML), prepared statements with parameter binding, batch execution of multiple queries, and cancellation of in-flight queries. Results are returned as Arrow RecordBatch. Execution goes through Connection methods such as `execute_statement()`, `execute_statement_update()`, `prepare()`, and `execute_prepared()`.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Cancel with timeout

* *GIVEN* a long-running query is executing
* *WHEN* the driver gives up on the query client-side because a cancellation acknowledgment does not arrive within the acknowledgment timeout
* *THEN* the driver MUST terminate the connection to release the session server-side
* *AND* the driver MUST NOT merely abandon the in-flight request locally while leaving the session open
* *AND* the driver SHALL return a cancellation error
<!-- /DELTA:CHANGED -->
