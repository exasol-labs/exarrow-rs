//! Arrow RecordBatch export functionality.
//!
//! This module provides utilities for exporting data from Exasol to Arrow RecordBatches
//! and Arrow IPC format. The export process:
//!
//! 1. Executes an EXPORT SQL to receive CSV via HTTP transport
//! 2. Parses CSV and converts to Arrow RecordBatches
//! 3. Returns as a Stream or writes to Arrow IPC format
//!
//! # Example
//!
//! ```ignore
//! use exarrow_rs::export::arrow::{ArrowExportOptions, CsvToArrowReader};
//! use arrow::datatypes::{Schema, Field, DataType};
//! use std::sync::Arc;
//!
//! // Create a schema
//! let schema = Arc::new(Schema::new(vec![
//!     Field::new("id", DataType::Int64, false),
//!     Field::new("name", DataType::Utf8, true),
//! ]));
//!
//! // Create options
//! let options = ArrowExportOptions::default()
//!     .with_batch_size(2048)
//!     .with_schema(schema);
//! ```

use std::sync::Arc;

use arrow::array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    TimestampMicrosecondBuilder,
};
use arrow::array::{ArrayRef, RecordBatch};
use arrow::datatypes::{DataType, Schema, TimeUnit};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::types::{
    conversion::{
        exasol_type_to_arrow as exasol_type_to_arrow_impl, parse_date_to_days,
        parse_decimal_to_i128, parse_timestamp_to_micros,
    },
    ExasolType,
};

/// Errors that can occur during Arrow export operations.
#[derive(Error, Debug)]
pub enum ExportError {
    /// CSV parsing error
    #[error("CSV parsing error at row {row}: {message}")]
    CsvParseError { row: usize, message: String },

    /// Type conversion error
    #[error("Type conversion error at row {row}, column {column}: {message}")]
    TypeConversionError {
        row: usize,
        column: usize,
        message: String,
    },

    /// Schema error
    #[error("Schema error: {0}")]
    SchemaError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(String),

    /// Arrow error
    #[error("Arrow error: {0}")]
    ArrowError(String),

    /// Transport error
    #[error("Transport error: {0}")]
    TransportError(String),
}

impl From<std::io::Error> for ExportError {
    fn from(err: std::io::Error) -> Self {
        ExportError::IoError(err.to_string())
    }
}

impl From<arrow::error::ArrowError> for ExportError {
    fn from(err: arrow::error::ArrowError) -> Self {
        ExportError::ArrowError(err.to_string())
    }
}

/// Options for Arrow export operations.
#[derive(Debug, Clone)]
pub struct ArrowExportOptions {
    /// Number of rows per RecordBatch (default: 1024)
    pub batch_size: usize,
    /// Custom NULL value representation in CSV (default: empty string)
    pub null_value: Option<String>,
    /// Optional explicit Arrow schema (if not provided, will be inferred)
    pub schema: Option<Arc<Schema>>,
    /// Column separator in CSV (default: ',')
    pub column_separator: char,
    /// Column delimiter/quote character (default: '"')
    pub column_delimiter: char,
    /// Exasol host for HTTP transport connection.
    /// This is typically the same host as the WebSocket connection.
    pub host: String,
    /// Exasol port for HTTP transport connection.
    /// This is typically the same port as the WebSocket connection.
    pub port: u16,
    /// Whether to use TLS for the HTTP transport tunnel.
    /// Default is `false` because the main WebSocket connection typically
    /// already handles TLS encryption.
    pub use_tls: bool,
}

impl Default for ArrowExportOptions {
    fn default() -> Self {
        Self {
            batch_size: 1024,
            null_value: None,
            schema: None,
            column_separator: ',',
            column_delimiter: '"',
            host: String::new(),
            port: 0,
            use_tls: false,
        }
    }
}

impl ArrowExportOptions {
    /// Creates new ArrowExportOptions with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    #[must_use]
    pub fn with_null_value(mut self, null_value: impl Into<String>) -> Self {
        self.null_value = Some(null_value.into());
        self
    }

    #[must_use]
    pub fn with_schema(mut self, schema: Arc<Schema>) -> Self {
        self.schema = Some(schema);
        self
    }

    #[must_use]
    pub fn with_column_separator(mut self, sep: char) -> Self {
        self.column_separator = sep;
        self
    }

    #[must_use]
    pub fn with_column_delimiter(mut self, delim: char) -> Self {
        self.column_delimiter = delim;
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

    /// Sets whether to use TLS for the HTTP transport tunnel.
    #[must_use]
    pub fn use_tls(mut self, v: bool) -> Self {
        self.use_tls = v;
        self
    }
}

/// CSV-to-Arrow streaming reader.
///
/// This struct reads CSV data from an async reader and converts it to Arrow RecordBatches.
/// It supports configurable batch sizes and handles NULL values and type parsing.
pub struct CsvToArrowReader<R> {
    reader: BufReader<R>,
    schema: Arc<Schema>,
    batch_size: usize,
    null_value: Option<String>,
    column_separator: char,
    column_delimiter: char,
    current_row: usize,
    finished: bool,
}

impl<R: AsyncBufRead + Unpin> CsvToArrowReader<R> {
    /// Creates a new CsvToArrowReader.
    ///
    /// # Arguments
    ///
    /// * `reader` - The async reader to read CSV data from
    /// * `schema` - The Arrow schema for the output RecordBatches
    /// * `options` - Export options including batch size and NULL value
    pub fn new(reader: R, schema: Arc<Schema>, options: &ArrowExportOptions) -> Self
    where
        R: tokio::io::AsyncRead,
    {
        Self {
            reader: BufReader::new(reader),
            schema,
            batch_size: options.batch_size,
            null_value: options.null_value.clone(),
            column_separator: options.column_separator,
            column_delimiter: options.column_delimiter,
            current_row: 0,
            finished: false,
        }
    }

    /// Creates a new CsvToArrowReader from an already buffered reader.
    ///
    /// # Arguments
    ///
    /// * `reader` - The buffered async reader to read CSV data from
    /// * `schema` - The Arrow schema for the output RecordBatches
    /// * `options` - Export options including batch size and NULL value
    pub fn from_buffered(
        reader: BufReader<R>,
        schema: Arc<Schema>,
        options: &ArrowExportOptions,
    ) -> Self
    where
        R: tokio::io::AsyncRead,
    {
        Self {
            reader,
            schema,
            batch_size: options.batch_size,
            null_value: options.null_value.clone(),
            column_separator: options.column_separator,
            column_delimiter: options.column_delimiter,
            current_row: 0,
            finished: false,
        }
    }

    /// Returns the Arrow schema.
    #[must_use]
    pub fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.schema)
    }

    /// Reads the next batch of rows and returns a RecordBatch.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(batch))` - A RecordBatch with up to `batch_size` rows
    /// * `Ok(None)` - No more data to read
    /// * `Err(e)` - An error occurred during parsing or conversion
    pub async fn next_batch(&mut self) -> Result<Option<RecordBatch>, ExportError> {
        if self.finished {
            return Ok(None);
        }

        let mut rows: Vec<Vec<String>> = Vec::with_capacity(self.batch_size);

        // Read up to batch_size rows
        for _ in 0..self.batch_size {
            let mut line = String::new();
            let bytes_read = self.reader.read_line(&mut line).await?;

            if bytes_read == 0 {
                self.finished = true;
                break;
            }

            // Remove trailing newline
            let line = line.trim_end_matches(&['\r', '\n'][..]);
            if line.is_empty() {
                continue;
            }

            let (fields, in_quotes) = crate::export::csv::parse_csv_row(
                line,
                self.column_separator,
                self.column_delimiter,
            );
            if in_quotes {
                return Err(ExportError::CsvParseError {
                    row: self.current_row,
                    message: "Unclosed quote in CSV row".to_string(),
                });
            }
            rows.push(fields);
            self.current_row += 1;
        }

        if rows.is_empty() {
            return Ok(None);
        }

        // Build RecordBatch from parsed rows
        let batch = self.build_record_batch(&rows)?;
        Ok(Some(batch))
    }

    /// Builds a RecordBatch from parsed CSV rows.
    fn build_record_batch(&self, rows: &[Vec<String>]) -> Result<RecordBatch, ExportError> {
        let num_columns = self.schema.fields().len();
        let num_rows = rows.len();

        // Build arrays for each column
        let arrays: Result<Vec<ArrayRef>, ExportError> = (0..num_columns)
            .map(|col_idx| {
                let field = self.schema.field(col_idx);
                let values: Vec<&str> = rows
                    .iter()
                    .map(|row| row.get(col_idx).map(|s| s.as_str()).unwrap_or(""))
                    .collect();

                build_array_from_strings(
                    &values,
                    field.data_type(),
                    field.is_nullable(),
                    &self.null_value,
                    self.current_row - num_rows,
                    col_idx,
                )
            })
            .collect();

        let arrays = arrays?;

        RecordBatch::try_new(Arc::clone(&self.schema), arrays)
            .map_err(|e| ExportError::ArrowError(e.to_string()))
    }
}

/// Builds an Arrow array from string values.
fn build_array_from_strings(
    values: &[&str],
    data_type: &DataType,
    nullable: bool,
    null_value: &Option<String>,
    start_row: usize,
    column: usize,
) -> Result<ArrayRef, ExportError> {
    match data_type {
        DataType::Boolean => build_boolean_array(values, nullable, null_value, start_row, column),
        DataType::Int64 => build_int64_array(values, nullable, null_value, start_row, column),
        DataType::Float64 => build_float64_array(values, nullable, null_value, start_row, column),
        DataType::Utf8 => build_string_array(values, nullable, null_value),
        DataType::Date32 => build_date32_array(values, nullable, null_value, start_row, column),
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            build_timestamp_array(values, nullable, null_value, start_row, column)
        }
        DataType::Decimal128(precision, scale) => build_decimal128_array(
            values, *precision, *scale, nullable, null_value, start_row, column,
        ),
        _ => Err(ExportError::SchemaError(format!(
            "Unsupported data type: {:?}",
            data_type
        ))),
    }
}

/// Checks if a value is NULL.
fn is_null_value(value: &str, null_value: &Option<String>) -> bool {
    if value.is_empty() {
        return true;
    }
    if let Some(nv) = null_value {
        return value == nv;
    }
    false
}

/// Builds a Boolean array from string values.
fn build_boolean_array(
    values: &[&str],
    nullable: bool,
    null_value: &Option<String>,
    start_row: usize,
    column: usize,
) -> Result<ArrayRef, ExportError> {
    let mut builder = BooleanBuilder::with_capacity(values.len());

    for (i, value) in values.iter().enumerate() {
        if is_null_value(value, null_value) {
            if nullable {
                builder.append_null();
            } else {
                return Err(ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: "NULL value in non-nullable column".to_string(),
                });
            }
        } else {
            let lower = value.to_lowercase();
            let b = match lower.as_str() {
                "true" | "1" | "t" | "yes" | "y" => true,
                "false" | "0" | "f" | "no" | "n" => false,
                _ => {
                    return Err(ExportError::TypeConversionError {
                        row: start_row + i,
                        column,
                        message: format!("Invalid boolean value: {}", value),
                    });
                }
            };
            builder.append_value(b);
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Builds an Int64 array from string values.
fn build_int64_array(
    values: &[&str],
    nullable: bool,
    null_value: &Option<String>,
    start_row: usize,
    column: usize,
) -> Result<ArrayRef, ExportError> {
    let mut builder = Int64Builder::with_capacity(values.len());

    for (i, value) in values.iter().enumerate() {
        if is_null_value(value, null_value) {
            if nullable {
                builder.append_null();
            } else {
                return Err(ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: "NULL value in non-nullable column".to_string(),
                });
            }
        } else {
            let n = value
                .parse::<i64>()
                .map_err(|e| ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: format!("Invalid integer value '{}': {}", value, e),
                })?;
            builder.append_value(n);
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Builds a Float64 array from string values.
fn build_float64_array(
    values: &[&str],
    nullable: bool,
    null_value: &Option<String>,
    start_row: usize,
    column: usize,
) -> Result<ArrayRef, ExportError> {
    let mut builder = Float64Builder::with_capacity(values.len());

    for (i, value) in values.iter().enumerate() {
        if is_null_value(value, null_value) {
            if nullable {
                builder.append_null();
            } else {
                return Err(ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: "NULL value in non-nullable column".to_string(),
                });
            }
        } else {
            // Handle special float values
            let f = match *value {
                "Infinity" | "inf" => f64::INFINITY,
                "-Infinity" | "-inf" => f64::NEG_INFINITY,
                "NaN" | "nan" => f64::NAN,
                _ => value
                    .parse::<f64>()
                    .map_err(|e| ExportError::TypeConversionError {
                        row: start_row + i,
                        column,
                        message: format!("Invalid float value '{}': {}", value, e),
                    })?,
            };
            builder.append_value(f);
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Builds a String array from string values.
fn build_string_array(
    values: &[&str],
    nullable: bool,
    null_value: &Option<String>,
) -> Result<ArrayRef, ExportError> {
    let mut builder =
        StringBuilder::with_capacity(values.len(), values.iter().map(|s| s.len()).sum());

    for value in values.iter() {
        if is_null_value(value, null_value) {
            if nullable {
                builder.append_null();
            } else {
                builder.append_value("");
            }
        } else {
            builder.append_value(value);
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Builds a Date32 array from string values (YYYY-MM-DD format).
fn build_date32_array(
    values: &[&str],
    nullable: bool,
    null_value: &Option<String>,
    start_row: usize,
    column: usize,
) -> Result<ArrayRef, ExportError> {
    let mut builder = Date32Builder::with_capacity(values.len());

    for (i, value) in values.iter().enumerate() {
        if is_null_value(value, null_value) {
            if nullable {
                builder.append_null();
            } else {
                return Err(ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: "NULL value in non-nullable column".to_string(),
                });
            }
        } else {
            let days = parse_date_to_days(value).map_err(|e| ExportError::TypeConversionError {
                row: start_row + i,
                column,
                message: e,
            })?;
            builder.append_value(days);
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Builds a Timestamp array from string values (YYYY-MM-DD HH:MM:SS format).
fn build_timestamp_array(
    values: &[&str],
    nullable: bool,
    null_value: &Option<String>,
    start_row: usize,
    column: usize,
) -> Result<ArrayRef, ExportError> {
    let mut builder = TimestampMicrosecondBuilder::with_capacity(values.len());

    for (i, value) in values.iter().enumerate() {
        if is_null_value(value, null_value) {
            if nullable {
                builder.append_null();
            } else {
                return Err(ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: "NULL value in non-nullable column".to_string(),
                });
            }
        } else {
            let micros =
                parse_timestamp_to_micros(value).map_err(|e| ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: e,
                })?;
            builder.append_value(micros);
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Builds a Decimal128 array from string values.
fn build_decimal128_array(
    values: &[&str],
    precision: u8,
    scale: i8,
    nullable: bool,
    null_value: &Option<String>,
    start_row: usize,
    column: usize,
) -> Result<ArrayRef, ExportError> {
    let mut builder = Decimal128Builder::with_capacity(values.len())
        .with_precision_and_scale(precision, scale)
        .map_err(|e| ExportError::ArrowError(e.to_string()))?;

    for (i, value) in values.iter().enumerate() {
        if is_null_value(value, null_value) {
            if nullable {
                builder.append_null();
            } else {
                return Err(ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: "NULL value in non-nullable column".to_string(),
                });
            }
        } else {
            let decimal = parse_decimal_to_i128(value, scale).map_err(|e| {
                ExportError::TypeConversionError {
                    row: start_row + i,
                    column,
                    message: e,
                }
            })?;
            builder.append_value(decimal);
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Maps Exasol types to Arrow DataTypes.
pub fn exasol_type_to_arrow(exasol_type: &ExasolType) -> Result<DataType, ExportError> {
    exasol_type_to_arrow_impl(exasol_type).map_err(ExportError::SchemaError)
}

/// Writes Arrow RecordBatches to Arrow IPC File format (with footer).
///
/// # Arguments
///
/// * `writer` - The async writer to write to
/// * `schema` - The Arrow schema
/// * `batches` - Iterator of RecordBatches to write
///
/// # Returns
///
/// The total number of rows written.
pub async fn write_arrow_ipc_file<W, I>(
    writer: &mut W,
    schema: Arc<Schema>,
    batches: I,
) -> Result<u64, ExportError>
where
    W: AsyncWrite + Unpin + Send,
    I: IntoIterator<Item = Result<RecordBatch, ExportError>>,
{
    use arrow::ipc::writer::FileWriter;
    use std::io::Cursor;

    let mut total_rows = 0u64;

    // We need to use synchronous Arrow IPC writer, then write to async
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut ipc_writer = FileWriter::try_new(&mut buffer, &schema)
            .map_err(|e| ExportError::ArrowError(e.to_string()))?;

        for batch_result in batches {
            let batch = batch_result?;
            total_rows += batch.num_rows() as u64;
            ipc_writer
                .write(&batch)
                .map_err(|e| ExportError::ArrowError(e.to_string()))?;
        }

        ipc_writer
            .finish()
            .map_err(|e| ExportError::ArrowError(e.to_string()))?;
    }

    // Write the buffer to the async writer
    let data = buffer.into_inner();
    writer
        .write_all(&data)
        .await
        .map_err(|e| ExportError::IoError(e.to_string()))?;
    writer
        .flush()
        .await
        .map_err(|e| ExportError::IoError(e.to_string()))?;

    Ok(total_rows)
}

// =============================================================================
// Transport-integrated export functions
// =============================================================================

use crate::query::export::ExportSource;
use crate::transport::TransportProtocol;

/// Exports data from an Exasol table or query to Arrow RecordBatches.
///
/// This function exports data from Exasol (table or query) and converts it
/// to Arrow RecordBatches.
///
/// # Arguments
///
/// * `transport` - Transport for executing SQL
/// * `source` - The data source (table or query)
/// * `options` - Export options
///
/// # Returns
///
/// A vector of RecordBatches on success.
///
/// # Errors
///
/// Returns `ExportError` if the export fails.
pub async fn export_to_record_batches<T: TransportProtocol + ?Sized>(
    transport: &mut T,
    source: ExportSource,
    options: ArrowExportOptions,
) -> Result<Vec<RecordBatch>, crate::export::csv::ExportError> {
    use crate::export::csv::{export_to_list, CsvExportOptions};

    // First, get the data as CSV via the existing export function
    let csv_options = CsvExportOptions::default()
        .column_separator(options.column_separator)
        .column_delimiter(options.column_delimiter)
        .with_column_names(false)
        .exasol_host(&options.host)
        .exasol_port(options.port)
        .use_tls(options.use_tls);

    // Get the CSV data as a list of rows
    let rows = export_to_list(transport, source, csv_options).await?;

    // If schema is provided, use it; otherwise return empty result
    let schema = match options.schema {
        Some(s) => s,
        None => {
            // Without a schema, we can't build RecordBatches
            return Err(crate::export::csv::ExportError::CsvParseError {
                row: 0,
                message: "Schema required for Arrow export".to_string(),
            });
        }
    };

    // Convert the rows to RecordBatches
    let mut batches = Vec::new();
    for chunk in rows.chunks(options.batch_size) {
        let arrays: Result<Vec<ArrayRef>, ExportError> = (0..schema.fields().len())
            .map(|col_idx| {
                let field = schema.field(col_idx);
                let values: Vec<&str> = chunk
                    .iter()
                    .map(|row| row.get(col_idx).map(|s| s.as_str()).unwrap_or(""))
                    .collect();

                build_array_from_strings(
                    &values,
                    field.data_type(),
                    field.is_nullable(),
                    &options.null_value,
                    0,
                    col_idx,
                )
            })
            .collect();

        let arrays = arrays.map_err(|e| crate::export::csv::ExportError::CsvParseError {
            row: 0,
            message: e.to_string(),
        })?;

        let batch = RecordBatch::try_new(Arc::clone(&schema), arrays).map_err(|e| {
            crate::export::csv::ExportError::CsvParseError {
                row: 0,
                message: e.to_string(),
            }
        })?;

        batches.push(batch);
    }

    Ok(batches)
}

/// Exports data from an Exasol table or query to an Arrow IPC file.
///
/// This function exports data from Exasol (table or query) to an Arrow IPC file.
///
/// # Arguments
///
/// * `transport` - Transport for executing SQL
/// * `source` - The data source (table or query)
/// * `file_path` - Path to the output Arrow IPC file
/// * `options` - Export options
///
/// # Returns
///
/// The number of rows exported on success.
///
/// # Errors
///
/// Returns `ExportError` if the export fails.
pub async fn export_to_arrow_ipc<T: TransportProtocol + ?Sized>(
    transport: &mut T,
    source: ExportSource,
    file_path: &std::path::Path,
    options: ArrowExportOptions,
) -> Result<u64, crate::export::csv::ExportError> {
    // Get the batches
    let batches = export_to_record_batches(transport, source, options.clone()).await?;

    // Get the schema
    let schema = options
        .schema
        .ok_or_else(|| crate::export::csv::ExportError::CsvParseError {
            row: 0,
            message: "Schema required for Arrow IPC export".to_string(),
        })?;

    // Write to the file
    let file = tokio::fs::File::create(file_path).await?;

    let mut file = tokio::io::BufWriter::new(file);

    let batch_results: Vec<Result<RecordBatch, ExportError>> =
        batches.into_iter().map(Ok).collect();

    let rows = write_arrow_ipc_file(&mut file, schema, batch_results)
        .await
        .map_err(|e| crate::export::csv::ExportError::CsvParseError {
            row: 0,
            message: e.to_string(),
        })?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::Field;
    use tokio::io::BufReader;

    // ==========================================================================
    // Tests for ArrowExportOptions
    // ==========================================================================

    #[test]
    fn test_arrow_export_options_default() {
        let options = ArrowExportOptions::default();
        assert_eq!(options.batch_size, 1024);
        assert!(options.null_value.is_none());
        assert!(options.schema.is_none());
        assert_eq!(options.column_separator, ',');
        assert_eq!(options.column_delimiter, '"');
        assert_eq!(options.host, "");
        assert_eq!(options.port, 0);
        assert!(!options.use_tls);
    }

    #[test]
    fn test_arrow_export_options_builder() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let options = ArrowExportOptions::new()
            .with_batch_size(2048)
            .with_null_value("NULL")
            .with_schema(Arc::clone(&schema))
            .with_column_separator(';')
            .with_column_delimiter('\'')
            .exasol_host("exasol.example.com")
            .exasol_port(8563)
            .use_tls(true);

        assert_eq!(options.batch_size, 2048);
        assert_eq!(options.null_value, Some("NULL".to_string()));
        assert!(options.schema.is_some());
        assert_eq!(options.column_separator, ';');
        assert_eq!(options.column_delimiter, '\'');
        assert_eq!(options.host, "exasol.example.com");
        assert_eq!(options.port, 8563);
        assert!(options.use_tls);
    }

    #[test]
    fn test_arrow_export_options_use_tls_builder() {
        assert!(ArrowExportOptions::default().use_tls(true).use_tls);
        assert!(!ArrowExportOptions::default().use_tls(false).use_tls);
    }

    // ==========================================================================
    // Tests for CSV parsing
    // ==========================================================================

    use crate::export::csv::parse_csv_row;

    #[test]
    fn test_parse_csv_row_simple() {
        let line = "1,hello,world";
        let (fields, in_quotes) = parse_csv_row(line, ',', '"');
        assert!(!in_quotes);
        assert_eq!(fields, vec!["1", "hello", "world"]);
    }

    #[test]
    fn test_parse_csv_row_quoted() {
        let line = r#"1,"hello, world","test""#;
        let (fields, in_quotes) = parse_csv_row(line, ',', '"');
        assert!(!in_quotes);
        assert_eq!(fields, vec!["1", "hello, world", "test"]);
    }

    #[test]
    fn test_parse_csv_row_escaped_quote() {
        let line = r#"1,"hello ""world""","test""#;
        let (fields, in_quotes) = parse_csv_row(line, ',', '"');
        assert!(!in_quotes);
        assert_eq!(fields, vec!["1", r#"hello "world""#, "test"]);
    }

    #[test]
    fn test_parse_csv_row_empty_fields() {
        let line = "1,,3";
        let (fields, in_quotes) = parse_csv_row(line, ',', '"');
        assert!(!in_quotes);
        assert_eq!(fields, vec!["1", "", "3"]);
    }

    #[test]
    fn test_parse_csv_row_custom_separator() {
        let line = "1;hello;world";
        let (fields, in_quotes) = parse_csv_row(line, ';', '"');
        assert!(!in_quotes);
        assert_eq!(fields, vec!["1", "hello", "world"]);
    }

    #[test]
    fn test_parse_csv_row_unclosed_quote_error() {
        let line = r#"1,"hello"#;
        let (_, in_quotes) = parse_csv_row(line, ',', '"');
        assert!(in_quotes, "unterminated quote should be reported");
    }

    // ==========================================================================
    // Tests for NULL detection
    // ==========================================================================

    #[test]
    fn test_is_null_value_empty() {
        assert!(is_null_value("", &None));
        assert!(is_null_value("", &Some("NULL".to_string())));
    }

    #[test]
    fn test_is_null_value_custom() {
        assert!(is_null_value("NULL", &Some("NULL".to_string())));
        assert!(!is_null_value("null", &Some("NULL".to_string())));
        assert!(!is_null_value("value", &Some("NULL".to_string())));
    }

    #[test]
    fn test_is_null_value_no_custom() {
        assert!(!is_null_value("NULL", &None));
        assert!(!is_null_value("value", &None));
    }

    // ==========================================================================
    // Tests for date parsing
    // ==========================================================================

    #[test]
    fn test_parse_date_to_days_epoch() {
        let days = parse_date_to_days("1970-01-01").unwrap();
        assert_eq!(days, 0);
    }

    #[test]
    fn test_parse_date_to_days_after_epoch() {
        let days = parse_date_to_days("1970-01-02").unwrap();
        assert_eq!(days, 1);
    }

    #[test]
    fn test_parse_date_to_days_before_epoch() {
        let days = parse_date_to_days("1969-12-31").unwrap();
        assert_eq!(days, -1);
    }

    #[test]
    fn test_parse_date_to_days_leap_year() {
        // 2000 is a leap year
        let mar1_2000 = parse_date_to_days("2000-03-01").unwrap();
        let feb28_2000 = parse_date_to_days("2000-02-28").unwrap();
        assert_eq!(mar1_2000 - feb28_2000, 2); // Feb 29 exists
    }

    #[test]
    fn test_parse_date_to_days_invalid_format() {
        assert!(parse_date_to_days("2024/01/15").is_err());
        assert!(parse_date_to_days("2024-01").is_err());
        assert!(parse_date_to_days("invalid").is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_values() {
        assert!(parse_date_to_days("2024-13-01").is_err()); // Invalid month
        assert!(parse_date_to_days("2024-01-32").is_err()); // Invalid day
        assert!(parse_date_to_days("2024-00-15").is_err()); // Month 0
    }

    // ==========================================================================
    // Tests for timestamp parsing
    // ==========================================================================

    #[test]
    fn test_parse_timestamp_to_micros_epoch() {
        let micros = parse_timestamp_to_micros("1970-01-01 00:00:00").unwrap();
        assert_eq!(micros, 0);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_time() {
        let micros = parse_timestamp_to_micros("1970-01-01 00:00:01").unwrap();
        assert_eq!(micros, 1_000_000);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_fractional() {
        let micros = parse_timestamp_to_micros("1970-01-01 00:00:00.123456").unwrap();
        assert_eq!(micros, 123_456);
    }

    #[test]
    fn test_parse_timestamp_to_micros_date_only() {
        let micros = parse_timestamp_to_micros("1970-01-02").unwrap();
        assert_eq!(micros, 86400 * 1_000_000);
    }

    // ==========================================================================
    // Tests for decimal parsing
    // ==========================================================================

    #[test]
    fn test_parse_decimal_to_i128_integer() {
        let result = parse_decimal_to_i128("123", 2).unwrap();
        assert_eq!(result, 12300); // 123 * 10^2
    }

    #[test]
    fn test_parse_decimal_to_i128_with_fraction() {
        let result = parse_decimal_to_i128("123.45", 2).unwrap();
        assert_eq!(result, 12345);
    }

    #[test]
    fn test_parse_decimal_to_i128_negative() {
        let result = parse_decimal_to_i128("-123.45", 2).unwrap();
        assert_eq!(result, -12345);
    }

    #[test]
    fn test_parse_decimal_to_i128_short_fraction() {
        let result = parse_decimal_to_i128("123.4", 2).unwrap();
        assert_eq!(result, 12340);
    }

    #[test]
    fn test_parse_decimal_to_i128_invalid() {
        assert!(parse_decimal_to_i128("abc", 2).is_err());
        assert!(parse_decimal_to_i128("1.2.3", 2).is_err());
    }

    // ==========================================================================
    // Tests for array builders
    // ==========================================================================

    #[test]
    fn test_build_boolean_array() {
        let values = vec!["true", "false", "", "1", "0"];
        let null_value = None;
        let array = build_boolean_array(&values, true, &null_value, 0, 0).unwrap();

        assert_eq!(array.len(), 5);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_boolean_array_invalid() {
        let values = vec!["invalid"];
        let null_value = None;
        let result = build_boolean_array(&values, true, &null_value, 0, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_int64_array() {
        let values = vec!["1", "2", "", "3"];
        let null_value = None;
        let array = build_int64_array(&values, true, &null_value, 0, 0).unwrap();

        assert_eq!(array.len(), 4);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_float64_array() {
        let values = vec!["1.5", "2.5", "Infinity", "-Infinity", "NaN", ""];
        let null_value = None;
        let array = build_float64_array(&values, true, &null_value, 0, 0).unwrap();

        assert_eq!(array.len(), 6);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_string_array() {
        let values = vec!["hello", "world", "", "test"];
        let null_value = None;
        let array = build_string_array(&values, true, &null_value).unwrap();

        assert_eq!(array.len(), 4);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_date32_array() {
        let values = vec!["2024-01-15", "2024-06-20", ""];
        let null_value = None;
        let array = build_date32_array(&values, true, &null_value, 0, 0).unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_timestamp_array() {
        let values = vec!["2024-01-15 10:30:00", "2024-06-20 14:45:30.123456", ""];
        let null_value = None;
        let array = build_timestamp_array(&values, true, &null_value, 0, 0).unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_decimal128_array() {
        let values = vec!["123.45", "678.90", ""];
        let null_value = None;
        let array = build_decimal128_array(&values, 10, 2, true, &null_value, 0, 0).unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    // ==========================================================================
    // Tests for schema building
    // ==========================================================================

    #[test]
    fn test_exasol_type_to_arrow() {
        assert_eq!(
            exasol_type_to_arrow(&ExasolType::Boolean).unwrap(),
            DataType::Boolean
        );
        assert_eq!(
            exasol_type_to_arrow(&ExasolType::Varchar { size: 100 }).unwrap(),
            DataType::Utf8
        );
        assert_eq!(
            exasol_type_to_arrow(&ExasolType::Double).unwrap(),
            DataType::Float64
        );
        assert_eq!(
            exasol_type_to_arrow(&ExasolType::Date).unwrap(),
            DataType::Date32
        );
    }

    // ==========================================================================
    // Tests for CsvToArrowReader
    // ==========================================================================

    #[tokio::test]
    async fn test_csv_to_arrow_reader_simple() {
        let csv_data = "1,hello,true\n2,world,false\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("active", DataType::Boolean, false),
        ]));

        let options = ArrowExportOptions::default().with_batch_size(10);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        let batch = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);

        // Should return None on next call
        assert!(arrow_reader.next_batch().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_with_nulls() {
        let csv_data = "1,hello\n2,\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        let options = ArrowExportOptions::default();
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        let batch = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.column(1).null_count(), 1);
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_batching() {
        let csv_data = "1\n2\n3\n4\n5\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let options = ArrowExportOptions::default().with_batch_size(2);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        // First batch: 2 rows
        let batch1 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch1.num_rows(), 2);

        // Second batch: 2 rows
        let batch2 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch2.num_rows(), 2);

        // Third batch: 1 row
        let batch3 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch3.num_rows(), 1);

        // No more data
        assert!(arrow_reader.next_batch().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_empty() {
        let csv_data = "";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let options = ArrowExportOptions::default();
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        assert!(arrow_reader.next_batch().await.unwrap().is_none());
    }

    // ==========================================================================
    // Tests for Arrow IPC writing
    // ==========================================================================

    #[tokio::test]
    async fn test_write_arrow_ipc_file() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(arrow::array::Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();

        let mut buffer = Vec::new();
        let rows = write_arrow_ipc_file(&mut buffer, schema, vec![Ok(batch)])
            .await
            .unwrap();

        assert_eq!(rows, 3);
        assert!(!buffer.is_empty());
        // IPC file format has ARROW1 magic at start
        assert_eq!(&buffer[0..6], b"ARROW1");
    }

    // ==========================================================================
    // Tests for type conversion edge cases
    // ==========================================================================

    #[test]
    fn test_boolean_conversion_variants() {
        let values = vec!["true", "false", "TRUE", "FALSE", "True", "False"];
        let null_value = None;
        let array = build_boolean_array(&values, false, &null_value, 0, 0).unwrap();
        assert_eq!(array.len(), 6);

        let values = vec!["1", "0", "t", "f", "yes", "no", "y", "n"];
        let array = build_boolean_array(&values, false, &null_value, 0, 0).unwrap();
        assert_eq!(array.len(), 8);
    }

    #[test]
    fn test_non_nullable_null_error() {
        let values = vec![""];
        let null_value = None;
        let result = build_int64_array(&values, false, &null_value, 0, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            ExportError::TypeConversionError { message, .. } => {
                assert!(message.contains("NULL value in non-nullable column"));
            }
            _ => panic!("Expected TypeConversionError"),
        }
    }

    #[test]
    fn test_custom_null_value() {
        let values = vec!["1", "NULL", "3"];
        let null_value = Some("NULL".to_string());
        let array = build_int64_array(&values, true, &null_value, 0, 0).unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    // ==========================================================================
    // Tests for schema derivation from Exasol metadata
    // ==========================================================================

    #[test]
    fn test_exasol_type_to_arrow_decimal() {
        let result = exasol_type_to_arrow(&ExasolType::Decimal {
            precision: 18,
            scale: 4,
        })
        .unwrap();
        assert_eq!(result, DataType::Decimal128(18, 4));
    }

    #[test]
    fn test_exasol_type_to_arrow_char() {
        let result = exasol_type_to_arrow(&ExasolType::Char { size: 50 }).unwrap();
        assert_eq!(result, DataType::Utf8);
    }

    #[test]
    fn test_exasol_type_to_arrow_timestamp() {
        let result = exasol_type_to_arrow(&ExasolType::Timestamp {
            with_local_time_zone: false,
        })
        .unwrap();
        assert_eq!(result, DataType::Timestamp(TimeUnit::Microsecond, None));
    }

    #[test]
    fn test_exasol_type_to_arrow_timestamp_with_tz() {
        let result = exasol_type_to_arrow(&ExasolType::Timestamp {
            with_local_time_zone: true,
        })
        .unwrap();
        assert_eq!(
            result,
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
    }

    #[test]
    fn test_exasol_type_to_arrow_interval_year_to_month() {
        let result = exasol_type_to_arrow(&ExasolType::IntervalYearToMonth).unwrap();
        assert_eq!(result, DataType::Int64);
    }

    #[test]
    fn test_exasol_type_to_arrow_interval_day_to_second() {
        let result =
            exasol_type_to_arrow(&ExasolType::IntervalDayToSecond { precision: 3 }).unwrap();
        assert_eq!(result, DataType::Int64);
    }

    #[test]
    fn test_exasol_type_to_arrow_geometry() {
        let result = exasol_type_to_arrow(&ExasolType::Geometry { srid: Some(4326) }).unwrap();
        assert_eq!(result, DataType::Binary);
    }

    #[test]
    fn test_exasol_type_to_arrow_hashtype() {
        let result = exasol_type_to_arrow(&ExasolType::Hashtype { byte_size: 32 }).unwrap();
        assert_eq!(result, DataType::Binary);
    }

    // ==========================================================================
    // Tests for configurable batch size
    // ==========================================================================

    #[tokio::test]
    async fn test_csv_to_arrow_reader_batch_size_1() {
        // Test with batch size 1 - should produce one batch per row
        let csv_data = "1,a\n2,b\n3,c\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));

        let options = ArrowExportOptions::default().with_batch_size(1);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        // Should get 3 batches, each with 1 row
        let batch1 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch1.num_rows(), 1);

        let batch2 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch2.num_rows(), 1);

        let batch3 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch3.num_rows(), 1);

        assert!(arrow_reader.next_batch().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_batch_size_larger_than_data() {
        // Test with batch size larger than available data
        let csv_data = "1,a\n2,b\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));

        let options = ArrowExportOptions::default().with_batch_size(1000);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        // Should get 1 batch with all 2 rows
        let batch = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2);

        assert!(arrow_reader.next_batch().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_batch_size_exact_multiple() {
        // Test with batch size that divides data exactly
        let csv_data = "1\n2\n3\n4\n5\n6\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let options = ArrowExportOptions::default().with_batch_size(3);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        // Should get 2 batches, each with 3 rows
        let batch1 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch1.num_rows(), 3);

        let batch2 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch2.num_rows(), 3);

        assert!(arrow_reader.next_batch().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_batch_size_with_partial_last_batch() {
        // Test batch size that doesn't divide data evenly
        let csv_data = "1\n2\n3\n4\n5\n6\n7\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let options = ArrowExportOptions::default().with_batch_size(3);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        // Should get 3 batches: 3 + 3 + 1
        let batch1 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch1.num_rows(), 3);

        let batch2 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch2.num_rows(), 3);

        let batch3 = arrow_reader.next_batch().await.unwrap().unwrap();
        assert_eq!(batch3.num_rows(), 1);

        assert!(arrow_reader.next_batch().await.unwrap().is_none());
    }

    #[test]
    fn test_arrow_export_options_batch_size_default() {
        let options = ArrowExportOptions::default();
        assert_eq!(options.batch_size, 1024);
    }

    #[test]
    fn test_arrow_export_options_batch_size_custom() {
        let options = ArrowExportOptions::default().with_batch_size(500);
        assert_eq!(options.batch_size, 500);
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_total_row_count_across_batches() {
        // Verify that total rows across all batches equals input rows
        let csv_data = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let options = ArrowExportOptions::default().with_batch_size(3);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        let mut total_rows = 0;
        while let Some(batch) = arrow_reader.next_batch().await.unwrap() {
            total_rows += batch.num_rows();
        }

        assert_eq!(total_rows, 10);
    }

    #[tokio::test]
    async fn test_csv_to_arrow_reader_preserves_data_across_batches() {
        // Verify data integrity across batches
        let csv_data = "1\n2\n3\n4\n5\n";
        let reader = BufReader::new(csv_data.as_bytes());

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let options = ArrowExportOptions::default().with_batch_size(2);
        let mut arrow_reader = CsvToArrowReader::from_buffered(reader, schema, &options);

        // Collect all values
        let mut all_values: Vec<i64> = Vec::new();
        while let Some(batch) = arrow_reader.next_batch().await.unwrap() {
            let array = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::Int64Array>()
                .unwrap();
            for i in 0..array.len() {
                all_values.push(array.value(i));
            }
        }

        assert_eq!(all_values, vec![1, 2, 3, 4, 5]);
    }
}
