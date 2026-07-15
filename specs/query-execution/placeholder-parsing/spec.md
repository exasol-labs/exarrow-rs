# Feature: Placeholder Parsing

Specifies how the driver distinguishes a positional `?` parameter placeholder from a literal `?` character inside SQL text, so string literals, identifiers, and comments are never mistaken for bind points.

## Background

`?` is the driver's positional parameter placeholder syntax. The driver scans SQL text to classify each `?` before binding: a `?` inside a single-quoted string literal, a double-quoted identifier, a line comment (`-- ...`), or a block comment (`/* ... */`) is literal text, not a placeholder.

## Scenarios

### Scenario: Literal question mark inside a single-quoted string

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement whose SQL text contains `?` inside a single-quoted string literal (for example `SELECT 'a?b' AS v`) and no parameters are bound
* *THEN* the system MUST send the SQL unmodified to Exasol
* *AND* the system MUST NOT raise a parameter-binding error for the embedded `?`
* *AND* the system SHALL return the literal value `a?b` in the result set

### Scenario: Escaped single quote inside a string literal containing a question mark

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement whose SQL text contains the SQL standard `''` escape inside a single-quoted string literal that also contains `?` (for example `SELECT 'It''s ?' AS v`)
* *THEN* the system MUST treat the doubled `'` as part of the string literal
* *AND* the system MUST NOT treat the `?` as a positional placeholder
* *AND* the system SHALL send the SQL unmodified to Exasol

### Scenario: Question mark inside a line comment

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement that contains a line comment with `?` (for example `SELECT 1 -- is this ?\n`)
* *THEN* the system MUST treat the entire comment, including `?`, as non-placeholder text up to the next newline
* *AND* the system MUST NOT raise a parameter-binding error

### Scenario: Question mark inside a block comment

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement that contains a block comment with `?` (for example `SELECT /* what? */ 1`)
* *THEN* the system MUST treat the entire `/* ... */` region as non-placeholder text
* *AND* the system MUST NOT raise a parameter-binding error

### Scenario: Question mark inside a double-quoted identifier

* *GIVEN* an authenticated connection exists to Exasol
* *WHEN* executing a statement that contains a `?` inside a double-quoted identifier (for example `SELECT 1 AS "col?name"`)
* *THEN* the system MUST treat the `?` as part of the identifier
* *AND* the system MUST NOT treat the `?` as a positional placeholder

### Scenario: Mixed placeholders and literal question marks

* *GIVEN* an authenticated connection exists to Exasol
* *AND* a statement with SQL `SELECT 'a?b' AS lit, ? AS bound` and a single parameter bound to integer `42`
* *WHEN* the statement is executed
* *THEN* the system SHALL substitute only the unquoted `?` with the bound value
* *AND* the system MUST leave the `?` inside the string literal intact
* *AND* the result SHALL contain the literal `a?b` and the value `42`
