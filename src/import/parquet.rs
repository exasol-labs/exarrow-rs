//! Parquet file import functionality for Exasol.
//!
//! This module provides functions to import Parquet files into Exasol tables.
//! Since Exasol only accepts CSV format over HTTP, Parquet data is converted
//! to CSV on-the-fly during the import process.
//!
//! # Example
//!

use std::future::Future;
use std::path::{Path, PathBuf};

use arrow::datatypes::{DataType, Schema};
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::ARROW_SCHEMA_META_KEY;
use parquet::file::reader::ChunkReader;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek};

use crate::query::import::{ImportFileEntry, ImportFormat, ImportQuery, RowSeparator};
use crate::transport::HttpTransportClient;
use crate::types::{infer_schema_from_parquet, infer_schema_from_parquet_files, ColumnNameMode};

use super::arrow::{check_schema_converts, format_row, CsvWriterOptions};
use super::parallel::{
    finish_import, resolve_stream_task, stream_in_parallel, stream_parquet_files_parallel,
    ParallelTransportPool, CHUNK_SIZE as CHUNKED_TRANSFER_SIZE,
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
    /// Whether to encrypt the HTTP transport tunnel with TLS (default: `false`).
    /// `true` encrypts the tunnel, including against Exasol Docker. The control
    /// connection's TLS does not cover the tunnel data.
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
/// On the native Parquet path (`use_native`), the function opens the file and
/// reads each range that Exasol requests in pieces of at most 64 KiB, so
/// memory does not grow with the file size. The file must stay unchanged until
/// the import ends, and a file that becomes shorter fails the import.
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
        if use_native {
            check_native_auto_create_footer(file_path)?;
        } else {
            check_csv_auto_create_footer(file_path)?;
        }
        let ddl = inferred_schema.to_ddl(table, options.schema.as_deref());
        create_table_from_ddl(&mut execute_sql, ddl).await?;
    }

    if use_native {
        let file = open_parquet_file(file_path).await?;
        return serve_parquet_file(execute_sql, table, &options, file).await;
    }

    let source = open_parquet_csv_reader(
        open_parquet_file(file_path).await?.into_std().await,
        &options,
    )?;
    if source.row_count == 0 {
        return Ok(0);
    }

    serve_parquet_csv(execute_sql, table, &options, source.reader).await
}

async fn open_parquet_file(file_path: &Path) -> Result<tokio::fs::File, ImportError> {
    tokio::fs::File::open(file_path).await.map_err(|e| {
        ImportError::IoError(std::io::Error::new(
            e.kind(),
            format!("failed to open Parquet file '{}': {e}", file_path.display()),
        ))
    })
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

/// Rejects a file that native Parquet import cannot create columns for, before auto-create
/// sends `CREATE TABLE`, so the rejected file leaves no table behind.
///
/// The native path accepts only the fields of [`native_auto_create_accepts`].
fn check_native_auto_create_footer(path: &Path) -> Result<(), ImportError> {
    let footer = read_footer(path)?;
    check_native_auto_create_types(footer.schema()).map_err(|reason| rejected_footer(path, reason))
}

/// Rejects a file that the CSV import path would convert wrongly, before auto-create sends
/// `CREATE TABLE`, so the rejected file leaves no table behind.
///
/// The CSV path applies [`check_intervals_keep_month_part`].
fn check_csv_auto_create_footer(path: &Path) -> Result<(), ImportError> {
    let footer = read_footer(path)?;
    check_intervals_keep_month_part(&footer).map_err(|reason| rejected_footer(path, reason))
}

fn rejected_footer(path: &Path, reason: String) -> ImportError {
    ImportError::SchemaInferenceError(format!("Parquet file '{}': {reason}", path.display()))
}

fn read_footer(path: &Path) -> Result<ArrowReaderMetadata, ImportError> {
    let unreadable = |e: &dyn std::fmt::Display| {
        ImportError::SchemaInferenceError(format!(
            "Failed to read the Parquet footer of '{}': {e}",
            path.display()
        ))
    };
    let file = std::fs::File::open(path).map_err(|e| unreadable(&e))?;
    ArrowReaderMetadata::load(&file, Default::default()).map_err(|e| unreadable(&e))
}

/// The Arrow types for which native Parquet import with table creation creates a column.
///
/// Nothing verifies what Exasol's Parquet reader loads for any other type, so a type that
/// `TypeMapper::arrow_to_exasol` maps later stays rejected here until it is added.
fn native_auto_create_accepts(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Boolean
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::Date32
            | DataType::Date64
            | DataType::Timestamp(..)
            | DataType::Decimal128(..)
            | DataType::Decimal256(..)
    )
}

fn check_native_auto_create_types(schema: &Schema) -> Result<(), String> {
    let rejected = schema
        .fields()
        .iter()
        .find(|field| !native_auto_create_accepts(field.data_type()));
    match rejected {
        Some(field) => Err(format!(
            "column '{}' has the Arrow type {:?}, for which native Parquet import cannot \
             create a column; import the file with with_native_parquet(Some(false))",
            field.name(),
            field.data_type()
        )),
        None => Ok(()),
    }
}

/// Rejects an Interval field of a file whose footer holds no Arrow schema.
///
/// The Parquet reader then reads a Parquet INTERVAL column as Interval(DayTime) and drops
/// its month part, so converting the column would store wrong values without an error.
fn check_intervals_keep_month_part(footer: &ArrowReaderMetadata) -> Result<(), String> {
    let has_arrow_schema = footer
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.key == ARROW_SCHEMA_META_KEY)
        });
    if has_arrow_schema {
        return Ok(());
    }
    let interval = footer
        .schema()
        .fields()
        .iter()
        .find(|field| matches!(field.data_type(), DataType::Interval(_)));
    match interval {
        Some(field) => Err(format!(
            "column '{}' holds Parquet INTERVAL values in a file without an Arrow schema, \
             which the Parquet reader reads as {:?}, so the column would lose its month part; \
             write the file with its Arrow schema to import it",
            field.name(),
            field.data_type()
        )),
        None => Ok(()),
    }
}

/// A Parquet source opened for CSV conversion.
pub(crate) struct ParquetCsvSource {
    /// The row count that the footer reports, known before any row group is decoded.
    pub(crate) row_count: u64,
    /// Decodes the row groups lazily, one RecordBatch of `batch_size` rows at a time.
    pub(crate) reader: ParquetRecordBatchReader,
}

/// Reads the footer of `source` and builds a reader at `options.batch_size`.
///
/// Only the footer is read here, so a caller can reject a broken file or an Interval column
/// that would lose its month part, and skip a row-less file, before it connects or runs any SQL.
pub(crate) fn open_parquet_csv_reader<R>(
    source: R,
    options: &ParquetImportOptions,
) -> Result<ParquetCsvSource, ImportError>
where
    R: ChunkReader + 'static,
{
    let footer = ArrowReaderMetadata::load(&source, Default::default())?;
    check_intervals_keep_month_part(&footer).map_err(ImportError::SchemaInferenceError)?;
    let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(source, footer);
    let footer_rows = builder.metadata().file_metadata().num_rows();
    let row_count = u64::try_from(footer_rows).map_err(|_| {
        ImportError::ParquetError(format!(
            "the Parquet footer reports a negative row count: {footer_rows}"
        ))
    })?;
    let reader = builder.with_batch_size(options.batch_size).build()?;
    Ok(ParquetCsvSource { row_count, reader })
}

/// Opens the Parquet file at `path` for CSV conversion, with errors that name
/// the file, as a multi-file import reports them.
pub(crate) fn open_parquet_csv_file(
    path: &Path,
    options: &ParquetImportOptions,
) -> Result<ParquetCsvSource, ImportError> {
    let file = std::fs::File::open(path).map_err(|e| {
        ImportError::ParallelImportError(format!(
            "Failed to open Parquet file {}: {e}",
            path.display()
        ))
    })?;
    open_parquet_csv_reader(file, options).map_err(|e| {
        ImportError::ParallelImportError(format!(
            "Failed to read Parquet file {}: {e}",
            path.display()
        ))
    })
}

/// Renders each RecordBatch of `reader` as one buffer of LF-terminated CSV
/// rows and hands it to `emit` before it decodes the next batch.
///
/// Memory therefore stays at one batch of CSV here, whatever the file size.
/// Rendering stops at the first error from the reader, the conversion, or `emit`.
pub(crate) fn render_parquet_csv<E>(
    reader: ParquetRecordBatchReader,
    options: &ParquetImportOptions,
    mut emit: E,
) -> Result<(), ImportError>
where
    E: FnMut(Vec<u8>) -> Result<(), ImportError>,
{
    for batch in reader {
        let mut buffer = Vec::new();
        for row in record_batch_to_csv(&batch?, options)? {
            buffer.extend_from_slice(row.as_bytes());
            buffer.push(b'\n');
        }
        emit(buffer)?;
    }
    Ok(())
}

/// How many rendered RecordBatches may wait between a CSV producer and its tunnel.
const RENDERED_BATCH_QUEUE: usize = 2;

/// Answers the IMPORT request on `client` and streams `reader` to it as CSV.
///
/// A blocking producer renders one RecordBatch at a time into a bounded queue,
/// so memory stays at a few batches per file and decoding stays off the async
/// workers. The body ends with the terminating chunk only after the whole file
/// converted: after a conversion error or a panic the connection closes without
/// it, so Exasol fails the IMPORT instead of loading part of the file. When this
/// future fails or is dropped, the closed queue stops the producer at its next batch.
pub(crate) async fn stream_parquet_csv(
    mut client: HttpTransportClient,
    reader: ParquetRecordBatchReader,
    options: ParquetImportOptions,
) -> Result<(), ImportError> {
    client.handle_import_request().await.map_err(|e| {
        ImportError::HttpTransportError(format!("Failed to handle import request: {e}"))
    })?;

    let (sender, mut receiver) = tokio::sync::mpsc::channel(RENDERED_BATCH_QUEUE);
    let producer = tokio::task::spawn_blocking(move || {
        render_parquet_csv(reader, &options, |buffer| {
            sender.blocking_send(buffer).map_err(|_| {
                ImportError::ChannelError("the HTTP tunnel stopped taking CSV".to_string())
            })
        })
    });

    while let Some(buffer) = receiver.recv().await {
        for piece in buffer.chunks(CHUNKED_TRANSFER_SIZE) {
            client
                .write_chunked_body(piece)
                .await
                .map_err(ImportError::TransportError)?;
        }
    }

    resolve_stream_task(producer.await)?;
    client
        .write_final_chunk()
        .await
        .map_err(ImportError::TransportError)
}

/// Serve a seekable Parquet source to the server over the native HEAD/GET-Range protocol.
///
/// The server parses the Parquet payload itself, so the IMPORT statement carries
/// no CSV format options. The tunnel reads `source` only when the server asks
/// for a range.
async fn serve_parquet_file<F, Fut, R>(
    execute_sql: F,
    table: &str,
    options: &ParquetImportOptions,
    source: R,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
    R: AsyncRead + AsyncSeek + Unpin + Send + 'static,
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
            .handle_parquet_import_requests(source)
            .await
            .map_err(ImportError::TransportError)
    });

    finish_import(execute_sql(sql).await, stream_handle).await
}

/// Run one CSV IMPORT that converts `reader` while it streams it.
async fn serve_parquet_csv<F, Fut>(
    execute_sql: F,
    table: &str,
    options: &ParquetImportOptions,
    reader: ParquetRecordBatchReader,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<u64, String>>,
{
    let client = connect_transport(options).await?;

    let query = build_single_file_csv_query(table, options, client.internal_address());
    let sql = with_public_key(query, client.public_key_fingerprint()).build();

    // Run data streaming and SQL execution concurrently: the SQL statement is
    // what makes the server open the tunnel the stream task writes into.
    let stream_handle = tokio::spawn(stream_parquet_csv(client, reader, options.clone()));

    finish_import(execute_sql(sql).await, stream_handle).await
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
/// This function reads Parquet data from an async reader and streams it to
/// Exasol via HTTP transport, as CSV or, on the native path, as Parquet.
///
/// # Memory
///
/// The function buffers the whole stream in memory on both import paths,
/// because Parquet keeps its metadata at the end of the file. To bound memory,
/// import from a file path with `import_from_parquet`, which streams the file
/// instead of buffering it. Or decode the Parquet data yourself and pass the
/// RecordBatches in groups to `import_from_record_batches`. Each call is one IMPORT statement and holds
/// the CSV of its own batches more than once. Each call commits separately
/// unless autocommit is off, so a failure in a later group leaves the earlier
/// groups in the table. `import_from_arrow_ipc` also reads its whole input.
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
        return serve_parquet_file(execute_sql, table, &options, std::io::Cursor::new(buffer))
            .await;
    }

    let source = open_parquet_csv_reader(Bytes::from(buffer), &options)?;
    if source.row_count == 0 {
        return Ok(0);
    }

    serve_parquet_csv(execute_sql, table, &options, source.reader).await
}

/// Imports data from multiple Parquet files in parallel into an Exasol table.
///
/// On the CSV conversion path, each file converts in its own spawn_blocking
/// task while its CSV streams through that file's HTTP transport connection,
/// so memory holds a few RecordBatches of CSV per file, not whole files. On the
/// native Parquet path, each file is opened and read one 64 KiB piece at a time
/// when Exasol requests a range, so memory does not grow with the file size.
/// Each file must stay unchanged until the import ends, and a file that
/// becomes shorter fails the import.
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
        for path in &paths {
            if use_native {
                check_native_auto_create_footer(path)?;
            } else {
                check_csv_auto_create_footer(path)?;
            }
        }
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

        return finish_import(execute_sql(sql).await, stream_handle).await;
    }

    let mut files = Vec::with_capacity(num_files);
    for path in paths {
        let source = open_parquet_csv_file(&path, &options)?;
        files.push((path, source));
    }
    if files.iter().all(|(_, source)| source.row_count == 0) {
        return Ok(0);
    }

    let pool =
        ParallelTransportPool::connect(&options.host, options.port, options.use_tls, num_files)
            .await?;
    let sql = build_multi_file_parquet_query(table, &options, pool.query_file_entries()).build();
    let readers = files
        .into_iter()
        .map(|(path, source)| (path, source.reader))
        .collect();
    let stream_handle = tokio::spawn(stream_parquet_files_as_csv(
        pool.into_connections(),
        readers,
        options,
    ));

    finish_import(execute_sql(sql).await, stream_handle).await
}

/// Streams each Parquet file as CSV through its own connection, all at once.
///
/// Each file starts converting when its connection receives Exasol's GET, and
/// Exasol requests every file of one IMPORT together. Every error names the
/// file; a bad value keeps the `ConversionError` variant of a single-file import.
async fn stream_parquet_files_as_csv(
    connections: Vec<HttpTransportClient>,
    files: Vec<(PathBuf, ParquetRecordBatchReader)>,
    options: ParquetImportOptions,
) -> Result<(), ImportError> {
    stream_in_parallel(connections, files, |client, (path, reader)| {
        let stream = stream_parquet_csv(client, reader, options.clone());
        async move {
            stream.await.map_err(|e| match e {
                ImportError::ConversionError(message) => {
                    ImportError::ConversionError(format!("{}: {message}", path.display()))
                }
                e => ImportError::ParallelImportError(format!("{}: {e}", path.display())),
            })
        }
    })
    .await
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

/// Converts an Arrow RecordBatch to CSV rows, one string per row without a row terminator.
///
/// The rows come from the import CSV conversion that RecordBatch import uses, with the
/// separator, delimiter, and null value of `options`, so a CSV-path Parquet import converts
/// every value as RecordBatch import does.
///
/// # Errors
///
/// Returns `ImportError::ConversionError` that names the first column whose Arrow type the
/// conversion does not convert, or that names a value it cannot convert.
pub fn record_batch_to_csv(
    batch: &RecordBatch,
    options: &ParquetImportOptions,
) -> Result<Vec<String>, ImportError> {
    check_schema_converts(batch.schema_ref())?;
    let csv_options = CsvWriterOptions {
        column_separator: options.column_separator,
        column_delimiter: options.column_delimiter,
        row_separator: "",
        null_value: options.null_value.clone(),
        ..CsvWriterOptions::default()
    };
    (0..batch.num_rows())
        .map(|row_idx| format_row(&csv_options, batch, row_idx))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::test_support::{
        finish_within, ChunkedBody, FakeExasolServer, DISCONNECT_BOUND, LOOPBACK_TEST_BOUND,
    };
    use arrow::array::{
        Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array,
        Float64Array, Int16Array, Int32Array, Int64Array, Int8Array, StringArray, StringBuilder,
        TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
        TimestampSecondArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
    };
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
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

    /// Scenario: TLS for HTTP transport is configured uniformly via use_tls
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

    /// A batch of one nullable field per array, named `c0`, `c1`, and so on.
    fn batch_of(columns: Vec<ArrayRef>) -> RecordBatch {
        let fields: Vec<Field> = columns
            .iter()
            .enumerate()
            .map(|(idx, column)| Field::new(format!("c{idx}"), column.data_type().clone(), true))
            .collect();
        RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).expect("batch")
    }

    /// Scenario: CSV-path Parquet import formats pre-epoch timestamps of every time unit as times before the epoch
    #[test]
    fn test_record_batch_to_csv_formats_pre_epoch_timestamps_in_every_unit() {
        let batch = batch_of(vec![
            Arc::new(TimestampSecondArray::from(vec![-1, 0])),
            Arc::new(TimestampMillisecondArray::from(vec![-1, 0])),
            Arc::new(TimestampMicrosecondArray::from(vec![-1, -500_000])),
            Arc::new(TimestampNanosecondArray::from(vec![-1, 0])),
        ]);

        let rows = record_batch_to_csv(&batch, &ParquetImportOptions::default()).unwrap();

        assert_eq!(
            rows,
            [
                "1969-12-31 23:59:59.000000,1969-12-31 23:59:59.999000,\
                 1969-12-31 23:59:59.999999,1969-12-31 23:59:59.999999",
                "1970-01-01 00:00:00.000000,1970-01-01 00:00:00.000000,\
                 1969-12-31 23:59:59.500000,1970-01-01 00:00:00.000000",
            ]
        );
    }

    /// Scenario: CSV-path Parquet import converts values as RecordBatch import does
    #[test]
    fn test_record_batch_to_csv_converts_values_as_record_batch_import_does() {
        use arrow::array::{DictionaryArray, FixedSizeBinaryArray};
        use arrow::datatypes::Int32Type;

        let batch = batch_of(vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(
                Decimal128Array::from(vec![-5])
                    .with_precision_and_scale(5, 2)
                    .unwrap(),
            ),
            Arc::new(DictionaryArray::<Int32Type>::new(
                Int32Array::from(vec![0]),
                Arc::new(StringArray::from(vec!["a;b"])),
            )),
            Arc::new(FixedSizeBinaryArray::try_from_iter([[0xABu8, 0xCD]].into_iter()).unwrap()),
        ]);
        let options = ParquetImportOptions::default().with_column_separator(';');

        let rows = record_batch_to_csv(&batch, &options).unwrap();

        assert_eq!(rows, ["1;-0.05;\"a;b\";abcd"]);
    }

    #[test]
    fn test_record_batch_to_csv_writes_floats_without_an_exponent() {
        let batch = batch_of(vec![Arc::new(Float64Array::from(vec![1e15, 1e-6]))]);

        let rows = record_batch_to_csv(&batch, &ParquetImportOptions::default()).unwrap();

        assert_eq!(rows, ["1000000000000000", "0.000001"]);
    }

    #[test]
    fn test_record_batch_to_csv_names_the_column_of_a_type_without_a_conversion() {
        let batch = batch_of(vec![
            Arc::new(Int32Array::from(vec![1])),
            Arc::new(arrow::array::Time32SecondArray::from(vec![1])),
        ]);

        let err = record_batch_to_csv(&batch, &ParquetImportOptions::default()).unwrap_err();

        assert!(
            matches!(&err, ImportError::ConversionError(message)
                if message.contains("'c1'") && message.contains("Time32(Second)")),
            "got: {err}"
        );
    }

    /// Scenario: CSV-path Parquet import rejects DATE and TIMESTAMP values outside Exasol's range
    #[test]
    fn test_record_batch_to_csv_rejects_dates_and_timestamps_outside_exasol_range() {
        let cases: [(ArrayRef, &str); 4] = [
            (Arc::new(Date32Array::from(vec![2_932_897])), "2932897"),
            (Arc::new(Date32Array::from(vec![-719_163])), "-719163"),
            (
                Arc::new(TimestampSecondArray::from(vec![i64::MAX])),
                "9223372036854775807",
            ),
            (
                Arc::new(TimestampMicrosecondArray::from(vec![
                    253_402_300_800_000_000,
                ])),
                "253402300800000000",
            ),
        ];

        for (array, value) in cases {
            let data_type = array.data_type().clone();
            let err = record_batch_to_csv(&batch_of(vec![array]), &ParquetImportOptions::default())
                .unwrap_err();
            assert!(
                matches!(&err, ImportError::ConversionError(message) if message.contains(value)),
                "{data_type:?} {value}: {err}"
            );
        }
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

    fn no_sql_may_run(sql: String) -> std::future::Ready<Result<u64, String>> {
        panic!("no SQL may run, got: {sql}")
    }

    /// Scenario: Import multiple Parquet files in parallel
    #[tokio::test]
    async fn test_import_from_parquet_files_returns_zero_when_every_file_is_row_less() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let first = create_empty_test_parquet_file(temp_dir.path(), "first.parquet");
        let second = create_empty_test_parquet_file(temp_dir.path(), "second.parquet");
        let options = ParquetImportOptions::default();

        let rows =
            import_from_parquet_files(no_sql_may_run, "t", vec![first, second], options, false)
                .await
                .expect("row-less files are not an error");

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn test_import_from_parquet_files_names_a_missing_file_before_any_sql() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let present = create_test_parquet_file(temp_dir.path(), "present.parquet");
        let missing = temp_dir.path().join("missing.parquet");
        let options = ParquetImportOptions::default();

        let err =
            import_from_parquet_files(no_sql_may_run, "t", vec![present, missing], options, false)
                .await
                .unwrap_err();

        assert!(
            matches!(err, ImportError::ParallelImportError(_)),
            "got: {err}"
        );
        assert!(
            err.to_string().contains("Failed to open Parquet file"),
            "got: {err}"
        );
        assert!(err.to_string().contains("missing.parquet"), "got: {err}");
    }

    #[tokio::test]
    async fn test_import_from_parquet_files_names_a_non_parquet_file_before_any_sql() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let valid = create_test_parquet_file(temp_dir.path(), "valid.parquet");
        let not_parquet = temp_dir.path().join("not_parquet.parquet");
        std::fs::write(&not_parquet, b"definitely not parquet").expect("write the fixture");

        let err = import_from_parquet_files(
            no_sql_may_run,
            "t",
            vec![valid, not_parquet],
            ParquetImportOptions::default(),
            false,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, ImportError::ParallelImportError(_)),
            "got: {err}"
        );
        assert!(
            err.to_string().contains("Failed to read Parquet file"),
            "got: {err}"
        );
        assert!(
            err.to_string().contains("not_parquet.parquet"),
            "got: {err}"
        );
    }

    /// Lets each of `parties` producers wait, up to `GATED_READ_BOUND`, until
    /// all of them have arrived.
    struct Rendezvous {
        parties: usize,
        arrived: std::sync::Mutex<usize>,
        all_arrived: std::sync::Condvar,
    }

    impl Rendezvous {
        fn new(parties: usize) -> Self {
            Self {
                parties,
                arrived: std::sync::Mutex::new(0),
                all_arrived: std::sync::Condvar::new(),
            }
        }

        fn arrive_and_wait(&self) -> Result<(), String> {
            let mut arrived = self.arrived.lock().expect("rendezvous");
            *arrived += 1;
            self.all_arrived.notify_all();
            let (arrived, wait) = self
                .all_arrived
                .wait_timeout_while(arrived, GATED_READ_BOUND, |arrived| *arrived < self.parties)
                .expect("rendezvous");
            if wait.timed_out() {
                return Err(format!(
                    "only {} of {} producers started",
                    *arrived, self.parties
                ));
            }
            Ok(())
        }
    }

    /// Scenario: Parquet conversion is parallelized
    #[tokio::test]
    async fn test_parquet_csv_producers_start_together_on_concurrent_gets() {
        let mut servers = [
            FakeExasolServer::requesting_import().await,
            FakeExasolServer::requesting_import().await,
        ];
        let rendezvous = Arc::new(Rendezvous::new(2));
        let options = ParquetImportOptions::default().with_batch_size(100);
        let mut files = Vec::new();
        for name in ["a.parquet", "b.parquet"] {
            let bytes = parquet_with_row_groups(&id_name_row_groups(2, 100));
            let first_row_group = ObservedParquet::new(bytes.clone()).row_group_range(0);
            let rendezvous = Arc::clone(&rendezvous);
            let file = ObservedParquet::watching(bytes, first_row_group, move || {
                rendezvous.arrive_and_wait()
            });
            let source = open_parquet_csv_reader(file, &options).expect("open");
            files.push((std::path::PathBuf::from(name), source.reader));
        }
        let connections = vec![
            servers[0].connect_tunnel().await,
            servers[1].connect_tunnel().await,
        ];

        finish_within(
            LOOPBACK_TEST_BOUND,
            stream_parquet_files_as_csv(connections, files, options),
        )
        .await
        .expect("both producers must run at the same time");

        for server in &mut servers {
            let received = finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
            assert!(ChunkedBody::parse(&received).terminated);
        }
    }

    /// Scenario: Parallel CSV-path Parquet import that fails on a value imports no rows
    /// Scenario: Fail-fast on Parquet conversion error
    #[tokio::test]
    async fn test_stream_parquet_files_as_csv_names_the_file_of_a_conversion_error() {
        let mut servers = [
            FakeExasolServer::requesting_import().await,
            FakeExasolServer::requesting_import().await,
        ];
        let options = ParquetImportOptions::default().with_batch_size(DATE_ROWS_PER_GROUP as usize);
        let good = parquet_with_row_groups(&id_name_row_groups(1, 10));
        let bad = parquet_with_row_groups(&row_groups_with_out_of_range_last_date());
        let files = vec![
            (
                std::path::PathBuf::from("good.parquet"),
                open_parquet_csv_reader(good, &options)
                    .expect("open")
                    .reader,
            ),
            (
                std::path::PathBuf::from("bad.parquet"),
                open_parquet_csv_reader(bad, &options).expect("open").reader,
            ),
        ];
        let connections = vec![
            servers[0].connect_tunnel().await,
            servers[1].connect_tunnel().await,
        ];

        let err = finish_within(
            LOOPBACK_TEST_BOUND,
            stream_parquet_files_as_csv(connections, files, options),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, ImportError::ConversionError(m) if m.starts_with("bad.parquet: ") && m.contains(&OUT_OF_RANGE_DATE32.to_string())),
            "got: {err}"
        );
        let received = finish_within(DISCONNECT_BOUND, servers[1].wait_for_disconnect()).await;
        assert!(!ChunkedBody::parse(&received).terminated);
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
        assert!(err.to_string().contains("missing.parquet"), "got: {err}");
    }

    #[tokio::test]
    async fn test_import_from_parquet_native_path_fails_on_a_missing_file_before_any_sql() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let sql_calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&sql_calls);
        let execute_sql = move |_sql: String| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok::<u64, String>(1) }
        };

        let err = import_from_parquet(
            execute_sql,
            "t",
            std::path::Path::new("/nonexistent/dir/missing.parquet"),
            ParquetImportOptions::default(),
            true,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::IoError(_)), "got: {err}");
        assert!(err.to_string().contains("missing.parquet"), "got: {err}");
        assert_eq!(sql_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_render_parquet_csv_renders_lf_terminated_rows() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = create_test_parquet_file(temp_dir.path(), "rows.parquet");
        let file = std::fs::File::open(&path).expect("open file");
        let options = ParquetImportOptions::default();
        let source = open_parquet_csv_reader(file, &options).expect("open");
        let mut csv = Vec::new();

        render_parquet_csv(source.reader, &options, |buffer| {
            csv.extend(buffer);
            Ok(())
        })
        .expect("conversion");

        assert_eq!(source.row_count, 2);
        assert_eq!(String::from_utf8(csv).unwrap(), "1,a\n2,b\n");
    }

    #[test]
    fn test_open_parquet_csv_reader_rejects_non_parquet_content() {
        let err = open_parquet_csv_reader(
            Bytes::from_static(b"definitely not parquet"),
            &ParquetImportOptions::default(),
        )
        .err()
        .expect("non-Parquet content must not open");

        assert!(matches!(err, ImportError::ParquetError(_)), "got: {err}");
    }

    #[test]
    fn test_render_parquet_csv_stops_at_the_first_emit_error() {
        let file = parquet_with_row_groups(&id_name_row_groups(3, 10));
        let options = ParquetImportOptions::default().with_batch_size(10);
        let source = open_parquet_csv_reader(file, &options).expect("open");
        let mut emitted = 0;

        let err = render_parquet_csv(source.reader, &options, |_| {
            emitted += 1;
            Err(ImportError::ChannelError("sink closed".to_string()))
        })
        .unwrap_err();

        assert!(matches!(err, ImportError::ChannelError(_)), "got: {err}");
        assert_eq!(emitted, 1);
    }

    /// Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time
    #[test]
    fn test_render_parquet_csv_emits_first_batch_before_decoding_last_row_group() {
        let row_groups = id_name_row_groups(8, 100);
        let file = ObservedParquet::new(parquet_with_row_groups(&row_groups));
        let last_row_group = file.row_group_range(7);
        let options = ParquetImportOptions::default().with_batch_size(100);
        let source = open_parquet_csv_reader(file.clone(), &options).expect("open");
        let mut buffers = Vec::new();
        let mut reads_at_first_buffer = None;

        render_parquet_csv(source.reader, &options, |buffer| {
            reads_at_first_buffer.get_or_insert_with(|| file.reads());
            buffers.push(buffer);
            Ok(())
        })
        .expect("render");

        let reads_at_first_buffer = reads_at_first_buffer.expect("a buffer must arrive");
        assert!(
            !touches(&reads_at_first_buffer, &last_row_group),
            "the last row group {last_row_group:?} was read before the first buffer: {reads_at_first_buffer:?}"
        );
        assert!(touches(&file.reads(), &last_row_group));
        assert_eq!(buffers.len(), 8);
        assert_eq!(buffers.concat(), expected_csv(&row_groups));
    }

    /// `ID` and `NAME` rows, one batch per row group, with ids counting up from 0.
    fn id_name_row_groups(row_groups: usize, rows_per_group: usize) -> Vec<RecordBatch> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("ID", DataType::Int64, false),
            Field::new("NAME", DataType::Utf8, false),
        ]));
        (0..row_groups)
            .map(|group| {
                let ids: Vec<i64> = (group * rows_per_group..(group + 1) * rows_per_group)
                    .map(|id| id as i64)
                    .collect();
                let names: Vec<String> = ids.iter().map(|id| format!("name_{id}")).collect();
                RecordBatch::try_new(
                    Arc::clone(&schema),
                    vec![
                        Arc::new(Int64Array::from(ids)),
                        Arc::new(StringArray::from(names)),
                    ],
                )
                .expect("batch")
            })
            .collect()
    }

    /// Writes each batch as its own row group.
    fn parquet_with_row_groups(row_groups: &[RecordBatch]) -> Bytes {
        let mut file = Vec::new();
        let mut writer =
            parquet::arrow::ArrowWriter::try_new(&mut file, row_groups[0].schema(), None)
                .expect("writer");
        for batch in row_groups {
            writer.write(batch).expect("write");
            writer.flush().expect("flush a row group");
        }
        writer.close().expect("close");
        Bytes::from(file)
    }

    fn expected_csv(batches: &[RecordBatch]) -> Vec<u8> {
        let options = ParquetImportOptions::default();
        batches
            .iter()
            .flat_map(|batch| record_batch_to_csv(batch, &options).expect("rows"))
            .flat_map(|row| [row.into_bytes(), b"\n".to_vec()])
            .flatten()
            .collect()
    }

    fn touches(reads: &[std::ops::Range<u64>], range: &std::ops::Range<u64>) -> bool {
        reads
            .iter()
            .any(|read| read.start < range.end && range.start < read.end)
    }

    type FirstTouchHook = Box<dyn FnOnce() -> Result<(), String> + Send>;

    /// A byte range and the hook that runs the first time a read reaches it.
    struct WatchedRange {
        range: std::ops::Range<u64>,
        on_first_touch: std::sync::Mutex<Option<FirstTouchHook>>,
    }

    /// An in-memory Parquet file that records the byte range of every read
    /// and runs a blocking hook the first time a read reaches a watched range.
    #[derive(Clone)]
    struct ObservedParquet {
        bytes: Bytes,
        reads: Arc<std::sync::Mutex<Vec<std::ops::Range<u64>>>>,
        watch: Option<Arc<WatchedRange>>,
    }

    impl ObservedParquet {
        fn new(bytes: Bytes) -> Self {
            Self {
                bytes,
                reads: Arc::default(),
                watch: None,
            }
        }

        fn watching(
            bytes: Bytes,
            range: std::ops::Range<u64>,
            on_first_touch: impl FnOnce() -> Result<(), String> + Send + 'static,
        ) -> Self {
            let on_first_touch: FirstTouchHook = Box::new(on_first_touch);
            Self {
                watch: Some(Arc::new(WatchedRange {
                    range,
                    on_first_touch: std::sync::Mutex::new(Some(on_first_touch)),
                })),
                ..Self::new(bytes)
            }
        }

        fn reads(&self) -> Vec<std::ops::Range<u64>> {
            self.reads.lock().expect("read log").clone()
        }

        /// The bytes from the first to the last column chunk of row group `index`.
        fn row_group_range(&self, index: usize) -> std::ops::Range<u64> {
            let builder =
                ParquetRecordBatchReaderBuilder::try_new(self.bytes.clone()).expect("footer");
            let chunks: Vec<(u64, u64)> = builder
                .metadata()
                .row_group(index)
                .columns()
                .iter()
                .map(|column| column.byte_range())
                .collect();
            let start = chunks
                .iter()
                .map(|(start, _)| *start)
                .min()
                .expect("a column");
            let end = chunks
                .iter()
                .map(|(start, length)| start + length)
                .max()
                .expect("a column");
            start..end
        }

        fn observe(&self, read: std::ops::Range<u64>) -> parquet::errors::Result<()> {
            self.reads.lock().expect("read log").push(read.clone());
            let Some(watch) = self.watch.as_deref() else {
                return Ok(());
            };
            if !touches(&[read], &watch.range) {
                return Ok(());
            }
            let hook = watch.on_first_touch.lock().expect("hook").take();
            hook.map_or(Ok(()), |hook| hook())
                .map_err(parquet::errors::ParquetError::General)
        }
    }

    impl parquet::file::reader::Length for ObservedParquet {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
    }

    impl parquet::file::reader::ChunkReader for ObservedParquet {
        type T = bytes::buf::Reader<Bytes>;

        /// A page header read starts inside its own column chunk, so its start
        /// offset is enough to place it.
        fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
            self.observe(start..start + 1)?;
            self.bytes.get_read(start)
        }

        fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
            self.observe(start..start + length as u64)?;
            self.bytes.get_bytes(start, length)
        }
    }

    /// How long a gated Parquet read waits before it fails the conversion, so
    /// a failing run ends instead of leaving a blocking thread behind.
    const GATED_READ_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

    const OUT_OF_RANGE_DATE32: i32 = 2_932_897;
    const DATE_ROW_GROUPS: i64 = 4;
    const DATE_ROWS_PER_GROUP: i64 = 1_000;

    async fn wait_for(signal: std::sync::mpsc::Receiver<()>) {
        tokio::task::spawn_blocking(move || signal.recv_timeout(GATED_READ_BOUND))
            .await
            .expect("the waiting thread must not panic")
            .expect("the signal must fire");
    }

    /// `DATE_ROW_GROUPS` row groups of `DATE_ROWS_PER_GROUP` `ID`/`D` rows whose very last date is after 9999-12-31.
    fn row_groups_with_out_of_range_last_date() -> Vec<RecordBatch> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("ID", DataType::Int64, false),
            Field::new("D", DataType::Date32, false),
        ]));
        let last_id = DATE_ROW_GROUPS * DATE_ROWS_PER_GROUP - 1;
        (0..DATE_ROW_GROUPS)
            .map(|group| {
                let ids: Vec<i64> =
                    (group * DATE_ROWS_PER_GROUP..(group + 1) * DATE_ROWS_PER_GROUP).collect();
                let dates: Vec<i32> = ids
                    .iter()
                    .map(|id| {
                        if *id == last_id {
                            OUT_OF_RANGE_DATE32
                        } else {
                            0
                        }
                    })
                    .collect();
                RecordBatch::try_new(
                    Arc::clone(&schema),
                    vec![
                        Arc::new(Int64Array::from(ids)),
                        Arc::new(Date32Array::from(dates)),
                    ],
                )
                .expect("batch")
            })
            .collect()
    }

    /// Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time
    #[tokio::test]
    async fn test_stream_parquet_csv_sends_every_batch_and_the_terminating_chunk() {
        let mut server = FakeExasolServer::requesting_import().await;
        let row_groups = id_name_row_groups(3, 5_000);
        let options = ParquetImportOptions::default().with_batch_size(5_000);
        let source =
            open_parquet_csv_reader(parquet_with_row_groups(&row_groups), &options).expect("open");

        finish_within(
            LOOPBACK_TEST_BOUND,
            stream_parquet_csv(server.connect_tunnel().await, source.reader, options),
        )
        .await
        .expect("the stream succeeds");

        let body = ChunkedBody::parse(
            &finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await,
        );
        assert_eq!(body.data(), expected_csv(&row_groups));
        assert!(body.terminated);
        assert!(
            body.chunks.len() > row_groups.len(),
            "a batch above 64 KiB spans pieces"
        );
        assert!(body
            .chunks
            .iter()
            .all(|chunk| chunk.len() <= CHUNKED_TRANSFER_SIZE));
    }

    /// Scenario: CSV-path Parquet import that fails on a later RecordBatch imports no rows
    #[tokio::test]
    async fn test_stream_parquet_csv_conversion_error_withholds_the_terminating_chunk() {
        let mut server = FakeExasolServer::requesting_import().await;
        let file = parquet_with_row_groups(&row_groups_with_out_of_range_last_date());
        let options = ParquetImportOptions::default().with_batch_size(DATE_ROWS_PER_GROUP as usize);
        let source = open_parquet_csv_reader(file, &options).expect("open");

        let err = finish_within(
            LOOPBACK_TEST_BOUND,
            stream_parquet_csv(server.connect_tunnel().await, source.reader, options),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, ImportError::ConversionError(message) if message.contains(&OUT_OF_RANGE_DATE32.to_string())),
            "got: {err}"
        );
        let body = ChunkedBody::parse(
            &finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await,
        );
        assert!(!body.terminated);
    }

    #[tokio::test]
    async fn test_stream_parquet_csv_producer_panic_withholds_the_terminating_chunk() {
        let mut server = FakeExasolServer::requesting_import().await;
        let bytes = parquet_with_row_groups(&id_name_row_groups(4, 100));
        let second_row_group = ObservedParquet::new(bytes.clone()).row_group_range(1);
        let file = ObservedParquet::watching(bytes, second_row_group, || -> Result<(), String> {
            panic!("the Parquet source failed mid-file")
        });
        let options = ParquetImportOptions::default().with_batch_size(100);
        let source = open_parquet_csv_reader(file, &options).expect("open");

        let err = finish_within(
            LOOPBACK_TEST_BOUND,
            stream_parquet_csv(server.connect_tunnel().await, source.reader, options),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::StreamError(_)), "got: {err}");
        let body = ChunkedBody::parse(
            &finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await,
        );
        assert!(!body.terminated);
    }

    /// Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time
    #[tokio::test]
    async fn test_stream_parquet_csv_writes_first_batch_before_decoding_last_row_group() {
        let mut server = FakeExasolServer::requesting_import().await;
        let first_body_chunk = server.first_body_chunk_signal();
        let row_groups = id_name_row_groups(8, 100);
        let bytes = parquet_with_row_groups(&row_groups);
        let last_row_group = ObservedParquet::new(bytes.clone()).row_group_range(7);
        let file = ObservedParquet::watching(bytes, last_row_group, move || {
            first_body_chunk
                .recv_timeout(GATED_READ_BOUND)
                .map_err(|_| {
                    "the last row group was read before the peer received a body chunk".to_string()
                })
        });
        let options = ParquetImportOptions::default().with_batch_size(100);
        let source = open_parquet_csv_reader(file, &options).expect("open");

        finish_within(
            LOOPBACK_TEST_BOUND,
            stream_parquet_csv(server.connect_tunnel().await, source.reader, options),
        )
        .await
        .expect("the stream succeeds");

        let body = ChunkedBody::parse(
            &finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await,
        );
        assert_eq!(body.data(), expected_csv(&row_groups));
        assert!(body.terminated);
    }

    #[tokio::test]
    async fn test_stream_parquet_csv_stops_the_producer_when_the_tunnel_task_is_aborted() {
        let mut server = FakeExasolServer::requesting_import().await;
        let first_body_chunk = server.first_body_chunk_signal();
        let bytes = parquet_with_row_groups(&id_name_row_groups(8, 100));
        let probe = ObservedParquet::new(bytes.clone());
        let (third_row_group, last_row_group) =
            (probe.row_group_range(2), probe.row_group_range(7));
        let (release, released) = std::sync::mpsc::channel::<()>();
        let file = ObservedParquet::watching(bytes, third_row_group, move || {
            released
                .recv_timeout(GATED_READ_BOUND)
                .map_err(|_| "the test never released the producer".to_string())
        });
        let options = ParquetImportOptions::default().with_batch_size(100);
        let source = open_parquet_csv_reader(file.clone(), &options).expect("open");
        let tunnel = tokio::spawn(stream_parquet_csv(
            server.connect_tunnel().await,
            source.reader,
            options,
        ));

        wait_for(first_body_chunk).await;
        tunnel.abort();
        let cancelled = finish_within(LOOPBACK_TEST_BOUND, tunnel)
            .await
            .unwrap_err();
        release
            .send(())
            .expect("the producer waits at the third row group");
        finish_within(LOOPBACK_TEST_BOUND, async {
            while Arc::strong_count(&file.reads) > 1 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;

        assert!(cancelled.is_cancelled());
        assert!(
            !touches(&file.reads(), &last_row_group),
            "the producer kept decoding after its tunnel task was aborted"
        );
    }

    /// The IMPORT statements that [`exasol_import`] ran, with the body each one received.
    type ImportLog = Arc<std::sync::Mutex<Vec<(String, ChunkedBody)>>>;

    /// Plays Exasol's side of one IMPORT statement against `server`: it ends
    /// when the driver closes the tunnel, and it fails when the body lacks the
    /// terminating chunk, as Exasol does. It returns the number of body lines.
    fn exasol_import(
        server: FakeExasolServer,
        log: ImportLog,
    ) -> impl FnMut(String) -> std::pin::Pin<Box<dyn Future<Output = Result<u64, String>> + Send>>
    {
        let mut server = Some(server);
        move |sql| {
            let mut server = server.take().expect("only one IMPORT statement runs");
            let log = Arc::clone(&log);
            Box::pin(async move {
                let received =
                    finish_within(LOOPBACK_TEST_BOUND, server.wait_for_disconnect()).await;
                let body = ChunkedBody::parse(&received);
                let outcome = if body.terminated {
                    Ok(body.data().iter().filter(|byte| **byte == b'\n').count() as u64)
                } else {
                    Err("the tunnel body ended without the terminating chunk".to_string())
                };
                log.lock().expect("import log").push((sql, body));
                outcome
            })
        }
    }

    fn tunnel_options(server: &FakeExasolServer, batch_size: usize) -> ParquetImportOptions {
        ParquetImportOptions::default()
            .with_exasol_host(server.host.clone())
            .with_exasol_port(server.port)
            .with_batch_size(batch_size)
    }

    /// Scenario: CSV-path Parquet import that fails on a later RecordBatch imports no rows
    #[tokio::test]
    async fn test_import_from_parquet_stream_conversion_error_fails_after_the_import_starts() {
        let server = FakeExasolServer::requesting_import().await;
        let options = tunnel_options(&server, DATE_ROWS_PER_GROUP as usize);
        let file = parquet_with_row_groups(&row_groups_with_out_of_range_last_date());
        let log = ImportLog::default();

        let err = finish_within(
            LOOPBACK_TEST_BOUND,
            import_from_parquet_stream(
                exasol_import(server, Arc::clone(&log)),
                "T",
                &file[..],
                options,
                false,
            ),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, ImportError::ConversionError(message) if message.contains(&OUT_OF_RANGE_DATE32.to_string())),
            "got: {err}"
        );
        let log = log.lock().expect("import log");
        let [(sql, body)] = log.as_slice() else {
            panic!("exactly one IMPORT statement must run, got {}", log.len());
        };
        assert!(sql.contains("FROM CSV"), "got: {sql}");
        assert!(!body.terminated);
    }

    /// Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time
    #[tokio::test]
    async fn test_import_from_parquet_streams_every_row_through_one_import() {
        let server = FakeExasolServer::requesting_import().await;
        let options = tunnel_options(&server, 100);
        let row_groups = id_name_row_groups(8, 100);
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = temp_dir.path().join("rows.parquet");
        std::fs::write(&path, parquet_with_row_groups(&row_groups)).expect("write file");
        let log = ImportLog::default();

        let rows = finish_within(
            LOOPBACK_TEST_BOUND,
            import_from_parquet(
                exasol_import(server, Arc::clone(&log)),
                "T",
                &path,
                options,
                false,
            ),
        )
        .await
        .expect("the import succeeds");

        assert_eq!(rows, 800);
        let log = log.lock().expect("import log");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].1.data(), expected_csv(&row_groups));
        assert!(log[0].1.terminated);
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

    /// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
    #[tokio::test]
    async fn test_serve_parquet_file_returns_statement_error_and_closes_a_silent_tunnel() {
        let hang_limit = std::time::Duration::from_secs(5);
        let mut server = FakeExasolServer::silent_after_handshake().await;
        let options = ParquetImportOptions::default()
            .with_exasol_host(server.host.clone())
            .with_exasol_port(server.port)
            .use_tls(false);

        let result = tokio::time::timeout(
            hang_limit,
            serve_parquet_file(
                |_sql: String| async { Err::<u64, String>("object T not found".to_string()) },
                "t",
                &options,
                std::io::Cursor::new(b"PAR1".to_vec()),
            ),
        )
        .await
        .expect("a failed IMPORT statement must not wait for the tunnel");

        assert!(
            matches!(&result, Err(ImportError::SqlError(m)) if m == "object T not found"),
            "got: {result:?}"
        );
        tokio::time::timeout(hang_limit, server.wait_for_disconnect())
            .await
            .expect("the tunnel connection must reach end of stream within 5 seconds");
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

    fn write_parquet(
        path: std::path::PathBuf,
        batch: &RecordBatch,
        options: parquet::arrow::arrow_writer::ArrowWriterOptions,
    ) -> std::path::PathBuf {
        let file = std::fs::File::create(&path).expect("create file");
        let mut writer =
            parquet::arrow::ArrowWriter::try_new_with_options(file, batch.schema(), options)
                .expect("writer");
        writer.write(batch).expect("write");
        writer.close().expect("close");
        path
    }

    fn with_arrow_schema() -> parquet::arrow::arrow_writer::ArrowWriterOptions {
        parquet::arrow::arrow_writer::ArrowWriterOptions::new()
    }

    fn without_arrow_schema() -> parquet::arrow::arrow_writer::ArrowWriterOptions {
        parquet::arrow::arrow_writer::ArrowWriterOptions::new().with_skip_arrow_metadata(true)
    }

    /// A batch with the non-nullable field `ID` (Int64) holding 1 and the field `X` holding `x`.
    fn id_and_x(x: ArrayRef) -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new("ID", DataType::Int64, false),
            Field::new("X", x.data_type().clone(), true),
        ]);
        RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(Int64Array::from(vec![1])), x],
        )
        .expect("batch")
    }

    fn fourteen_months() -> ArrayRef {
        Arc::new(arrow::array::IntervalYearMonthArray::from(vec![14]))
    }

    type SqlLog = Arc<std::sync::Mutex<Vec<String>>>;

    /// Records every SQL statement and fails it, so an import stops at its first statement.
    fn record_and_fail_sql(
        log: &SqlLog,
    ) -> impl FnMut(String) -> std::future::Ready<Result<u64, String>> {
        let log = Arc::clone(log);
        move |sql| {
            log.lock().expect("sql log").push(sql);
            std::future::ready(Err("the test stops at the first statement".to_string()))
        }
    }

    fn auto_create_options() -> ParquetImportOptions {
        ParquetImportOptions::default().with_create_table_if_not_exists(true)
    }

    fn assert_schema_inference_error(err: &ImportError, parts: &[&str]) {
        assert!(
            matches!(err, ImportError::SchemaInferenceError(message)
                if parts.iter().all(|part| message.contains(part))),
            "expected a schema inference error containing {parts:?}, got: {err}"
        );
    }

    /// Scenario: Native Parquet import with table creation rejects Utf8View, dictionary, BinaryView, FixedSizeBinary, and interval columns
    #[tokio::test]
    async fn test_native_parquet_auto_create_rejects_csv_only_types_before_any_sql() {
        use arrow::array::{
            BinaryViewArray, DictionaryArray, FixedSizeBinaryArray, IntervalDayTimeArray,
            StringViewArray,
        };
        use arrow::datatypes::{Int32Type, IntervalDayTime};

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let accepted = write_parquet(
            temp_dir.path().join("accepted.parquet"),
            &id_and_x(Arc::new(StringArray::from(vec!["a"]))),
            with_arrow_schema(),
        );
        let cases: [(ArrayRef, &str); 6] = [
            (Arc::new(StringViewArray::from(vec!["a"])), "Utf8View"),
            (
                Arc::new(DictionaryArray::<Int32Type>::new(
                    Int32Array::from(vec![0]),
                    Arc::new(StringArray::from(vec!["a"])),
                )),
                "Dictionary(Int32, Utf8)",
            ),
            (
                Arc::new(BinaryViewArray::from(vec![&[1u8][..]])),
                "BinaryView",
            ),
            (
                Arc::new(FixedSizeBinaryArray::try_from_iter([[7u8; 16]].into_iter()).unwrap()),
                "FixedSizeBinary(16)",
            ),
            (fourteen_months(), "Interval(YearMonth)"),
            (
                Arc::new(IntervalDayTimeArray::from(vec![IntervalDayTime::new(1, 0)])),
                "Interval(DayTime)",
            ),
        ];

        for (index, (x, type_name)) in cases.into_iter().enumerate() {
            let name = format!("rejected_{index}.parquet");
            let rejected = write_parquet(
                temp_dir.path().join(&name),
                &id_and_x(x),
                with_arrow_schema(),
            );
            let parts = [
                "X",
                type_name,
                "with_native_parquet(Some(false))",
                name.as_str(),
            ];

            let log = SqlLog::default();
            let err = import_from_parquet(
                record_and_fail_sql(&log),
                "T",
                &rejected,
                auto_create_options(),
                true,
            )
            .await
            .unwrap_err();
            assert_schema_inference_error(&err, &parts);
            assert!(log.lock().unwrap().is_empty(), "{type_name}");

            let log = SqlLog::default();
            let err = import_from_parquet_files(
                record_and_fail_sql(&log),
                "T",
                vec![accepted.clone(), rejected],
                auto_create_options(),
                true,
            )
            .await
            .unwrap_err();
            assert_schema_inference_error(&err, &parts);
            assert!(log.lock().unwrap().is_empty(), "{type_name}");
        }
    }

    #[tokio::test]
    async fn test_native_parquet_auto_create_accepts_every_allow_listed_type() {
        use arrow::array::{Date64Array, Decimal256Array, LargeBinaryArray, LargeStringArray};
        use arrow::datatypes::i256;

        let columns: Vec<ArrayRef> = vec![
            Arc::new(BooleanArray::from(vec![true])),
            Arc::new(Int8Array::from(vec![1])),
            Arc::new(Int16Array::from(vec![1])),
            Arc::new(Int32Array::from(vec![1])),
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(UInt8Array::from(vec![1])),
            Arc::new(UInt16Array::from(vec![1])),
            Arc::new(UInt32Array::from(vec![1])),
            Arc::new(UInt64Array::from(vec![1])),
            Arc::new(Float32Array::from(vec![1.5])),
            Arc::new(Float64Array::from(vec![1.5])),
            Arc::new(StringArray::from(vec!["s"])),
            Arc::new(LargeStringArray::from(vec!["s"])),
            Arc::new(BinaryArray::from(vec![&[1u8][..]])),
            Arc::new(LargeBinaryArray::from(vec![&[1u8][..]])),
            Arc::new(Date32Array::from(vec![1])),
            Arc::new(Date64Array::from(vec![86_400_000])),
            Arc::new(TimestampSecondArray::from(vec![1])),
            Arc::new(TimestampMicrosecondArray::from(vec![1]).with_timezone("UTC")),
            Arc::new(
                Decimal128Array::from(vec![1])
                    .with_precision_and_scale(10, 2)
                    .unwrap(),
            ),
            Arc::new(
                Decimal256Array::from(vec![i256::from_i128(1)])
                    .with_precision_and_scale(36, 2)
                    .unwrap(),
            ),
        ];
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = write_parquet(
            temp_dir.path().join("allowed.parquet"),
            &batch_of(columns),
            with_arrow_schema(),
        );
        let log = SqlLog::default();

        let err = import_from_parquet(
            record_and_fail_sql(&log),
            "T",
            &path,
            auto_create_options(),
            true,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::SqlError(_)), "got: {err}");
        let log = log.lock().unwrap();
        assert!(log[0].starts_with("CREATE TABLE"), "got: {log:?}");
    }

    /// Scenario: CSV-path Parquet import rejects an INTERVAL column from a file without an Arrow schema
    #[tokio::test]
    async fn test_csv_path_parquet_import_rejects_interval_without_arrow_schema_before_any_sql() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let batch = id_and_x(fourteen_months());
        let accepted = write_parquet(
            temp_dir.path().join("accepted.parquet"),
            &batch,
            with_arrow_schema(),
        );
        let rejected = write_parquet(
            temp_dir.path().join("rejected.parquet"),
            &batch,
            without_arrow_schema(),
        );
        let parts = ["'X'", "would lose its month part", "rejected.parquet"];

        let log = SqlLog::default();
        let err = import_from_parquet(
            record_and_fail_sql(&log),
            "T",
            &rejected,
            auto_create_options(),
            false,
        )
        .await
        .unwrap_err();
        assert_schema_inference_error(&err, &parts);
        assert!(log.lock().unwrap().is_empty());

        let log = SqlLog::default();
        let err = import_from_parquet_files(
            record_and_fail_sql(&log),
            "T",
            vec![accepted, rejected],
            auto_create_options(),
            false,
        )
        .await
        .unwrap_err();
        assert_schema_inference_error(&err, &parts);
        assert!(log.lock().unwrap().is_empty());
    }

    /// Scenario: CSV-path Parquet import rejects an INTERVAL column from a file without an Arrow schema
    #[test]
    fn test_open_parquet_csv_reader_rejects_interval_without_arrow_schema() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = write_parquet(
            temp_dir.path().join("interval.parquet"),
            &id_and_x(fourteen_months()),
            without_arrow_schema(),
        );
        let bytes = Bytes::from(std::fs::read(&path).expect("read file"));

        let err = open_parquet_csv_reader(bytes, &ParquetImportOptions::default())
            .err()
            .expect("the reader must reject the interval column");

        assert_schema_inference_error(&err, &["'X'", "would lose its month part"]);
    }

    #[test]
    fn test_auto_create_footer_checks_name_a_file_whose_footer_cannot_be_read() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let missing = temp_dir.path().join("missing.parquet");
        let not_parquet = temp_dir.path().join("not_parquet.parquet");
        std::fs::write(&not_parquet, b"definitely not parquet").expect("write the fixture");

        for path in [missing, not_parquet] {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            for err in [
                check_native_auto_create_footer(&path).unwrap_err(),
                check_csv_auto_create_footer(&path).unwrap_err(),
            ] {
                assert_schema_inference_error(&err, &["Failed to read the Parquet footer", &name]);
            }
        }
    }

    #[tokio::test]
    async fn test_csv_path_parquet_auto_create_accepts_interval_from_a_file_with_its_arrow_schema()
    {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = write_parquet(
            temp_dir.path().join("interval.parquet"),
            &id_and_x(fourteen_months()),
            with_arrow_schema(),
        );
        let log = SqlLog::default();

        let err = import_from_parquet(
            record_and_fail_sql(&log),
            "T",
            &path,
            auto_create_options(),
            false,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ImportError::SqlError(_)), "got: {err}");
        let log = log.lock().unwrap();
        assert!(
            log[0].starts_with("CREATE TABLE") && log[0].contains("INTERVAL YEAR(9) TO MONTH"),
            "got: {log:?}"
        );
    }

    #[tokio::test]
    async fn test_csv_path_parquet_import_accepts_a_file_without_arrow_schema_and_without_intervals(
    ) {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = write_parquet(
            temp_dir.path().join("plain.parquet"),
            &id_and_x(Arc::new(StringArray::from(vec!["a"]))),
            without_arrow_schema(),
        );
        let options = ParquetImportOptions::default();
        let source = open_parquet_csv_reader(std::fs::File::open(&path).expect("open"), &options)
            .expect("a file without an Interval column needs no Arrow schema");
        let mut csv = Vec::new();

        render_parquet_csv(source.reader, &options, |buffer| {
            csv.extend(buffer);
            Ok(())
        })
        .expect("conversion");
        assert_eq!(String::from_utf8(csv).unwrap(), "1,a\n");

        let log = SqlLog::default();
        let err = import_from_parquet(
            record_and_fail_sql(&log),
            "T",
            &path,
            auto_create_options(),
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ImportError::SqlError(_)), "got: {err}");
        let log = log.lock().unwrap();
        assert!(log[0].starts_with("CREATE TABLE"), "got: {log:?}");
    }

    #[test]
    fn test_render_parquet_csv_converts_interval_from_a_file_with_its_arrow_schema() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let path = write_parquet(
            temp_dir.path().join("interval.parquet"),
            &id_and_x(fourteen_months()),
            with_arrow_schema(),
        );
        let options = ParquetImportOptions::default();
        let source = open_parquet_csv_reader(std::fs::File::open(&path).expect("open"), &options)
            .expect("a file with its Arrow schema keeps the interval unit");
        let mut csv = Vec::new();

        render_parquet_csv(source.reader, &options, |buffer| {
            csv.extend(buffer);
            Ok(())
        })
        .expect("conversion");

        assert_eq!(String::from_utf8(csv).unwrap(), "1,1-02\n");
    }
}
