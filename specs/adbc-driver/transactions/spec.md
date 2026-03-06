# Feature: Transactions (ADBC)

Implements proper ADBC transaction support by wiring the autocommit attribute through to the Exasol WebSocket execute command, ensuring that begin/commit/rollback operations correctly control transaction boundaries.

## Background

Exasol supports transactions through the `autocommit` attribute on execute commands. When autocommit is `true` (the default), each statement is immediately committed. When autocommit is `false`, statements accumulate in an open transaction until an explicit `COMMIT` or `ROLLBACK` is issued. The ADBC specification exposes transactions through `Connection::commit()` and `Connection::rollback()` methods, with autocommit controlled via the `AutoCommit` connection option.

## Scenarios

### Scenario: Autocommit enabled by default

* *GIVEN* a new ADBC connection to Exasol
* *WHEN* a DML statement is executed without explicitly disabling autocommit
* *THEN* the execute request SHALL include the `autocommit` attribute set to `true`
* *AND* the statement's effects SHALL be immediately visible to other sessions

### Scenario: Disable autocommit for explicit transactions

* *GIVEN* an ADBC connection with autocommit set to `false`
* *WHEN* a DML statement is executed
* *THEN* the execute request SHALL include the `autocommit` attribute set to `false`
* *AND* the statement's effects SHALL NOT be visible to other sessions until commit

### Scenario: Commit transaction

* *GIVEN* an ADBC connection with autocommit disabled
* *AND* one or more DML statements have been executed
* *WHEN* `commit()` is called on the connection
* *THEN* the driver SHALL execute a `COMMIT` statement
* *AND* all pending changes SHALL become visible to other sessions
* *AND* the connection SHALL remain in manual commit mode

### Scenario: Rollback transaction

* *GIVEN* an ADBC connection with autocommit disabled
* *AND* one or more DML statements have been executed
* *WHEN* `rollback()` is called on the connection
* *THEN* the driver SHALL execute a `ROLLBACK` statement
* *AND* all pending changes SHALL be discarded
* *AND* the connection SHALL remain in manual commit mode

### Scenario: Toggle autocommit

* *GIVEN* an ADBC connection with autocommit in any state
* *WHEN* the `AutoCommit` connection option is changed
* *THEN* subsequent execute requests SHALL use the new autocommit value
* *AND* if switching from manual to auto mode, any open transaction SHALL be committed first
