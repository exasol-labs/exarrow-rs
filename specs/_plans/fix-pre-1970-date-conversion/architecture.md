# Architecture Delta: fix-pre-1970-date-conversion
<!-- BASE: e69f6946f6165fd3ba1d8ce39272f560164a28cb -->

<!-- DELTA:CHANGED -->
## Constraints

- Rust edition 2021, and CI pins the stable toolchain 1.92.0
- Cargo feature `native` is the default transport, `websocket` is optional, `ffi` enables `native`, and a build without either transport rejects every connection
- TLS and server certificate validation are on by default
- Connection timeout defaults to 30 seconds and cannot exceed 300 seconds, and idle timeout defaults to 600 seconds
- The native transport fetches up to the server's maximum data message size per batch, with a 64 MiB fallback
- The FFI runtime is a multi-thread Tokio runtime with 2 workers so that import can run WebSocket and HTTP I/O at the same time inside `block_on`
- Native Parquet import requires Exasol 2025.1.11 or later, and older servers receive Parquet converted to CSV
- Password encryption uses RSA PKCS#1 v1.5 through num-bigint because Exasol servers can send 1024-bit keys, which aws-lc-rs rejects
- Credentials are never logged or exposed, and Connection debug output omits the password
- Results stream as Arrow RecordBatches, and conversion is Arrow-native and zero-copy where possible
- CI rejects clippy warnings, cargo-deny license findings, cargo-deny advisory findings in the dependencies of every Cargo feature, an integration test target that the integration job does not run, production line coverage below 80 percent, any file below 50 percent, and a failed SonarQube Cloud quality gate (intended to become a required check once rolled out)
- Integration tests require a running Exasol instance on port 8563, CI uses the image `exasol/docker-db:2025.2.1`, and the CI integration job runs every integration test target under `tests/`
- DATE and TIMESTAMP values convert between Exasol and Arrow by the year, month, and day that Exasol reports, counted in the proleptic Gregorian calendar, on every read and write path
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Components

- adbc (src/adbc/): public ADBC API, where Driver opens a Database from a URI and Connection runs statements, prepared statements, transactions, metadata queries, imports, exports, and blocking wrappers | owns: transport handle shared as Arc<Mutex<dyn TransportProtocol>>, session state, connection parameters, a process-global current-thread Tokio runtime for blocking calls | depends on: connection, query, transport, import, export, error
- adbc_ffi (src/adbc_ffi.rs): C ABI ADBC driver for driver managers, built only with the `ffi` feature, that maps ADBC options, GetObjects, statement binding, and bulk ingest onto the adbc Connection | owns: process-global multi-thread Tokio runtime with 2 workers | depends on: adbc, connection, query, import, transport, types, error
- connection (src/connection/): parses `exasol://` URIs and builder settings, holds session state, and checks the server version for features | owns: ConnectionParams, Session state, native Parquet support flag | depends on: error
- query (src/query/): Statement data container, PreparedStatement, ResultSet with lazy batch fetching, and builders for IMPORT and EXPORT SQL | owns: result set handles, bound parameters | depends on: transport, types, error
- transport core (src/transport/protocol.rs, src/transport/messages.rs): TransportProtocol trait and shared message types for connect, authenticate, execute, fetch, prepare, and close | owns: Credentials, which clear the password on drop | depends on: error
- native transport (src/transport/native/): default Exasol binary TCP protocol with TLS, RSA password login, ChaCha20 stream encryption, 21-byte little-endian message headers, and fetch results parsed directly into Arrow RecordBatches | owns: TCP stream, cipher state, message serial counter | depends on: transport core, tls, types, error
- websocket transport (src/transport/websocket.rs): optional Exasol JSON-over-WebSocket protocol, built only with the `websocket` feature, that returns results as row-major JSON | owns: WebSocket stream | depends on: transport core, tls, error
- http_transport (src/transport/http_transport.rs): bulk data tunnel that opens an outbound TCP connection to Exasol, runs the EXA magic-packet handshake, wraps the socket in TLS with an ad-hoc certificate, and speaks chunked HTTP and byte-range HTTP | owns: tunnel socket, generated TLS certificate and its fingerprint | depends on: tls, error
- tls (src/transport/tls.rs): shared rustls certificate verifiers that accept any certificate or match a SHA-256 fingerprint | owns: none | depends on: none
- import (src/import/): imports CSV, Parquet, Arrow RecordBatches, and Arrow IPC into tables, converts non-CSV input to CSV while streaming or serves Parquet files natively, and runs parallel multi-file imports | owns: ParallelTransportPool of tunnel connections | depends on: http_transport, query, types, error
- export (src/export/): exports tables or queries to CSV files, streams, lists, callbacks, Parquet, Arrow RecordBatches, and Arrow IPC by parsing the CSV stream that Exasol sends, and takes a Parquet export's column names and types from the prepared-statement result-set metadata of the export source | owns: none | depends on: http_transport, transport core, query, types, error
- types (src/types/): maps Exasol types to Arrow types, infers table schemas from CSV and Parquet files, quotes identifiers, and converts DATE and TIMESTAMP values between Exasol text and Arrow day and microsecond counts in both directions | owns: ExasolType and TypeMapper definitions | depends on: import, error
- arrow_conversion (src/arrow_conversion/): public utility that converts Exasol JSON column data into Arrow arrays, with no caller inside the crate | owns: none | depends on: transport core, types, error
- error (src/error.rs): crate error types for connection, query, conversion, and transport failures | owns: none | depends on: none
<!-- /DELTA:CHANGED -->
