# Feature: Driver Interface

Defines the ADBC driver interface for exarrow-rs, covering driver registration, database connection management, and error handling through ADBC-compliant interfaces.

## Background

The system implements the ADBC (Arrow Database Connectivity) driver interface to provide standardized database connectivity for Exasol. All connections use WebSocket transport and authenticate with provided credentials. Connection strings follow the format `exasol://host:port`. Errors are propagated through ADBC-compliant error interfaces with detailed messages, error codes, and SQL state where applicable.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Driver registration

* *GIVEN* an ADBC driver manager is ready to load drivers
* *WHEN* the driver is loaded by an ADBC driver manager
* *THEN* it SHALL expose driver metadata including name, version, and vendor information
* *AND* it SHALL be compatible with ADBC driver manager version 0.24
<!-- /DELTA:CHANGED -->
