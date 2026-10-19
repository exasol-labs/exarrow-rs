# Feature: Query Timeout

Specifies the server-enforced query timeout and the deprecated idle timeout.

## Background

The query timeout is a server-enforced session setting and defaults to unset. The driver has no idle timeout, so the `idle_timeout` connection-string parameter is deprecated and has no effect.

## Scenarios

### Scenario: Query timeout

* *GIVEN* an explicit query timeout is configured via the `query_timeout=` connection-string parameter, the `ConnectionParams::query_timeout()` builder, or `Statement::set_timeout()`
* *WHEN* the connection is established or the statement executes
* *THEN* the driver SHALL forward the configured timeout to Exasol as the `queryTimeout` session attribute
* *AND* the server SHALL abort a query whose execution time exceeds the configured timeout and SHALL report the abort as an error through the normal response cycle
* *AND* the driver MUST NOT wrap query execution through `Connection::execute_statement()` in a client-side timer
* *AND* the connection SHALL remain usable for subsequent statements after a server-reported timeout

### Scenario: No query timeout by default

* *GIVEN* a connection with no query timeout configured
* *WHEN* the connection is established and a query is executed via `Connection::execute_statement()`
* *THEN* the driver MUST NOT set the `queryTimeout` session attribute
* *AND* the driver MUST NOT wrap query execution in a client-side timer
* *AND* IF a prior statement on the same connection set the `queryTimeout` session attribute, the driver MUST reset it to `0` (unlimited) before executing so no stale server timeout is inherited
* *AND* the query SHALL run until the server returns a result or the server-side `QUERY_TIMEOUT` setting terminates it

### Scenario: Idle timeout

* *GIVEN* the connection URI `exasol://alice:Secret1@db.example.com:8563?idle_timeout=120`
* *WHEN* the URI is parsed into connection parameters
* *THEN* parsing SHALL succeed
* *AND* the driver SHALL emit a warning through the `log` crate at level `Warn` that names `idle_timeout` and states that the parameter is deprecated and has no effect
* *AND* the warning MUST NOT contain the value `120`

### Scenario: Idle timeout with an invalid value

* *GIVEN* the connection URI `exasol://alice:Secret1@db.example.com:8563?idle_timeout=Qx7`
* *WHEN* the URI is parsed into connection parameters
* *THEN* parsing SHALL fail with `ConnectionError::InvalidParameter` whose parameter is `idle_timeout`
* *AND* the error message MUST NOT contain `Qx7`
