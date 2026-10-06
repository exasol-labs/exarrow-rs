# Decisions: fix-export-parquet-transport-roundtrip

## ADR: A transport Parquet export takes its schema from prepared-statement result-set metadata

**ID:** transport-parquet-export-schema-from-prepared-statement
**Plan:** fix-export-parquet-transport-roundtrip
**Status:** Accepted

### Context

A transport Parquet export needs the column names and Exasol types of its export source before it writes a file. The EXPORT statement returns CSV text and no type information.

### Decision

Before the EXPORT statement runs, the driver prepares the SELECT statement that the export source describes. It builds the Arrow schema from the prepared statement's result-set column metadata and closes the prepared statement.

### Options Considered

| Option | Verdict |
|--------|---------|
| Prepare the source SELECT and read its result-set metadata | ✓ Chosen. Prepare runs no user statement and reports the same metadata on both transports |
| Run a zero-row probe such as `SELECT * FROM (<query>) WHERE FALSE` | ✗ Rejected. It executes and rewrites user SQL |
| Request `WITH COLUMN NAMES` on the EXPORT | ✗ Rejected. The header carries names but no types |
| Infer types from the exported values | ✗ Rejected. It guesses, and an empty export has nothing to infer from |
| Require the caller to pass a schema | ✗ Rejected. Every caller would have to look up the schema itself |

### Consequences

- A transport Parquet export costs one prepare and one close round trip more than before.
- The export source must produce a result set, which the EXPORT statement already requires.
- The SELECT text embeds the caller's identifiers and query exactly as the EXPORT statement does, so the decision adds no SQL injection surface.
