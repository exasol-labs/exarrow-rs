# Feature: Parameter Binding

Specifies positional parameter binding for Exasol prepared statements: counting placeholders, validating and converting bound values, and encoding them onto the wire.

## Background

Parameter binding is positional (1-indexed) and type-safe, with validation against expected parameter types. The placeholder count comes from scanning the SQL text for `?` characters outside string literals, identifiers, and comments.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: TIMESTAMP WITH LOCAL TIME ZONE text parameters are stored the same on both transports

* *GIVEN* a connection to Exasol is established over the native TCP transport or the WebSocket transport, and its session time zone is set to `UTC` or to `EUROPE/BERLIN`
* *AND* an empty table `T` exists with the column `TSL TIMESTAMP(6) WITH LOCAL TIME ZONE`
* *WHEN* binding the text `2024-01-01 08:00:00` to `TSL` and executing the prepared statement `INSERT INTO T VALUES (?)`
* *THEN* the execution SHALL report 1 affected row
* *AND* `SELECT TO_CHAR(TSL, 'YYYY-MM-DD HH24:MI:SS') FROM T` in the same session SHALL return `2024-01-01 08:00:00` on both transports and in both session time zones, because Exasol reads the bound text in the session time zone
<!-- /DELTA:NEW -->
