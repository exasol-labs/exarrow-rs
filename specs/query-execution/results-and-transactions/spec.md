# Feature: Results and Transactions

Specifies transaction management and query metadata for SQL query execution.

## Background

Transaction management supports explicit begin/commit/rollback operations as well as auto-commit mode (enabled by default). Query execution metadata including timing, row counts, and warnings is available after each query completes.

## Scenarios

### Scenario: Explicit transaction begin

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* beginning a transaction explicitly
* *THEN* it SHALL execute BEGIN or START TRANSACTION
* *AND* it SHALL track transaction state

### Scenario: Transaction commit

* *GIVEN* autocommit is disabled on the connection
* *AND* a transaction is in progress
* *WHEN* committing a transaction
* *THEN* it SHALL execute COMMIT
* *AND* it SHALL update transaction state to committed
* *AND* it SHALL NOT re-enable autocommit on the server

### Scenario: Transaction rollback

* *GIVEN* autocommit is disabled on the connection
* *AND* a transaction is in progress
* *WHEN* rolling back a transaction
* *THEN* it SHALL execute ROLLBACK
* *AND* it SHALL update transaction state to rolled back
* *AND* it SHALL NOT re-enable autocommit on the server

### Scenario: Commit after rollback with autocommit disabled

* *GIVEN* autocommit is disabled on the connection
* *AND* a rollback has been performed
* *WHEN* a commit is called
* *THEN* the commit SHALL succeed as a no-op
* *AND* the connection SHALL remain in manual transaction mode

### Scenario: Rollback without active transaction

* *GIVEN* autocommit is disabled on the connection
* *AND* no transaction is currently active
* *WHEN* a rollback is called
* *THEN* the rollback SHALL succeed as a no-op
* *AND* the connection SHALL remain in manual transaction mode

### Scenario: Auto-commit mode

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* auto-commit is enabled (default)
* *THEN* each statement SHALL be automatically committed
* *AND* explicit transaction commands SHALL override auto-commit temporarily

### Scenario: Execution statistics

* *GIVEN* a query has been executed
* *WHEN* a query completes
* *THEN* it SHALL provide execution time
* *AND* it SHALL provide rows affected/returned
* *AND* it SHALL provide any warnings from the database

### Scenario: Query plan information

* *GIVEN* a query has been executed
* *WHEN* EXPLAIN is used
* *THEN* it SHALL return query execution plan information
* *AND* it SHALL format plan data appropriately for display
