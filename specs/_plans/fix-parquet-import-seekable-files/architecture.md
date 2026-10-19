# Architecture Delta: fix-parquet-import-seekable-files
<!-- BASE: 420b1b55ddad7d81e807d696c159beea2aa895cf -->

<!-- DELTA:CHANGED -->
## Constraints

- Rust edition 2021, and CI pins the stable toolchain
- Cargo feature `native` is the default transport, `websocket` is optional, `ffi` enables `native`, and a build without either transport rejects every connection
- TLS and server certificate validation are on by default
- Connection timeout defaults to 30 seconds and cannot exceed 300 seconds, and the driver has no idle timeout
- The connection timeout is one deadline that bounds the TCP connect, the TLS handshake, the WebSocket upgrade, and the login together, and its error names the step that ran out
- HTTP tunnel setup has two bounds of 30 seconds for every import and export, and the connection timeout changes neither: one deadline bounds the TCP connect and the EXA handshake together, and with TLS a separate bound covers the TLS handshake, starts at the first tunnel read or write, which can precede the IMPORT or EXPORT statement, and includes the wait for Exasol's first TLS byte, so time spent in the TCP connect and the EXA handshake does not shorten the 30-second TLS handshake bound; the opt-in CSV export timeout (`CsvExportOptions::timeout_ms`) starts after the EXA handshake, also covers the TLS handshake of a TLS export, and can elapse before the TLS bound with `ExportError::Timeout`; each tunnel setup error names the step that ran out
- A CSV export to a file, stream, or callback holds no more than 16 queued body pieces of at most 64 KiB each, the one piece that http_transport has read and not yet queued, and the piece that a file or stream export is writing with its decoded output, so its buffered body does not grow with the size of the export
- A native Parquet import from a file on disk reads each byte range from the file when Exasol requests it, in pieces of at most 64 KiB, so the file data it holds does not grow with the size of the file or of the range; a Parquet import from a stream holds the whole stream
- The native transport fetches up to the server's maximum data message size per batch, with a 64 MiB fallback
- The native transport runs the parameter set of a statement that returns an affected-row count as consecutive prepared-statement executions when one data message would exceed the server's maximum data message size, and each execution's data message stays within that size unless it holds a single row
- The FFI runtime is a multi-thread Tokio runtime with 2 workers so that import can run WebSocket and HTTP I/O at the same time inside `block_on`
- Native Parquet import requires Exasol 2025.1.11 or later, and older servers receive Parquet converted to CSV
- Password encryption uses RSA PKCS#1 v1.5 through num-bigint because Exasol servers can send 1024-bit keys, which aws-lc-rs rejects
- Credentials are never logged or exposed, the Debug output of Connection, ConnectionParams, and ConnectionBuilder omits the password, and a connection URI parse error names the field, the query key, or the query parameter position, never repeats the userinfo or a query value, and names an unknown query key as written, so a password with an unencoded `?` can show a fragment in that error
- Runtime warnings go through the `log` crate facade, so the application's logger decides whether and where they appear
- Results stream as Arrow RecordBatches, and conversion is Arrow-native and zero-copy where possible
- CI rejects clippy warnings, cargo-deny license findings, cargo-deny advisory findings in the dependencies of every Cargo feature, a failing library unit test with the default features, the `websocket` feature, or the `ffi` feature, an integration test target that the integration job does not run, production line coverage below 80 percent, any file below 50 percent, and a failed SonarQube Cloud quality gate (intended to become a required check once rolled out)
- Library unit tests run in CI without a database server, so a unit test uses a test double, such as a mock transport or a local fake server, and never connects to Exasol
- The `ffi` unit tests run in CI outside the coverage run, because cargo-llvm-cov with the `ffi` feature deadlocks
- Integration tests require a running Exasol instance on port 8563, CI uses the `exasol/docker-db` image, and the CI integration job runs every integration test target under `tests/`
- DATE and TIMESTAMP values convert between Exasol and Arrow by the year, month, and day that Exasol reports, counted in the proleptic Gregorian calendar, on every read and write path
<!-- /DELTA:CHANGED -->
