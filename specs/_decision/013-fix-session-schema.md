# Decisions: fix-session-schema

## ADR: A URI schema must exist and is set with set-attributes after the login

**ID:** uri-schema-must-exist-and-is-set-with-set-attributes
**Plan:** fix-session-schema
**Status:** Accepted

### Context

A schema named in the connection URI changes how every later unqualified statement resolves. A connect that silently drops a mistyped schema hides the error from the caller.

### Decision

On both transports, the driver sets a URI or `ConnectionParams` schema with the protocol's set-attributes command right after the login. Any rejection closes the session and fails the connect. The driver never sends `OPEN SCHEMA` for it and never inspects the server's error text.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep the best-effort default | ✗ Rejected. A mistyped schema connects silently. The dbt case belongs in the dbt adapter |
| Send the schema as a login attribute | ✗ Rejected. A missing schema and a wrong password fail with the same SQL state, so the driver cannot tell them apart without matching error text |
| Quote the name in `OPEN SCHEMA "name"` | ✗ Rejected. Quoting drops the server's upper-case fallback, so `/myschema` would stop opening `MYSCHEMA` |

### Consequences

- The connect error is `ConnectionError::ConnectionFailed`, and its message names the schema and contains the server's message.
- A connect with a URI schema takes two round trips after the login (set-attributes, then get-attributes), where `OPEN SCHEMA` took one.
- The schema step stays outside the connection-timeout deadline.
- A dbt run whose target schema does not exist yet fails at connect until the dbt adapter connects without the schema.
