# Decisions: fix-zero-row-result-schema

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-004: Zero-row result sets carry their column schema

**ID:** zero-row-result-sets-carry-column-schema
**Plan:** fix-zero-row-result-schema
**Status:** Accepted

### Context

`ResultSet::from_transport_result` built the Arrow schema from the result set's column metadata (always present, even with no rows) but then produced an empty `Vec<RecordBatch>` whenever the first data payload had zero rows. The schema lived only on `QueryMetadata`, so any consumer that reads the schema from the batches lost the columns. The ADBC FFI layer (`FfiStatement::execute`) does exactly this: with no batches it falls back to `Schema::empty()`. dbt Fusion derives a query's column schema via `get_empty_subquery_sql` (`SELECT * FROM (...) WHERE FALSE LIMIT 0`); the empty schema made it see zero columns, breaking snapshots (`get_snapshot_get_time_data_type` indexing `[0]` on an empty list), contract validation (`get_assert_columns_equivalent`), unit tests, and `get_columns_in_query`. `payload_to_record_batch`/`column_major_to_record_batch` already build a correct zero-row batch from the schema, so the bug was purely the empty-list short-circuit.

### Decision

In `from_transport_result`, always emit one batch for a result set: when the payload is empty, build a zero-row batch from the authoritative column schema (new `empty_record_batch` helper) instead of returning `Vec::new()`. A result set always has columns, so it always yields at least one schema-carrying batch.

### Options Considered

| Option | Verdict |
|--------|---------|
| Emit a zero-row batch built from the column schema | ✓ Chosen — fixes the deficiency at its source; schema is conveyed uniformly regardless of row count or downstream consumer |
| Fix only the ADBC `execute()` fallback to reuse `QueryMetadata.schema` | ✗ Rejected — leaves the empty-batch-list footgun for every other consumer (`fetch_all`, iterators) |
| Work around it downstream (e.g. dbt `LIMIT 1` probe) | ✗ Rejected — a band-aid in every consumer for a driver-layer defect; an empty result set legitimately has columns |

### Consequences

Empty result sets behave like every other ADBC driver's: the schema survives zero rows. dbt Fusion schema introspection (snapshots, contracts, unit tests, `get_columns_in_query`) works against Exasol without consumer-side workarounds. Regression test `test_empty_result_set_preserves_schema` guards it.
