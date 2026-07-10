# Decisions: add-batch-prepared-execution

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-005: Fail-fast per-row arity check before any transport call in batch execution

**ID:** fail-fast-per-row-arity-check-in-batch-execution
**Plan:** add-batch-prepared-execution
**Status:** Accepted

### Context

`build_batch_parameters_data` transposes a row-major slice of `Parameter` rows into column-major wire data. Column-major assembly requires that every input row has the same length as `parameter_count()`. A row of the wrong width would silently misalign columns and corrupt the batch on the wire — column `c` would contain values from multiple different logical columns. This class of silent data corruption is worse than a binding error because it produces wrong results rather than an obvious failure.

### Decision

Each input row's length MUST equal `parameter_count()`. If any row's length differs, return `QueryError::ParameterBindingError` before assembling any columns or making any transport call.

### Options Considered

| Option | Verdict |
|--------|---------|
| Fail-fast arity check before column assembly | ✓ Chosen — prevents silent column misalignment; consistent with the principle that validation must precede I/O |
| Trust the caller (no arity check) | ✗ Rejected — wrong-width rows silently corrupt column-major data on the wire, producing incorrect query results |
| Pad or truncate rows to fit `parameter_count()` | ✗ Rejected — silently discards or fabricates parameter values; caller intent cannot be inferred from mismatched arity |

### Consequences

Any caller supplying rows of inconsistent or wrong arity receives an immediate `ParameterBindingError` with no transport call made. Future maintainers extending the batch path MUST preserve this invariant: column-major transpose correctness depends on uniform row width. The check is unit-tested via `test_build_batch_params_arity_mismatch`.
