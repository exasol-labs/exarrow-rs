# Architecture Delta: fix-ffi-bound-execute
<!-- BASE: 35d6222b02ed3458266a38f69dc44bc6d199cf6a -->

<!-- DELTA:CHANGED -->
## Constraints

- Rust edition 2021, and CI pins the stable toolchain 1.92.0
- Cargo feature `native` is the default transport, `websocket` is optional, `ffi` enables `native`, and a build without either transport rejects every connection
- TLS and server certificate validation are on by default
- Connection timeout defaults to 30 seconds and cannot exceed 300 seconds, and idle timeout defaults to 600 seconds
- The connection timeout is one deadline that bounds the TCP connect, the TLS handshake, the WebSocket upgrade, and the login together, and its error names the step that ran out
- HTTP tunnel setup (TCP connect, EXA handshake, TLS handshake) is bounded by one 30-second deadline for every import and export, independent of the connection timeout and of the opt-in CSV export timeout, and its error names the step that ran out
- The native transport fetches up to the server's maximum data message size per batch, with a 64 MiB fallback
- The native transport runs the parameter set of a statement that returns an affected-row count as consecutive prepared-statement executions when one data message would exceed the server's maximum data message size, and each execution's data message stays within that size unless it holds a single row
- The FFI runtime is a multi-thread Tokio runtime with 2 workers so that import can run WebSocket and HTTP I/O at the same time inside `block_on`
- Native Parquet import requires Exasol 2025.1.11 or later, and older servers receive Parquet converted to CSV
- Password encryption uses RSA PKCS#1 v1.5 through num-bigint because Exasol servers can send 1024-bit keys, which aws-lc-rs rejects
- Credentials are never logged or exposed, the Debug output of Connection, ConnectionParams, and ConnectionBuilder omits the password, a credential query parameter is never kept as a connection attribute, and a connection URI parse error names the field or the query parameter position and never repeats a value from the URI
- Results stream as Arrow RecordBatches, and conversion is Arrow-native and zero-copy where possible
- CI rejects clippy warnings, cargo-deny license findings, cargo-deny advisory findings in the dependencies of every Cargo feature, a failing library unit test with the default features, the `websocket` feature, or the `ffi` feature, an integration test target that the integration job does not run, production line coverage below 80 percent, any file below 50 percent, and a failed SonarQube Cloud quality gate (intended to become a required check once rolled out)
- Library unit tests run in CI without a database server, so a unit test uses a test double, such as a mock transport or a local fake server, and never connects to Exasol
- The `ffi` unit tests run in CI outside the coverage run, because cargo-llvm-cov with the `ffi` feature deadlocks
- Integration tests require a running Exasol instance on port 8563, CI uses the image `exasol/docker-db:2025.2.1`, and the CI integration job runs every integration test target under `tests/`
- DATE and TIMESTAMP values convert between Exasol and Arrow by the year, month, and day that Exasol reports, counted in the proleptic Gregorian calendar, on every read and write path
<!-- /DELTA:CHANGED -->
