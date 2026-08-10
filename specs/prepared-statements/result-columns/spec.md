# Feature: Prepared Statement Result-Set Columns

Specifies the result-set column metadata a prepared statement handle reports alongside its parameter metadata: which statements get columns, what those columns are named and typed, and how the two transports must agree.

## Background

Exasol classifies a statement at prepare time as result-set-producing (`SELECT`, `DESCRIBE`) or row-count-producing (every other statement). Only the former carries a result-set description in the `createPreparedStatement` / `CMD_CREATE_PREPARED` reply; see `prepared-statements/binding-and-execution` for how that reply is received and stored, and `native-client/prepared-statement-protocol` / `websocket-client/prepared-statements` for the per-transport wire format.

## Scenarios

### Scenario: Result-set column metadata for a parameterized SELECT

* *GIVEN* a table `T` exists with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(50)`
* *WHEN* preparing `SELECT ID, NAME FROM T WHERE ID = ? AND NAME = ?`
* *THEN* the prepared statement handle SHALL report two parameters and two result-set columns in select-list order
* *AND* the result-set columns SHALL be `ID` with Exasol type `DECIMAL(18,0)` followed by `NAME` with Exasol type `VARCHAR(50)`
* *AND* the reported result-set columns MUST be identical over the native TCP transport and the WebSocket transport for the identical SQL text

### Scenario: Result-set column metadata for a derived select list

* *GIVEN* a table `T` exists with columns `ID DECIMAL(18,0)` and `NAME VARCHAR(50)`
* *WHEN* preparing `SELECT ID*2 AS DOUBLED, UPPER(NAME) FROM T`
* *THEN* the prepared statement handle SHALL report zero parameters and two result-set columns
* *AND* the aliased column SHALL be named `DOUBLED`
* *AND* the unaliased derived column SHALL carry the name Exasol assigned it rather than an empty name or a positional placeholder

### Scenario: Row-count-producing statements report no result-set columns

* *GIVEN* a connection to Exasol is established
* *WHEN* preparing a statement that Exasol classifies as row-count-producing rather than result-set-producing, such as `INSERT INTO T VALUES (?,?)` or `DELETE FROM T WHERE ID = ?`
* *THEN* preparation SHALL succeed and the prepared statement handle SHALL report the statement's parameters
* *AND* the prepared statement handle SHALL report zero result-set columns
* *AND* the system SHALL NOT report an error for the absent result-set description, because Exasol classifies every statement other than `SELECT` and `DESCRIBE` as row-count-producing, computes no result-set description for it, and therefore sends none
