# Architecture Delta: fix-result-set-schema-metadata
<!-- BASE: 2cbaaa2173eacf47174b66c4015c5f702c19c774 -->

<!-- DELTA:CHANGED -->
## Data Flow

- connection URI -> Driver -> Database -> Connection::from_params -> native transport (default) or websocket transport (`transport=websocket`) -> Exasol: TCP or WebSocket connect, TLS, RSA-encrypted login that reports the client name and client version from ConnectionParams and the driver name `exarrow-rs`, session info, all within one connection-timeout deadline that the transport starts in connect and spends in authenticate; then a schema from the URI goes to Exasol as the current-schema session attribute through set-attributes, and a schema that Exasol rejects closes the session and fails the connect
- Connection::set_schema or the ADBC `adbc.connection.db_schema` option -> TransportProtocol set-attributes -> Exasol -> get-attributes -> transport records the current schema that Exasol opened
- every Exasol response that carries the current-schema attribute -> native or websocket transport -> recorded current schema -> Connection::current_schema; an ADBC read of `adbc.connection.db_schema` asks Exasol with get-attributes instead
- SQL text -> Connection::execute_statement -> TransportProtocol::execute_query -> Exasol -> ResultSet: the native transport returns one Arrow RecordBatch per fetch, and the websocket transport returns JSON rows that query converts to RecordBatches with TypeMapper; query builds the result set metadata schema from the column metadata with SchemaBuilder, which attaches each column's `exasol:` field metadata, and puts that field metadata on every batch of both transports, keeping the Arrow data types that the native transport decoded
- ResultSet -> fetch_results per batch -> caller: batches stream lazily through ResultSetIterator, and Connection::query collects all batches in memory
- file, stream, or RecordBatch -> import -> http_transport handshake returns an internal address -> IMPORT SQL through the Connection transport -> with TLS, Exasol starts the TLS handshake on the tunnel as the client and checks the driver certificate against the `PUBLIC KEY` fingerprint -> Exasol pulls CSV chunks, or Parquet byte ranges on servers from 2025.1.11, through the tunnel; when the IMPORT SQL fails, import stops serving the tunnel and returns the error
- Parquet export source -> SELECT text -> TransportProtocol::create_prepared_statement -> result-set column metadata -> Arrow schema -> TransportProtocol::close_prepared_statement, all before the EXPORT SQL runs
- EXPORT SQL through the Connection transport -> with TLS, Exasol starts the TLS handshake on the tunnel as the client -> Exasol pushes CSV through the http_transport tunnel while the statement runs -> http_transport forwards the PUT body in pieces of at most 64 KiB through a 16-slot channel -> export passes each piece to a callback as it arrives, compressed as Exasol sent it, decompresses each piece when compression is set and writes it to a file or stream as it arrives, or collects all pieces for a list, Parquet, RecordBatches, or Arrow IPC; a full channel stops the tunnel read, so the server waits for a slow consumer
- ADBC driver manager -> AdbcDriverExasolInit or ExarrowDriverInit -> adbc_ffi -> adbc Connection on the 2-worker runtime -> RecordBatchReader returned over the C ABI
- ADBC database options `uri`, `username`, and `password` -> adbc_ffi at connection creation -> connection parses the URI once and merges the option credentials into ConnectionParams -> adbc_ffi keeps the ConnectionParams, never the URI string -> Connection::from_params on first use
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Interfaces

- Rust API: crate `exarrow_rs` exposes `adbc::{Driver, Database, Connection, Statement}`, import and export functions and options, query builders, and type utilities, all async with `blocking_*` variants for import and export
- Connection string: `exasol://user[:password]@host[:port][/schema][?key=value&...]`, default port 8563, where `/schema` names an existing schema that becomes the session's current schema, keys `user`, `username`, `password`, and `pass` for a credential the userinfo omits, and keys `timeout`, `query_timeout`, `tls`, `validateservercertificate`, `certificate_fingerprint`, `client_name`, `client_version`, `transport=native|websocket`, and their aliases; the deprecated key `idle_timeout` is accepted with a warning and has no effect, and any other key fails the parse with an error that names the key; a username is required and the driver has no default user
- ADBC C ABI: cdylib from the `ffi` feature with entry points `AdbcDriverExasolInit` and `ExarrowDriverInit`, database options `uri`, `username`, and `password`, where a set `username` or `password` option replaces the URI value verbatim, connection option `adbc.connection.db_schema`, which sets the server's current schema and reads it from the server, bulk ingest through option `adbc.ingest.target_table`
- Arrow field metadata: every result-set field, in query results, ADBC RecordBatchReaders, and ADBC `execute_schema`, carries the keys `exasol:type_name` and `exasol:type`, plus `exasol:precision` and `exasol:scale` for DECIMAL and `exasol:size` for CHAR and VARCHAR; every result-set field is nullable, because Exasol reports no column nullability in result set metadata
- Exasol native protocol: binary TCP messages with a 21-byte little-endian header and ChaCha20 encryption after login
- Exasol WebSocket protocol: JSON commands and responses over WebSocket with TLS
- Exasol HTTP tunnel: EXA magic packet handshake; with TLS, a TLS handshake that Exasol starts as the client after it receives the IMPORT or EXPORT statement, with the driver as the TLS server; then HTTP/1.1 chunked CSV transfer or byte-range requests for Parquet files
- Benchmark binaries: `generate_data` and `benchmark`, built only with the `benchmark` feature
<!-- /DELTA:CHANGED -->
