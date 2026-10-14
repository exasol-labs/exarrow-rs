<!-- DELTA:CHANGED -->
# Feature: Credential Sources

The driver takes the database username and password from the ADBC `username` and `password` database options, from the userinfo part of the connection URI, or from the URI query parameters. It resolves them into one set of connection parameters, so the credentials reach the server exactly as the caller gave them and a connection never logs in as a user the caller did not name. A password that is not percent-encoded can shift into the host, port, or query part of the URI. An error from parsing the URI names the field at fault, the query key, or the position of a query parameter without `=`, and never repeats the userinfo or a query value. The error names an unknown query key as written, so a password with an unencoded `?` can show a fragment in that error.
<!-- /DELTA:CHANGED -->

## Background

* Each credential field resolves on its own, in this order: the ADBC database option when it is set, then the URI userinfo, then the URI query parameters (`user` before `username`, `password` before `pass`)

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Query credentials fill only what the userinfo omits

* *GIVEN* the connection URI `exasol://alice@db.example.com:8563?user=bob&username=carol&password=Secret1&pass=Other1`
* *WHEN* the URI is parsed into connection parameters
* *THEN* the username SHALL be `alice`
* *AND* the password SHALL be `Secret1`
* *AND* parsing MUST NOT reject `user`, `username`, `password`, or `pass` as an unknown parameter
<!-- /DELTA:CHANGED -->
