# Feature: Parameter Binding

Specifies type-safe, positional parameter binding for Exasol prepared statements: counting placeholders, validating and converting bound values, and encoding them onto the wire. Statement creation and execution are specified in `prepared-statements/binding-and-execution`.

## Background

Parameter binding is positional (1-indexed) and type-safe, with validation against expected parameter types. The placeholder count comes from scanning the SQL text for `?` characters outside string literals, identifiers, and comments.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: DATE and TIMESTAMP text parameters are stored or rejected on both transports

* *GIVEN* a connection to Exasol is established over the native TCP transport or the WebSocket transport
* *AND* a table `T` exists with the columns `TS TIMESTAMP(6)` and `D DATE`, and `T` is empty before each execution below
* *WHEN* binding the text `2024-01-01 08:00:00.123456` to `TS` and the text `2024-01-02` to `D` and executing the prepared statement `INSERT INTO T VALUES (?, ?)`
* *THEN* the execution SHALL report 1 affected row, and `T` SHALL hold one row with the timestamp `2024-01-01 08:00:00.123456` and the date `2024-01-02`
* *WHEN* binding the text `2024-01-01 08:00:00` with one leading and one trailing space to `TS` and the text `2024-01-02 08:30` to `D` and executing the prepared statement
* *THEN* `T` SHALL hold one row with the timestamp `2024-01-01 08:00:00` and the date `2024-01-02`
* *WHEN* binding empty text to `TS` and to `D` and executing the prepared statement
* *THEN* `T` SHALL hold one row with NULL in `TS` and in `D`
* *WHEN* binding the text `2024-01-01T08:00:00`, `garbage`, or text of only spaces to `TS`, or the text `2024/01/02` or `2024-01-02 25:00:00` to `D`, and executing the prepared statement
* *THEN* the execution SHALL fail with an error, and `T` SHALL remain empty
* *AND* the connection SHALL remain usable for later statements
<!-- /DELTA:NEW -->
