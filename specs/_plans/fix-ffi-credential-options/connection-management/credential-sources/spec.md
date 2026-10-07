# Feature: Credential Sources

The driver takes the database username and password from the ADBC `username` and `password` database options, from the userinfo part of the connection URI, or from the URI query parameters. It resolves them into one set of connection parameters, so the credentials reach the server exactly as the caller gave them and a connection never logs in as a user the caller did not name.

## Background

* Each credential field resolves on its own, in this order: the ADBC database option when it is set, then the URI userinfo, then the URI query parameters (`user` before `username`, `password` before `pass`)

## Scenarios

### Scenario: Option credentials take precedence over URI credentials

* *GIVEN* an ADBC database whose `uri` option is `exasol://nobody:Wrong1@db.example.com:8563`
* *AND* its `username` option is `alice` and its `password` option is `Secret1`
* *WHEN* a connection is created from the database
* *THEN* the driver SHALL log in as `alice` with the password `Secret1`
* *AND* the driver MUST NOT send the URI username or the URI password to the server

### Scenario: A single credential option replaces only its own field

* *GIVEN* an ADBC database whose `uri` option is `exasol://alice:Wrong1@db.example.com:8563`
* *AND* its `password` option is `Secret1` and its `username` option is not set
* *WHEN* a connection is created from the database
* *THEN* the driver SHALL log in as `alice` with the password `Secret1`

### Scenario: Option password reaches the server verbatim

* *GIVEN* an ADBC database whose `uri` option carries no username and no password
* *AND* its `username` option names an existing database user
* *AND* its `password` option is that user's password, one of `Ab?cd1234`, `Ab%41cd1234`, or `Ab@:/#cd1234`
* *WHEN* a connection is created from the database and a statement runs on it
* *THEN* the driver SHALL send the `password` option value byte for byte, without percent-decoding it or splitting it
* *AND* the login SHALL succeed

### Scenario: An at sign in a query parameter value does not change the host

* *GIVEN* an ADBC database whose `uri` option is `exasol://db.example.com:8563?client_name=dbt@ci`
* *AND* its `username` and `password` options are set
* *WHEN* a connection is created from the database
* *THEN* the driver SHALL connect to host `db.example.com` on port 8563
* *AND* the connection parameters SHALL hold the `client_name` value `dbt@ci`

### Scenario: Missing username is rejected

* *GIVEN* an ADBC database whose `uri` option names no user in the userinfo or in the query parameters
* *AND* its `username` option is not set and its `password` option is `Secret1`
* *WHEN* a connection is created from the database
* *THEN* the driver SHALL fail with ADBC status `InvalidArguments` and an error message that contains `Username is required`
* *AND* the driver MUST NOT log in as `sys` or as any other default user
* *AND* the error message MUST NOT contain `Secret1`

### Scenario: URI credentials apply when no credential option is set

* *GIVEN* an ADBC database whose `uri` option is `exasol://alice:Ab%3Fcd1234@db.example.com:8563`
* *AND* neither its `username` option nor its `password` option is set
* *WHEN* a connection is created from the database
* *THEN* the driver SHALL log in as `alice` with the percent-decoded password `Ab?cd1234`

### Scenario: Query credentials fill only what the userinfo omits

* *GIVEN* the connection URI `exasol://alice@db.example.com:8563?user=bob&username=carol&password=Secret1&pass=Other1`
* *WHEN* the URI is parsed into connection parameters
* *THEN* the username SHALL be `alice`
* *AND* the password SHALL be `Secret1`
* *AND* the connection attributes MUST NOT contain the keys `user`, `username`, `password`, or `pass`

### Scenario: Query user applies when only the password option is set

* *GIVEN* an ADBC database whose `uri` option is `exasol://db.example.com:8563?user=bob&username=carol`
* *AND* its `password` option is `Secret1` and its `username` option is not set
* *WHEN* a connection is created from the database
* *THEN* the driver SHALL log in as `bob` with the password `Secret1`
* *AND* the driver MUST NOT log in as `sys`

### Scenario: Password stays out of Debug output

* *GIVEN* connection parameters whose password came from the ADBC `password` option, the URI userinfo, or a URI query parameter
* *AND* a connection builder with a password set
* *WHEN* the connection parameters or the connection builder are formatted with `Debug`
* *THEN* the output MUST NOT contain the password
