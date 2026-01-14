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

use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;

use crate::query::export::{Compression, ExportQuery, ExportSource, RowSeparator};
use crate::transport::protocol::TransportProtocol;
use crate::transport::HttpTransportClient;

/// Default buffer size for data pipe channels (number of chunks).
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

    /// CSV parsing error (alternative for parquet module compatibility).
    #[error("CSV parsing error at row {row}: {message}")]
    CsvParse { row: usize, message: String },

    /// Decompression error.
    #[error("Decompression error: {0}")]
    DecompressionError(String),

    /// Channel communication error.
    #[error("Channel error: {0}")]
    ChannelError(String),

    /// Export timeout.
    #[error("Export timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// Export was cancelled.
    #[error("Export was cancelled")]
    Cancelled,

    /// Arrow error.
    #[error("Arrow error: {0}")]
    Arrow(String),

    /// Schema error.
    #[error("Schema error: {0}")]
    Schema(String),

    /// Parquet error.
    #[error("Parquet error: {0}")]
    Parquet(String),
}

impl From<parquet::errors::ParquetError> for ExportError {
    fn from(err: parquet::errors::ParquetError) -> Self {
        ExportError::Parquet(err.to_string())
    }
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

    /// Use TLS for the HTTP transport (default: true).
    pub use_tls: bool,

    /// Timeout in milliseconds for the export operation (default: 300000 = 5 minutes).
    pub timeout_ms: u64,

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
            timeout_ms: 300_000, // 5 minutes
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

    /// Sets the column separator character.
    #[must_use]
    pub fn column_separator(mut self, sep: char) -> Self {
        self.column_separator = sep;
        self
    }

    /// Sets the column delimiter (quote) character.
    #[must_use]
    pub fn column_delimiter(mut self, delim: char) -> Self {
        self.column_delimiter = delim;
        self
    }

    /// Sets the row separator.
    #[must_use]
    pub fn row_separator(mut self, sep: RowSeparator) -> Self {
        self.row_separator = sep;
        self
    }

    /// Sets the character encoding.
    #[must_use]
    pub fn encoding(mut self, enc: &str) -> Self {
        self.encoding = enc.to_string();
        self
    }

    /// Sets the NULL value representation.
    #[must_use]
    pub fn null_value(mut self, val: &str) -> Self {
        self.null_value = Some(val.to_string());
        self
    }

    /// Sets the compression type.
    #[must_use]
    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Sets whether to include column headers.
    #[must_use]
    pub fn with_column_names(mut self, include: bool) -> Self {
        self.with_column_names = include;
        self
    }

    /// Sets whether to use TLS for the HTTP transport.
    #[must_use]
    pub fn use_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    /// Sets the timeout in milliseconds.
    #[must_use]
    pub fn timeout_ms(mut self, timeout: u64) -> Self {
        self.timeout_ms = timeout;
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

/// Receiver end of the data pipe for processing exported data.
pub struct DataPipeReceiver {
    rx: mpsc::Receiver<Vec<u8>>,
}

impl DataPipeReceiver {
    /// Receives the next chunk of data.
    ///
    /// Returns `None` when all data has been received.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.rx.recv().await
    }
}

/// Exports data from an Exasol table or query to a file.
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
/// ```ignore
/// use exarrow_rs::export::{export_to_file, CsvExportOptions};
/// use exarrow_rs::query::export::ExportSource;
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let rows = export_to_file(
///     &mut ws_transport,
///     ExportSource::Table {
///         schema: None,
///         name: "users".to_string(),
///         columns: vec![],
///     },
///     Path::new("/tmp/users.csv"),
///     CsvExportOptions::default(),
/// ).await?;
///
/// println!("Exported {} rows", rows);
/// # Ok(())
/// # }
/// ```
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

    // Use the callback variant to process data
    export_to_callback(ws_transport, source, options, |mut receiver| async move {
        let mut row_count = 0u64;
        let mut buffer = Vec::new();

        // Collect all data first
        while let Some(chunk) = receiver.recv().await {
            buffer.extend_from_slice(&chunk);
        }

        // Decompress if needed
        let data = match compression {
            Compression::Gzip => {
                let decoder = GzDecoder::new(buffer.as_slice());
                let mut decompressed = Vec::new();
                std::io::Read::read_to_end(
                    &mut std::io::BufReader::new(decoder),
                    &mut decompressed,
                )
                .map_err(|e| ExportError::DecompressionError(e.to_string()))?;
                decompressed
            }
            Compression::Bzip2 => {
                let decoder = BzDecoder::new(buffer.as_slice());
                let mut decompressed = Vec::new();
                std::io::Read::read_to_end(
                    &mut std::io::BufReader::new(decoder),
                    &mut decompressed,
                )
                .map_err(|e| ExportError::DecompressionError(e.to_string()))?;
                decompressed
            }
            Compression::None => buffer,
        };

        // Write to output and count rows
        writer.write_all(&data).await?;
        writer.flush().await?;

        // Count rows by counting newlines
        for byte in &data {
            if *byte == b'\n' {
                row_count += 1;
            }
        }

        Ok(row_count)
    })
    .await
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
/// ```ignore
/// use exarrow_rs::export::{export_to_list, CsvExportOptions};
/// use exarrow_rs::query::export::ExportSource;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let rows = export_to_list(
///     &mut ws_transport,
///     ExportSource::Query { sql: "SELECT id, name FROM users".to_string() },
///     CsvExportOptions::default().with_column_names(true),
/// ).await?;
///
/// for row in rows {
///     println!("{:?}", row);
/// }
/// # Ok(())
/// # }
/// ```
pub async fn export_to_list<T: TransportProtocol + ?Sized>(
    ws_transport: &mut T,
    source: ExportSource,
    options: CsvExportOptions,
) -> Result<Vec<Vec<String>>, ExportError> {
    let separator = options.column_separator;
    let delimiter = options.column_delimiter;
    let compression = options.compression;

    export_to_callback(ws_transport, source, options, |mut receiver| async move {
        let mut buffer = Vec::new();

        // Collect all data
        while let Some(chunk) = receiver.recv().await {
            buffer.extend_from_slice(&chunk);
        }

        // Decompress if needed
        let data = match compression {
            Compression::Gzip => {
                let decoder = GzDecoder::new(buffer.as_slice());
                let mut decompressed = Vec::new();
                std::io::Read::read_to_end(
                    &mut std::io::BufReader::new(decoder),
                    &mut decompressed,
                )
                .map_err(|e| ExportError::DecompressionError(e.to_string()))?;
                decompressed
            }
            Compression::Bzip2 => {
                let decoder = BzDecoder::new(buffer.as_slice());
                let mut decompressed = Vec::new();
                std::io::Read::read_to_end(
                    &mut std::io::BufReader::new(decoder),
                    &mut decompressed,
                )
                .map_err(|e| ExportError::DecompressionError(e.to_string()))?;
                decompressed
            }
            Compression::None => buffer,
        };

        // Parse CSV data
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
/// Returns `ExportError` if the export fails.
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
    let mut client = HttpTransportClient::connect(&options.host, options.port, options.use_tls)
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

    // Create channel for data transfer
    let (tx, rx) = mpsc::channel::<Vec<u8>>(DEFAULT_PIPE_BUFFER_SIZE);
    let receiver = DataPipeReceiver { rx };

    // Spawn task to handle the export request from Exasol
    // This task:
    // 1. Waits for HTTP PUT request from Exasol
    // 2. Reads CSV data from PUT request body (chunked or content-length)
    // 3. Sends HTTP 200 OK response after receiving all data
    let http_task =
        tokio::spawn(async move {
            // Use handle_export_request() to properly handle the EXPORT protocol
            let (_request, body) = client.handle_export_request().await.map_err(|e| {
                ExportError::HttpTransportError {
                    message: format!("Failed to handle export request: {e}"),
                }
            })?;

            // Send all data to receiver
            if !body.is_empty() && tx.send(body).await.is_err() {
                return Err(ExportError::ChannelError("Receiver dropped".to_string()));
            }

            // Shutdown connection gracefully
            let _ = client.shutdown().await;

            Ok::<(), ExportError>(())
        });

    // Execute the EXPORT SQL in parallel
    // This triggers Exasol to send data through the established connection
    let sql_task = async {
        ws_transport
            .execute_query(&export_sql)
            .await
            .map_err(|e| ExportError::SqlExecutionError {
                message: e.to_string(),
            })
    };

    // Run callback with the receiver
    let callback_task = callback(receiver);

    // Use tokio::select to run all tasks concurrently
    let timeout = tokio::time::Duration::from_millis(options.timeout_ms);

    let result = tokio::time::timeout(timeout, async {
        // Execute SQL first (this triggers Exasol to send data through our connection)
        let sql_result = sql_task.await;

        // Then wait for HTTP task and callback
        let (http_result, callback_result) = tokio::join!(http_task, callback_task);

        // Check for errors
        sql_result?;
        http_result.map_err(|e| ExportError::HttpTransportError {
            message: format!("HTTP task panicked: {}", e),
        })??;

        callback_result
    })
    .await
    .map_err(|_| ExportError::Timeout {
        timeout_ms: options.timeout_ms,
    })?;

    result
}

/// Parses CSV data into a vector of rows.
fn parse_csv(
    data: &str,
    separator: char,
    delimiter: char,
) -> Result<Vec<Vec<String>>, ExportError> {
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut row_num = 0;

    let chars: Vec<char> = data.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if in_quotes {
            if c == delimiter {
                // Check for escaped delimiter (two consecutive delimiters)
                if i + 1 < chars.len() && chars[i + 1] == delimiter {
                    current_field.push(delimiter);
                    i += 2;
                    continue;
                }
                // End of quoted field
                in_quotes = false;
            } else {
                current_field.push(c);
            }
        } else if c == delimiter {
            // Start of quoted field
            in_quotes = true;
        } else if c == separator {
            // End of field
            current_row.push(current_field);
            current_field = String::new();
        } else if c == '\n' {
            // End of row
            current_row.push(current_field);
            current_field = String::new();
            rows.push(current_row);
            current_row = Vec::new();
            row_num += 1;
        } else if c == '\r' {
            // Handle CRLF - skip the CR, the LF will handle the row end
            if i + 1 < chars.len() && chars[i + 1] == '\n' {
                // CRLF, skip CR and let LF handle it
            } else {
                // Just CR (old Mac-style)
                current_row.push(current_field);
                current_field = String::new();
                rows.push(current_row);
                current_row = Vec::new();
                row_num += 1;
            }
        } else {
            current_field.push(c);
        }
        i += 1;
    }

    // Handle the last field/row if not empty
    if !current_field.is_empty() || !current_row.is_empty() {
        current_row.push(current_field);
        rows.push(current_row);
    }

    // Check for unclosed quotes
    if in_quotes {
        return Err(ExportError::CsvParseError {
            row: row_num,
            message: "Unclosed quote at end of data".to_string(),
        });
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(options.timeout_ms, 300_000);
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
        assert_eq!(options.timeout_ms, 60_000);
        assert_eq!(options.host, "exasol.example.com");
        assert_eq!(options.port, 8563);
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

        let err = ExportError::Timeout { timeout_ms: 5000 };
        assert!(err.to_string().contains("5000ms"));
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

    // Tests for decompression

    #[test]
    fn test_gzip_decompression() {
        use flate2::write::GzEncoder;
        use std::io::Write;

        // Create gzip compressed data
        let original = b"hello,world\n1,2\n";
        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        // Decompress using our logic
        let decoder = GzDecoder::new(compressed.as_slice());
        let mut decompressed = Vec::new();
        std::io::Read::read_to_end(&mut std::io::BufReader::new(decoder), &mut decompressed)
            .unwrap();

        assert_eq!(decompressed, original);

        // Parse the decompressed CSV
        let csv_string = String::from_utf8(decompressed).unwrap();
        let rows = parse_csv(&csv_string, ',', '"').unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["hello", "world"]);
        assert_eq!(rows[1], vec!["1", "2"]);
    }

    #[test]
    fn test_bzip2_decompression() {
        use bzip2::write::BzEncoder;
        use std::io::Write;

        // Create bzip2 compressed data
        let original = b"foo,bar\na,b\n";
        let mut encoder = BzEncoder::new(Vec::new(), bzip2::Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        // Decompress using our logic
        let decoder = BzDecoder::new(compressed.as_slice());
        let mut decompressed = Vec::new();
        std::io::Read::read_to_end(&mut std::io::BufReader::new(decoder), &mut decompressed)
            .unwrap();

        assert_eq!(decompressed, original);

        // Parse the decompressed CSV
        let csv_string = String::from_utf8(decompressed).unwrap();
        let rows = parse_csv(&csv_string, ',', '"').unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["foo", "bar"]);
        assert_eq!(rows[1], vec!["a", "b"]);
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
