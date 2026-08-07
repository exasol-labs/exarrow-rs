# Feature: Parameter Binding

Specifies type-safe, positional parameter binding for Exasol prepared statements: counting placeholders, validating and converting bound values, and encoding them onto the wire. Statement creation and execution are specified in `prepared-statements/binding-and-execution`.

## Background

Parameter binding is positional (1-indexed) and type-safe, with validation against expected parameter types. The placeholder count comes from scanning the SQL text for `?` characters outside string literals, identifiers, and comments.

## Scenarios

### Scenario: Parameter metadata retrieval

* *GIVEN* a connection to Exasol is established
* *WHEN* a prepared statement is created
* *THEN* it SHALL provide parameter count
* *AND* it SHALL provide parameter types when available from the database
* *AND* it SHALL allow querying parameter information before binding

### Scenario: Parameter value binding

* *GIVEN* a connection to Exasol is established
* *WHEN* binding parameter values to a prepared statement
* *THEN* it SHALL validate value types match expected parameter types
* *AND* it SHALL convert Rust types to Exasol wire format
* *AND* it SHALL send parameters separately from SQL text
* *AND* it MUST count only `?` characters that occur outside single-quoted string literals, double-quoted identifiers, line comments (`-- ...`), and block comments (`/* ... */`) when determining the positional placeholder index

### Scenario: NULL parameter binding

* *GIVEN* a connection to Exasol is established
* *WHEN* binding a NULL value to a parameter
* *THEN* it SHALL correctly represent NULL in the wire protocol
* *AND* it SHALL handle typed NULLs appropriately

### Scenario: Multiple parameter binding

* *GIVEN* a connection to Exasol is established
* *WHEN* binding multiple parameters
* *THEN* it SHALL bind parameters by position (1-indexed)
* *AND* it SHALL validate all required parameters are bound before execution

### Scenario: Prepared statement with question mark inside a string literal

* *GIVEN* a connection to Exasol is established
* *WHEN* preparing a statement whose SQL text contains `?` inside a single-quoted string literal (for example `SELECT 'a?b'`)
* *THEN* the system MUST report zero positional parameters
* *AND* the system MUST NOT include the embedded `?` in the parameter count returned by parameter metadata

### Scenario: Non-ASCII VARCHAR parameter round-trip

* *GIVEN* a connection to Exasol is established over the native TCP transport
* *AND* a table `T` exists with a column `NAME VARCHAR(50)`
* *WHEN* binding a parameter value containing non-ASCII UTF-8 characters and executing the prepared statement
* *THEN* Exasol SHALL accept the parameter data
* *AND* selecting the stored value back SHALL return the bound string unchanged, byte for byte
