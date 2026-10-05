# Decisions: fix-placeholder-scanning-and-in-clause-results

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-001: Hand-rolled five-state lexer for SQL placeholder scanning

**ID:** hand-rolled-five-state-lexer-for-sql-placeholder-scanning
**Plan:** fix-placeholder-scanning-and-in-clause-results
**Status:** Accepted

### Context

`Statement::build_sql` counted every `?` as a positional placeholder, including those inside string literals, quoted identifiers, and comments. That produced wrong parameter counts and mangled SQL.

### Decision

`Statement` scans SQL with a private linear-pass state machine, `scan_placeholders`. It tracks five states: `Normal`, `SingleQuoted`, `DoubleQuoted`, `LineComment`, and `BlockComment`. Only a `?` in the `Normal` state is a placeholder.

### Options Considered

| Option | Verdict |
|--------|---------|
| Hand-rolled five-state lexer | ✓ Chosen. Linear, allocation-free, no new dependency, same approach as pyexasol and JDBC |
| `sqlparser` crate | ✗ Rejected. Full SQL parsing is too heavy for lexical-state tracking |
| Regex that strips strings and comments first | ✗ Rejected. It cannot handle `''` escaping without becoming a state machine |

### Consequences

The scanner handles `''` and `""` escapes and is safe on multi-byte UTF-8.
