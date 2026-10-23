# Decisions: fix-zero-row-result-schema

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional, set it only when this ADR replaces an earlier one. -->

## ADR-004: Zero-row result sets carry their column schema

**ID:** zero-row-result-sets-carry-column-schema
**Plan:** fix-zero-row-result-schema
**Status:** Accepted

### Context

A query that returns no rows produced no batches, so consumers that read the schema from batches saw zero columns. Column-probing tools such as dbt fail on this.

### Decision

A result set always holds at least one batch. When a result has no rows, the driver emits a zero-row batch that carries the column schema.

### Options Considered

| Option | Verdict |
|--------|---------|
| Zero-row batch built from the column schema | ✓ Chosen. Fixes the defect at its source for every consumer |
| Fix only the ADBC `execute()` fallback | ✗ Rejected. `fetch_all` and iterators keep the empty-batch-list defect |
| Downstream workaround such as a dbt `LIMIT 1` probe | ✗ Rejected. It patches a driver defect in every consumer |

### Consequences

The schema survives zero rows, as in other ADBC drivers.
