# Feature: Parameter Validation

Defines how the driver parses and validates connection-string parameters for Exasol database connections.

## Background

Connection parameters are required for Exasol connectivity and SHALL be validated before use.

## Scenarios

### Scenario: Connection string parsing

* *GIVEN* connection parameters are configured
* *WHEN* a connection string is provided in the format `exasol://host:port`
* *THEN* it SHALL parse host and port correctly
* *AND* it SHALL support optional parameters (schema, encryption settings)
* *AND* it SHALL parse `certificate_fingerprint` (alias `certificatefingerprint`) as the expected server certificate fingerprint

### Scenario: Parameter validation

* *GIVEN* connection parameters are configured
* *WHEN* connection parameters are validated
* *THEN* it SHALL reject missing required parameters (host, credentials)
* *AND* it SHALL validate port numbers are in valid range (1-65535)
* *AND* it SHALL provide clear error messages for invalid parameters

### Scenario: Unknown connection-string parameter is rejected

* *GIVEN* the connection URIs `exasol://alice:Secret1@db.example.com:8563?autocommit=Qx7` and `exasol://alice:Secret1@db.example.com:8563?queryTimeout=Qx7`
* *WHEN* each URI is parsed into connection parameters
* *THEN* parsing SHALL fail with `ConnectionError::InvalidParameter` whose parameter is the query key, `autocommit` or `queryTimeout`
* *AND* the error message SHALL list the accepted parameter names, including `query_timeout`, and MUST NOT list the deprecated `idle_timeout`
* *AND* the error message MUST NOT contain `Qx7` or `Secret1`
