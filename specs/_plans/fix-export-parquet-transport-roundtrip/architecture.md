# Architecture Delta: fix-export-parquet-transport-roundtrip
<!-- BASE: 51e1eee775c59c00593d50fe74fe4ab5da9b6bb9 -->

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
- types (src/types/): maps Exasol types to Arrow types, infers table schemas from CSV and Parquet files, and quotes identifiers | owns: ExasolType and TypeMapper definitions | depends on: import, error
- arrow_conversion (src/arrow_conversion/): public utility that converts Exasol JSON column data into Arrow arrays, with no caller inside the crate | owns: none | depends on: transport core, types, error
- error (src/error.rs): crate error types for connection, query, conversion, and transport failures | owns: none | depends on: none
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Data Flow

- connection URI -> Driver -> Database -> Connection::from_params -> native transport (default) or websocket transport (`transport=websocket`) -> Exasol: TCP or WebSocket connect, TLS, RSA-encrypted login, session info
- SQL text -> Connection::execute_statement -> TransportProtocol::execute_query -> Exasol -> ResultSet: the native transport returns one Arrow RecordBatch per fetch, and the websocket transport returns JSON rows that query converts to RecordBatches with TypeMapper
- ResultSet -> fetch_results per batch -> caller: batches stream lazily through ResultSetIterator, and Connection::query collects all batches in memory
- file, stream, or RecordBatch -> import -> http_transport handshake returns an internal address -> IMPORT SQL through the Connection transport -> Exasol pulls CSV chunks, or Parquet byte ranges on servers from 2025.1.11, through the tunnel
- Parquet export source -> SELECT text -> TransportProtocol::create_prepared_statement -> result-set column metadata -> Arrow schema -> TransportProtocol::close_prepared_statement, all before the EXPORT SQL runs
- EXPORT SQL through the Connection transport -> Exasol pushes CSV through the http_transport tunnel -> export -> file, stream, list, callback, Parquet, RecordBatches, or Arrow IPC
- ADBC driver manager -> AdbcDriverExasolInit or ExarrowDriverInit -> adbc_ffi -> adbc Connection on the 2-worker runtime -> RecordBatchReader returned over the C ABI
<!-- /DELTA:CHANGED -->
