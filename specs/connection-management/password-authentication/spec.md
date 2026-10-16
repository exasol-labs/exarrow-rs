# Feature: Password Authentication and Client Identification

Defines username and password authentication, credential security, and client identification for Exasol database connections.

## Background

Authentication mechanisms SHALL operate securely, with credentials protected in memory and never exposed through logging.

## Scenarios

### Scenario: Username and password authentication

* *GIVEN* a connection to Exasol is being established
* *WHEN* authenticating with username and password
* *THEN* it SHALL send credentials securely over the connection
* *AND* it SHALL support encrypted password transmission
* *AND* it SHALL encrypt passwords using RSA PKCS#1 v1.5 with any server-provided public key whose modulus is between 1024 and 8192 bits inclusive

### Scenario: Password encryption with 1024-bit RSA public key

* *GIVEN* the Exasol server responds to the login request with a 1024-bit RSA public key in PKCS#1 PEM format
* *WHEN* the driver encrypts the user's password for transmission
* *THEN* the driver SHALL successfully parse the 1024-bit modulus and public exponent
* *AND* the driver SHALL apply PKCS#1 v1.5 padding and compute `ciphertext = message^e mod n`
* *AND* the driver SHALL produce a base64-encoded ciphertext whose decoded length equals the modulus length in bytes (128 bytes for a 1024-bit key)
* *AND* the driver SHALL complete the login handshake without raising a key-size-related error

### Scenario: Password encryption rejects out-of-range RSA key sizes

* *GIVEN* the Exasol server responds with an RSA public key whose modulus is smaller than 1024 bits or larger than 8192 bits
* *WHEN* the driver attempts to encrypt the password
* *THEN* the driver SHALL return a `ProtocolError` indicating the unsupported key size
* *AND* the driver SHALL NOT attempt the modular exponentiation

### Scenario: Authentication failure

* *GIVEN* a connection to Exasol is being established
* *WHEN* authentication fails
* *THEN* it SHALL return an error with the authentication failure reason
* *AND* it SHALL close the connection
* *AND* it SHALL NOT retry automatically to avoid account lockout

### Scenario: Authentication success

* *GIVEN* a connection to Exasol is being established
* *WHEN* authentication succeeds
* *THEN* it SHALL establish an authenticated session
* *AND* it SHALL store session information for subsequent requests

### Scenario: Secure credential storage

* *GIVEN* credentials are provided for authentication
* *WHEN* credentials are stored temporarily
* *THEN* it SHALL minimize credential lifetime in memory
* *AND* it SHALL consider zeroing credential memory after use where feasible

### Scenario: No credential logging

* *GIVEN* credentials are provided for authentication
* *WHEN* logging connection events
* *THEN* it SHALL NOT log passwords or sensitive authentication tokens
* *AND* it SHALL redact credentials from error messages

### Scenario: Client name and version identify the session

* *GIVEN* a running Exasol database
* *AND* a connection URI with the query parameters `client_name=dbt` and `client_version=9.9`
* *WHEN* a connection is opened with that URI over the native transport, or over the WebSocket transport (`transport=websocket`) in a build with the `websocket` feature
* *THEN* the query `SELECT CLIENT FROM EXA_DBA_SESSIONS WHERE SESSION_ID = CURRENT_SESSION` on that connection SHALL return `dbt 9.9`
* *AND* the login message SHALL carry `dbt` as the client name, `9.9` as the client version, and `exarrow-rs` as the driver name

### Scenario: Default client identification

* *GIVEN* a running Exasol database
* *AND* a connection URI without the query parameters `client_name` and `client_version`
* *WHEN* a connection is opened with that URI
* *THEN* the query `SELECT CLIENT FROM EXA_DBA_SESSIONS WHERE SESSION_ID = CURRENT_SESSION` on that connection SHALL return `exarrow-rs`, a space, and the version of the `exarrow-rs` crate
