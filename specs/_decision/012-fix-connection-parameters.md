# Decisions: fix-connection-parameters

## ADR: The connection string accepts only the keys the driver implements

**ID:** connection-string-accepts-only-implemented-keys
**Plan:** fix-connection-parameters
**Status:** Accepted

### Context

Every later connection parameter must join one accepted set of URI query keys. A silently dropped key hides typos such as `queryTimeout` and settings the driver never applied, such as `autocommit`.

### Decision

A URI query key that the driver does not implement fails the parse with an error that names the key and lists the accepted keys, and never repeats the value. The driver forwards no URI query key to Exasol as a session attribute.

### Options Considered

| Option | Verdict |
|--------|---------|
| Forward unknown keys as session attributes | ✗ Rejected. The driver never did, and some session attributes are read-only, so the result depends on each key |
| Keep dropping unknown keys | ✗ Rejected. Callers keep believing a setting applies |
| An allow-list of writable session attributes | ✗ Deferred to a follow-up issue |

### Consequences

- An unencoded `?`, and any `&` after it, in a password can split off a fragment that becomes a query key, and the error then names that fragment.
- The docs' URL Encoding section states this risk next to the percent-encoding rule.
