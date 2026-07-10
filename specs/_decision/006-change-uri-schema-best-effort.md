# Decisions: change-uri-schema-best-effort

<!-- One fragment per plan. Add one ## ADR block per promoted decision below. -->
<!-- ID is a kebab-case slug, unique across every file in specs/_decision. -->
<!-- Supersedes is optional — set it only when this ADR replaces an earlier one. -->

## ADR-006: A URI-specified schema is a best-effort default, not a connect-time requirement

**ID:** uri-specified-schema-is-best-effort-default
**Plan:** change-uri-schema-best-effort
**Status:** Accepted

### Context

In 0.11.0 a schema named in the connection URI was opened eagerly after login via `OPEN SCHEMA`, and ANY failure of that implicit activation aborted the connection: `connect_with_transport` closed the transport and returned `ConnectionError::ConnectionFailed`. The spec encoded this as a hard requirement ("if the server-side schema activation fails, `connect()` MUST return a `ConnectionError`"). This breaks tools such as dbt, which connect first and create their target schema afterwards while fully qualifying every relation. At connect time the schema does not exist yet, so the connection failed before the tool could create it -- a normal bootstrap state was treated as fatal.

### Decision

Treat a URI-specified schema as a best-effort default. During `connect_with_transport`, if the implicit `set_schema` (`OPEN SCHEMA`) fails, classify the failure: if it is a missing-schema error, swallow it and leave the session open with no active schema (the schema can be `OPEN`ed later, once it exists); if it is any other failure, keep the existing fatal behavior -- close the transport and return `ConnectionError::ConnectionFailed`. Classification is performed by a small free function `schema_open_error_is_missing_schema(&QueryError) -> bool` that inspects the lowercased error message for "not found".

### Options Considered

| Option | Verdict |
|--------|---------|
| Best-effort default: swallow missing-schema, keep other failures fatal | ✓ Chosen -- unblocks dbt-style bootstrap while preserving the safety guarantee that a genuinely broken connection never returns a half-open `Connection` |
| Keep any schema-activation failure fatal (0.11.0 behavior) | ✗ Rejected -- treats a normal not-yet-existing default schema as a corrupt connection; blocks dbt and similar tools |
| Eagerly `CREATE SCHEMA` the missing schema during connect | ✗ Rejected -- the driver must not silently perform DDL or assume create permissions on the user's behalf; schema creation is the application's decision |

### Consequences

dbt and similar tools can connect against a not-yet-existing default schema and create it afterwards; fully qualified relations continue to resolve, and the caller may activate the schema later via `set_schema()`. Non-missing failures (auth, permissions, transport) still close the transport and return a clear error, so no half-open connection is ever leaked. The missing-schema classification is message-based (substring "not found" on the lowercased error text), which is a known brittleness: if Exasol changes its "schema ... not found" wording, a missing schema could be misclassified as fatal (or vice versa). The behavior is guarded by unit tests `schema_open_error_missing_schema_is_recognized` and `schema_open_error_unrelated_is_fatal`, and by the ignored live integration test `test_connect_with_nonexistent_uri_schema_succeeds`.
