# Decisions: fix-placeholder-scanning-and-in-clause-results

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-001: Hand-rolled five-state lexer for SQL placeholder scanning

**ID:** hand-rolled-five-state-lexer-for-sql-placeholder-scanning
**Plan:** fix-placeholder-scanning-and-in-clause-results
**Status:** Accepted

### Context

The naive `sql.find('?')` loop in `Statement::build_sql` treated every `?` character as a positional placeholder, including those inside single-quoted string literals, double-quoted identifiers, line comments, and block comments. This caused incorrect parameter counts and mangled SQL for queries such as `SELECT 'a?b'` or `INSERT INTO t VALUES ('it''s a test?')`.

### Decision

Replace the naive loop with a private linear-pass state machine `scan_placeholders` that tracks five lexical states — `Normal`, `SingleQuoted`, `DoubleQuoted`, `LineComment`, and `BlockComment` — and only treats `?` in the `Normal` state as a positional placeholder.

### Options Considered

| Option | Verdict |
|--------|---------|
| Hand-rolled five-state lexer in `src/query/statement.rs` | ✓ Chosen — O(n), allocation-free, no new dependencies, matches the approach used by pyexasol and JDBC |
| Pull in `sqlparser` crate | ✗ Rejected — heavyweight dependency for a single bug fix; ANSI SQL parsing is overkill for lexical-state tracking |
| Use a regex to strip strings/comments first | ✗ Rejected — cannot correctly handle SQL standard `''` escaping without becoming a state machine anyway |

### Consequences

The scanner is easy to unit-test independently. SQL standard `''` and `""` escape sequences inside string literals are handled correctly. Multi-byte UTF-8 input is safe because the scanner operates on `char_indices()` with explicit ASCII checks. No new crate dependency is introduced.
