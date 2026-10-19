//! CSV export functionality for Exasol.
//!
//! This module provides functions for exporting data from Exasol tables or query results
//! to CSV format via files, streams, in-memory lists, or custom callbacks.
//!
//! # Architecture
//!
//! The export process uses **client mode** where:
//! 1. We connect TO Exasol (outbound connection - works through firewalls)
//! 2. Perform EXA tunneling handshake to receive an internal address
//! 3. Execute an EXPORT SQL statement using the internal address
//! 4. Exasol sends data through the established connection
//! 5. We read and process the data stream
//!
//! This client mode approach works with cloud Exasol instances and through NAT/firewalls.

use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bzip2::read::BzDecoder;
use flate2::read::MultiGzDecoder;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

use crate::query::export::{Compression, ExportQuery, ExportSource, RowSeparator};
use crate::transport::protocol::TransportProtocol;
use crate::transport::HttpTransportClient;

/// How many export body pieces wait for the consumer before the driver stops
/// reading the tunnel.
const DEFAULT_PIPE_BUFFER_SIZE: usize = 16;

/// Error types for export operations.
#[derive(Error, Debug)]
pub enum ExportError {
    /// I/O error during export.
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    /// Transport error during export.
    #[error("Transport error: {0}")]
    TransportError(#[from] crate::error::TransportError),

    /// HTTP transport setup error.
    #[error("HTTP transport error: {message}")]
    HttpTransportError { message: String },

    /// SQL execution error.
    #[error("SQL execution error: {message}")]
    SqlExecutionError { message: String },

    /// CSV parsing error.
    #[error("CSV parsing error at row {row}: {message}")]
    CsvParseError { row: usize, message: String },

    /// Decompression error.
    #[error("Decompression error: {0}")]
    DecompressionError(String),

    /// Channel communication error.
    #[error("Channel error: {0}")]
    ChannelError(String),

    /// An explicitly configured export timeout elapsed.
    ///
    /// `transport_terminated` reports whether the driver had to terminate the
    /// transport, so a caller decides whether to reconnect without probing the
    /// connection. It is `true` when the timeout elapsed before the EXPORT
    /// response was read, because that response can no longer be matched to a
    /// request.
    #[error(
        "Export timed out after {timeout_ms}ms; {}",
        if *transport_terminated {
            "the transport was terminated, reconnect before the next operation"
        } else {
            "the transport is still usable"
        }
    )]
    Timeout {
        /// The configured timeout in milliseconds that elapsed.
        timeout_ms: u64,
        /// Whether the driver terminated the transport before returning.
        transport_terminated: bool,
    },

    /// Export was cancelled.
    #[error("Export was cancelled")]
    Cancelled,
}

/// Options for CSV export configuration.
#[derive(Debug, Clone)]
pub struct CsvExportOptions {
    /// Column separator character (default: ',').
    pub column_separator: char,

    /// Column delimiter character for quoting (default: '"').
    pub column_delimiter: char,

    /// Row separator (default: LF).
    pub row_separator: RowSeparator,

    /// Character encoding (default: "UTF-8").
    pub encoding: String,

    /// Custom NULL value representation (default: None, empty string).
    pub null_value: Option<String>,

    /// Compression type (default: None).
    pub compression: Compression,

    /// Whether to include column headers in the output (default: false).
    pub with_column_names: bool,

    /// Whether to encrypt the HTTP transport tunnel with TLS (default: `false`).
    /// `true` encrypts the tunnel, including against Exasol Docker. The control
    /// connection's TLS does not cover the tunnel data.
    pub use_tls: bool,

    /// Client-side timeout in milliseconds for the export operation (default: `None`).
    ///
    /// `None` arms no timer, so the export runs until the server finishes the
    /// EXPORT statement and a server-enforced query timeout is the only bound.
    /// `Some(ms)` bounds SQL execution, tunnel transfer, and the caller's
    /// callback together. When such a bound elapses before the EXPORT response
    /// has been read, the driver terminates the transport, because that
    /// response can no longer be matched to a request.
    ///
    /// Tunnel setup comes before this bound and has its own 30-second limit.
    pub timeout_ms: Option<u64>,

    /// Exasol host for HTTP transport connection.
    /// This is typically the same host as the WebSocket connection.
    pub host: String,

    /// Exasol port for HTTP transport connection.
    /// This is typically the same port as the WebSocket connection.
    pub port: u16,
}

impl Default for CsvExportOptions {
    fn default() -> Self {
        Self {
            column_separator: ',',
            column_delimiter: '"',
            row_separator: RowSeparator::LF,
            encoding: "UTF-8".to_string(),
            null_value: None,
            compression: Compression::None,
            with_column_names: false,
            use_tls: false,
            timeout_ms: None,
            host: String::new(),
            port: 0,
        }
    }
}

impl CsvExportOptions {
    /// Creates new export options with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn column_separator(mut self, sep: char) -> Self {
        self.column_separator = sep;
        self
    }

    #[must_use]
    pub fn column_delimiter(mut self, delim: char) -> Self {
        self.column_delimiter = delim;
        self
    }

    #[must_use]
    pub fn row_separator(mut self, sep: RowSeparator) -> Self {
        self.row_separator = sep;
        self
    }

    #[must_use]
    pub fn encoding(mut self, enc: &str) -> Self {
        self.encoding = enc.to_string();
        self
    }

    #[must_use]
    pub fn null_value(mut self, val: &str) -> Self {
        self.null_value = Some(val.to_string());
        self
    }

    #[must_use]
    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    #[must_use]
    pub fn with_column_names(mut self, include: bool) -> Self {
        self.with_column_names = include;
        self
    }

    #[must_use]
    pub fn use_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    /// Bounds SQL execution, tunnel transfer, and the callback together with a
    /// client-side timer. Tunnel setup is not part of it.
    ///
    /// Unset by default. An elapsed bound can terminate the transport; see the
    /// `timeout_ms` field for when that happens.
    #[must_use]
    pub fn timeout_ms(mut self, timeout: u64) -> Self {
        self.timeout_ms = Some(timeout);
        self
    }

    /// Sets the Exasol host for HTTP transport connection.
    ///
    /// This is typically the same host as the WebSocket connection.
    #[must_use]
    pub fn exasol_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Sets the Exasol port for HTTP transport connection.
    ///
    /// This is typically the same port as the WebSocket connection.
    #[must_use]
    pub fn exasol_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
}

/// Fields the Arrow and Parquet export paths share when they build the
/// `CsvExportOptions` for their underlying CSV export. Named fields, rather
/// than five positional parameters, because two of the five are adjacent
/// `char`s and a transposed pair would compile while silently producing wrong
/// CSV.
pub(crate) struct SharedCsvExportParams<'a> {
    pub column_separator: char,
    pub column_delimiter: char,
    pub host: &'a str,
    pub port: u16,
    pub use_tls: bool,
}

/// Builds the `CsvExportOptions` shared by the Arrow and Parquet export
/// paths, which pipe their own export through a CSV export and then
/// reinterpret the rows. Column headers are always disabled, because the
/// caller reinterprets rows positionally rather than by name, and no export
/// timeout is set, because neither `ArrowExportOptions` nor
/// `ParquetExportOptions` exposes one for its caller to configure.
pub(crate) fn shared_csv_export_options(params: SharedCsvExportParams<'_>) -> CsvExportOptions {
    CsvExportOptions::default()
        .column_separator(params.column_separator)
        .column_delimiter(params.column_delimiter)
        .with_column_names(false)
        .exasol_host(params.host)
        .exasol_port(params.port)
        .use_tls(params.use_tls)
}

/// Receiver end of the data pipe for processing exported data.
pub struct DataPipeReceiver {
    rx: mpsc::Receiver<Vec<u8>>,
}

impl DataPipeReceiver {
    /// Receives the next piece of the export body while the EXPORT statement
    /// runs.
    ///
    /// A piece holds at most 64 KiB (65536 bytes) of the body, in the order
    /// the tunnel delivers it. With compression configured, a piece carries
    /// the compressed bytes as Exasol sent them. Piece boundaries follow the
    /// tunnel transfer, not CSV records, so a record can span two pieces.
    ///
    /// At most 16 pieces wait here. While they wait, the driver stops reading
    /// the tunnel and Exasol stops sending, so a slow consumer slows the
    /// EXPORT statement.
    ///
    /// Returns `None` once the body has ended, completely or cut short by a
    /// broken tunnel; [`export_to_callback`] reports which.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.rx.recv().await
    }
}

fn data_pipe() -> (mpsc::Sender<Vec<u8>>, DataPipeReceiver) {
    let (sink, rx) = mpsc::channel(DEFAULT_PIPE_BUFFER_SIZE);
    (sink, DataPipeReceiver { rx })
}

/// Exports data from an Exasol table or query to a file.
///
/// The file receives the body while the EXPORT statement runs: each piece of
/// at most 64 KiB (65536 bytes) is written as it arrives, and a compressed
/// piece is decompressed first. Slow writes to the file slow the EXPORT
/// statement on the server. When the export fails, the file can hold partial
/// output. Errors follow the precedence that [`export_to_callback`] documents.
///
/// # Arguments
///
/// * `ws_transport` - WebSocket transport for executing SQL
/// * `source` - The data source (table or query)
/// * `file_path` - Path to the output file
/// * `options` - Export options
///
/// # Returns
///
/// The number of rows exported on success.
///
/// # Errors
///
/// Returns `ExportError` if the export fails.
///
/// # Example
///
pub async fn export_to_file<T: TransportProtocol + ?Sized>(
    ws_transport: &mut T,
    source: ExportSource,
    file_path: &Path,
    options: CsvExportOptions,
) -> Result<u64, ExportError> {
    // Create the output file
    let file = File::create(file_path).await?;
    let writer = BufWriter::new(file);

    export_to_stream(ws_transport, source, writer, options).await
}

/// Exports data from an Exasol table or query to an async writer.
///
/// The writer receives the body while the EXPORT statement runs: each piece
/// of at most 64 KiB (65536 bytes) is written as it arrives, and a compressed
/// piece is decompressed first. A slow writer slows the EXPORT statement on
/// the server. When the export fails, the writer can hold partial output.
/// Errors follow the precedence that [`export_to_callback`] documents.
///
/// # Arguments
///
/// * `ws_transport` - WebSocket transport for executing SQL
/// * `source` - The data source (table or query)
/// * `writer` - Async writer to write the CSV data to
/// * `options` - Export options
///
/// # Returns
///
/// The number of rows exported on success.
///
/// # Errors
///
/// Returns `ExportError` if the export fails.
pub async fn export_to_stream<T: TransportProtocol + ?Sized, W: AsyncWrite + Unpin>(
    ws_transport: &mut T,
    source: ExportSource,
    mut writer: W,
    options: CsvExportOptions,
) -> Result<u64, ExportError> {
    let compression = options.compression;

    export_to_callback(ws_transport, source, options, |mut receiver| async move {
        write_decoded(&mut receiver, compression, &mut writer).await
    })
    .await
}

fn count_line_feeds(data: &[u8]) -> u64 {
    data.iter().filter(|&&byte| byte == b'\n').count() as u64
}

/// Exports data from an Exasol table or query to an in-memory list of rows.
///
/// Each row is represented as a vector of string values.
///
/// # Arguments
///
/// * `ws_transport` - WebSocket transport for executing SQL
/// * `source` - The data source (table or query)
/// * `options` - Export options
///
/// # Returns
///
/// A vector of rows, where each row is a vector of column values.
///
/// # Errors
///
/// Returns `ExportError` if the export fails.
///
/// # Example
///
pub async fn export_to_list<T: TransportProtocol + ?Sized>(
    ws_transport: &mut T,
    source: ExportSource,
    options: CsvExportOptions,
) -> Result<Vec<Vec<String>>, ExportError> {
    let separator = options.column_separator;
    let delimiter = options.column_delimiter;
    let compression = options.compression;

    export_to_callback(ws_transport, source, options, |mut receiver| async move {
        let mut data = Vec::new();
        write_decoded(&mut receiver, compression, &mut data).await?;

        let csv_string = String::from_utf8(data).map_err(|e| ExportError::CsvParseError {
            row: 0,
            message: format!("Invalid UTF-8: {}", e),
        })?;

        parse_csv(&csv_string, separator, delimiter)
    })
    .await
}

/// Exports data from an Exasol table or query using a custom callback.
///
/// This is the most flexible export method, allowing you to process the data
/// stream however you need.
///
/// # Streaming
///
/// The callback runs while the EXPORT statement runs and receives the body
/// through its [`DataPipeReceiver`] in pieces of at most 64 KiB (65536
/// bytes), in tunnel order. With compression configured, the pieces carry the
/// compressed bytes as Exasol sent them. A CSV record can span two pieces.
///
/// At most 16 pieces wait for the callback. While they wait, the driver stops
/// reading the tunnel, so a slow callback slows the EXPORT statement on the
/// server, and memory does not grow with the size of the export. Exasol
/// returns the EXPORT statement only after the driver has read the whole body,
/// which is why the callback cannot wait for the statement before it reads.
///
/// A callback may return `Ok` before it has received every piece. The driver
/// then reads and discards the rest of the body, lets the statement finish,
/// and returns the callback's value when the statement succeeds.
///
/// # Timeout behavior
///
/// The tunnel's TCP connect and EXA handshake come first and are bounded by
/// the 30-second setup deadline of `HttpTransportClient::connect`, whatever
/// `options.timeout_ms` says. A stalled step fails with
/// `ExportError::HttpTransportError`.
///
/// `options.timeout_ms` bounds what follows (SQL execution, tunnel transfer,
/// and the callback) and is unset by default. With TLS, the tunnel task runs
/// the TLS handshake under its own 30-second bound inside that window, so a
/// shorter `timeout_ms` can elapse first. Only a TLS tunnel's handshake has a
/// bound: the tunnel read after it has none, so a failing EXPORT statement
/// aborts the pending transport task instead of being awaited alongside it.
///
/// When a configured bound elapses, the transport is terminated only if the EXPORT response
/// had not been read yet, because an elapse during the callback leaves the
/// transport in sync and terminating it would break a healthy connection.
/// A slow callback that stalls the transfer keeps the response unread, so a
/// timeout during such a stall terminates the transport.
/// Tracking that with an `AtomicBool` rather than a `Cell` is deliberate:
/// `&Cell<bool>` is not `Send`, which would make this future `!Send` and stop
/// callers from spawning a long export.
///
/// # Arguments
///
/// * `ws_transport` - WebSocket transport for executing SQL
/// * `source` - The data source (table or query)
/// * `options` - Export options
/// * `callback` - A callback function that receives a `DataPipeReceiver` and processes the data
///
/// # Returns
///
/// The result of the callback function.
///
/// # Errors
///
/// When several parts fail, the first matching rule decides the error:
///
/// 1. The callback fails after a broken tunnel ended its data early: the
///    driver waits for the statement and returns
///    `ExportError::SqlExecutionError`, or the tunnel's
///    `ExportError::HttpTransportError` if the statement succeeded.
/// 2. The callback fails before the statement responds: the driver closes the
///    tunnel, waits for the statement, and returns the callback's error.
/// 3. The statement fails: the driver returns `ExportError::SqlExecutionError`
///    at once, without waiting for the callback or the tunnel. The callback can
///    have received part of the body.
/// 4. The tunnel failed: `ExportError::HttpTransportError`.
/// 5. Otherwise the callback's result.
pub async fn export_to_callback<T, F, Fut, R>(
    ws_transport: &mut T,
    source: ExportSource,
    options: CsvExportOptions,
    callback: F,
) -> Result<R, ExportError>
where
    T: TransportProtocol + ?Sized,
    F: FnOnce(DataPipeReceiver) -> Fut,
    Fut: Future<Output = Result<R, ExportError>>,
{
    // Connect to Exasol via HTTP transport client (performs handshake automatically)
    let client = HttpTransportClient::connect(&options.host, options.port, options.use_tls)
        .await
        .map_err(|e| ExportError::HttpTransportError {
            message: format!("Failed to connect to Exasol: {e}"),
        })?;

    // Get internal address and fingerprint from the handshake response
    let internal_addr = client.internal_address().to_string();
    let fingerprint = client.public_key_fingerprint().map(|s| s.to_string());

    // Build the EXPORT query
    let mut query_builder = match source {
        ExportSource::Table {
            ref schema,
            ref name,
            ref columns,
        } => {
            let mut builder = ExportQuery::from_table(name);
            if let Some(s) = schema {
                builder = builder.schema(s);
            }
            if !columns.is_empty() {
                builder = builder.columns(columns.iter().map(|s| s.as_str()).collect());
            }
            builder
        }
        ExportSource::Query { ref sql } => ExportQuery::from_query(sql),
    };

    // Apply options to the query builder, using internal address from handshake
    query_builder = query_builder
        .at_address(&internal_addr)
        .column_separator(options.column_separator)
        .column_delimiter(options.column_delimiter)
        .row_separator(options.row_separator)
        .encoding(&options.encoding)
        .with_column_names(options.with_column_names)
        .compressed(options.compression);

    if let Some(ref null_val) = options.null_value {
        query_builder = query_builder.null_value(null_val);
    }

    if let Some(ref fp) = fingerprint {
        query_builder = query_builder.with_public_key(fp);
    }

    let export_sql = query_builder.build();

    let (sink, receiver) = data_pipe();
    let mut tunnel = TunnelTask::spawn(client, sink);

    let sql_response_read = AtomicBool::new(false);

    // Execute the EXPORT SQL in parallel
    // This triggers Exasol to send data through the established connection
    //
    // The flag is set here, inside `sql_task` itself, rather than after
    // `sql_task.await` resolves in `work` below. Setting it as part of
    // `sql_task` completing (success or error) rules out any dependence on
    // `work` reaching the next statement before a cancellation could
    // intervene, regardless of how `work`'s composition evolves later.
    let sql_task = async {
        let result = ws_transport.execute_query(&export_sql).await.map_err(|e| {
            ExportError::SqlExecutionError {
                message: e.to_string(),
            }
        });
        sql_response_read.store(true, Ordering::Relaxed);
        result
    };

    let callback_task = callback(receiver);

    // The callback must drain the pipe while the statement runs, because Exasol
    // answers the statement only after the driver read the whole body.
    let work = settle_export(sql_task, callback_task, &mut tunnel);

    let outcome = match options.timeout_ms {
        Some(bound_ms) => tokio::time::timeout(tokio::time::Duration::from_millis(bound_ms), work)
            .await
            .map_err(|_| bound_ms),
        None => Ok(work.await),
    };

    match outcome {
        Ok(result) => result,
        Err(bound_ms) => {
            tunnel.abort();
            let transport_terminated = !sql_response_read.load(Ordering::Relaxed);
            if transport_terminated {
                ws_transport.terminate();
            }
            Err(ExportError::Timeout {
                timeout_ms: bound_ms,
                transport_terminated,
            })
        }
    }
}

/// Forwards the EXPORT body from the tunnel to `sink` and answers 200 OK.
///
/// A failure is recorded in `failed` before `sink` is dropped, so a consumer
/// that sees its data end can tell a body cut short by a broken tunnel from a
/// complete one, without racing the end of this task.
async fn serve_export_tunnel(
    mut client: HttpTransportClient,
    sink: mpsc::Sender<Vec<u8>>,
    failed: Arc<AtomicBool>,
) -> Result<(), ExportError> {
    let forwarded = client.handle_export_request(&sink).await;
    if forwarded.is_err() {
        failed.store(true, Ordering::Release);
    }
    drop(sink);

    if let Err(e) = forwarded {
        return Err(ExportError::HttpTransportError {
            message: format!("Failed to handle export request: {e}"),
        });
    }
    let _ = client.shutdown().await;
    Ok(())
}

/// Hands the pieces from the tunnel to `sink` until `discard` fires, then
/// keeps draining the tunnel without forwarding. A consumer that returned but
/// kept its receiver alive would otherwise leave the tunnel blocked on `send`.
async fn relay_pieces(
    mut from_tunnel: mpsc::Receiver<Vec<u8>>,
    sink: mpsc::Sender<Vec<u8>>,
    discard: Arc<Notify>,
) {
    let mut discarding = false;
    while let Some(piece) = from_tunnel.recv().await {
        if discarding {
            continue;
        }
        tokio::select! {
            sent = sink.send(piece) => discarding = sent.is_err(),
            () = discard.notified() => discarding = true,
        }
    }
}

struct TunnelTask {
    handle: JoinHandle<Result<(), ExportError>>,
    relay: JoinHandle<()>,
    discard: Arc<Notify>,
    failed: Arc<AtomicBool>,
}

impl TunnelTask {
    fn spawn(client: HttpTransportClient, sink: mpsc::Sender<Vec<u8>>) -> Self {
        let failed = Arc::new(AtomicBool::new(false));
        let discard = Arc::new(Notify::new());
        let (tunnel_sink, from_tunnel) = mpsc::channel(1);
        let relay = tokio::spawn(relay_pieces(from_tunnel, sink, Arc::clone(&discard)));
        let handle = tokio::spawn(serve_export_tunnel(
            client,
            tunnel_sink,
            Arc::clone(&failed),
        ));
        Self {
            handle,
            relay,
            discard,
            failed,
        }
    }

    /// Stops forwarding pieces to the consumer, which has returned. The tunnel
    /// still reads the body to its end so the request can be answered.
    fn discard(&self) {
        self.discard.notify_one();
    }

    fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    fn abort(&self) {
        self.handle.abort();
        self.relay.abort();
    }

    async fn join(&mut self) -> Result<(), ExportError> {
        (&mut self.handle)
            .await
            .map_err(|e| ExportError::HttpTransportError {
                message: format!("HTTP task panicked: {e}"),
            })?
    }
}

/// Awaits the statement, the callback, and the tunnel together and applies
/// the error precedence that [`export_to_callback`] documents.
async fn settle_export<Q, R>(
    sql_task: impl Future<Output = Result<Q, ExportError>>,
    callback_task: impl Future<Output = Result<R, ExportError>>,
    tunnel: &mut TunnelTask,
) -> Result<R, ExportError> {
    tokio::pin!(sql_task, callback_task);
    tokio::select! {
        biased;
        callback_result = &mut callback_task => match callback_result {
            Ok(value) => {
                tunnel.discard();
                if let Err(sql_error) = sql_task.await {
                    tunnel.abort();
                    return Err(sql_error);
                }
                tunnel.join().await?;
                Ok(value)
            }
            // The tunnel failed before the callback saw its data end, so
            // the callback failed on a body that a broken tunnel cut short.
            Err(callback_error) if tunnel.has_failed() => {
                sql_task.await?;
                tunnel.join().await?;
                Err(callback_error)
            }
            Err(callback_error) => {
                tunnel.abort();
                // Closing the tunnel fails the statement; reading its
                // response keeps the transport in sync.
                let _ = sql_task.await;
                Err(callback_error)
            }
        },
        sql_result = &mut sql_task => match sql_result {
            Err(sql_error) => {
                tunnel.abort();
                Err(sql_error)
            }
            Ok(_) => {
                let (tunnel_result, callback_result) =
                    tokio::join!(tunnel.join(), callback_task);
                tunnel_result?;
                callback_result
            }
        },
    }
}

/// Upper bound of one decoded block. A highly compressible piece decodes to
/// many blocks, and the consumer awaits each one before the next is decoded.
const DECODED_BLOCK_MAX_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct FeedState {
    data: Vec<u8>,
    position: usize,
    ended: bool,
}

/// The input of a decoder: it hands out the pieces pushed so far and reports
/// `WouldBlock` when it needs the next one, so a decoder never has to buffer
/// the output of a piece it cannot finish.
struct PieceFeed(Arc<Mutex<FeedState>>);

impl io::Read for PieceFeed {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let len = out.len().min(state.data.len() - state.position);
        if len == 0 {
            return match state.ended || out.is_empty() {
                true => Ok(0),
                false => Err(io::ErrorKind::WouldBlock.into()),
            };
        }
        out[..len].copy_from_slice(&state.data[state.position..state.position + len]);
        state.position += len;
        Ok(len)
    }
}

/// Decodes an export body piece by piece into blocks of at most
/// [`DECODED_BLOCK_MAX_BYTES`], so neither a compressed body nor the decoded
/// output of one compressed piece has to be complete in memory.
struct PieceDecoder {
    feed: Arc<Mutex<FeedState>>,
    reader: Box<dyn io::Read + Send>,
    codec: &'static str,
}

impl PieceDecoder {
    fn new(compression: Compression) -> Self {
        let feed = Arc::new(Mutex::new(FeedState::default()));
        let input = PieceFeed(Arc::clone(&feed));
        let (reader, codec): (Box<dyn io::Read + Send>, _) = match compression {
            Compression::None => (Box::new(input), "plain"),
            Compression::Gzip => (Box::new(MultiGzDecoder::new(input)), "gzip"),
            Compression::Bzip2 => (Box::new(BzDecoder::new(input)), "bzip2"),
        };
        Self {
            feed,
            reader,
            codec,
        }
    }

    fn feed_state(&self) -> std::sync::MutexGuard<'_, FeedState> {
        self.feed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds the next piece of the body. Call [`Self::next_block`] until it
    /// returns `None` before the next push.
    fn push(&mut self, piece: Vec<u8>) {
        let mut state = self.feed_state();
        if state.position == state.data.len() {
            state.data = piece;
            state.position = 0;
        } else {
            state.data.extend_from_slice(&piece);
        }
    }

    /// Marks the end of the body. `next_block` then fails when the compressed
    /// stream did not reach its end.
    fn end(&mut self) {
        self.feed_state().ended = true;
    }

    /// Returns the next decoded block, or `None` when the pushed input is used
    /// up (or, after [`Self::end`], the body is complete).
    fn next_block(&mut self) -> Result<Option<Vec<u8>>, ExportError> {
        let mut block = vec![0u8; DECODED_BLOCK_MAX_BYTES];
        match self.reader.read(&mut block) {
            Ok(0) => Ok(None),
            Ok(len) => {
                block.truncate(len);
                Ok(Some(block))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                Err(ExportError::DecompressionError(format!(
                    "The {} stream of the export body ended before it was complete: {e}",
                    self.codec
                )))
            }
            Err(e) => Err(ExportError::DecompressionError(format!(
                "Failed to decode the {} export body: {e}",
                self.codec
            ))),
        }
    }
}

/// Decodes the body from `receiver` into `writer` and returns the number of
/// line feeds written. Each block is written before the next one is decoded.
async fn write_decoded<W: AsyncWrite + Unpin>(
    receiver: &mut DataPipeReceiver,
    compression: Compression,
    writer: &mut W,
) -> Result<u64, ExportError> {
    let mut decoder = PieceDecoder::new(compression);
    let mut line_feeds = 0;

    while let Some(piece) = receiver.recv().await {
        decoder.push(piece);
        line_feeds += write_blocks(&mut decoder, writer).await?;
    }
    decoder.end();
    line_feeds += write_blocks(&mut decoder, writer).await?;
    writer.flush().await?;

    Ok(line_feeds)
}

async fn write_blocks<W: AsyncWrite + Unpin>(
    decoder: &mut PieceDecoder,
    writer: &mut W,
) -> Result<u64, ExportError> {
    let mut line_feeds = 0;
    while let Some(block) = decoder.next_block()? {
        writer.write_all(&block).await?;
        line_feeds += count_line_feeds(&block);
    }
    Ok(line_feeds)
}

fn parse_csv(
    data: &str,
    separator: char,
    delimiter: char,
) -> Result<Vec<Vec<String>>, ExportError> {
    csv_rows(data, separator, delimiter).collect()
}

/// Reads CSV text one parsed row at a time.
///
/// A row ends at a line feed, a carriage return and line feed pair, or a lone carriage return
/// outside quotes. Line breaks inside quotes stay in the field. A doubled delimiter inside
/// quotes is one literal delimiter. The iterator yields an error when the text ends inside a
/// quote, naming the 0-based index of the row that was open, and ends after that error.
pub(crate) fn csv_rows(data: &str, separator: char, delimiter: char) -> CsvRows<'_> {
    CsvRows {
        chars: data.chars().peekable(),
        separator,
        delimiter,
        rows_yielded: 0,
    }
}

/// Row iterator returned by [`csv_rows`].
pub(crate) struct CsvRows<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    separator: char,
    delimiter: char,
    rows_yielded: usize,
}

impl CsvRows<'_> {
    fn complete_row(&mut self, mut row: Vec<String>, last_field: String) -> Vec<String> {
        row.push(last_field);
        self.rows_yielded += 1;
        row
    }
}

impl Iterator for CsvRows<'_> {
    type Item = Result<Vec<String>, ExportError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut row = Vec::new();
        let mut field = String::new();
        let mut in_quotes = false;
        let mut consumed_any = false;

        while let Some(c) = self.chars.next() {
            consumed_any = true;
            if in_quotes {
                if c != self.delimiter {
                    field.push(c);
                } else if self.chars.peek() == Some(&self.delimiter) {
                    field.push(self.delimiter);
                    self.chars.next();
                } else {
                    in_quotes = false;
                }
            } else if c == self.delimiter {
                in_quotes = true;
            } else if c == self.separator {
                row.push(std::mem::take(&mut field));
            } else if c == '\n' {
                return Some(Ok(self.complete_row(row, field)));
            } else if c == '\r' {
                // A CR before LF belongs to the CRLF pair, and the LF ends the row.
                if self.chars.peek() != Some(&'\n') {
                    return Some(Ok(self.complete_row(row, field)));
                }
            } else {
                field.push(c);
            }
        }

        if in_quotes {
            return Some(Err(ExportError::CsvParseError {
                row: self.rows_yielded,
                message: "Unclosed quote at end of data".to_string(),
            }));
        }
        if !consumed_any {
            return None;
        }
        Some(Ok(self.complete_row(row, field)))
    }
}

/// Parses a single CSV line into its fields, handling RFC-style quoting.
///
/// A field is opened/closed by `delimiter`; a doubled delimiter inside a quoted
/// field is an escaped delimiter. `separator` splits fields when not inside
/// quotes. This operates on a single already-split line and therefore does not
/// handle newlines embedded in quoted fields (callers split on newlines first).
///
/// Returns the parsed fields together with a flag indicating whether the line
/// ended while still inside a quoted field (unterminated quote). Callers decide
/// whether an unterminated quote is an error.
pub(crate) fn parse_csv_row(line: &str, separator: char, delimiter: char) -> (Vec<String>, bool) {
    let mut fields = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == delimiter {
                // Check for escaped quote (doubled delimiter)
                if chars.peek() == Some(&delimiter) {
                    current_field.push(delimiter);
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current_field.push(c);
            }
        } else if c == delimiter {
            in_quotes = true;
        } else if c == separator {
            fields.push(current_field);
            current_field = String::new();
        } else {
            current_field.push(c);
        }
    }

    // Don't forget the last field
    fields.push(current_field);

    (fields, in_quotes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_csv_rows_yields_rows_one_at_a_time_keeping_quoted_line_breaks() {
        let data = "1,\"a\nb\r c\"\n2,plain\r\n3,last";
        let mut rows = csv_rows(data, ',', '"');

        assert_eq!(rows.next().unwrap().unwrap(), vec!["1", "a\nb\r c"]);
        assert_eq!(rows.next().unwrap().unwrap(), vec!["2", "plain"]);
        assert_eq!(rows.next().unwrap().unwrap(), vec!["3", "last"]);
        assert!(rows.next().is_none());
    }

    #[test]
    fn test_csv_rows_keeps_a_final_quoted_empty_field() {
        let rows: Vec<_> = csv_rows("\"\"", ',', '"')
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![vec![String::new()]]);
    }

    #[test]
    fn test_csv_rows_ends_after_an_unclosed_quote_error() {
        let mut rows = csv_rows("1,x\n2,\"open", ',', '"');

        assert_eq!(rows.next().unwrap().unwrap(), vec!["1", "x"]);
        match rows.next().unwrap() {
            Err(ExportError::CsvParseError { row, .. }) => assert_eq!(row, 1),
            other => panic!("expected an unclosed quote error, got {other:?}"),
        }
        assert!(rows.next().is_none());
    }

    #[test]
    fn test_csv_rows_ends_a_row_at_a_lone_carriage_return_outside_quotes() {
        let mut rows = csv_rows("1,a\r2,b\r", ',', '"');

        assert_eq!(rows.next().unwrap().unwrap(), vec!["1", "a"]);
        assert_eq!(rows.next().unwrap().unwrap(), vec!["2", "b"]);
        assert!(rows.next().is_none());
    }
    use crate::error::TransportError;
    use crate::transport::protocol::QueryResult;
    use crate::transport::test_support::{
        finish_within, FakeExasolServer, GatedQueryTransport, MockTransport, DISCONNECT_BOUND,
        LOOPBACK_TEST_BOUND,
    };
    use std::io::Write;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::sync::oneshot;

    // Tests for CsvExportOptions

    #[test]
    fn test_csv_export_options_default() {
        let options = CsvExportOptions::default();

        assert_eq!(options.column_separator, ',');
        assert_eq!(options.column_delimiter, '"');
        assert_eq!(options.row_separator, RowSeparator::LF);
        assert_eq!(options.encoding, "UTF-8");
        assert!(options.null_value.is_none());
        assert_eq!(options.compression, Compression::None);
        assert!(!options.with_column_names);
        assert!(!options.use_tls);
        assert_eq!(options.timeout_ms, None);
        assert_eq!(options.host, "");
        assert_eq!(options.port, 0);
    }

    #[test]
    fn test_csv_export_options_builder() {
        let options = CsvExportOptions::new()
            .column_separator(';')
            .column_delimiter('\'')
            .row_separator(RowSeparator::CRLF)
            .encoding("ISO-8859-1")
            .null_value("NULL")
            .compression(Compression::Gzip)
            .with_column_names(true)
            .use_tls(false)
            .timeout_ms(60_000)
            .exasol_host("exasol.example.com")
            .exasol_port(8563);

        assert_eq!(options.column_separator, ';');
        assert_eq!(options.column_delimiter, '\'');
        assert_eq!(options.row_separator, RowSeparator::CRLF);
        assert_eq!(options.encoding, "ISO-8859-1");
        assert_eq!(options.null_value, Some("NULL".to_string()));
        assert_eq!(options.compression, Compression::Gzip);
        assert!(options.with_column_names);
        assert!(!options.use_tls);
        assert_eq!(options.timeout_ms, Some(60_000));
        assert_eq!(options.host, "exasol.example.com");
        assert_eq!(options.port, 8563);
    }

    #[test]
    fn test_shared_csv_options_configure_no_export_timeout() {
        let options = shared_csv_export_options(SharedCsvExportParams {
            column_separator: ';',
            column_delimiter: '\'',
            host: "exasol.example.com",
            port: 8563,
            use_tls: true,
        });

        assert_eq!(options.timeout_ms, None);
    }

    // Tests for export_to_callback

    /// Pins the `Send` guarantee of the export future, which every public
    /// `Connection::export_*` method awaits and which a caller may hand to
    /// `tokio::spawn`. The type check is the whole assertion: the future is
    /// never polled, so no transport call happens.
    #[test]
    fn test_export_to_callback_future_is_send() {
        fn assert_send<T: Send>(_: T) {}

        let mut transport = MockTransport::new();
        let export = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT 1".to_string(),
            },
            CsvExportOptions::default(),
            |_receiver: DataPipeReceiver| async { Ok::<(), ExportError>(()) },
        );

        assert_send(export);
    }

    /// Short enough to keep the deadline tests quick, yet orders of magnitude
    /// above the loopback round trip they race against, so which side wins is
    /// decided by the scenario and not by scheduling noise.
    const SHORT_DEADLINE_MS: u64 = 50;

    /// Longer than any loopback exchange these tests perform, so an export
    /// that completes normally can never trip it.
    const UNREACHABLE_DEADLINE_MS: u64 = 30_000;

    /// Far beyond the test's own lifetime: whatever awaits it never finishes.
    const NEVER: Duration = Duration::from_secs(3600);

    /// The fixed five-minute default bound this release removed (#52).
    const REMOVED_DEFAULT_BOUND_MS: u64 = 300_000;

    fn tunnel_options(server: &FakeExasolServer) -> CsvExportOptions {
        CsvExportOptions::default()
            .exasol_host(&server.host)
            .exasol_port(server.port)
            .use_tls(false)
    }

    async fn collect_body(mut receiver: DataPipeReceiver) -> Result<String, ExportError> {
        let mut collected = Vec::new();
        while let Some(chunk) = receiver.recv().await {
            collected.extend_from_slice(&chunk);
        }
        String::from_utf8(collected).map_err(|e| ExportError::CsvParseError {
            row: 0,
            message: e.to_string(),
        })
    }

    fn succeeding_transport() -> MockTransport {
        let mut transport = MockTransport::new();
        transport
            .expect_execute_query()
            .returning(|_| Ok(QueryResult::row_count(1)));
        transport
    }

    fn unwrap_timeout(error: ExportError) -> (u64, bool) {
        let ExportError::Timeout {
            timeout_ms,
            transport_terminated,
        } = error
        else {
            panic!("expected a Timeout, got {error:?}");
        };
        (timeout_ms, transport_terminated)
    }

    #[tokio::test]
    async fn test_export_to_callback_without_a_deadline_delivers_the_tunnel_body() {
        let server = FakeExasolServer::serving_csv("1,alice\n2,bob\n").await;
        let mut transport = succeeding_transport();

        let delivered = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT * FROM users".to_string(),
            },
            tunnel_options(&server),
            collect_body,
        )
        .await
        .expect("an export with no deadline must run to completion");

        assert_eq!(delivered, "1,alice\n2,bob\n");
    }

    /// The unset default arms no timer at all, not merely a longer one — the
    /// executable guard on `No client-side export timeout by default`
    /// (`specs/import-export/csv-export-timeout`), which asserting the field
    /// alone would not provide.
    ///
    /// The clock is paused only after the export has had real time to open its
    /// tunnel and enter the SQL leg: auto-advance would otherwise jump past a
    /// timer that was not armed yet. Starting late cannot turn this red, since
    /// `GatedQueryTransport::stalled` never resolves.
    ///
    /// Scenario: No client-side export timeout by default
    #[tokio::test]
    async fn test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset() {
        let server = FakeExasolServer::silent_after_handshake().await;
        let mut mock = MockTransport::new();
        // Only reached if the guard regresses; allowed so the panic below
        // reports that rather than an unexpected mock call.
        mock.expect_terminate().return_const(());
        let mut transport = GatedQueryTransport::stalled(mock);

        let options = tunnel_options(&server);
        assert_eq!(
            options.timeout_ms, None,
            "the export below only tests the unset default if the options carry it"
        );

        let export = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT * FROM users".to_string(),
            },
            options,
            collect_body,
        );
        tokio::pin!(export);

        let started = tokio::time::timeout(Duration::from_millis(500), &mut export).await;
        assert!(
            started.is_err(),
            "an EXPORT statement that never responds cannot let the export finish"
        );

        tokio::time::pause();
        tokio::time::advance(Duration::from_millis(REMOVED_DEFAULT_BOUND_MS * 2)).await;

        if let Ok(outcome) = tokio::time::timeout(Duration::from_millis(50), &mut export).await {
            panic!(
                "the export stopped waiting after {}ms, so an unset timeout_ms still armed a \
                 client-side timer: {outcome:?}",
                REMOVED_DEFAULT_BOUND_MS * 2
            );
        }
    }

    /// Starts an export against a tunnel peer that never answers the magic
    /// packet and returns its error once the paused clock reaches the setup
    /// deadline. The clock pauses only after the peer holds the magic packet,
    /// because auto-advance would otherwise fire the deadline during the connect.
    async fn export_against_a_stalled_tunnel_setup(timeout_ms: Option<u64>) -> ExportError {
        let mut server = FakeExasolServer::silent_before_handshake().await;
        // No expectations: the EXPORT statement must not be sent.
        let mut transport = MockTransport::new();
        let mut options = tunnel_options(&server);
        if let Some(timeout_ms) = timeout_ms {
            options = options.timeout_ms(timeout_ms);
        }

        let export = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT * FROM users".to_string(),
            },
            options,
            collect_body,
        );
        tokio::pin!(export);

        tokio::select! {
            outcome = &mut export => panic!("the export ended before the peer saw its handshake: {outcome:?}"),
            () = server.wait_for_magic_packet() => {}
        }
        tokio::time::pause();
        export
            .await
            .expect_err("a stalled tunnel setup must fail the export")
    }

    fn assert_tunnel_setup_timed_out_at_the_exa_handshake(error: ExportError) {
        let ExportError::HttpTransportError { message } = error else {
            panic!("expected an HttpTransportError, got {error:?}");
        };
        assert!(
            message.contains("HTTP tunnel setup timeout after 30000ms (EXA handshake)"),
            "{message}"
        );
    }

    /// Scenario: Tunnel setup is bounded by 30 seconds by default
    /// Scenario: No client-side export timeout by default
    #[tokio::test]
    async fn test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds() {
        let error = export_against_a_stalled_tunnel_setup(None).await;

        assert_tunnel_setup_timed_out_at_the_exa_handshake(error);
    }

    /// Scenario: Tunnel setup is bounded by 30 seconds by default
    #[tokio::test]
    async fn test_export_to_callback_bounds_a_stalled_tunnel_setup_even_with_an_export_timeout() {
        let error = export_against_a_stalled_tunnel_setup(Some(1_000)).await;

        assert_tunnel_setup_timed_out_at_the_exa_handshake(error);
    }

    #[tokio::test]
    async fn test_export_to_callback_within_its_deadline_delivers_the_tunnel_body() {
        let server = FakeExasolServer::serving_csv("1,alice\n").await;
        let mut transport = succeeding_transport();

        let delivered = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT * FROM users".to_string(),
            },
            tunnel_options(&server).timeout_ms(UNREACHABLE_DEADLINE_MS),
            collect_body,
        )
        .await
        .expect("an export that beats its deadline must not report a timeout");

        assert_eq!(delivered, "1,alice\n");
    }

    /// The tunnel read has no deadline of its own, so a failing EXPORT
    /// statement has to abort the tunnel task instead of being awaited
    /// alongside it. The peer here never sends the `PUT`, so anything less
    /// than a hard abort hangs.
    #[tokio::test]
    async fn test_export_to_callback_abandons_the_tunnel_when_the_export_statement_fails() {
        let server = FakeExasolServer::silent_after_handshake().await;
        let mut transport = MockTransport::new();
        transport.expect_execute_query().returning(|_| {
            Err(TransportError::ProtocolError(
                "object USERS not found".to_string(),
            ))
        });

        let error = tokio::time::timeout(
            Duration::from_secs(10),
            export_to_callback(
                &mut transport,
                ExportSource::Query {
                    sql: "SELECT * FROM users".to_string(),
                },
                tunnel_options(&server),
                collect_body,
            ),
        )
        .await
        .expect("a failed EXPORT statement must not leave the export waiting on the tunnel")
        .expect_err("a failed EXPORT statement must fail the export");

        let ExportError::SqlExecutionError { message } = error else {
            panic!("expected a SqlExecutionError, got {error:?}");
        };
        assert!(
            message.contains("object USERS not found"),
            "the statement failure must reach the caller intact: {message}"
        );
    }

    #[tokio::test]
    async fn test_export_to_callback_terminates_the_transport_when_the_deadline_precedes_the_sql_response(
    ) {
        let server = FakeExasolServer::silent_after_handshake().await;
        let mut mock = MockTransport::new();
        mock.expect_terminate().times(1).return_const(());
        let mut transport = GatedQueryTransport::stalled(mock);

        let error = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT * FROM users".to_string(),
            },
            tunnel_options(&server).timeout_ms(SHORT_DEADLINE_MS),
            collect_body,
        )
        .await
        .expect_err("an outstanding EXPORT statement must fail once the deadline elapses");

        let (timeout_ms, transport_terminated) = unwrap_timeout(error);
        assert_eq!(timeout_ms, SHORT_DEADLINE_MS);
        assert!(
            transport_terminated,
            "an unread EXPORT response can no longer be matched to a request"
        );
    }

    /// The 30-second TLS handshake bound of the tunnel, which the export does not configure.
    const TLS_HANDSHAKE_BOUND: Duration = Duration::from_secs(30);

    /// An export timeout far below the TLS handshake bound.
    const TLS_EXPORT_TIMEOUT_MS: u64 = 300;

    /// Scenario: Explicit export timeout covers a pending TLS handshake
    #[tokio::test]
    async fn test_export_to_callback_with_tls_times_out_while_the_tls_handshake_is_pending() {
        let mut server = FakeExasolServer::silent_after_handshake().await;
        let mut mock = MockTransport::new();
        mock.expect_terminate().times(1).return_const(());
        let mut transport = GatedQueryTransport::stalled(mock);

        let error = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_callback(
                &mut transport,
                ExportSource::Query {
                    sql: "SELECT * FROM users".to_string(),
                },
                tunnel_options(&server)
                    .use_tls(true)
                    .timeout_ms(TLS_EXPORT_TIMEOUT_MS),
                collect_body,
            ),
        )
        .await
        .expect_err("an export timeout shorter than the TLS bound must fail the export first");

        assert_eq!(unwrap_timeout(error), (TLS_EXPORT_TIMEOUT_MS, true));
        finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
    }

    /// The clock pauses only after real time let the tunnel task reach its TLS
    /// handshake, and resumes before the disconnect is awaited: with a paused
    /// clock, a timer bounding that wait could fire before the peer task runs.
    ///
    /// Scenario: No client-side export timeout by default
    #[tokio::test]
    async fn test_export_to_callback_with_tls_closes_the_tunnel_at_the_tls_handshake_bound_without_an_export_timer(
    ) {
        let mut server = FakeExasolServer::silent_after_handshake().await;
        let mut mock = MockTransport::new();
        mock.expect_terminate().never();
        let mut transport = GatedQueryTransport::stalled(mock);

        let export = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT * FROM users".to_string(),
            },
            tunnel_options(&server).use_tls(true),
            collect_body,
        );
        tokio::pin!(export);
        let started = tokio::time::timeout(Duration::from_millis(500), &mut export).await;
        assert!(started.is_err(), "the EXPORT statement never responds");

        assert!(
            tokio::time::timeout(Duration::from_millis(50), server.wait_for_disconnect())
                .await
                .is_err(),
            "the tunnel must stay open until the TLS handshake bound elapses"
        );

        tokio::time::pause();
        tokio::time::sleep(TLS_HANDSHAKE_BOUND).await;
        tokio::time::resume();

        finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        if let Ok(outcome) = tokio::time::timeout(Duration::from_millis(50), &mut export).await {
            panic!("the export ended without an export timeout: {outcome:?}");
        }
    }

    #[tokio::test]
    async fn test_export_to_callback_keeps_the_transport_when_the_deadline_falls_in_the_callback() {
        let server = FakeExasolServer::serving_csv("1,alice\n").await;
        let mut transport = succeeding_transport();
        transport.expect_terminate().never();

        let error = export_to_callback(
            &mut transport,
            ExportSource::Query {
                sql: "SELECT * FROM users".to_string(),
            },
            tunnel_options(&server).timeout_ms(SHORT_DEADLINE_MS),
            |_receiver: DataPipeReceiver| async {
                tokio::time::sleep(NEVER).await;
                Ok::<String, ExportError>(String::new())
            },
        )
        .await
        .expect_err("a callback that outlives the deadline must fail the export");

        let (timeout_ms, transport_terminated) = unwrap_timeout(error);
        assert_eq!(timeout_ms, SHORT_DEADLINE_MS);
        assert!(
            !transport_terminated,
            "the EXPORT response was already read, so the transport is still in sync"
        );
    }

    // Tests for streaming the export body

    fn users_query() -> ExportSource {
        ExportSource::Query {
            sql: "SELECT * FROM users".to_string(),
        }
    }

    /// A transport whose EXPORT statement answers `result` once `release`
    /// fires, and which must never be terminated.
    fn gated_transport(
        release: oneshot::Receiver<()>,
        result: Result<QueryResult, TransportError>,
    ) -> GatedQueryTransport {
        let mut mock = MockTransport::new();
        mock.expect_terminate().never();
        GatedQueryTransport::new(mock, release, result)
    }

    fn statement_failure() -> TransportError {
        TransportError::ProtocolError("the EXPORT tunnel was closed".to_string())
    }

    fn already_open_gate() -> oneshot::Receiver<()> {
        let (open, gate) = oneshot::channel();
        open.send(()).unwrap();
        gate
    }

    async fn collect_pieces(mut receiver: DataPipeReceiver) -> Result<Vec<Vec<u8>>, ExportError> {
        let mut pieces = Vec::new();
        while let Some(piece) = receiver.recv().await {
            pieces.push(piece);
        }
        Ok(pieces)
    }

    fn assert_statement_failure(outcome: Result<impl std::fmt::Debug, ExportError>) {
        match outcome {
            Err(ExportError::SqlExecutionError { message }) => {
                assert!(
                    message.contains("the EXPORT tunnel was closed"),
                    "{message}"
                );
            }
            other => panic!("expected the statement's SqlExecutionError, got {other:?}"),
        }
    }

    fn assert_200_ok(after_last_chunk: &[u8]) {
        assert!(
            after_last_chunk.starts_with(b"HTTP/1.1 200 OK\r\n"),
            "the peer must read 200 OK after its last chunk, read {:?}",
            String::from_utf8_lossy(after_last_chunk)
        );
    }

    /// The server sends its second and third chunk only after the callback
    /// holds the first piece, so a driver that collects the body first hangs.
    ///
    /// Scenario: Export data flow (Exasol sends data to client)
    /// Scenario: Export to callback
    #[tokio::test]
    async fn test_export_to_callback_receives_the_first_piece_before_the_server_sends_the_last() {
        let chunks = vec![
            b"1,alice\n".to_vec(),
            vec![b'x'; 65_536 + 1_000],
            b"3,carol\n".to_vec(),
        ];
        let (open_gate, gate) = oneshot::channel();
        let (mut server, response_received) =
            FakeExasolServer::serving_chunked_put(chunks.clone(), gate).await;
        let mut transport = gated_transport(response_received, Ok(QueryResult::row_count(3)));

        let pieces = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_callback(
                &mut transport,
                users_query(),
                tunnel_options(&server),
                |mut receiver| async move {
                    let first = receiver.recv().await.expect("the first piece");
                    let _ = open_gate.send(());
                    let mut pieces = vec![first];
                    pieces.extend(collect_pieces(receiver).await?);
                    Ok(pieces)
                },
            ),
        )
        .await
        .expect("the export must complete once the callback drains the body");

        let sizes: Vec<usize> = pieces.iter().map(Vec::len).collect();
        assert_eq!(sizes, [8, 65_536, 1_000, 8]);
        assert_eq!(pieces.concat(), chunks.concat());
        assert_200_ok(&finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await);
    }

    /// Scenario: Export to callback
    #[tokio::test]
    async fn test_export_to_callback_delivers_compressed_pieces_as_sent() {
        let body = gzip(&csv_text(1_000));
        let chunks: Vec<Vec<u8>> = body.chunks(100).map(<[u8]>::to_vec).collect();
        let (server, response_received) =
            FakeExasolServer::serving_chunked_put(chunks.clone(), already_open_gate()).await;
        let mut transport = gated_transport(response_received, Ok(QueryResult::row_count(1_000)));

        let pieces = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_callback(
                &mut transport,
                users_query(),
                tunnel_options(&server).compression(Compression::Gzip),
                collect_pieces,
            ),
        )
        .await
        .expect("a compressed export must complete");

        assert_eq!(pieces, chunks);
    }

    /// Scenario: Export to async stream
    #[tokio::test]
    async fn test_export_to_stream_writes_the_first_piece_before_the_server_sends_the_last() {
        let first_rows = b"1,alice\n";
        let last_rows = b"2,bob\n3,carol\n";
        let (open_gate, gate) = oneshot::channel();
        let (server, response_received) = FakeExasolServer::serving_chunked_put(
            vec![first_rows.to_vec(), last_rows.to_vec()],
            gate,
        )
        .await;
        let mut transport = gated_transport(response_received, Ok(QueryResult::row_count(3)));
        let (writer, mut written) = tokio::io::duplex(1024);

        let (rows, output) = finish_within(LOOPBACK_TEST_BOUND, async {
            tokio::join!(
                export_to_stream(
                    &mut transport,
                    users_query(),
                    writer,
                    tunnel_options(&server)
                ),
                async {
                    let mut output = vec![0u8; first_rows.len()];
                    written.read_exact(&mut output).await.unwrap();
                    let _ = open_gate.send(());
                    written.read_to_end(&mut output).await.unwrap();
                    output
                }
            )
        })
        .await;

        assert_eq!(rows.expect("the export must complete"), 3);
        assert_eq!(output, [&first_rows[..], last_rows].concat());
    }

    /// Scenario: Export with compression
    /// Scenario: Export to async stream
    #[tokio::test]
    async fn test_export_to_stream_decompresses_gzip_pieces_before_the_server_sends_the_last() {
        let first_rows = b"1,alice\n2,bob\n";
        let last_rows = b"3,carol\n";
        let (first_piece, last_piece) = gzip_flushed_after(first_rows, last_rows);
        let (open_gate, gate) = oneshot::channel();
        let (server, response_received) =
            FakeExasolServer::serving_chunked_put(vec![first_piece, last_piece], gate).await;
        let mut transport = gated_transport(response_received, Ok(QueryResult::row_count(3)));
        let (writer, mut written) = tokio::io::duplex(1024);

        let (rows, output) = finish_within(LOOPBACK_TEST_BOUND, async {
            tokio::join!(
                export_to_stream(
                    &mut transport,
                    users_query(),
                    writer,
                    tunnel_options(&server).compression(Compression::Gzip),
                ),
                async {
                    let mut output = vec![0u8; first_rows.len()];
                    written.read_exact(&mut output).await.unwrap();
                    let _ = open_gate.send(());
                    written.read_to_end(&mut output).await.unwrap();
                    output
                }
            )
        })
        .await;

        assert_eq!(rows.expect("the export must complete"), 3);
        assert_eq!(output, [&first_rows[..], last_rows].concat());
    }

    /// Scenario: Export with compression
    /// Scenario: Export to in-memory list
    #[tokio::test]
    async fn test_export_to_list_parses_rows_from_compressed_pieces() {
        let body = bzip2(&csv_text(1_000));
        let chunks: Vec<Vec<u8>> = body.chunks(100).map(<[u8]>::to_vec).collect();
        let (server, response_received) =
            FakeExasolServer::serving_chunked_put(chunks, already_open_gate()).await;
        let mut transport = gated_transport(response_received, Ok(QueryResult::row_count(1_000)));

        let rows = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_list(
                &mut transport,
                users_query(),
                tunnel_options(&server).compression(Compression::Bzip2),
            ),
        )
        .await
        .expect("a compressed export to a list must complete");

        assert_eq!(rows.len(), 1_000);
        assert_eq!(rows[0], ["0", "name_0"]);
        assert_eq!(rows[999], ["999", "name_999"]);
    }

    /// The statement answers only after the callback saw its data end and
    /// returned success.
    ///
    /// Scenario: Export statement fails while data streams
    #[tokio::test]
    async fn test_export_to_callback_returns_the_sql_error_after_the_callback_received_pieces() {
        let (server, _closed) =
            FakeExasolServer::closing_after_first_chunk(b"1,alice\n".to_vec()).await;
        let (release, released) = oneshot::channel();
        let mut transport = gated_transport(released, Err(statement_failure()));
        let (report_data, data_ended) = oneshot::channel();

        let (outcome, received) = finish_within(LOOPBACK_TEST_BOUND, async {
            tokio::join!(
                export_to_callback(
                    &mut transport,
                    users_query(),
                    tunnel_options(&server),
                    |receiver| async move {
                        let pieces = collect_pieces(receiver).await?;
                        let _ = report_data.send(pieces.concat());
                        Ok(pieces)
                    },
                ),
                async {
                    let received = data_ended.await.expect("the callback sees its data end");
                    let _ = release.send(());
                    received
                }
            )
        })
        .await;

        assert_eq!(received, b"1,alice\n");
        assert_statement_failure(outcome);
    }

    /// Scenario: Export statement fails while data streams
    /// Scenario: Export with compression
    #[tokio::test]
    async fn test_export_to_stream_returns_the_sql_error_when_a_gzip_tunnel_closes_mid_body() {
        let (first_piece, _never_sent) = gzip_flushed_after(b"1,alice\n", b"2,bob\n");
        let (server, closed) = FakeExasolServer::closing_after_first_chunk(first_piece).await;
        let mut transport = gated_transport(closed, Err(statement_failure()));
        let mut written = Vec::new();

        let outcome = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_stream(
                &mut transport,
                users_query(),
                &mut written,
                tunnel_options(&server).compression(Compression::Gzip),
            ),
        )
        .await;

        assert_statement_failure(outcome);
    }

    /// The callback fails on its cut-short data before the statement answers.
    ///
    /// Scenario: Export statement fails while data streams
    #[tokio::test]
    async fn test_export_to_callback_returns_the_sql_error_when_the_callback_fails_on_a_cut_short_body(
    ) {
        let (server, _closed) =
            FakeExasolServer::closing_after_first_chunk(b"1,alice\n".to_vec()).await;
        let (release, released) = oneshot::channel();
        let mut transport = gated_transport(released, Err(statement_failure()));

        let outcome = export_failing_on_a_cut_short_body(&mut transport, &server, release).await;

        assert_statement_failure(outcome);
    }

    #[tokio::test]
    async fn test_export_to_callback_returns_the_tunnel_error_when_the_statement_succeeds_after_the_tunnel_broke(
    ) {
        let (server, _closed) =
            FakeExasolServer::closing_after_first_chunk(b"1,alice\n".to_vec()).await;
        let (release, released) = oneshot::channel();
        let mut transport = gated_transport(released, Ok(QueryResult::row_count(1)));

        let outcome = export_failing_on_a_cut_short_body(&mut transport, &server, release).await;

        match outcome {
            Err(ExportError::HttpTransportError { message }) => {
                assert!(
                    message.contains("Failed to handle export request"),
                    "{message}"
                );
            }
            other => panic!("expected the tunnel's HttpTransportError, got {other:?}"),
        }
    }

    /// Runs an export whose callback fails with a `DecompressionError` once its
    /// data ends, and fires `release` only after that failure.
    async fn export_failing_on_a_cut_short_body(
        transport: &mut GatedQueryTransport,
        server: &FakeExasolServer,
        release: oneshot::Sender<()>,
    ) -> Result<(), ExportError> {
        let (report_failure, callback_failed) = oneshot::channel();
        let (outcome, ()) = finish_within(LOOPBACK_TEST_BOUND, async {
            tokio::join!(
                export_to_callback(
                    transport,
                    users_query(),
                    tunnel_options(server),
                    |receiver| async move {
                        collect_pieces(receiver).await?;
                        let _ = report_failure.send(());
                        Err(ExportError::DecompressionError("cut short".to_string()))
                    },
                ),
                async {
                    callback_failed.await.expect("the callback fails");
                    let _ = release.send(());
                }
            )
        })
        .await;
        outcome
    }

    /// The callback drops its receiver after the first of 40 pieces, before
    /// the server sends the other 39.
    ///
    /// Scenario: Export callback returns before the transfer completes
    #[tokio::test]
    async fn test_export_to_callback_returns_the_callback_value_when_it_stops_before_the_last_piece(
    ) {
        let chunks: Vec<Vec<u8>> = (0..40).map(|i| format!("{i},row\n").into_bytes()).collect();
        let (open_gate, gate) = oneshot::channel();
        let (mut server, response_received) =
            FakeExasolServer::serving_chunked_put(chunks.clone(), gate).await;
        let mut transport = gated_transport(response_received, Ok(QueryResult::row_count(40)));

        let first = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_callback(
                &mut transport,
                users_query(),
                tunnel_options(&server),
                |mut receiver| async move {
                    let first = receiver.recv().await;
                    drop(receiver);
                    let _ = open_gate.send(());
                    Ok(first)
                },
            ),
        )
        .await
        .expect("the statement finishes after the driver discarded the rest");

        assert_eq!(first, Some(chunks[0].clone()));
        assert_200_ok(&finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await);
    }

    /// The statement answers only after the server saw the driver close the
    /// tunnel, and the export must wait for that answer.
    ///
    /// Scenario: Export consumer fails while data streams
    #[tokio::test]
    async fn test_export_to_callback_returns_the_callback_error_after_the_sql_response() {
        let (_gate_never_opens, gate) = oneshot::channel();
        let chunks = vec![b"1,alice\n".to_vec(), b"2,bob\n".to_vec()];
        let (mut server, _response_received) =
            FakeExasolServer::serving_chunked_put(chunks, gate).await;
        let (release, released) = oneshot::channel();
        let mut transport = gated_transport(released, Err(statement_failure()));
        let statement_answered = AtomicBool::new(false);
        let options = tunnel_options(&server);

        let ((outcome, answered_before_return), ()) = finish_within(LOOPBACK_TEST_BOUND, async {
            tokio::join!(
                async {
                    let outcome = export_to_callback(
                        &mut transport,
                        users_query(),
                        options,
                        |mut receiver| async move {
                            receiver.recv().await;
                            Err::<(), _>(ExportError::IoError(io::Error::other("disk full")))
                        },
                    )
                    .await;
                    (outcome, statement_answered.load(Ordering::SeqCst))
                },
                async {
                    server.wait_for_disconnect().await;
                    statement_answered.store(true, Ordering::SeqCst);
                    let _ = release.send(());
                }
            )
        })
        .await;

        match outcome {
            Err(ExportError::IoError(error)) => assert_eq!(error.to_string(), "disk full"),
            other => panic!("expected the callback's IoError, got {other:?}"),
        }
        assert!(
            answered_before_return,
            "the export must wait for the EXPORT response before it returns"
        );
    }

    // Tests for CSV parsing

    #[test]
    fn test_parse_csv_simple() {
        let data = "a,b,c\n1,2,3\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec!["a", "b", "c"]);
        assert_eq!(result[1], vec!["1", "2", "3"]);
    }

    #[test]
    fn test_parse_csv_with_quotes() {
        let data = "\"hello\",\"world\"\n\"foo\",\"bar\"\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec!["hello", "world"]);
        assert_eq!(result[1], vec!["foo", "bar"]);
    }

    #[test]
    fn test_parse_csv_with_escaped_quotes() {
        let data = "\"hello \"\"world\"\"\",normal\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec!["hello \"world\"", "normal"]);
    }

    #[test]
    fn test_parse_csv_with_separator_in_quotes() {
        let data = "\"a,b,c\",\"d,e\"\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec!["a,b,c", "d,e"]);
    }

    #[test]
    fn test_parse_csv_with_newline_in_quotes() {
        let data = "\"line1\nline2\",normal\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec!["line1\nline2", "normal"]);
    }

    #[test]
    fn test_parse_csv_with_crlf() {
        let data = "a,b\r\nc,d\r\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec!["a", "b"]);
        assert_eq!(result[1], vec!["c", "d"]);
    }

    #[test]
    fn test_parse_csv_with_semicolon_separator() {
        let data = "a;b;c\n1;2;3\n";
        let result = parse_csv(data, ';', '"').unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec!["a", "b", "c"]);
        assert_eq!(result[1], vec!["1", "2", "3"]);
    }

    #[test]
    fn test_parse_csv_empty_fields() {
        let data = "a,,c\n,b,\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec!["a", "", "c"]);
        assert_eq!(result[1], vec!["", "b", ""]);
    }

    #[test]
    fn test_parse_csv_unclosed_quote_error() {
        let data = "\"unclosed,quote\n";
        let result = parse_csv(data, ',', '"');

        assert!(result.is_err());
        if let Err(ExportError::CsvParseError { row, message }) = result {
            assert_eq!(row, 0);
            assert!(message.contains("Unclosed quote"));
        } else {
            panic!("Expected CsvParseError");
        }
    }

    #[test]
    fn test_parse_csv_empty_data() {
        let data = "";
        let result = parse_csv(data, ',', '"').unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_csv_single_field() {
        let data = "single\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec!["single"]);
    }

    #[test]
    fn test_parse_csv_no_trailing_newline() {
        let data = "a,b,c";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec!["a", "b", "c"]);
    }

    // Tests for ExportError

    #[test]
    fn test_export_error_display() {
        let err = ExportError::IoError(io::Error::new(io::ErrorKind::NotFound, "file not found"));
        assert!(err.to_string().contains("I/O error"));

        let err = ExportError::HttpTransportError {
            message: "connection refused".to_string(),
        };
        assert!(err.to_string().contains("HTTP transport error"));
        assert!(err.to_string().contains("connection refused"));

        let err = ExportError::CsvParseError {
            row: 5,
            message: "invalid data".to_string(),
        };
        assert!(err.to_string().contains("row 5"));
        assert!(err.to_string().contains("invalid data"));

        let err = ExportError::Timeout {
            timeout_ms: 5000,
            transport_terminated: true,
        };
        assert_eq!(
            err.to_string(),
            "Export timed out after 5000ms; the transport was terminated, \
             reconnect before the next operation"
        );

        let err = ExportError::Timeout {
            timeout_ms: 5000,
            transport_terminated: false,
        };
        assert_eq!(
            err.to_string(),
            "Export timed out after 5000ms; the transport is still usable"
        );
    }

    #[test]
    fn test_export_error_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let export_err: ExportError = io_err.into();

        assert!(matches!(export_err, ExportError::IoError(_)));
    }

    // Tests for DataPipeReceiver

    #[tokio::test]
    async fn test_data_pipe_receiver_recv() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(16);
        let mut receiver = DataPipeReceiver { rx };

        tx.send(vec![1, 2, 3]).await.unwrap();
        tx.send(vec![4, 5, 6]).await.unwrap();
        drop(tx);

        let chunk1 = receiver.recv().await;
        assert_eq!(chunk1, Some(vec![1, 2, 3]));

        let chunk2 = receiver.recv().await;
        assert_eq!(chunk2, Some(vec![4, 5, 6]));

        let chunk3 = receiver.recv().await;
        assert!(chunk3.is_none());
    }

    /// Scenario: Export applies backpressure to a slow consumer
    #[tokio::test]
    async fn test_export_pipe_holds_at_most_16_pieces() {
        let (sink, mut receiver) = data_pipe();
        for piece in 0..16u8 {
            sink.try_send(vec![piece])
                .expect("the pipe holds 16 waiting pieces");
        }

        assert!(matches!(
            sink.try_send(vec![16]),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        assert_eq!(receiver.recv().await, Some(vec![0]));
        sink.try_send(vec![16])
            .expect("a taken piece frees one slot");
    }

    #[tokio::test]
    async fn test_data_pipe_receiver_empty() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(16);
        let mut receiver = DataPipeReceiver { rx };

        drop(tx);

        let chunk = receiver.recv().await;
        assert!(chunk.is_none());
    }

    // Tests for CSV parsing with column headers

    #[test]
    fn test_parse_csv_with_column_headers() {
        let data = "id,name,email\n1,Alice,alice@example.com\n2,Bob,bob@example.com\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 3);
        // First row is the header
        assert_eq!(result[0], vec!["id", "name", "email"]);
        // Data rows
        assert_eq!(result[1], vec!["1", "Alice", "alice@example.com"]);
        assert_eq!(result[2], vec!["2", "Bob", "bob@example.com"]);
    }

    #[test]
    fn test_parse_csv_with_quoted_headers() {
        let data = "\"Column A\",\"Column B\",\"Column C\"\n1,2,3\n";
        let result = parse_csv(data, ',', '"').unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec!["Column A", "Column B", "Column C"]);
        assert_eq!(result[1], vec!["1", "2", "3"]);
    }

    // Tests for PieceDecoder

    fn csv_text(rows: usize) -> Vec<u8> {
        (0..rows)
            .map(|i| format!("{i},name_{i}\n"))
            .collect::<String>()
            .into_bytes()
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn bzip2(data: &[u8]) -> Vec<u8> {
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn decode_in_pieces(
        compression: Compression,
        body: &[u8],
        piece_len: usize,
    ) -> Result<Vec<u8>, ExportError> {
        let mut decoder = PieceDecoder::new(compression);
        let mut decoded = Vec::new();
        for piece in body.chunks(piece_len) {
            decoder.push(piece.to_vec());
            while let Some(block) = decoder.next_block()? {
                decoded.extend(block);
            }
        }
        decoder.end();
        while let Some(block) = decoder.next_block()? {
            decoded.extend(block);
        }
        Ok(decoded)
    }

    /// Scenario: Export with compression
    #[test]
    fn test_piece_decoder_decodes_gzip_split_into_pieces() {
        let csv = csv_text(5_000);

        let decoded = decode_in_pieces(Compression::Gzip, &gzip(&csv), 7).unwrap();

        assert_eq!(decoded, csv);
    }

    /// Scenario: Export with compression
    #[test]
    fn test_piece_decoder_decodes_bzip2_split_into_pieces() {
        let csv = csv_text(5_000);

        let decoded = decode_in_pieces(Compression::Bzip2, &bzip2(&csv), 7).unwrap();

        assert_eq!(decoded, csv);
    }

    /// Compresses `first_rows` and `last_rows` into one gzip stream with a sync
    /// flush between them, as a streaming compressor emits it, and returns the
    /// two halves.
    fn gzip_flushed_after(first_rows: &[u8], last_rows: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(first_rows).unwrap();
        encoder.flush().unwrap();
        let first_half = std::mem::take(encoder.get_mut());
        encoder.write_all(last_rows).unwrap();
        (first_half, encoder.finish().unwrap())
    }

    /// Scenario: Export with compression
    #[test]
    fn test_piece_decoder_yields_output_before_the_last_piece() {
        let first_rows = b"1,alice\n2,bob\n";
        let (first_piece, last_piece) = gzip_flushed_after(first_rows, b"3,carol\n");
        let mut decoder = PieceDecoder::new(Compression::Gzip);

        decoder.push(first_piece);
        let decoded_first = decoder.next_block().unwrap().unwrap();
        assert!(decoder.next_block().unwrap().is_none());
        decoder.push(last_piece);
        let mut decoded_rest = Vec::new();
        while let Some(block) = decoder.next_block().unwrap() {
            decoded_rest.extend(block);
        }
        decoder.end();
        assert!(decoder.next_block().unwrap().is_none());

        assert_eq!(decoded_first, first_rows);
        assert_eq!(decoded_rest, b"3,carol\n");
    }

    /// Scenario: Export with compression
    #[test]
    fn test_piece_decoder_rejects_a_truncated_gzip_stream() {
        let body = gzip(&csv_text(100));

        let error = decode_in_pieces(Compression::Gzip, &body[..body.len() - 4], 7)
            .expect_err("a gzip stream without its trailer is incomplete");

        assert!(
            matches!(error, ExportError::DecompressionError(_)),
            "{error:?}"
        );
    }

    /// Scenario: Export with compression
    #[test]
    fn test_piece_decoder_rejects_a_truncated_bzip2_stream() {
        let body = bzip2(&csv_text(100));

        let error = decode_in_pieces(Compression::Bzip2, &body[..body.len() - 4], 7)
            .expect_err("a bzip2 stream without its end marker is incomplete");

        assert!(
            matches!(error, ExportError::DecompressionError(_)),
            "{error:?}"
        );
    }

    #[test]
    fn test_piece_decoder_passes_an_uncompressed_piece_through() {
        let mut decoder = PieceDecoder::new(Compression::None);

        decoder.push(b"1,alice\n".to_vec());

        assert_eq!(decoder.next_block().unwrap().unwrap(), b"1,alice\n");
        assert!(decoder.next_block().unwrap().is_none());
    }

    #[test]
    fn test_piece_decoder_rejects_a_corrupt_gzip_piece() {
        let mut decoder = PieceDecoder::new(Compression::Gzip);

        decoder.push(b"not a gzip stream".to_vec());
        let error = decoder
            .next_block()
            .expect_err("bytes without a gzip header are corrupt");

        assert!(
            matches!(error, ExportError::DecompressionError(_)),
            "{error:?}"
        );
    }

    /// Records the largest single write, which is what the decoder bounds.
    #[derive(Default)]
    struct WriteSizeRecorder {
        largest_write: usize,
    }

    impl AsyncWrite for WriteSizeRecorder {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            self.largest_write = self.largest_write.max(buf.len());
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// One piece of 64 KiB at most that decodes to 4 MiB reaches the writer
    /// in bounded blocks.
    ///
    /// Scenario: Export with compression
    async fn assert_compressible_body_reaches_the_writer_in_bounded_blocks(
        compression: Compression,
        compressed: Vec<u8>,
    ) {
        const ROWS: u64 = 1024 * 1024;
        assert!(compressed.len() <= 64 * 1024, "{} bytes", compressed.len());
        let (open_gate, gate) = oneshot::channel();
        open_gate.send(()).unwrap();
        let (server, response_received) =
            FakeExasolServer::serving_chunked_put(vec![compressed], gate).await;
        let mut transport =
            gated_transport(response_received, Ok(QueryResult::row_count(ROWS as i64)));
        let mut recorder = WriteSizeRecorder::default();

        let rows = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_stream(
                &mut transport,
                users_query(),
                &mut recorder,
                tunnel_options(&server).compression(compression),
            ),
        )
        .await
        .expect("the export decodes the body");

        assert_eq!(rows, ROWS);
        assert!(
            recorder.largest_write <= DECODED_BLOCK_MAX_BYTES,
            "one write held {} bytes",
            recorder.largest_write
        );
    }

    #[tokio::test]
    async fn test_export_to_stream_writes_a_highly_compressible_gzip_body_in_bounded_blocks() {
        let csv = "1,x\n".repeat(1024 * 1024);

        assert_compressible_body_reaches_the_writer_in_bounded_blocks(
            Compression::Gzip,
            gzip(csv.as_bytes()),
        )
        .await;
    }

    #[tokio::test]
    async fn test_export_to_stream_writes_a_highly_compressible_bzip2_body_in_bounded_blocks() {
        let csv = "1,x\n".repeat(1024 * 1024);

        assert_compressible_body_reaches_the_writer_in_bounded_blocks(
            Compression::Bzip2,
            bzip2(csv.as_bytes()),
        )
        .await;
    }

    /// The valid callback `|receiver| async move { Ok(receiver) }` keeps its
    /// receiver alive, so the tunnel must discard the rest of the body.
    ///
    /// Scenario: Export callback returns before the transfer completes
    #[tokio::test]
    async fn test_export_to_callback_finishes_when_the_callback_returns_with_its_receiver_alive() {
        let chunks: Vec<Vec<u8>> = (0..40).map(|i| format!("{i},row\n").into_bytes()).collect();
        let (open_gate, gate) = oneshot::channel();
        open_gate.send(()).unwrap();
        let (mut server, response_received) =
            FakeExasolServer::serving_chunked_put(chunks, gate).await;
        let mut transport = gated_transport(response_received, Ok(QueryResult::row_count(40)));

        let receiver = finish_within(
            LOOPBACK_TEST_BOUND,
            export_to_callback(
                &mut transport,
                users_query(),
                tunnel_options(&server),
                |receiver| async move { Ok(receiver) },
            ),
        )
        .await
        .expect("the statement finishes after the driver discarded the body");

        assert_200_ok(&finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await);
        drop(receiver);
    }

    // Tests for export options with column names

    #[test]
    fn test_csv_export_options_with_column_names() {
        let options = CsvExportOptions::new().with_column_names(true);
        assert!(options.with_column_names);

        let options = CsvExportOptions::new().with_column_names(false);
        assert!(!options.with_column_names);
    }

    // Tests for export options with compression

    #[test]
    fn test_csv_export_options_compression_gzip() {
        let options = CsvExportOptions::new().compression(Compression::Gzip);
        assert_eq!(options.compression, Compression::Gzip);
    }

    #[test]
    fn test_csv_export_options_compression_bzip2() {
        let options = CsvExportOptions::new().compression(Compression::Bzip2);
        assert_eq!(options.compression, Compression::Bzip2);
    }

    #[test]
    fn test_csv_export_options_compression_none() {
        let options = CsvExportOptions::new().compression(Compression::None);
        assert_eq!(options.compression, Compression::None);
    }

    // Tests for error types

    #[test]
    fn test_export_error_decompression() {
        let err = ExportError::DecompressionError("invalid gzip header".to_string());
        assert!(err.to_string().contains("Decompression error"));
        assert!(err.to_string().contains("invalid gzip header"));
    }

    #[test]
    fn test_export_error_channel() {
        let err = ExportError::ChannelError("receiver dropped".to_string());
        assert!(err.to_string().contains("Channel error"));
        assert!(err.to_string().contains("receiver dropped"));
    }

    #[test]
    fn test_export_error_cancelled() {
        let err = ExportError::Cancelled;
        assert!(err.to_string().contains("cancelled"));
    }

    // Test DataPipeReceiver with multiple chunks

    #[tokio::test]
    async fn test_data_pipe_receiver_multiple_chunks() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(DEFAULT_PIPE_BUFFER_SIZE);
        let mut receiver = DataPipeReceiver { rx };

        // Send CSV data in chunks
        let chunk1 = b"id,name\n".to_vec();
        let chunk2 = b"1,Alice\n".to_vec();
        let chunk3 = b"2,Bob\n".to_vec();

        tx.send(chunk1.clone()).await.unwrap();
        tx.send(chunk2.clone()).await.unwrap();
        tx.send(chunk3.clone()).await.unwrap();
        drop(tx);

        // Receive and concatenate
        let mut buffer = Vec::new();
        while let Some(chunk) = receiver.recv().await {
            buffer.extend_from_slice(&chunk);
        }

        let csv_string = String::from_utf8(buffer).unwrap();
        let rows = parse_csv(&csv_string, ',', '"').unwrap();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["id", "name"]);
        assert_eq!(rows[1], vec!["1", "Alice"]);
        assert_eq!(rows[2], vec!["2", "Bob"]);
    }
}
