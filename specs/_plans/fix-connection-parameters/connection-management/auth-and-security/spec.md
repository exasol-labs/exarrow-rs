# Feature: Auth and Security

Defines connection parameter handling, authentication mechanisms, and credential security for Exasol database connections.

## Background

Connection parameters are required for Exasol connectivity and SHALL be validated before use. Authentication mechanisms SHALL operate securely, with credentials protected in memory and never exposed through logging. TLS is enabled by default for production Exasol connections.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Unknown connection-string parameter is rejected

* *GIVEN* the connection URIs `exasol://alice:Secret1@db.example.com:8563?autocommit=Qx7` and `exasol://alice:Secret1@db.example.com:8563?queryTimeout=Qx7`
* *WHEN* each URI is parsed into connection parameters
* *THEN* parsing SHALL fail with `ConnectionError::InvalidParameter` whose parameter is the query key, `autocommit` or `queryTimeout`
* *AND* the error message SHALL list the accepted parameter names, including `query_timeout`, and MUST NOT list the deprecated `idle_timeout`
* *AND* the error message MUST NOT contain `Qx7` or `Secret1`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Client name and version identify the session

* *GIVEN* a running Exasol database
* *AND* a connection URI with the query parameters `client_name=dbt` and `client_version=9.9`
* *WHEN* a connection is opened with that URI over the native transport, or over the WebSocket transport (`transport=websocket`) in a build with the `websocket` feature
* *THEN* the query `SELECT CLIENT FROM EXA_DBA_SESSIONS WHERE SESSION_ID = CURRENT_SESSION` on that connection SHALL return `dbt 9.9`
* *AND* the login message SHALL carry `dbt` as the client name, `9.9` as the client version, and `exarrow-rs` as the driver name
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Default client identification

* *GIVEN* a running Exasol database
* *AND* a connection URI without the query parameters `client_name` and `client_version`
* *WHEN* a connection is opened with that URI
* *THEN* the query `SELECT CLIENT FROM EXA_DBA_SESSIONS WHERE SESSION_ID = CURRENT_SESSION` on that connection SHALL return `exarrow-rs`, a space, and the version of the `exarrow-rs` crate
<!-- /DELTA:NEW -->
