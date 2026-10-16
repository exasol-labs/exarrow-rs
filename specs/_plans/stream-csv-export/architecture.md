# Architecture Delta: stream-csv-export
<!-- BASE: 08625e9372c98e4b915b341c6264ae2da050e424 -->

<!-- DELTA:CHANGED -->
## Data Flow

- connection URI -> Driver -> Database -> Connection::from_params -> native transport (default) or websocket transport (`transport=websocket`) -> Exasol: TCP or WebSocket connect, TLS, RSA-encrypted login, session info, all within one connection-timeout deadline that the transport starts in connect and spends in authenticate; then a schema from the URI goes to Exasol as the current-schema session attribute through set-attributes, and a schema that Exasol rejects closes the session and fails the connect
- Connection::set_schema or the ADBC `adbc.connection.db_schema` option -> TransportProtocol set-attributes -> Exasol -> get-attributes -> transport records the current schema that Exasol opened
- every Exasol response that carries the current-schema attribute -> native or websocket transport -> recorded current schema -> Connection::current_schema; an ADBC read of `adbc.connection.db_schema` asks Exasol with get-attributes instead
- SQL text -> Connection::execute_statement -> TransportProtocol::execute_query -> Exasol -> ResultSet: the native transport returns one Arrow RecordBatch per fetch, and the websocket transport returns JSON rows that query converts to RecordBatches with TypeMapper
- ResultSet -> fetch_results per batch -> caller: batches stream lazily through ResultSetIterator, and Connection::query collects all batches in memory
- file, stream, or RecordBatch -> import -> http_transport handshake returns an internal address -> IMPORT SQL through the Connection transport -> with TLS, Exasol starts the TLS handshake on the tunnel as the client and checks the driver certificate against the `PUBLIC KEY` fingerprint -> Exasol pulls CSV chunks, or Parquet byte ranges on servers from 2025.1.11, through the tunnel; when the IMPORT SQL fails, import stops serving the tunnel and returns the error
- Parquet export source -> SELECT text -> TransportProtocol::create_prepared_statement -> result-set column metadata -> Arrow schema -> TransportProtocol::close_prepared_statement, all before the EXPORT SQL runs
- EXPORT SQL through the Connection transport -> with TLS, Exasol starts the TLS handshake on the tunnel as the client -> Exasol pushes CSV through the http_transport tunnel while the statement runs -> http_transport forwards the PUT body in pieces of at most 64 KiB through a 16-slot channel -> export passes each piece to a callback as it arrives, compressed as Exasol sent it, decompresses each piece when compression is set and writes it to a file or stream as it arrives, or collects all pieces for a list, Parquet, RecordBatches, or Arrow IPC; a full channel stops the tunnel read, so the server waits for a slow consumer
- ADBC driver manager -> AdbcDriverExasolInit or ExarrowDriverInit -> adbc_ffi -> adbc Connection on the 2-worker runtime -> RecordBatchReader returned over the C ABI
- ADBC database options `uri`, `username`, and `password` -> adbc_ffi at connection creation -> connection parses the URI once and merges the option credentials into ConnectionParams -> adbc_ffi keeps the ConnectionParams, never the URI string -> Connection::from_params on first use
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Constraints

- Rust edition 2021, and CI pins the stable toolchain 1.92.0
- Cargo feature `native` is the default transport, `websocket` is optional, `ffi` enables `native`, and a build without either transport rejects every connection
- TLS and server certificate validation are on by default
- Connection timeout defaults to 30 seconds and cannot exceed 300 seconds, and idle timeout defaults to 600 seconds
- The connection timeout is one deadline that bounds the TCP connect, the TLS handshake, the WebSocket upgrade, and the login together, and its error names the step that ran out
- HTTP tunnel setup has two bounds of 30 seconds for every import and export, and the connection timeout changes neither: one deadline bounds the TCP connect and the EXA handshake together, and with TLS a separate bound covers the TLS handshake, starts at the first tunnel read or write, which can precede the IMPORT or EXPORT statement, and includes the wait for Exasol's first TLS byte, so time spent in the TCP connect and the EXA handshake does not shorten the 30-second TLS handshake bound; the opt-in CSV export timeout (`CsvExportOptions::timeout_ms`) starts after the EXA handshake, also covers the TLS handshake of a TLS export, and can elapse before the TLS bound with `ExportError::Timeout`; each tunnel setup error names the step that ran out
- A CSV export to a file, stream, or callback holds no more than 16 queued body pieces of at most 64 KiB each, the one piece that http_transport has read and not yet queued, and the piece that a file or stream export is writing with its decoded output, so its buffered body does not grow with the size of the export
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
