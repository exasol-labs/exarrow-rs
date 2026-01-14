//! CSV import functionality for Exasol.
//!
//! This module provides functions for importing CSV data into Exasol tables
//! using the HTTP transport layer.

use std::future::Future;
use std::io::Write;
use std::path::Path;

use bzip2::write::BzEncoder;
use flate2::write::GzEncoder;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;

use crate::query::import::{Compression, ImportQuery, RowSeparator, TrimMode};
use crate::transport::HttpTransportClient;

use super::ImportError;

/// Default buffer size for reading data (64KB).
const DEFAULT_BUFFER_SIZE: usize = 64 * 1024;

/// Channel buffer size for data pipe.
const CHANNEL_BUFFER_SIZE: usize = 16;

/// Options for CSV import.
#[derive(Debug, Clone)]
pub struct CsvImportOptions {
    /// Character encoding (default: UTF-8).
    pub encoding: String,

    /// Column separator character (default: ',').
    pub column_separator: char,

    /// Column delimiter character for quoting (default: '"').
    pub column_delimiter: char,

    /// Row separator (default: LF).
    pub row_separator: RowSeparator,

    /// Number of header rows to skip (default: 0).
    pub skip_rows: u32,

    /// Custom NULL value representation (default: None, empty string is NULL).
    pub null_value: Option<String>,

    /// Trim mode for imported values (default: None).
    pub trim_mode: TrimMode,

    /// Compression type (default: None).
    pub compression: Compression,

    /// Maximum number of invalid rows before failure (default: None = fail on first error).
    pub reject_limit: Option<u32>,

    /// Use TLS encryption for HTTP transport (default: true).
    pub use_tls: bool,

    /// Target schema (optional).
    pub schema: Option<String>,

    /// Target columns (optional, imports all if not specified).
    pub columns: Option<Vec<String>>,

    /// Exasol host for HTTP transport connection.
    /// This is typically the same host as the WebSocket connection.
    pub host: String,

    /// Exasol port for HTTP transport connection.
    /// This is typically the same port as the WebSocket connection.
    pub port: u16,
}

impl Default for CsvImportOptions {
    fn default() -> Self {
        Self {
            encoding: "UTF-8".to_string(),
            column_separator: ',',
            column_delimiter: '"',
            row_separator: RowSeparator::LF,
            skip_rows: 0,
            null_value: None,
            trim_mode: TrimMode::None,
            compression: Compression::None,
            reject_limit: None,
            use_tls: false,
            schema: None,
            columns: None,
            host: String::new(),
            port: 0,
        }
    }
}

impl CsvImportOptions {
    /// Create new import options with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the character encoding.
    #[must_use]
    pub fn encoding(mut self, encoding: &str) -> Self {
        self.encoding = encoding.to_string();
        self
    }

    /// Set the column separator character.
    #[must_use]
    pub fn column_separator(mut self, sep: char) -> Self {
        self.column_separator = sep;
        self
    }

    /// Set the column delimiter character.
    #[must_use]
    pub fn column_delimiter(mut self, delim: char) -> Self {
        self.column_delimiter = delim;
        self
    }

    /// Set the row separator.
    #[must_use]
    pub fn row_separator(mut self, sep: RowSeparator) -> Self {
        self.row_separator = sep;
        self
    }

    /// Set the number of header rows to skip.
    #[must_use]
    pub fn skip_rows(mut self, rows: u32) -> Self {
        self.skip_rows = rows;
        self
    }

    /// Set a custom NULL value representation.
    #[must_use]
    pub fn null_value(mut self, val: &str) -> Self {
        self.null_value = Some(val.to_string());
        self
    }

    /// Set the trim mode.
    #[must_use]
    pub fn trim_mode(mut self, mode: TrimMode) -> Self {
        self.trim_mode = mode;
        self
    }

    /// Set the compression type.
    #[must_use]
    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Set the reject limit.
    #[must_use]
    pub fn reject_limit(mut self, limit: u32) -> Self {
        self.reject_limit = Some(limit);
        self
    }

    /// Enable or disable TLS encryption.
    #[must_use]
    pub fn use_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    /// Set the target schema.
    #[must_use]
    pub fn schema(mut self, schema: &str) -> Self {
        self.schema = Some(schema.to_string());
        self
    }

    /// Set the target columns.
    #[must_use]
    pub fn columns(mut self, columns: Vec<String>) -> Self {
        self.columns = Some(columns);
        self
    }

    /// Set the Exasol host for HTTP transport connection.
    ///
    /// This should be the same host as used for the WebSocket connection.
    ///
    /// # Example
    ///
    /// ```
    /// use exarrow_rs::import::CsvImportOptions;
    ///
    /// let options = CsvImportOptions::default()
    ///     .exasol_host("exasol.example.com")
    ///     .exasol_port(8563);
    /// ```
    #[must_use]
    pub fn exasol_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Set the Exasol port for HTTP transport connection.
    ///
    /// This should be the same port as used for the WebSocket connection.
    #[must_use]
    pub fn exasol_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Build an ImportQuery from these options.
    fn build_query(&self, table: &str, address: &str, public_key: Option<&str>) -> ImportQuery {
        let mut query = ImportQuery::new(table).at_address(address);

        if let Some(ref schema) = self.schema {
            query = query.schema(schema);
        }

        if let Some(ref columns) = self.columns {
            let cols: Vec<&str> = columns.iter().map(String::as_str).collect();
            query = query.columns(cols);
        }

        if let Some(pk) = public_key {
            query = query.with_public_key(pk);
        }

        query = query
            .encoding(&self.encoding)
            .column_separator(self.column_separator)
            .column_delimiter(self.column_delimiter)
            .row_separator(self.row_separator)
            .skip(self.skip_rows)
            .trim(self.trim_mode)
            .compressed(self.compression);

        if let Some(ref null_val) = self.null_value {
            query = query.null_value(null_val);
        }

        if let Some(limit) = self.reject_limit {
            query = query.reject_limit(limit);
        }

        query
    }
}

/// Sender for streaming data to the HTTP transport.
///
/// This is used with the callback-based import to send data chunks.
pub struct DataPipeSender {
    tx: mpsc::Sender<Vec<u8>>,
}

impl DataPipeSender {
    /// Create a new data pipe sender.
    fn new(tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self { tx }
    }

    /// Send a chunk of data.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to send.
    ///
    /// # Errors
    ///
    /// Returns `ImportError::ChannelError` if the channel is closed.
    pub async fn send(&self, data: Vec<u8>) -> Result<(), ImportError> {
        self.tx
            .send(data)
            .await
            .map_err(|e| ImportError::ChannelError(format!("Failed to send data: {e}")))
    }

    /// Send a CSV row as formatted data.
    ///
    /// # Arguments
    ///
    /// * `row` - Iterator of field values.
    /// * `separator` - Column separator character.
    /// * `delimiter` - Column delimiter character.
    /// * `row_separator` - Row separator to append.
    ///
    /// # Errors
    ///
    /// Returns `ImportError::ChannelError` if the channel is closed.
    pub async fn send_row<I, T>(
        &self,
        row: I,
        separator: char,
        delimiter: char,
        row_separator: &RowSeparator,
    ) -> Result<(), ImportError>
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let formatted = format_csv_row(row, separator, delimiter, row_separator);
        self.send(formatted.into_bytes()).await
    }
}

/// Format a row of values as a CSV line.
fn format_csv_row<I, T>(
    row: I,
    separator: char,
    delimiter: char,
    row_separator: &RowSeparator,
) -> String
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut line = String::new();
    let mut first = true;

    for field in row {
        if !first {
            line.push(separator);
        }
        first = false;

        let value = field.as_ref();
        // Check if the value needs quoting
        let needs_quoting = value.contains(separator)
            || value.contains(delimiter)
            || value.contains('\n')
            || value.contains('\r');

        if needs_quoting {
            line.push(delimiter);
            // Escape delimiter characters by doubling them
            for ch in value.chars() {
                if ch == delimiter {
                    line.push(delimiter);
                }
                line.push(ch);
            }
            line.push(delimiter);
        } else {
            line.push_str(value);
        }
    }

    // Add row separator
    match row_separator {
        RowSeparator::LF => line.push('\n'),
        RowSeparator::CR => line.push('\r'),
        RowSeparator::CRLF => {
            line.push('\r');
            line.push('\n');
        }
    }

    line
}

/// Detects compression type from file extension if not explicitly set.
///
/// # Arguments
///
/// * `file_path` - Path to the file
/// * `explicit_compression` - Explicitly set compression type
///
/// # Returns
///
/// The detected or explicit compression type.
fn detect_compression(file_path: &Path, explicit_compression: Compression) -> Compression {
    if explicit_compression != Compression::None {
        return explicit_compression;
    }

    let path_str = file_path.to_string_lossy().to_lowercase();

    if path_str.ends_with(".gz") || path_str.ends_with(".gzip") {
        Compression::Gzip
    } else if path_str.ends_with(".bz2") || path_str.ends_with(".bzip2") {
        Compression::Bzip2
    } else {
        Compression::None
    }
}

/// Checks if a file is already compressed based on extension.
///
/// # Arguments
///
/// * `file_path` - Path to the file
///
/// # Returns
///
/// `true` if the file appears to be compressed.
fn is_compressed_file(file_path: &Path) -> bool {
    let path_str = file_path.to_string_lossy().to_lowercase();
    path_str.ends_with(".gz")
        || path_str.ends_with(".gzip")
        || path_str.ends_with(".bz2")
        || path_str.ends_with(".bzip2")
}

/// Import CSV data from a file path.
///
/// This function reads CSV data from the specified file and imports it into
/// the target table using the HTTP transport layer.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements. Takes SQL string and returns row count.
/// * `table` - Name of the target table.
/// * `file_path` - Path to the CSV file.
/// * `options` - Import options.
///
/// # Returns
///
/// The number of rows imported on success.
///
/// # Errors
///
/// Returns `ImportError` if the import fails.
///
/// # Example
///
/// ```no_run
/// use exarrow_rs::import::{import_from_file, CsvImportOptions};
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // This would typically be called with a session's execute function
/// let executor = |sql: String| async move { Ok::<u64, String>(100) };
/// let rows = import_from_file(
///     executor,
///     "my_table",
///     Path::new("data.csv"),
///     CsvImportOptions::default()
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn import_from_file<F, Fut>(
    execute_sql: F,
    table: &str,
    file_path: &Path,
    options: CsvImportOptions,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    // Detect compression from file extension if not explicitly set
    let compression = detect_compression(file_path, options.compression);
    let options = CsvImportOptions {
        compression,
        ..options
    };

    // Check if file is already compressed before moving into async closure
    let file_is_compressed = is_compressed_file(file_path);

    // Read the file synchronously
    let data = std::fs::read(file_path)?;

    import_csv_internal(
        execute_sql,
        table,
        options,
        move |mut client, compression| {
            Box::pin(async move {
                // Apply compression if needed (unless file is already compressed)
                let compressed_data = if file_is_compressed {
                    // File is already compressed, send as-is
                    data
                } else {
                    compress_data(&data, compression)?
                };

                // Wait for HTTP GET from Exasol and send response with chunked encoding
                send_import_response(&mut client, &compressed_data).await?;

                Ok(())
            })
        },
    )
    .await
}

/// Import CSV data from an async reader (stream).
///
/// This function reads CSV data from an async reader and imports it into
/// the target table using the HTTP transport layer.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements.
/// * `table` - Name of the target table.
/// * `reader` - Async reader providing CSV data.
/// * `options` - Import options.
///
/// # Returns
///
/// The number of rows imported on success.
///
/// # Errors
///
/// Returns `ImportError` if the import fails.
pub async fn import_from_stream<R, F, Fut>(
    execute_sql: F,
    table: &str,
    reader: R,
    options: CsvImportOptions,
) -> Result<u64, ImportError>
where
    R: AsyncRead + Unpin + Send + 'static,
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    import_csv_internal(execute_sql, table, options, |mut client, compression| {
        Box::pin(async move { stream_reader_to_connection(reader, &mut client, compression).await })
    })
    .await
}

/// Import CSV data from an iterator of rows.
///
/// This function converts iterator rows to CSV format and imports them into
/// the target table using the HTTP transport layer.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements.
/// * `table` - Name of the target table.
/// * `rows` - Iterator of rows, where each row is an iterator of field values.
/// * `options` - Import options.
///
/// # Returns
///
/// The number of rows imported on success.
///
/// # Errors
///
/// Returns `ImportError` if the import fails.
///
/// # Example
///
/// ```no_run
/// use exarrow_rs::import::{import_from_iter, CsvImportOptions};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let data = vec![
///     vec!["1", "John", "john@example.com"],
///     vec!["2", "Jane", "jane@example.com"],
/// ];
///
/// let executor = |sql: String| async move { Ok::<u64, String>(2) };
/// let rows = import_from_iter(
///     executor,
///     "users",
///     data,
///     CsvImportOptions::default()
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn import_from_iter<I, T, S, F, Fut>(
    execute_sql: F,
    table: &str,
    rows: I,
    options: CsvImportOptions,
) -> Result<u64, ImportError>
where
    I: IntoIterator<Item = T> + Send + 'static,
    T: IntoIterator<Item = S> + Send,
    S: AsRef<str>,
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    let separator = options.column_separator;
    let delimiter = options.column_delimiter;
    let row_sep = options.row_separator;

    import_csv_internal(
        execute_sql,
        table,
        options,
        move |mut client, compression| {
            Box::pin(async move {
                // Format all rows as CSV
                let mut data = Vec::new();

                for row in rows {
                    let formatted = format_csv_row(row, separator, delimiter, &row_sep);
                    data.extend_from_slice(formatted.as_bytes());
                }

                // Apply compression if needed
                let compressed_data = compress_data(&data, compression)?;

                // Wait for HTTP GET from Exasol and send response with chunked encoding
                send_import_response(&mut client, &compressed_data).await?;

                Ok(())
            })
        },
    )
    .await
}

/// Import CSV data using a callback function.
///
/// This function allows custom data generation through a callback that receives
/// a `DataPipeSender` for streaming data to the import.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements.
/// * `table` - Name of the target table.
/// * `callback` - Callback function that generates data.
/// * `options` - Import options.
///
/// # Returns
///
/// The number of rows imported on success.
///
/// # Errors
///
/// Returns `ImportError` if the import fails.
///
/// # Example
///
/// ```no_run
/// use exarrow_rs::import::{import_from_callback, CsvImportOptions, DataPipeSender, ImportError};
/// use exarrow_rs::query::import::RowSeparator;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let executor = |sql: String| async move { Ok::<u64, String>(100) };
/// let rows = import_from_callback(
///     executor,
///     "my_table",
///     |sender: DataPipeSender| async move {
///         for i in 0..100 {
///             sender.send_row(
///                 vec![i.to_string(), format!("item_{}", i)],
///                 ',',
///                 '"',
///                 &RowSeparator::LF,
///             ).await?;
///         }
///         Ok::<(), ImportError>(())
///     },
///     CsvImportOptions::default()
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn import_from_callback<F, Fut, C, CFut>(
    execute_sql: F,
    table: &str,
    callback: C,
    options: CsvImportOptions,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
    C: FnOnce(DataPipeSender) -> CFut + Send + 'static,
    CFut: Future<Output = Result<(), ImportError>> + Send,
{
    import_csv_internal(execute_sql, table, options, |mut client, compression| {
        Box::pin(async move {
            // Create a channel for data streaming
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(CHANNEL_BUFFER_SIZE);
            let sender = DataPipeSender::new(tx);

            // Spawn the callback task
            let callback_handle = tokio::spawn(async move { callback(sender).await });

            // Collect data from the channel
            let mut all_data = Vec::new();
            while let Some(chunk) = rx.recv().await {
                all_data.extend_from_slice(&chunk);
            }

            // Wait for callback to complete
            callback_handle
                .await
                .map_err(|e| ImportError::StreamError(format!("Callback task panicked: {e}")))?
                .map_err(|e| ImportError::StreamError(format!("Callback error: {e}")))?;

            // Apply compression if needed
            let compressed_data = compress_data(&all_data, compression)?;

            // Wait for HTTP GET from Exasol and send response with chunked encoding
            send_import_response(&mut client, &compressed_data).await?;

            Ok(())
        })
    })
    .await
}

/// Internal function to orchestrate CSV import.
///
/// This function implements the correct IMPORT protocol flow:
/// 1. Connects to Exasol via HTTP transport client (client mode)
/// 2. Gets the internal address from the handshake response
/// 3. Builds the IMPORT SQL statement using the internal address
/// 4. Starts IMPORT SQL execution in parallel via WebSocket
/// 5. Waits for HTTP GET request from Exasol
/// 6. Sends HTTP response with CSV data using chunked encoding
/// 7. Coordinates completion between HTTP and SQL tasks
///
/// # Protocol Flow
///
/// ```text
/// Client                           Exasol
///   |                                |
///   |-- Connect to port ------------>|
///   |<- EXA handshake (internal IP) -|
///   |                                |
///   |-- Execute IMPORT SQL --------->| (parallel, via WebSocket)
///   |                                |
///   |<- HTTP GET /001.csv -----------| (Exasol requests data)
///   |                                |
///   |-- HTTP 200 (chunked) --------->| (headers)
///   |-- Chunk: CSV data ------------>|
///   |-- Chunk: more data ----------->|
///   |-- Final chunk (0\r\n\r\n) ---->|
///   |                                |
///   |<- SQL result (row count) ------|
/// ```
async fn import_csv_internal<F, Fut, S, SFut>(
    execute_sql: F,
    table: &str,
    options: CsvImportOptions,
    stream_fn: S,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
    S: FnOnce(HttpTransportClient, Compression) -> SFut + Send + 'static,
    SFut: Future<Output = Result<(), ImportError>> + Send,
{
    // Connect to Exasol via HTTP transport client (performs handshake automatically)
    let client = HttpTransportClient::connect(&options.host, options.port, options.use_tls)
        .await
        .map_err(|e| {
            ImportError::HttpTransportError(format!("Failed to connect to Exasol: {e}"))
        })?;

    // Get internal address from the handshake response
    let internal_addr = client.internal_address().to_string();
    let public_key = client.public_key_fingerprint().map(String::from);

    // Build the IMPORT SQL statement using internal address
    let query = options.build_query(table, &internal_addr, public_key.as_deref());
    let sql = query.build();

    let compression = options.compression;

    // Spawn the data streaming task - this will wait for GET request from Exasol
    let stream_handle = tokio::spawn(async move {
        // Stream data through the established connection
        // The stream_fn is responsible for:
        // 1. Waiting for HTTP GET from Exasol (handle_import_request)
        // 2. Sending chunked response data
        // 3. Sending final chunk
        stream_fn(client, compression).await?;

        Ok::<(), ImportError>(())
    });

    // Execute the IMPORT SQL in parallel
    // This triggers Exasol to send the HTTP GET request through the tunnel
    let sql_result = execute_sql(sql).await;

    // Wait for the stream task to complete
    let stream_result = stream_handle.await;

    // Handle results - check stream task first as it may have protocol errors
    match stream_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => {
            return Err(ImportError::StreamError(format!(
                "Stream task panicked: {e}"
            )))
        }
    }

    // Return the row count from SQL execution
    sql_result.map_err(ImportError::SqlError)
}

/// Stream data from an async reader to the HTTP transport client.
///
/// This function:
/// 1. Waits for HTTP GET request from Exasol
/// 2. Reads all data from the reader
/// 3. Applies compression if configured
/// 4. Sends HTTP response with chunked encoding
async fn stream_reader_to_connection<R>(
    mut reader: R,
    client: &mut HttpTransportClient,
    compression: Compression,
) -> Result<(), ImportError>
where
    R: AsyncRead + Unpin,
{
    // Wait for HTTP GET request from Exasol before sending any data
    client.handle_import_request().await.map_err(|e| {
        ImportError::HttpTransportError(format!("Failed to handle import request: {e}"))
    })?;

    // Read all data from the reader
    let mut data = Vec::new();
    reader.read_to_end(&mut data).await?;

    // Apply compression if needed
    let compressed_data = compress_data(&data, compression)?;

    // Write data using chunked encoding
    write_chunked_data(client, &compressed_data).await
}

/// Write data using HTTP chunked transfer encoding.
///
/// The HTTP response headers (with chunked encoding) should already be sent
/// by `handle_import_request()`. This function writes the data chunks and
/// the final empty chunk.
async fn write_chunked_data(
    client: &mut HttpTransportClient,
    data: &[u8],
) -> Result<(), ImportError> {
    // Write data in chunks using HTTP chunked transfer encoding
    for chunk in data.chunks(DEFAULT_BUFFER_SIZE) {
        client
            .write_chunked_body(chunk)
            .await
            .map_err(ImportError::TransportError)?;
    }

    // Send final empty chunk to signal end of transfer
    client
        .write_final_chunk()
        .await
        .map_err(ImportError::TransportError)?;

    Ok(())
}

/// Send HTTP response for import with the given data.
///
/// This function:
/// 1. Waits for HTTP GET request from Exasol
/// 2. Sends HTTP response with chunked encoding
/// 3. Writes the data in chunks
/// 4. Sends the final empty chunk
async fn send_import_response(
    client: &mut HttpTransportClient,
    data: &[u8],
) -> Result<(), ImportError> {
    // Wait for HTTP GET request from Exasol
    client.handle_import_request().await.map_err(|e| {
        ImportError::HttpTransportError(format!("Failed to handle import request: {e}"))
    })?;

    // Write data using chunked encoding
    write_chunked_data(client, data).await
}

/// Compress data using the specified compression type.
fn compress_data(data: &[u8], compression: Compression) -> Result<Vec<u8>, ImportError> {
    match compression {
        Compression::None => Ok(data.to_vec()),
        Compression::Gzip => {
            let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(data).map_err(|e| {
                ImportError::CompressionError(format!("Gzip compression failed: {e}"))
            })?;
            encoder.finish().map_err(|e| {
                ImportError::CompressionError(format!("Gzip finalization failed: {e}"))
            })
        }
        Compression::Bzip2 => {
            let mut encoder = BzEncoder::new(Vec::new(), bzip2::Compression::default());
            encoder.write_all(data).map_err(|e| {
                ImportError::CompressionError(format!("Bzip2 compression failed: {e}"))
            })?;
            encoder.finish().map_err(|e| {
                ImportError::CompressionError(format!("Bzip2 finalization failed: {e}"))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test CsvImportOptions defaults
    #[test]
    fn test_csv_import_options_default() {
        let opts = CsvImportOptions::default();

        assert_eq!(opts.encoding, "UTF-8");
        assert_eq!(opts.column_separator, ',');
        assert_eq!(opts.column_delimiter, '"');
        assert_eq!(opts.row_separator, RowSeparator::LF);
        assert_eq!(opts.skip_rows, 0);
        assert!(opts.null_value.is_none());
        assert_eq!(opts.trim_mode, TrimMode::None);
        assert_eq!(opts.compression, Compression::None);
        assert!(opts.reject_limit.is_none());
        assert!(!opts.use_tls);
        assert!(opts.schema.is_none());
        assert!(opts.columns.is_none());
        assert_eq!(opts.host, "");
        assert_eq!(opts.port, 0);
    }

    // Test CsvImportOptions builder methods
    #[test]
    fn test_csv_import_options_builder() {
        let opts = CsvImportOptions::new()
            .encoding("ISO-8859-1")
            .column_separator(';')
            .column_delimiter('\'')
            .row_separator(RowSeparator::CRLF)
            .skip_rows(1)
            .null_value("NULL")
            .trim_mode(TrimMode::Trim)
            .compression(Compression::Gzip)
            .reject_limit(100)
            .use_tls(false)
            .schema("my_schema")
            .columns(vec!["col1".to_string(), "col2".to_string()])
            .exasol_host("exasol.example.com")
            .exasol_port(8563);

        assert_eq!(opts.encoding, "ISO-8859-1");
        assert_eq!(opts.column_separator, ';');
        assert_eq!(opts.column_delimiter, '\'');
        assert_eq!(opts.row_separator, RowSeparator::CRLF);
        assert_eq!(opts.skip_rows, 1);
        assert_eq!(opts.null_value, Some("NULL".to_string()));
        assert_eq!(opts.trim_mode, TrimMode::Trim);
        assert_eq!(opts.compression, Compression::Gzip);
        assert_eq!(opts.reject_limit, Some(100));
        assert!(!opts.use_tls);
        assert_eq!(opts.schema, Some("my_schema".to_string()));
        assert_eq!(
            opts.columns,
            Some(vec!["col1".to_string(), "col2".to_string()])
        );
        assert_eq!(opts.host, "exasol.example.com");
        assert_eq!(opts.port, 8563);
    }

    // Test ImportQuery building from options
    #[test]
    fn test_build_query_basic() {
        let opts = CsvImportOptions::default();
        let query = opts.build_query("my_table", "127.0.0.1:8080", None);
        let sql = query.build();

        assert!(sql.contains("IMPORT INTO my_table"));
        assert!(sql.contains("FROM CSV AT 'http://127.0.0.1:8080'"));
        assert!(sql.contains("ENCODING = 'UTF-8'"));
        assert!(sql.contains("COLUMN SEPARATOR = ','"));
    }

    #[test]
    fn test_build_query_with_schema_and_columns() {
        let opts = CsvImportOptions::default()
            .schema("test_schema")
            .columns(vec!["id".to_string(), "name".to_string()]);

        let query = opts.build_query("users", "127.0.0.1:8080", None);
        let sql = query.build();

        assert!(sql.contains("IMPORT INTO test_schema.users"));
        assert!(sql.contains("(id, name)"));
    }

    #[test]
    fn test_build_query_with_tls() {
        let opts = CsvImportOptions::default();
        let fingerprint = "ABC123DEF456";
        let query = opts.build_query("my_table", "127.0.0.1:8080", Some(fingerprint));
        let sql = query.build();

        assert!(sql.contains("FROM CSV AT 'https://127.0.0.1:8080'"));
        assert!(sql.contains(&format!("PUBLIC KEY '{}'", fingerprint)));
    }

    #[test]
    fn test_build_query_with_all_options() {
        let opts = CsvImportOptions::default()
            .encoding("ISO-8859-1")
            .column_separator(';')
            .column_delimiter('\'')
            .row_separator(RowSeparator::CRLF)
            .skip_rows(2)
            .null_value("\\N")
            .trim_mode(TrimMode::LTrim)
            .compression(Compression::Bzip2)
            .reject_limit(50);

        let query = opts.build_query("data", "127.0.0.1:8080", None);
        let sql = query.build();

        assert!(sql.contains("ENCODING = 'ISO-8859-1'"));
        assert!(sql.contains("COLUMN SEPARATOR = ';'"));
        assert!(sql.contains("COLUMN DELIMITER = '''"));
        assert!(sql.contains("ROW SEPARATOR = 'CRLF'"));
        assert!(sql.contains("SKIP = 2"));
        assert!(sql.contains("NULL = '\\N'"));
        assert!(sql.contains("TRIM = 'LTRIM'"));
        assert!(sql.contains("FILE '001.csv.bz2'"));
        assert!(sql.contains("REJECT LIMIT 50"));
    }

    // Test CSV row formatting
    #[test]
    fn test_format_csv_row_basic() {
        let row = vec!["a", "b", "c"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);
        assert_eq!(formatted, "a,b,c\n");
    }

    #[test]
    fn test_format_csv_row_with_different_separator() {
        let row = vec!["a", "b", "c"];
        let formatted = format_csv_row(row, ';', '"', &RowSeparator::LF);
        assert_eq!(formatted, "a;b;c\n");
    }

    #[test]
    fn test_format_csv_row_with_crlf() {
        let row = vec!["a", "b", "c"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::CRLF);
        assert_eq!(formatted, "a,b,c\r\n");
    }

    #[test]
    fn test_format_csv_row_with_cr() {
        let row = vec!["a", "b"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::CR);
        assert_eq!(formatted, "a,b\r");
    }

    #[test]
    fn test_format_csv_row_needs_quoting_separator() {
        let row = vec!["a,b", "c"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);
        assert_eq!(formatted, "\"a,b\",c\n");
    }

    #[test]
    fn test_format_csv_row_needs_quoting_newline() {
        let row = vec!["a\nb", "c"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);
        assert_eq!(formatted, "\"a\nb\",c\n");
    }

    #[test]
    fn test_format_csv_row_needs_quoting_delimiter() {
        let row = vec!["a\"b", "c"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);
        // Delimiter should be escaped by doubling
        assert_eq!(formatted, "\"a\"\"b\",c\n");
    }

    #[test]
    fn test_format_csv_row_empty_fields() {
        let row = vec!["", "b", ""];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);
        assert_eq!(formatted, ",b,\n");
    }

    #[test]
    fn test_format_csv_row_single_field() {
        let row = vec!["only"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);
        assert_eq!(formatted, "only\n");
    }

    // Test compression
    #[test]
    fn test_compress_data_none() {
        let data = b"test data";
        let result = compress_data(data, Compression::None).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_compress_data_gzip() {
        let data = b"test data for gzip compression";
        let result = compress_data(data, Compression::Gzip).unwrap();

        // Verify it's valid gzip (starts with gzip magic bytes)
        assert!(result.len() >= 2);
        assert_eq!(result[0], 0x1f);
        assert_eq!(result[1], 0x8b);
    }

    #[test]
    fn test_compress_data_bzip2() {
        let data = b"test data for bzip2 compression";
        let result = compress_data(data, Compression::Bzip2).unwrap();

        // Verify it's valid bzip2 (starts with "BZ" magic)
        assert!(result.len() >= 2);
        assert_eq!(result[0], b'B');
        assert_eq!(result[1], b'Z');
    }

    // Test error types
    #[test]
    fn test_import_error_display() {
        let err = ImportError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert!(err.to_string().contains("IO error"));

        let err = ImportError::SqlError("syntax error".to_string());
        assert!(err.to_string().contains("SQL execution failed"));

        let err = ImportError::HttpTransportError("connection refused".to_string());
        assert!(err.to_string().contains("HTTP transport failed"));

        let err = ImportError::CompressionError("invalid data".to_string());
        assert!(err.to_string().contains("Compression error"));
    }

    // Test DataPipeSender
    #[tokio::test]
    async fn test_data_pipe_sender_send() {
        let (tx, mut rx) = mpsc::channel(10);
        let sender = DataPipeSender::new(tx);

        sender.send(b"test data".to_vec()).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, b"test data");
    }

    #[tokio::test]
    async fn test_data_pipe_sender_send_row() {
        let (tx, mut rx) = mpsc::channel(10);
        let sender = DataPipeSender::new(tx);

        sender
            .send_row(vec!["a", "b", "c"], ',', '"', &RowSeparator::LF)
            .await
            .unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, b"a,b,c\n");
    }

    #[tokio::test]
    async fn test_data_pipe_sender_closed_channel() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(10);
        let sender = DataPipeSender::new(tx);

        // Drop receiver to close channel
        drop(rx);

        let result = sender.send(b"test".to_vec()).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ImportError::ChannelError(_)));
    }

    // Test building SQL with compression
    #[test]
    fn test_build_query_compression_gzip() {
        let opts = CsvImportOptions::default().compression(Compression::Gzip);
        let query = opts.build_query("table", "127.0.0.1:8080", None);
        let sql = query.build();

        assert!(sql.contains("FILE '001.csv.gz'"));
    }

    #[test]
    fn test_build_query_compression_bzip2() {
        let opts = CsvImportOptions::default().compression(Compression::Bzip2);
        let query = opts.build_query("table", "127.0.0.1:8080", None);
        let sql = query.build();

        assert!(sql.contains("FILE '001.csv.bz2'"));
    }

    // Test detect_compression function
    #[test]
    fn test_detect_compression_explicit_overrides() {
        // Explicit compression should override file extension detection
        let path = Path::new("data.csv");
        assert_eq!(
            detect_compression(path, Compression::Gzip),
            Compression::Gzip
        );
        assert_eq!(
            detect_compression(path, Compression::Bzip2),
            Compression::Bzip2
        );
    }

    #[test]
    fn test_detect_compression_from_extension_gzip() {
        let path = Path::new("data.csv.gz");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::Gzip
        );

        let path = Path::new("data.csv.gzip");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::Gzip
        );

        // Case insensitive
        let path = Path::new("DATA.CSV.GZ");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::Gzip
        );
    }

    #[test]
    fn test_detect_compression_from_extension_bzip2() {
        let path = Path::new("data.csv.bz2");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::Bzip2
        );

        let path = Path::new("data.csv.bzip2");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::Bzip2
        );
    }

    #[test]
    fn test_detect_compression_no_compression() {
        let path = Path::new("data.csv");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::None
        );

        let path = Path::new("data.txt");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::None
        );
    }

    // Test is_compressed_file function
    #[test]
    fn test_is_compressed_file_gzip() {
        assert!(is_compressed_file(Path::new("data.csv.gz")));
        assert!(is_compressed_file(Path::new("data.csv.gzip")));
        assert!(is_compressed_file(Path::new("DATA.CSV.GZ"))); // case insensitive
    }

    #[test]
    fn test_is_compressed_file_bzip2() {
        assert!(is_compressed_file(Path::new("data.csv.bz2")));
        assert!(is_compressed_file(Path::new("data.csv.bzip2")));
    }

    #[test]
    fn test_is_compressed_file_uncompressed() {
        assert!(!is_compressed_file(Path::new("data.csv")));
        assert!(!is_compressed_file(Path::new("data.txt")));
        assert!(!is_compressed_file(Path::new("data")));
    }

    // Test write_chunked_data with mock (indirectly via compress + format)
    #[test]
    fn test_csv_row_formatting_with_compression() {
        // Test that compression works with formatted CSV data
        let row = vec!["1", "test data", "value"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);

        let compressed = compress_data(formatted.as_bytes(), Compression::Gzip).unwrap();

        // Verify gzip magic bytes
        assert!(compressed.len() >= 2);
        assert_eq!(compressed[0], 0x1f);
        assert_eq!(compressed[1], 0x8b);
    }

    #[test]
    fn test_multiple_rows_formatting() {
        // Test formatting multiple rows as CSV
        let rows = vec![
            vec!["1", "Alice", "alice@example.com"],
            vec!["2", "Bob", "bob@example.com"],
            vec!["3", "Charlie", "charlie@example.com"],
        ];

        let mut data = Vec::new();
        for row in rows {
            let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);
            data.extend_from_slice(formatted.as_bytes());
        }

        let expected =
            "1,Alice,alice@example.com\n2,Bob,bob@example.com\n3,Charlie,charlie@example.com\n";
        assert_eq!(String::from_utf8(data).unwrap(), expected);
    }

    #[test]
    fn test_csv_special_characters_in_data() {
        // Test that special characters are properly escaped
        let row = vec!["1", "Hello, World!", "Contains \"quotes\""];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);

        // Field with comma should be quoted, field with quotes should be quoted and escaped
        assert_eq!(
            formatted,
            "1,\"Hello, World!\",\"Contains \"\"quotes\"\"\"\n"
        );
    }

    #[test]
    fn test_csv_row_with_newlines() {
        // Test that newlines in data are properly quoted
        let row = vec!["1", "Line1\nLine2", "normal"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);

        assert_eq!(formatted, "1,\"Line1\nLine2\",normal\n");
    }

    #[test]
    fn test_csv_row_with_carriage_return() {
        // Test that carriage returns in data are properly quoted
        let row = vec!["1", "Line1\rLine2", "normal"];
        let formatted = format_csv_row(row, ',', '"', &RowSeparator::LF);

        assert_eq!(formatted, "1,\"Line1\rLine2\",normal\n");
    }

    // Test import flow documentation (protocol flow)
    #[test]
    fn test_import_protocol_flow_documentation() {
        // This test documents the expected protocol flow for IMPORT operations
        // The actual flow is tested in integration tests, but this ensures
        // the documentation in import_csv_internal matches expectations

        // Protocol flow for IMPORT:
        // 1. Client connects to Exasol and gets internal address via handshake
        // 2. Client starts executing IMPORT SQL via WebSocket (async)
        // 3. Exasol sends HTTP GET request through the tunnel connection
        // 4. Client receives GET, sends HTTP response headers (chunked encoding)
        // 5. Client streams CSV data as chunked body
        // 6. Client sends final chunk (0\r\n\r\n)
        // 7. IMPORT SQL completes

        // This test is informational - actual integration testing is done
        // against a real Exasol instance
    }

    // Test compression detection with various path formats
    #[test]
    fn test_detect_compression_path_variations() {
        // Test with absolute paths
        let path = Path::new("/home/user/data/file.csv.gz");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::Gzip
        );

        // Test with relative paths
        let path = Path::new("./data/file.csv.bz2");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::Bzip2
        );

        // Test with just filename
        let path = Path::new("file.csv");
        assert_eq!(
            detect_compression(path, Compression::None),
            Compression::None
        );
    }
}
