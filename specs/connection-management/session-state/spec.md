# Feature: Session State

Specifies the client-side session state of a connection and how statement executions, prepares, and transaction starts keep it consistent.

## Background

The client-side session state SHALL be one of `Ready`, `InTransaction`, `Closing`, or `Closed`. A statement execution, a prepare, a prepared-statement execution, or a transaction start SHALL begin only in `Ready` or `InTransaction`, and the driver SHALL keep the session in that state while the operation runs, so that no exit path of an operation, including an error or an abandoned execution, leaves a state that blocks the next operation. Session establishment, termination, and timeouts are specified in `connection-management/session-and-lifecycle`.

## Scenarios

### Scenario: A failed statement leaves the session usable

* *GIVEN* an open connection with no active transaction
* *WHEN* a statement or prepared-statement execution on that connection fails, because the server reports an error, a parameter value cannot be converted, or the server rejects the `queryTimeout` attribute
* *THEN* the session state SHALL be `Ready`
* *AND* the next statement on the connection SHALL execute
* *AND* `begin_transaction()` and setting the ADBC `AutoCommit` connection option to `false` SHALL succeed and MUST NOT report the connection as closed

### Scenario: A failed statement inside a transaction keeps the transaction

* *GIVEN* an open connection with an active transaction
* *WHEN* a statement or prepared-statement execution on that connection fails
* *THEN* the session state SHALL be `InTransaction`
* *AND* `commit()` and `rollback()` SHALL end the transaction and return the session to `Ready`

### Scenario: An abandoned execution leaves the session state unchanged

* *GIVEN* an open connection in state `Ready` or `InTransaction`
* *WHEN* the caller drops a statement execution before it completes
* *THEN* the session state SHALL be the state the session had before the execution started

### Scenario: A closed session rejects operations as closed

* *GIVEN* a connection whose session state is `Closing` or `Closed`
* *WHEN* the caller starts a statement execution, a prepare, a prepared-statement execution, or a transaction
* *THEN* the operation MUST fail with an error that reports the connection as closed
* *AND* the driver MUST NOT send any request for the operation to the server

### Scenario: Starting a second transaction reports the active transaction

* *GIVEN* an open connection with an active transaction
* *WHEN* the caller calls `begin_transaction()` again
* *THEN* the call MUST fail with an error that states a transaction is already active
* *AND* the error MUST NOT report the connection as closed
* *AND* the driver MUST NOT send a request to the server for the rejected call
