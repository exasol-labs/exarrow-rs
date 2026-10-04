# Decisions: fix-zero-row-result-schema

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-004: Zero-row result sets carry their column schema

**ID:** zero-row-result-sets-carry-column-schema
**Plan:** fix-zero-row-result-schema
**Status:** Accepted

### Context

`ResultSet::from_transport_result` returned an empty `Vec<RecordBatch>` when the first payload had no rows, so the schema existed only on `QueryMetadata`. Consumers that read the schema from batches, such as the ADBC FFI `FfiStatement::execute`, saw an empty schema. dbt Fusion probes columns with `SELECT * FROM (...) WHERE FALSE LIMIT 0`, so it saw zero columns and failed snapshots, contracts, unit tests, and `get_columns_in_query`.

### Decision

`from_transport_result` always emits at least one batch. When the payload has no rows, it builds a zero-row batch from the column schema with the `empty_record_batch` helper.

### Options Considered

| Option | Verdict |
|--------|---------|
| Zero-row batch built from the column schema | ✓ Chosen. Fixes the defect at its source for every consumer |
| Fix only the ADBC `execute()` fallback | ✗ Rejected. `fetch_all` and iterators keep the empty-batch-list defect |
| Downstream workaround such as a dbt `LIMIT 1` probe | ✗ Rejected. It patches a driver defect in every consumer |

### Consequences

The schema survives zero rows, as in other ADBC drivers.
