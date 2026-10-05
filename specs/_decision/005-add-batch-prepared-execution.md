# Decisions: add-batch-prepared-execution

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-005: Fail-fast per-row arity check before any transport call in batch execution

**ID:** fail-fast-per-row-arity-check-in-batch-execution
**Plan:** add-batch-prepared-execution
**Status:** Accepted

### Context

`build_batch_parameters_data` transposes row-major `Parameter` rows into column-major wire data. A row whose width differs from `parameter_count()` misaligns columns and silently corrupts the batch.

### Decision

Each input row length MUST equal `parameter_count()`. If any row differs, the driver returns `QueryError::ParameterBindingError` before it assembles columns or calls the transport.

### Options Considered

| Option | Verdict |
|--------|---------|
| Fail-fast arity check before column assembly | ✓ Chosen. Validation precedes I/O and prevents silent corruption |
| Trust the caller | ✗ Rejected. Wrong-width rows corrupt data on the wire |
| Pad or truncate rows | ✗ Rejected. It drops or invents values, and caller intent cannot be inferred |

### Consequences

Future changes to the batch path MUST keep this check, because the column-major transpose needs uniform row width.
