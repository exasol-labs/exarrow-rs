//! Parquet file import functionality for Exasol.
//!
//! This module provides functions to import Parquet files into Exasol tables.
//! Since Exasol only accepts CSV format over HTTP, Parquet data is converted
//! to CSV on-the-fly during the import process.
//!
//! # Example
//!

use std::future::Future;
use std::path::Path;

use arrow::array::{
    Array, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array, Float64Array,
    Int16Array, Int32Array, Int64Array, Int8Array, LargeBinaryArray, LargeStringArray, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::query::import::{ImportFileEntry, ImportFormat, ImportQuery, RowSeparator};
use crate::transport::HttpTransportClient;
use crate::types::{infer_schema_from_parquet, infer_schema_from_parquet_files, ColumnNameMode};

use super::parallel::{
    convert_parquet_files_to_csv, resolve_stream_task, stream_files_parallel,
    stream_parquet_files_parallel, ParallelTransportPool, CHUNK_SIZE as CHUNKED_TRANSFER_SIZE,
};
use super::source::IntoFileSources;
use super::ImportError;

/// Error substring used to detect idempotent DDL failures (table already exists).
const DDL_ALREADY_EXISTS_MARKER: &str = "already exists";

/// Options for Parquet import operations.
#[derive(Debug, Clone)]
pub struct ParquetImportOptions {
    /// Target schema for the import (optional, uses default if not specified)
    pub schema: Option<String>,
    /// Columns to import (optional, imports all columns if not specified)
    pub columns: Option<Vec<String>>,
    /// Batch size for reading Parquet records (default: 1024)
    pub batch_size: usize,
    /// String representation of NULL values in CSV output
    pub null_value: String,
    /// Column separator for CSV output (default: ',')
    pub column_separator: char,
    /// Column delimiter for CSV output (default: '"')
    pub column_delimiter: char,
    /// Whether to use TLS for the HTTP transport tunnel
    pub use_tls: bool,

    /// Exasol host for HTTP transport connection.
    /// This is typically the same host as the WebSocket connection.
    pub host: String,

    /// Exasol port for HTTP transport connection.
    /// This is typically the same port as the WebSocket connection.
    pub port: u16,

    /// If true, create the target table before import if it doesn't exist.
    ///
    /// When enabled, the system will:
    /// 1. Infer the schema from the Parquet file(s)
    /// 2. Generate CREATE TABLE DDL
    /// 3. Execute the DDL (table may already exist, which is fine)
    /// 4. Proceed with the import
    ///
    /// Default: false
    pub create_table_if_not_exists: bool,

    /// Column name handling mode for auto-created tables.
    ///
    /// Only used when `create_table_if_not_exists` is true.
    /// - `Quoted`: Preserve original column names (default)
    /// - `Sanitize`: Convert to uppercase, replace invalid chars
    pub column_name_mode: ColumnNameMode,

    /// Override auto-detection of native Parquet import.
    /// - `None`: auto-detect from server version (default)
    /// - `Some(true)`: force native Parquet import path
    /// - `Some(false)`: force CSV conversion path
    pub native_parquet_override: Option<bool>,
}

impl Default for ParquetImportOptions {
    fn default() -> Self {
        Self {
            schema: None,
            columns: None,
            batch_size: 1024,
            null_value: String::new(),
            column_separator: ',',
            column_delimiter: '"',
            use_tls: false,
            host: String::new(),
            port: 0,
            create_table_if_not_exists: false,
            column_name_mode: ColumnNameMode::default(),
            native_parquet_override: None,
        }
    }
}

impl ParquetImportOptions {
    /// Create a new `ParquetImportOptions` with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    #[must_use]
    pub fn with_columns(mut self, columns: Vec<String>) -> Self {
        self.columns = Some(columns);
        self
    }

    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    #[must_use]
    pub fn with_null_value(mut self, null_value: impl Into<String>) -> Self {
        self.null_value = null_value.into();
        self
    }

    #[must_use]
    pub fn with_column_separator(mut self, separator: char) -> Self {
        self.column_separator = separator;
        self
    }

    #[must_use]
    pub fn with_column_delimiter(mut self, delimiter: char) -> Self {
        self.column_delimiter = delimiter;
        self
    }

    /// Sets whether to use TLS for the HTTP transport tunnel.
    #[must_use]
    pub fn use_tls(mut self, v: bool) -> Self {
        self.use_tls = v;
        self
    }

    /// Set the Exasol host for HTTP transport connection.
    ///
    /// This should be the same host as used for the WebSocket connection.
    #[must_use]
    pub fn with_exasol_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Set the Exasol port for HTTP transport connection.
    ///
    /// This should be the same port as used for the WebSocket connection.
    #[must_use]
    pub fn with_exasol_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Enable automatic table creation before import.
    ///
    /// When enabled, the system will infer the schema from the Parquet file(s)
    /// and create the target table if it doesn't exist.
    ///
    /// # Arguments
    ///
    /// * `create` - Whether to create the table automatically
    #[must_use]
    pub fn with_create_table_if_not_exists(mut self, create: bool) -> Self {
        self.create_table_if_not_exists = create;
        self
    }

    /// Set the column name handling mode for auto-created tables.
    ///
    /// Only applies when `create_table_if_not_exists` is true.
    ///
    /// # Arguments
    ///
    /// * `mode` - How to handle column names in DDL generation
    #[must_use]
    pub fn with_column_name_mode(mut self, mode: ColumnNameMode) -> Self {
        self.column_name_mode = mode;
        self
    }

    /// Override auto-detection of native Parquet import.
    ///
    /// By default (`None`), the import path is selected automatically based
    /// on the connected Exasol server version. Pass `Some(true)` to force the
    /// native Parquet import path, or `Some(false)` to force the CSV
    /// conversion path.
    ///
    /// # Arguments
    ///
    /// * `override_` - Native Parquet override
    #[must_use]
    pub fn with_native_parquet(mut self, override_: Option<bool>) -> Self {
        self.native_parquet_override = override_;
        self
    }
}

/// Imports data from a Parquet file into an Exasol table.
///
/// This function reads a Parquet file, converts each RecordBatch to CSV format,
/// and streams the data to Exasol via HTTP transport.
///
/// # Protocol Flow
///
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements. Takes SQL string and returns row count.
///   Must be callable multiple times if `create_table_if_not_exists` is enabled.
/// * `table` - The target table name
/// * `file_path` - Path to the Parquet file
/// * `options` - Import options
///
/// # Returns
///
/// The number of rows imported on success.
///
/// # Errors
///
/// Returns `ImportError` if:
/// - The file cannot be read
/// - Parquet parsing fails
/// - Data conversion fails
/// - HTTP transport fails
/// - Schema inference fails (when `create_table_if_not_exists` is enabled)
pub async fn import_from_parquet<F, Fut>(
    mut execute_sql: F,
    table: &str,
    file_path: &Path,
    options: ParquetImportOptions,
    use_native: bool,
) -> Result<u64, ImportError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    if options.create_table_if_not_exists {
        let inferred_schema = infer_schema_from_parquet(file_path, options.column_name_mode)?;
        let ddl = inferred_schema.to_ddl(table, options.schema.as_deref());
        create_table_from_ddl(&mut execute_sql, ddl).await?;
    }

    if use_native {
        // Native Parquet path: read raw file bytes and serve them via HTTP
        // range requests. The server parses the Parquet directly, so all
        // CSV-specific options (encoding, separator, delimiter, null_value)
        // are skipped.
        let file_bytes = tokio::fs::read(file_path).await?;
        return serve_parquet_bytes(execute_sql, table, &options, file_bytes).await;
    }

    let csv_data = parquet_to_csv_bytes(std::fs::File::open(file_path)?, &options)?;

    if csv_data.is_empty() {
        return Ok(0);
    }

    stream_csv_bytes(execute_sql, table, &options, csv_data).await
}

/// Execute a CREATE TABLE statement, treating an "already exists" failure as success.
///
/// Auto-creation is best-effort by design: a concurrent importer may have
/// created the table already, which must not fail the import.
async fn create_table_from_ddl<F, Fut>(execute_sql: F, ddl: String) -> Result<(), ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    match execute_sql(ddl).await {
        Ok(_) => Ok(()),
        Err(e) if e.to_lowercase().contains(DDL_ALREADY_EXISTS_MARKER) => Ok(()),
        Err(e) => Err(ImportError::SqlError(e)),
    }
}

/// Read a Parquet source and render every batch as CSV rows terminated by LF.
fn parquet_to_csv_bytes<R>(
    source: R,
    options: &ParquetImportOptions,
) -> Result<Vec<u8>, ImportError>
where
    R: parquet::file::reader::ChunkReader + 'static,
{
    let builder = ParquetRecordBatchReaderBuilder::try_new(source)?;
    let reader = builder.with_batch_size(options.batch_size).build()?;

    let mut csv_data = Vec::new();
    for batch_result in reader {
        let batch = batch_result?;
        for row in record_batch_to_csv(&batch, options)? {
            csv_data.extend_from_slice(row.as_bytes());
            csv_data.push(b'\n');
        }
    }

    Ok(csv_data)
}

/// Serve raw Parquet bytes to the server over the native HEAD/GET-Range protocol.
///
/// The server parses the Parquet payload itself, so the IMPORT statement carries
/// no CSV format options.
async fn serve_parquet_bytes<F, Fut>(
    execute_sql: F,
    table: &str,
    options: &ParquetImportOptions,
    file_bytes: Vec<u8>,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    let mut client = connect_transport(options).await?;

    let query = apply_schema_and_columns(
        ImportQuery::new(table)
            .at_address(client.internal_address())
            .with_format(ImportFormat::Parquet)
            .file_name("001"),
        options,
    );
    let sql = with_public_key(query, client.public_key_fingerprint()).build();

    let stream_handle = tokio::spawn(async move {
        client
            .handle_parquet_import_requests(&file_bytes)
            .await
            .map_err(ImportError::TransportError)
    });

    let sql_result = execute_sql(sql).await;
    resolve_stream_task(stream_handle.await)?;

    sql_result.map_err(ImportError::SqlError)
}

/// Stream converted CSV bytes to the server using chunked transfer encoding.
async fn stream_csv_bytes<F, Fut>(
    execute_sql: F,
    table: &str,
    options: &ParquetImportOptions,
    csv_data: Vec<u8>,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    let mut client = connect_transport(options).await?;

    let query = build_single_file_csv_query(table, options, client.internal_address());
    let sql = with_public_key(query, client.public_key_fingerprint()).build();

    // Run data streaming and SQL execution concurrently: the SQL statement is
    // what makes the server open the tunnel the stream task writes into.
    let stream_handle = tokio::spawn(async move {
        // This call also sends the chunked response headers
        client.handle_import_request().await.map_err(|e| {
            ImportError::HttpTransportError(format!("Failed to handle import request: {e}"))
        })?;

        for chunk in csv_data.chunks(CHUNKED_TRANSFER_SIZE) {
            client
                .write_chunked_body(chunk)
                .await
                .map_err(ImportError::TransportError)?;
        }

        client
            .write_final_chunk()
            .await
            .map_err(ImportError::TransportError)
    });

    let sql_result = execute_sql(sql).await;
    // Check the stream task first as it may hold the underlying protocol error.
    resolve_stream_task(stream_handle.await)?;

    sql_result.map_err(ImportError::SqlError)
}

async fn connect_transport(
    options: &ParquetImportOptions,
) -> Result<HttpTransportClient, ImportError> {
    HttpTransportClient::connect(&options.host, options.port, options.use_tls)
        .await
        .map_err(|e| ImportError::HttpTransportError(format!("Failed to connect to Exasol: {e}")))
}

fn build_single_file_csv_query(
    table: &str,
    options: &ParquetImportOptions,
    internal_addr: &str,
) -> ImportQuery {
    let query =
        apply_schema_and_columns(ImportQuery::new(table).at_address(internal_addr), options);
    apply_csv_format(query, options)
}

fn with_public_key(query: ImportQuery, public_key: Option<&str>) -> ImportQuery {
    match public_key {
        Some(pk) => query.with_public_key(pk),
        None => query,
    }
}

fn apply_schema_and_columns(mut query: ImportQuery, options: &ParquetImportOptions) -> ImportQuery {
    if let Some(ref schema) = options.schema {
        query = query.schema(schema);
    }

    if let Some(ref columns) = options.columns {
        let cols: Vec<&str> = columns.iter().map(String::as_str).collect();
        query = query.columns(cols);
    }

    query
}

fn apply_csv_format(query: ImportQuery, options: &ParquetImportOptions) -> ImportQuery {
    let query = query
        .encoding("UTF-8")
        .column_separator(options.column_separator)
        .column_delimiter(options.column_delimiter)
        .row_separator(RowSeparator::LF);

    if options.null_value.is_empty() {
        query
    } else {
        query.null_value(&options.null_value)
    }
}

/// Imports data from a Parquet stream into an Exasol table.
///
/// This function reads Parquet data from an async reader, converts each
/// RecordBatch to CSV format, and streams the data to Exasol via HTTP transport.
///
/// Note: Since Parquet files require random access to read footer metadata,
/// the entire stream is buffered into memory before parsing. For very large
/// Parquet files, consider using `import_from_parquet()` with a file path instead.
///
/// # Protocol Flow
///
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements. Takes SQL string and returns row count.
/// * `table` - The target table name
/// * `reader` - An async reader providing Parquet data
/// * `options` - Import options
///
/// # Returns
///
/// The number of rows imported on success.
///
/// # Errors
///
/// Returns `ImportError` if:
/// - The stream cannot be read
/// - Parquet parsing fails
/// - Data conversion fails
/// - HTTP transport fails
pub async fn import_from_parquet_stream<F, Fut, R>(
    execute_sql: F,
    table: &str,
    mut reader: R,
    options: ParquetImportOptions,
    use_native: bool,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
    R: AsyncRead + Unpin + Send,
{
    // Read the entire stream into memory
    // Note: Parquet files require random access to read footer metadata,
    // so we need to buffer the entire stream before parsing
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer).await?;

    if buffer.is_empty() {
        return Ok(0);
    }

    if use_native {
        // Native Parquet path: forward buffered bytes directly to the server
        // via HTTP range requests. Skip CSV conversion entirely.
        return serve_parquet_bytes(execute_sql, table, &options, buffer).await;
    }

    let csv_data = parquet_to_csv_bytes(Bytes::from(buffer), &options)?;

    if csv_data.is_empty() {
        return Ok(0);
    }

    stream_csv_bytes(execute_sql, table, &options, csv_data).await
}

/// Imports data from multiple Parquet files in parallel into an Exasol table.
///
/// This function converts each Parquet file to CSV format concurrently using
/// tokio spawn_blocking tasks, then streams the converted data through parallel
/// HTTP transport connections.
///
/// For a single file, this function delegates to `import_from_parquet` for
/// optimal single-file performance.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements. Takes SQL string and returns row count.
/// * `table` - The target table name
/// * `file_paths` - File paths (accepts single path, Vec, array, or slice)
/// * `options` - Import options
///
/// # Returns
///
/// The number of rows imported on success.
///
/// # Errors
///
/// Returns `ImportError` if any conversion or import fails. Uses fail-fast semantics.
///
/// # Example
///
/// ```no_run
/// use exarrow_rs::import::parquet::{import_from_parquet_files, ParquetImportOptions};
/// use std::path::PathBuf;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let files = vec![
///     PathBuf::from("/data/part1.parquet"),
///     PathBuf::from("/data/part2.parquet"),
///     PathBuf::from("/data/part3.parquet"),
/// ];
///
/// let options = ParquetImportOptions::default()
///     .with_exasol_host("localhost")
///     .with_exasol_port(8563);
///
/// // let rows = import_from_parquet_files(execute_sql, "my_table", files, options).await?;
/// # Ok(())
/// # }
/// ```
pub async fn import_from_parquet_files<F, Fut, S>(
    mut execute_sql: F,
    table: &str,
    file_paths: S,
    options: ParquetImportOptions,
    use_native: bool,
) -> Result<u64, ImportError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
    S: IntoFileSources,
{
    let paths = file_paths.into_sources();

    // Delegate to single-file implementation for one file. The chosen
    // transport mode (native vs CSV) is forwarded so the single-file path
    // makes the matching decision.
    if paths.len() == 1 {
        return import_from_parquet(execute_sql, table, &paths[0], options, use_native).await;
    }

    if paths.is_empty() {
        return Err(ImportError::InvalidConfig(
            "No files provided for import".to_string(),
        ));
    }

    // Auto-create table if enabled (for multi-file, infer union schema)
    if options.create_table_if_not_exists {
        let inferred_schema = infer_schema_from_parquet_files(&paths, options.column_name_mode)?;
        let ddl = inferred_schema.to_ddl(table, options.schema.as_deref());
        create_table_from_ddl(&mut execute_sql, ddl).await?;
    }

    // Calculate connection count before transferring paths ownership
    let num_files = paths.len();

    if use_native {
        // Native Parquet path: skip CSV conversion entirely. The pool's
        // generated `001.csv`-style file names are reused as-is because
        // `ImportQuery::with_format(ImportFormat::Parquet)` rewrites the
        // suffix to `.parquet` via `get_file_name_for`.
        let pool =
            ParallelTransportPool::connect(&options.host, options.port, options.use_tls, num_files)
                .await?;

        let query =
            build_multi_file_parquet_native_query(table, &options, pool.query_file_entries());
        let sql = query.build();

        let connections = pool.into_connections();

        let stream_handle =
            tokio::spawn(async move { stream_parquet_files_parallel(connections, paths).await });

        let sql_result = execute_sql(sql).await;
        resolve_stream_task(stream_handle.await)?;

        return sql_result.map_err(ImportError::SqlError);
    }

    // Convert all Parquet files to CSV in parallel
    let csv_data_vec = convert_parquet_files_to_csv(
        paths,
        options.batch_size,
        options.null_value.clone(),
        options.column_separator,
        options.column_delimiter,
    )
    .await?;

    // Check for empty data
    if csv_data_vec.iter().all(|d| d.is_empty()) {
        return Ok(0);
    }

    // Establish parallel connections
    let pool =
        ParallelTransportPool::connect(&options.host, options.port, options.use_tls, num_files)
            .await?;

    // Build multi-file IMPORT SQL
    let query = build_multi_file_parquet_query(table, &options, pool.query_file_entries());
    let sql = query.build();

    // Get connections for streaming
    let connections = pool.into_connections();

    // Spawn parallel streaming task
    let stream_handle = tokio::spawn(async move {
        stream_files_parallel(
            connections,
            csv_data_vec,
            crate::query::import::Compression::None,
        )
        .await
    });

    // Execute the IMPORT SQL in parallel
    let sql_result = execute_sql(sql).await;

    // Wait for streaming to complete; check it first as it holds protocol errors
    resolve_stream_task(stream_handle.await)?;

    // Return the row count from SQL execution
    sql_result.map_err(ImportError::SqlError)
}

/// Build an ImportQuery for multi-file Parquet import (CSV conversion path).
fn build_multi_file_parquet_query(
    table: &str,
    options: &ParquetImportOptions,
    entries: Vec<ImportFileEntry>,
) -> ImportQuery {
    let query = apply_schema_and_columns(ImportQuery::new(table).with_files(entries), options);
    apply_csv_format(query, options)
}

/// Build an ImportQuery for multi-file native Parquet import.
///
/// Unlike the CSV variant, this query selects `ImportFormat::Parquet` so the
/// generated SQL emits `FROM PARQUET` and rewrites file extensions to
/// `.parquet`. CSV-specific options (encoding, separator, delimiter,
/// row separator, null_value) are intentionally omitted: the server reads the
/// schema from the Parquet payload itself.
fn build_multi_file_parquet_native_query(
    table: &str,
    options: &ParquetImportOptions,
    entries: Vec<ImportFileEntry>,
) -> ImportQuery {
    apply_schema_and_columns(
        ImportQuery::new(table)
            .with_files(entries)
            .with_format(ImportFormat::Parquet),
        options,
    )
}

/// Converts an Arrow RecordBatch to CSV rows.
///
/// This function handles all Arrow data types and converts them to CSV-compatible
/// string representations suitable for Exasol import.
///
/// # Arguments
///
/// * `batch` - The RecordBatch to convert
/// * `options` - Import options containing CSV formatting settings
///
/// # Returns
///
/// A vector of CSV row strings.
///
/// # Errors
///
/// Returns `ImportError::ConversionError` if a value cannot be converted.
pub fn record_batch_to_csv(
    batch: &RecordBatch,
    options: &ParquetImportOptions,
) -> Result<Vec<String>, ImportError> {
    let num_rows = batch.num_rows();
    let num_cols = batch.num_columns();

    if num_rows == 0 {
        return Ok(Vec::new());
    }

    let mut rows = Vec::with_capacity(num_rows);
    let separator = options.column_separator;
    let delimiter = options.column_delimiter;
    let null_value = &options.null_value;

    for row_idx in 0..num_rows {
        let mut row_parts = Vec::with_capacity(num_cols);

        for col_idx in 0..num_cols {
            let column = batch.column(col_idx);
            let value = format_arrow_value(column.as_ref(), row_idx, delimiter, null_value)?;
            row_parts.push(value);
        }

        rows.push(row_parts.join(&separator.to_string()));
    }

    Ok(rows)
}

/// Formats a single Arrow array value at the given index as a CSV string.
///
/// # Arguments
///
/// * `array` - The Arrow array
/// * `row_idx` - The row index to format
/// * `delimiter` - The CSV delimiter character for quoting
/// * `null_value` - The string representation of NULL
///
/// # Returns
///
/// The formatted value as a string.
fn format_arrow_value(
    array: &dyn Array,
    row_idx: usize,
    delimiter: char,
    null_value: &str,
) -> Result<String, ImportError> {
    if array.is_null(row_idx) {
        return Ok(null_value.to_string());
    }

    let data_type = array.data_type();
    let result = match data_type {
        DataType::Boolean => {
            let arr = array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Boolean array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::Int8 => {
            let arr = array
                .as_any()
                .downcast_ref::<Int8Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Int8 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::Int16 => {
            let arr = array
                .as_any()
                .downcast_ref::<Int16Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Int16 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::Int32 => {
            let arr = array
                .as_any()
                .downcast_ref::<Int32Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Int32 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::Int64 => {
            let arr = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Int64 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::UInt8 => {
            let arr = array
                .as_any()
                .downcast_ref::<UInt8Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid UInt8 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::UInt16 => {
            let arr = array
                .as_any()
                .downcast_ref::<UInt16Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid UInt16 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::UInt32 => {
            let arr = array
                .as_any()
                .downcast_ref::<UInt32Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid UInt32 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::UInt64 => {
            let arr = array
                .as_any()
                .downcast_ref::<UInt64Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid UInt64 array".to_string()))?;
            Ok(arr.value(row_idx).to_string())
        }

        DataType::Float32 => {
            let arr = array
                .as_any()
                .downcast_ref::<Float32Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Float32 array".to_string()))?;
            Ok(format_float(f64::from(arr.value(row_idx))))
        }

        DataType::Float64 => {
            let arr = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Float64 array".to_string()))?;
            Ok(format_float(arr.value(row_idx)))
        }

        DataType::Utf8 => {
            let arr = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Utf8 array".to_string()))?;
            Ok(escape_csv_string(arr.value(row_idx), delimiter))
        }

        DataType::LargeUtf8 => {
            let arr = array
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .ok_or_else(|| {
                    ImportError::ConversionError("Invalid LargeUtf8 array".to_string())
                })?;
            Ok(escape_csv_string(arr.value(row_idx), delimiter))
        }

        DataType::Binary => {
            let arr = array
                .as_any()
                .downcast_ref::<BinaryArray>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Binary array".to_string()))?;
            Ok(hex::encode(arr.value(row_idx)))
        }

        DataType::LargeBinary => {
            let arr = array
                .as_any()
                .downcast_ref::<LargeBinaryArray>()
                .ok_or_else(|| {
                    ImportError::ConversionError("Invalid LargeBinary array".to_string())
                })?;
            Ok(hex::encode(arr.value(row_idx)))
        }

        DataType::Date32 => {
            let arr = array
                .as_any()
                .downcast_ref::<Date32Array>()
                .ok_or_else(|| ImportError::ConversionError("Invalid Date32 array".to_string()))?;
            Ok(format_date32(arr.value(row_idx)))
        }

        DataType::Timestamp(unit, _) => format_timestamp(array, row_idx, unit),

        DataType::Decimal128(_, scale) => {
            let arr = array
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .ok_or_else(|| {
                    ImportError::ConversionError("Invalid Decimal128 array".to_string())
                })?;
            Ok(format_decimal128(arr.value(row_idx), *scale))
        }

        _ => Err(ImportError::ConversionError(format!(
            "Unsupported Arrow type: {:?}",
            data_type
        ))),
    };

    result
}

/// Formats a floating-point number for CSV output.
///
/// Uses standard notation for normal numbers and scientific notation for
/// very large or very small numbers.
fn format_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }

    // Use standard notation for most numbers
    let abs = value.abs();
    if abs == 0.0 || (0.0001..1_000_000_000_000.0).contains(&abs) {
        format!("{value}")
    } else {
        // Use scientific notation for very large or very small numbers
        format!("{value:e}")
    }
}

/// Escapes a string value for CSV output.
///
/// Quotes the string if it contains the delimiter, separator, or newlines.
/// Also escapes any existing delimiter characters by doubling them.
fn escape_csv_string(value: &str, delimiter: char) -> String {
    let needs_quoting = value.contains(delimiter)
        || value.contains(',')
        || value.contains('\n')
        || value.contains('\r');

    if needs_quoting {
        let escaped = value.replace(delimiter, &format!("{delimiter}{delimiter}"));
        format!("{delimiter}{escaped}{delimiter}")
    } else {
        value.to_string()
    }
}

/// Formats a Date32 value (days since epoch) as YYYY-MM-DD.
fn format_date32(days: i32) -> String {
    // Days since Unix epoch (1970-01-01)
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let date = epoch + chrono::Duration::days(i64::from(days));
    date.format("%Y-%m-%d").to_string()
}

/// Formats a timestamp value as YYYY-MM-DD HH:MM:SS.ffffff.
fn format_timestamp(
    array: &dyn Array,
    row_idx: usize,
    unit: &TimeUnit,
) -> Result<String, ImportError> {
    let timestamp_micros = match unit {
        TimeUnit::Second => {
            let arr = array
                .as_any()
                .downcast_ref::<TimestampSecondArray>()
                .ok_or_else(|| {
                    ImportError::ConversionError("Invalid TimestampSecond array".to_string())
                })?;
            arr.value(row_idx) * 1_000_000
        }
        TimeUnit::Millisecond => {
            let arr = array
                .as_any()
                .downcast_ref::<TimestampMillisecondArray>()
                .ok_or_else(|| {
                    ImportError::ConversionError("Invalid TimestampMillisecond array".to_string())
                })?;
            arr.value(row_idx) * 1_000
        }
        TimeUnit::Microsecond => {
            let arr = array
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .ok_or_else(|| {
                    ImportError::ConversionError("Invalid TimestampMicrosecond array".to_string())
                })?;
            arr.value(row_idx)
        }
        TimeUnit::Nanosecond => {
            let arr = array
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .ok_or_else(|| {
                    ImportError::ConversionError("Invalid TimestampNanosecond array".to_string())
                })?;
            arr.value(row_idx) / 1_000
        }
    };

    let secs = timestamp_micros / 1_000_000;
    let micros = (timestamp_micros % 1_000_000).unsigned_abs() as u32;

    let datetime = chrono::DateTime::from_timestamp(secs, micros * 1_000)
        .ok_or_else(|| ImportError::ConversionError("Invalid timestamp value".to_string()))?;

    Ok(datetime.format("%Y-%m-%d %H:%M:%S%.6f").to_string())
}

/// Formats a Decimal128 value with the given scale.
fn format_decimal128(value: i128, scale: i8) -> String {
    if scale <= 0 {
        // No decimal places needed
        let multiplier = 10i128.pow((-scale) as u32);
        return (value * multiplier).to_string();
    }

    let scale_u32 = scale as u32;
    let divisor = 10i128.pow(scale_u32);
    let integer_part = value / divisor;
    let fractional_part = (value % divisor).abs();

    format!(
        "{integer_part}.{fractional_part:0>width$}",
        width = scale_u32 as usize
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{ArrayRef, StringBuilder};
    use arrow::datatypes::{Field, Schema};
    use std::sync::Arc;

    fn create_test_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("active", DataType::Boolean, false),
        ]));

        let id_array: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
        let name_array: ArrayRef =
            Arc::new(StringArray::from(vec![Some("Alice"), Some("Bob"), None]));
        let active_array: ArrayRef = Arc::new(BooleanArray::from(vec![true, false, true]));

        RecordBatch::try_new(schema, vec![id_array, name_array, active_array]).unwrap()
    }

    #[test]
    fn test_record_batch_to_csv_basic() {
        let batch = create_test_batch();
        let options = ParquetImportOptions::default();

        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], "1,Alice,true");
        assert_eq!(rows[1], "2,Bob,false");
        assert_eq!(rows[2], "3,,true"); // NULL name
    }

    #[test]
    fn test_record_batch_to_csv_with_null_value() {
        let batch = create_test_batch();
        let options = ParquetImportOptions::default().with_null_value("NULL");

        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows[2], "3,NULL,true");
    }

    #[test]
    fn test_record_batch_to_csv_custom_separator() {
        let batch = create_test_batch();
        let options = ParquetImportOptions::default().with_column_separator(';');

        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows[0], "1;Alice;true");
    }

    #[test]
    fn test_escape_csv_string_no_special_chars() {
        let result = escape_csv_string("hello", '"');
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_escape_csv_string_with_comma() {
        let result = escape_csv_string("hello,world", '"');
        assert_eq!(result, "\"hello,world\"");
    }

    #[test]
    fn test_escape_csv_string_with_delimiter() {
        let result = escape_csv_string("say \"hello\"", '"');
        assert_eq!(result, "\"say \"\"hello\"\"\"");
    }

    #[test]
    fn test_escape_csv_string_with_newline() {
        let result = escape_csv_string("line1\nline2", '"');
        assert_eq!(result, "\"line1\nline2\"");
    }

    #[test]
    fn test_format_float_normal() {
        // Use slightly different values to avoid clippy::approx_constant
        assert_eq!(format_float(3.14158), "3.14158");
        assert_eq!(format_float(-123.456), "-123.456");
        assert_eq!(format_float(0.0), "0");
    }

    #[test]
    fn test_format_float_scientific() {
        let result = format_float(1e15);
        assert!(result.contains('e') || result == "1000000000000000");

        let result = format_float(1e-6);
        assert!(result.contains('e') || result == "0.000001");
    }

    #[test]
    fn test_format_float_special() {
        assert_eq!(format_float(f64::NAN), "NaN");
        assert_eq!(format_float(f64::INFINITY), "Infinity");
        assert_eq!(format_float(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn test_format_date32() {
        // 2024-01-15 is day 19737 since epoch
        let days = chrono::NaiveDate::from_ymd_opt(2024, 1, 15)
            .unwrap()
            .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32;

        let result = format_date32(days);
        assert_eq!(result, "2024-01-15");
    }

    #[test]
    fn test_format_decimal128() {
        // 12345 with scale 2 should be 123.45
        assert_eq!(format_decimal128(12345, 2), "123.45");

        // 100 with scale 0 should be 100
        assert_eq!(format_decimal128(100, 0), "100");

        // Negative scale multiplies
        assert_eq!(format_decimal128(5, -2), "500");

        // Fractional with leading zeros
        assert_eq!(format_decimal128(1001, 3), "1.001");
    }

    #[test]
    fn test_parquet_import_options_builder() {
        let options = ParquetImportOptions::new()
            .with_schema("my_schema")
            .with_columns(vec!["col1".to_string(), "col2".to_string()])
            .with_batch_size(2048)
            .with_null_value("\\N")
            .with_column_separator(';')
            .with_column_delimiter('\'')
            .use_tls(true)
            .with_exasol_host("exasol.example.com")
            .with_exasol_port(8563);

        assert_eq!(options.schema, Some("my_schema".to_string()));
        assert_eq!(
            options.columns,
            Some(vec!["col1".to_string(), "col2".to_string()])
        );
        assert_eq!(options.batch_size, 2048);
        assert_eq!(options.null_value, "\\N");
        assert_eq!(options.column_separator, ';');
        assert_eq!(options.column_delimiter, '\'');
        assert!(options.use_tls);
        assert_eq!(options.host, "exasol.example.com");
        assert_eq!(options.port, 8563);
    }

    #[test]
    fn test_parquet_import_options_use_tls_builder() {
        assert!(ParquetImportOptions::default().use_tls(true).use_tls);
        assert!(!ParquetImportOptions::default().use_tls(false).use_tls);
    }

    #[test]
    fn test_parquet_import_options_native_override_default_none() {
        let options = ParquetImportOptions::default();
        assert_eq!(options.native_parquet_override, None);
    }

    #[test]
    fn test_parquet_import_options_with_native_parquet_some_true() {
        let options = ParquetImportOptions::default().with_native_parquet(Some(true));
        assert_eq!(options.native_parquet_override, Some(true));
    }

    #[test]
    fn test_parquet_import_options_with_native_parquet_some_false() {
        let options = ParquetImportOptions::default().with_native_parquet(Some(false));
        assert_eq!(options.native_parquet_override, Some(false));
    }

    #[test]
    fn test_parquet_import_options_with_native_parquet_none_resets() {
        // First force a value, then reset back to None.
        let options = ParquetImportOptions::default()
            .with_native_parquet(Some(true))
            .with_native_parquet(None);
        assert_eq!(options.native_parquet_override, None);
    }

    #[test]
    fn test_record_batch_empty() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));
        let batch = RecordBatch::new_empty(schema);
        let options = ParquetImportOptions::default();

        let rows = record_batch_to_csv(&batch, &options).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_integer_types_conversion() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("i8", DataType::Int8, false),
            Field::new("i16", DataType::Int16, false),
            Field::new("i32", DataType::Int32, false),
            Field::new("i64", DataType::Int64, false),
            Field::new("u8", DataType::UInt8, false),
            Field::new("u16", DataType::UInt16, false),
            Field::new("u32", DataType::UInt32, false),
            Field::new("u64", DataType::UInt64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int8Array::from(vec![-128i8])) as ArrayRef,
                Arc::new(Int16Array::from(vec![-32768i16])) as ArrayRef,
                Arc::new(Int32Array::from(vec![-2147483648i32])) as ArrayRef,
                Arc::new(Int64Array::from(vec![-9223372036854775808i64])) as ArrayRef,
                Arc::new(UInt8Array::from(vec![255u8])) as ArrayRef,
                Arc::new(UInt16Array::from(vec![65535u16])) as ArrayRef,
                Arc::new(UInt32Array::from(vec![4294967295u32])) as ArrayRef,
                Arc::new(UInt64Array::from(vec![18446744073709551615u64])) as ArrayRef,
            ],
        )
        .unwrap();

        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(
            rows[0],
            "-128,-32768,-2147483648,-9223372036854775808,255,65535,4294967295,18446744073709551615"
        );
    }

    #[test]
    fn test_float_types_conversion() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("f32", DataType::Float32, false),
            Field::new("f64", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                // Use slightly different values to avoid clippy::approx_constant
                Arc::new(Float32Array::from(vec![3.14158f32])) as ArrayRef,
                Arc::new(Float64Array::from(vec![2.71f64])) as ArrayRef,
            ],
        )
        .unwrap();

        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        // Float32 loses precision (using non-standard float values to avoid clippy::approx_constant)
        assert!(rows[0].starts_with("3.14158"));
        assert!(rows[0].contains("2.71"));
    }

    #[test]
    fn test_binary_type_conversion() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "data",
            DataType::Binary,
            false,
        )]));

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![&[0xDE, 0xAD, 0xBE, 0xEF][..]])) as ArrayRef],
        )
        .unwrap();

        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows[0], "deadbeef");
    }

    #[test]
    fn test_date32_type_conversion() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "date",
            DataType::Date32,
            false,
        )]));

        // 2024-01-15 is day 19737 since epoch
        let days = chrono::NaiveDate::from_ymd_opt(2024, 1, 15)
            .unwrap()
            .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32;

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Date32Array::from(vec![days])) as ArrayRef],
        )
        .unwrap();

        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows[0], "2024-01-15");
    }

    #[test]
    fn test_timestamp_type_conversion() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        )]));

        // 2024-01-15 12:40:45.123456 UTC (1705322445.123456 seconds since epoch)
        let timestamp_micros = 1705322445123456i64;

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(TimestampMicrosecondArray::from(vec![timestamp_micros])) as ArrayRef],
        )
        .unwrap();

        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert!(rows[0].starts_with("2024-01-15"));
        assert!(
            rows[0].contains("12:40:45"),
            "Expected 12:40:45 but got: {}",
            rows[0]
        );
        assert!(rows[0].contains(".123456"));
    }

    #[test]
    fn test_decimal128_type_conversion() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            DataType::Decimal128(10, 2),
            false,
        )]));

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(
                Decimal128Array::from(vec![12345i128])
                    .with_precision_and_scale(10, 2)
                    .unwrap(),
            ) as ArrayRef],
        )
        .unwrap();

        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows[0], "123.45");
    }

    #[test]
    fn test_null_handling_across_types() {
        // Test NULL handling for various Arrow types
        let schema = Arc::new(Schema::new(vec![
            Field::new("int_col", DataType::Int32, true),
            Field::new("float_col", DataType::Float64, true),
            Field::new("string_col", DataType::Utf8, true),
            Field::new("bool_col", DataType::Boolean, true),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![Some(1), None, Some(3)])) as ArrayRef,
                Arc::new(Float64Array::from(vec![Some(1.0), Some(2.0), None])) as ArrayRef,
                Arc::new(StringArray::from(vec![None, Some("b"), Some("c")])) as ArrayRef,
                Arc::new(BooleanArray::from(vec![Some(true), None, Some(false)])) as ArrayRef,
            ],
        )
        .unwrap();

        // Test with default empty NULL value
        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], "1,1,,true"); // string NULL -> empty
        assert_eq!(rows[1], ",2,b,"); // int and bool NULL -> empty
        assert_eq!(rows[2], "3,,c,false"); // float NULL -> empty
    }

    #[test]
    fn test_null_handling_with_custom_null_marker() {
        // Test NULL handling with custom NULL marker like "\N" used by Exasol
        let schema = Arc::new(Schema::new(vec![
            Field::new("int_col", DataType::Int32, true),
            Field::new("string_col", DataType::Utf8, true),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![Some(1), None])) as ArrayRef,
                Arc::new(StringArray::from(vec![None, Some("text")])) as ArrayRef,
            ],
        )
        .unwrap();

        let options = ParquetImportOptions::default().with_null_value("\\N");
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], "1,\\N");
        assert_eq!(rows[1], "\\N,text");
    }

    #[test]
    fn test_all_null_row() {
        // Test a row where all values are NULL
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int32, true),
            Field::new("b", DataType::Utf8, true),
            Field::new("c", DataType::Float64, true),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![None as Option<i32>])) as ArrayRef,
                Arc::new(StringArray::from(vec![None as Option<&str>])) as ArrayRef,
                Arc::new(Float64Array::from(vec![None as Option<f64>])) as ArrayRef,
            ],
        )
        .unwrap();

        let options = ParquetImportOptions::default().with_null_value("NULL");
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], "NULL,NULL,NULL");
    }

    #[test]
    fn test_string_with_special_characters() {
        let schema = Arc::new(Schema::new(vec![Field::new("text", DataType::Utf8, false)]));

        let mut builder = StringBuilder::new();
        builder.append_value("hello, world");
        builder.append_value("say \"hi\"");
        builder.append_value("line1\nline2");

        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(builder.finish()) as ArrayRef]).unwrap();

        let options = ParquetImportOptions::default();
        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows[0], "\"hello, world\"");
        assert_eq!(rows[1], "\"say \"\"hi\"\"\"");
        assert_eq!(rows[2], "\"line1\nline2\"");
    }

    /// Helper to create a temporary Parquet file for DDL error handling tests.
    fn create_test_parquet_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        use parquet::arrow::ArrowWriter;
        use parquet::file::properties::WriterProperties;

        let path = dir.join(name);
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        let id_array = Int32Array::from(vec![1, 2]);
        let name_array = StringArray::from(vec![Some("a"), Some("b")]);
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(id_array), Arc::new(name_array)],
        )
        .expect("Failed to create RecordBatch");

        let file = std::fs::File::create(&path).expect("Failed to create file");
        let props = WriterProperties::builder().build();
        let mut writer =
            ArrowWriter::try_new(file, schema, Some(props)).expect("Failed to create writer");
        writer.write(&batch).expect("Failed to write batch");
        writer.close().expect("Failed to close writer");

        path
    }

    #[tokio::test]
    async fn test_import_from_parquet_propagates_ddl_error() {
        let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
        let parquet_path = create_test_parquet_file(temp_dir.path(), "test.parquet");

        let options = ParquetImportOptions::default()
            .with_create_table_if_not_exists(true)
            .with_exasol_host("localhost")
            .with_exasol_port(1);

        // The closure returns an error that does NOT contain "already exists"
        let execute_sql = |_sql: String| async {
            Err::<u64, String>("schema NONEXISTENT_SCHEMA does not exist".to_string())
        };

        let result =
            import_from_parquet(execute_sql, "test_table", &parquet_path, options, false).await;

        assert!(result.is_err(), "Should return an error");
        let err = result.unwrap_err();
        let err_msg = format!("{}", err);
        assert!(
            err_msg.contains("SQL execution failed"),
            "Error should be SqlError, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("NONEXISTENT_SCHEMA"),
            "Error should contain original message, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_import_from_parquet_ignores_already_exists_error() {
        let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
        let parquet_path = create_test_parquet_file(temp_dir.path(), "test.parquet");

        let options = ParquetImportOptions::default()
            .with_create_table_if_not_exists(true)
            .with_exasol_host("localhost")
            .with_exasol_port(1);

        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let execute_sql = {
            let call_count = call_count.clone();
            move |_sql: String| {
                let count = call_count.clone();
                async move {
                    let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if n == 1 {
                        Err("object TEST_TABLE already exists".to_string())
                    } else {
                        Ok(0u64)
                    }
                }
            }
        };

        let result =
            import_from_parquet(execute_sql, "test_table", &parquet_path, options, false).await;

        // The function should NOT have returned SqlError (the DDL error was ignored).
        // It will fail at HTTP transport since there's no real Exasol, but that's fine.
        match &result {
            Err(ImportError::SqlError(_)) => {
                panic!("DDL 'already exists' error should have been ignored, but got SqlError");
            }
            _ => {
                // Any other result is acceptable: either Ok (unlikely without real Exasol)
                // or a transport/connection error (expected since no real Exasol is running)
            }
        }
    }

    #[tokio::test]
    async fn test_import_from_parquet_files_propagates_ddl_error() {
        let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
        // Create at least 2 files to exercise the multi-file path (not single-file delegation)
        let parquet_path1 = create_test_parquet_file(temp_dir.path(), "test1.parquet");
        let parquet_path2 = create_test_parquet_file(temp_dir.path(), "test2.parquet");

        let options = ParquetImportOptions::default()
            .with_create_table_if_not_exists(true)
            .with_exasol_host("localhost")
            .with_exasol_port(1);

        // The closure returns an error that does NOT contain "already exists"
        let execute_sql = |_sql: String| async {
            Err::<u64, String>("schema NONEXISTENT_SCHEMA does not exist".to_string())
        };

        let result = import_from_parquet_files(
            execute_sql,
            "test_table",
            vec![parquet_path1, parquet_path2],
            options,
            false,
        )
        .await;

        assert!(result.is_err(), "Should return an error");
        let err = result.unwrap_err();
        let err_msg = format!("{}", err);
        assert!(
            err_msg.contains("SQL execution failed"),
            "Error should be SqlError, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("NONEXISTENT_SCHEMA"),
            "Error should contain original message, got: {}",
            err_msg
        );
    }

    fn multi_file_entries() -> Vec<ImportFileEntry> {
        vec![
            ImportFileEntry::new("10.0.0.1:8000".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new(
                "10.0.0.2:8000".to_string(),
                "002.csv".to_string(),
                Some("ab:cd".to_string()),
            ),
        ]
    }

    #[test]
    fn test_build_multi_file_parquet_query_emits_csv_clauses_for_every_entry() {
        let options = ParquetImportOptions::default();

        let sql = build_multi_file_parquet_query("target", &options, multi_file_entries()).build();

        assert!(sql.starts_with("IMPORT INTO target"), "got: {sql}");
        assert!(sql.contains("FROM CSV"), "got: {sql}");
        assert!(
            sql.contains("AT 'http://10.0.0.1:8000' FILE '001.csv'"),
            "got: {sql}"
        );
        assert!(
            sql.contains("AT 'https://10.0.0.2:8000' PUBLIC KEY 'ab:cd' FILE '002.csv'"),
            "got: {sql}"
        );
        assert!(sql.contains("ENCODING = 'UTF-8'"), "got: {sql}");
        assert!(sql.contains("COLUMN SEPARATOR = ','"), "got: {sql}");
        assert!(sql.contains("COLUMN DELIMITER = '\"'"), "got: {sql}");
        assert!(sql.contains("ROW SEPARATOR = 'LF'"), "got: {sql}");
        assert!(
            !sql.contains("NULL = "),
            "empty null_value must be omitted: {sql}"
        );
    }

    #[test]
    fn test_build_multi_file_parquet_query_applies_schema_columns_and_null_value() {
        let options = ParquetImportOptions::default()
            .with_schema("staging")
            .with_columns(vec!["a".to_string(), "b".to_string()])
            .with_null_value("\\N")
            .with_column_separator(';')
            .with_column_delimiter('\'');

        let sql = build_multi_file_parquet_query("target", &options, multi_file_entries()).build();

        assert!(
            sql.starts_with("IMPORT INTO staging.target (a, b)"),
            "got: {sql}"
        );
        assert!(sql.contains("COLUMN SEPARATOR = ';'"), "got: {sql}");
        assert!(sql.contains("COLUMN DELIMITER = '''"), "got: {sql}");
        assert!(sql.contains("NULL = '\\N'"), "got: {sql}");
    }

    #[test]
    fn test_build_multi_file_parquet_native_query_omits_all_csv_clauses() {
        let options = ParquetImportOptions::default()
            .with_schema("staging")
            .with_columns(vec!["a".to_string()])
            .with_null_value("\\N")
            .with_column_separator(';');

        let sql =
            build_multi_file_parquet_native_query("target", &options, multi_file_entries()).build();

        assert!(
            sql.starts_with("IMPORT INTO staging.target (a)"),
            "got: {sql}"
        );
        assert!(sql.contains("FROM PARQUET"), "got: {sql}");
        assert!(
            sql.contains("AT 'http://10.0.0.1:8000;MaxConcurrentReads=1' FILE '001.parquet'"),
            "got: {sql}"
        );
        assert!(sql.contains("FILE '002.parquet'"), "got: {sql}");
        assert!(!sql.contains("ENCODING"), "got: {sql}");
        assert!(!sql.contains("COLUMN SEPARATOR"), "got: {sql}");
        assert!(!sql.contains("ROW SEPARATOR"), "got: {sql}");
        assert!(!sql.contains("NULL = "), "got: {sql}");
    }

    #[test]
    fn test_format_arrow_value_large_utf8_is_escaped() {
        let array = LargeStringArray::from(vec![Some("a,b"), Some("plain")]);

        assert_eq!(
            format_arrow_value(&array, 0, '"', "").unwrap(),
            "\"a,b\"".to_string()
        );
        assert_eq!(
            format_arrow_value(&array, 1, '"', "").unwrap(),
            "plain".to_string()
        );
    }

    #[test]
    fn test_format_arrow_value_large_binary_is_hex_encoded() {
        let array = LargeBinaryArray::from(vec![Some(&[0xDEu8, 0xADu8][..])]);

        assert_eq!(format_arrow_value(&array, 0, '"', "").unwrap(), "dead");
    }

    #[test]
    fn test_format_arrow_value_rejects_unsupported_type() {
        let array = arrow::array::Time32SecondArray::from(vec![1]);

        let err = format_arrow_value(&array, 0, '"', "").unwrap_err();

        assert!(
            err.to_string().contains("Unsupported Arrow type"),
            "got: {err}"
        );
        assert!(err.to_string().contains("Time32"), "got: {err}");
    }

    #[test]
    fn test_format_arrow_value_null_uses_null_marker() {
        let array = LargeStringArray::from(vec![None::<&str>]);

        assert_eq!(format_arrow_value(&array, 0, '"', "\\N").unwrap(), "\\N");
    }

    #[test]
    fn test_format_timestamp_second_unit() {
        let array = TimestampSecondArray::from(vec![86_400 + 3_661]);

        let formatted = format_timestamp(&array, 0, &TimeUnit::Second).unwrap();

        assert_eq!(formatted, "1970-01-02 01:01:01.000000");
    }

    #[test]
    fn test_format_timestamp_millisecond_unit() {
        let array = TimestampMillisecondArray::from(vec![1_500]);

        let formatted = format_timestamp(&array, 0, &TimeUnit::Millisecond).unwrap();

        assert_eq!(formatted, "1970-01-01 00:00:01.500000");
    }

    #[test]
    fn test_format_timestamp_microsecond_unit() {
        let array = TimestampMicrosecondArray::from(vec![1_000_123]);

        let formatted = format_timestamp(&array, 0, &TimeUnit::Microsecond).unwrap();

        assert_eq!(formatted, "1970-01-01 00:00:01.000123");
    }

    fn create_empty_test_parquet_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        use arrow::datatypes::{Field, Schema};
        use parquet::arrow::ArrowWriter;

        let path = dir.join(name);
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));
        let file = std::fs::File::create(&path).expect("create file");
        let writer = ArrowWriter::try_new(file, schema, None).expect("writer");
        writer.close().expect("close");

        path
    }

    #[tokio::test]
    async fn test_import_from_parquet_stream_returns_zero_for_empty_input() {
        let options = ParquetImportOptions::default();
        let execute_sql = |_sql: String| async { Ok::<u64, String>(1) };

        let rows = import_from_parquet_stream(execute_sql, "t", &b""[..], options, false)
            .await
            .expect("empty stream is not an error");

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn test_import_from_parquet_returns_zero_for_a_row_less_file() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = create_empty_test_parquet_file(temp_dir.path(), "empty.parquet");
        let options = ParquetImportOptions::default();
        let execute_sql = |_sql: String| async { Ok::<u64, String>(1) };

        let rows = import_from_parquet(execute_sql, "t", &path, options, false)
            .await
            .expect("row-less file is not an error");

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn test_import_from_parquet_stream_returns_zero_for_a_row_less_file() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = create_empty_test_parquet_file(temp_dir.path(), "empty.parquet");
        let bytes = std::fs::read(&path).expect("read file");
        let options = ParquetImportOptions::default();
        let execute_sql = |_sql: String| async { Ok::<u64, String>(1) };

        let rows = import_from_parquet_stream(execute_sql, "t", &bytes[..], options, false)
            .await
            .expect("row-less file is not an error");

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn test_import_from_parquet_files_rejects_an_empty_file_list() {
        let options = ParquetImportOptions::default();
        let execute_sql = |_sql: String| async { Ok::<u64, String>(1) };

        let err = import_from_parquet_files(
            execute_sql,
            "t",
            Vec::<std::path::PathBuf>::new(),
            options,
            false,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::InvalidConfig(_)), "got: {err}");
        assert!(
            err.to_string().contains("No files provided for import"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_import_from_parquet_files_delegates_a_single_path_to_the_single_file_import() {
        let options = ParquetImportOptions::default();
        let execute_sql = |_sql: String| async { Ok::<u64, String>(1) };

        let err = import_from_parquet_files(
            execute_sql,
            "t",
            vec![std::path::PathBuf::from("/nonexistent/dir/missing.parquet")],
            options,
            false,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::IoError(_)), "got: {err}");
    }

    #[test]
    fn test_with_column_name_mode_selects_the_ddl_naming_strategy() {
        let sanitized =
            ParquetImportOptions::default().with_column_name_mode(ColumnNameMode::Sanitize);
        assert_eq!(sanitized.column_name_mode, ColumnNameMode::Sanitize);

        let quoted = sanitized.with_column_name_mode(ColumnNameMode::Quoted);
        assert_eq!(quoted.column_name_mode, ColumnNameMode::Quoted);
    }

    #[tokio::test]
    async fn test_import_from_parquet_files_returns_zero_when_every_file_is_row_less() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let first = create_empty_test_parquet_file(temp_dir.path(), "first.parquet");
        let second = create_empty_test_parquet_file(temp_dir.path(), "second.parquet");
        let options = ParquetImportOptions::default();
        let execute_sql = |_sql: String| async { Ok::<u64, String>(1) };

        let rows = import_from_parquet_files(execute_sql, "t", vec![first, second], options, false)
            .await
            .expect("row-less files are not an error");

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn test_import_from_parquet_reports_an_unreadable_file() {
        let options = ParquetImportOptions::default();
        let execute_sql = |_sql: String| async { Ok::<u64, String>(1) };

        let err = import_from_parquet(
            execute_sql,
            "t",
            std::path::Path::new("/nonexistent/dir/missing.parquet"),
            options,
            false,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::IoError(_)), "got: {err}");
    }

    #[test]
    fn test_parquet_to_csv_bytes_renders_lf_terminated_rows() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = create_test_parquet_file(temp_dir.path(), "rows.parquet");
        let file = std::fs::File::open(&path).expect("open file");

        let csv = parquet_to_csv_bytes(file, &ParquetImportOptions::default()).expect("conversion");

        assert_eq!(String::from_utf8(csv).unwrap(), "1,a\n2,b\n");
    }

    #[test]
    fn test_parquet_to_csv_bytes_rejects_non_parquet_content() {
        let err = parquet_to_csv_bytes(
            Bytes::from_static(b"definitely not parquet"),
            &ParquetImportOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(err, ImportError::ParquetError(_)), "got: {err}");
    }

    #[tokio::test]
    async fn test_create_table_from_ddl_reports_non_idempotent_failures() {
        let err = create_table_from_ddl(
            |_sql: String| async { Err::<u64, String>("insufficient privileges".to_string()) },
            "CREATE TABLE t (a INT)".to_string(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::SqlError(_)), "got: {err}");
    }

    #[tokio::test]
    async fn test_create_table_from_ddl_tolerates_an_existing_table() {
        let already_exists = create_table_from_ddl(
            |_sql: String| async { Err::<u64, String>("object T already exists".to_string()) },
            "CREATE TABLE t (a INT)".to_string(),
        )
        .await;
        assert!(already_exists.is_ok());

        let created = create_table_from_ddl(
            |_sql: String| async { Ok::<u64, String>(0) },
            "CREATE TABLE t (a INT)".to_string(),
        )
        .await;
        assert!(created.is_ok());
    }

    #[test]
    fn test_build_single_file_csv_query_emits_address_and_format_options() {
        let options = ParquetImportOptions::default()
            .with_schema("staging")
            .with_null_value("\\N");

        let sql = build_single_file_csv_query("t", &options, "10.0.0.1:8000").build();

        assert!(sql.starts_with("IMPORT INTO staging.t"), "got: {sql}");
        assert!(
            sql.contains("FROM CSV AT 'http://10.0.0.1:8000'"),
            "got: {sql}"
        );
        assert!(sql.contains("FILE '001.csv'"), "got: {sql}");
        assert!(sql.contains("NULL = '\\N'"), "got: {sql}");
    }

    #[test]
    fn test_with_public_key_switches_the_url_scheme() {
        let options = ParquetImportOptions::default();
        let query = build_single_file_csv_query("t", &options, "10.0.0.1:8000");

        let without = with_public_key(query.clone(), None).build();
        let with = with_public_key(query, Some("ab:cd")).build();

        assert!(
            without.contains("AT 'http://10.0.0.1:8000'"),
            "got: {without}"
        );
        assert!(!without.contains("PUBLIC KEY"), "got: {without}");
        assert!(with.contains("AT 'https://10.0.0.1:8000'"), "got: {with}");
        assert!(with.contains("PUBLIC KEY 'ab:cd'"), "got: {with}");
    }

    #[test]
    fn test_format_timestamp_nanosecond_unit_truncates_to_micros() {
        let array = TimestampNanosecondArray::from(vec![1_000_123_999]);

        let formatted = format_timestamp(&array, 0, &TimeUnit::Nanosecond).unwrap();

        assert_eq!(formatted, "1970-01-01 00:00:01.000123");
    }
}
