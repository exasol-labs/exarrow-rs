# Decisions: change-uri-schema-best-effort

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-006: A URI-specified schema is a best-effort default, not a connect-time requirement

**ID:** uri-specified-schema-is-best-effort-default
**Plan:** change-uri-schema-best-effort
**Status:** Accepted

### Context

The driver opens a schema named in the connection URI with `OPEN SCHEMA` after login. Tools such as dbt connect first and create the target schema afterwards. At connect time the schema does not exist, and a hard failure blocked that normal bootstrap.

### Decision

A URI-specified schema is a best-effort default. If the implicit `OPEN SCHEMA` fails with a missing-schema error, `connect_with_transport` keeps the session open with no active schema. Any other failure closes the transport and returns `ConnectionError::ConnectionFailed`. The free function `schema_open_error_is_missing_schema(&QueryError) -> bool` classifies the error by searching the lowercased message for "not found".

### Options Considered

| Option | Verdict |
|--------|---------|
| Swallow missing-schema, keep other failures fatal | ✓ Chosen. Unblocks bootstrap and never returns a half-open `Connection` |
| Any activation failure is fatal | ✗ Rejected. A not-yet-existing schema is normal and blocks dbt |
| `CREATE SCHEMA` during connect | ✗ Rejected. The driver must not run DDL or assume permissions for the user |

### Consequences

The caller can activate the schema later with `set_schema()`. Auth, permission, and transport failures still fail the connect. The classifier depends on Exasol's error wording. A wording change could misclassify a missing schema as fatal.
