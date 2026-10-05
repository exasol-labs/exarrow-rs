//! Parquet export functionality for exarrow-rs.
//!
//! This module provides functionality to export data from Exasol to Parquet format.
//! Exasol exports data as CSV via HTTP transport; this module converts that CSV
//! to Arrow RecordBatches and then writes Parquet files.
//!
//! # Architecture
//!
//! The transport export ([`export_to_parquet_via_transport`]) runs these steps:
//!
//! 1. Prepare the export source's SELECT statement, derive the Arrow schema from its
//!    result-set metadata, and close the prepared statement
//! 2. Execute EXPORT SQL to receive CSV via HTTP transport
//! 3. Convert the CSV rows to typed Arrow RecordBatches
//! 4. Write RecordBatches to Parquet format
//!
//! The CSV-bytes entry points ([`export_to_parquet`], [`export_to_parquet_stream`],
//! [`csv_to_record_batches`]) skip steps 1 and 2: the caller supplies the CSV and the Arrow
//! schema, and the same conversion and writing steps follow.
//!
//! # Example
//!
//! ```ignore
//! use exarrow_rs::export::parquet::{export_to_parquet, ParquetExportOptions, ExportSource};
//! use std::path::Path;
//!
//! // Export a table to Parquet
//! let rows = export_to_parquet(
//!     &mut session,
//!     ExportSource::Table { schema: None, name: "users".into(), columns: vec![] },
//!     Path::new("/tmp/users.parquet"),
//!     ParquetExportOptions::default(),
//! ).await?;
//! ```

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, StringBuilder,
    TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression as ParquetCompressionCodec, Encoding};
use parquet::file::properties::WriterProperties;
use thiserror::Error;

use crate::types::{
    conversion::{
        exasol_type_to_arrow as exasol_type_to_arrow_impl,
        parse_date_to_days as parse_date_to_days_impl,
        parse_decimal_to_i128 as parse_decimal_to_i128_impl,
        parse_timestamp_to_micros as parse_timestamp_to_micros_impl,
    },
    ExasolType,
};

/// Errors that can occur during Parquet export operations.
#[derive(Error, Debug)]
pub enum ParquetExportError {
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Parquet error
    #[error("Parquet error: {0}")]
    Parquet(String),

    /// Arrow error
    #[error("Arrow error: {0}")]
    Arrow(String),

    /// CSV parsing error
    #[error("CSV parsing error at row {row}: {message}")]
    CsvParse { row: usize, message: String },

    /// Schema error
    #[error("Schema error: {0}")]
    Schema(String),
}

impl From<arrow::error::ArrowError> for ParquetExportError {
    fn from(err: arrow::error::ArrowError) -> Self {
        ParquetExportError::Arrow(err.to_string())
    }
}

impl From<parquet::errors::ParquetError> for ParquetExportError {
    fn from(err: parquet::errors::ParquetError) -> Self {
        ParquetExportError::Parquet(err.to_string())
    }
}

/// Compression options for Parquet export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParquetCompression {
    /// No compression
    None,
    /// Snappy compression (fast, moderate ratio)
    #[default]
    Snappy,
    /// Gzip compression (slower, better ratio)
    Gzip,
    /// LZ4 compression (very fast, lower ratio)
    Lz4,
    /// Zstd compression (good balance of speed and ratio)
    Zstd,
}

impl ParquetCompression {
    /// Convert to parquet compression codec.
    fn to_codec(self) -> ParquetCompressionCodec {
        match self {
            ParquetCompression::None => ParquetCompressionCodec::UNCOMPRESSED,
            ParquetCompression::Snappy => ParquetCompressionCodec::SNAPPY,
            ParquetCompression::Gzip => ParquetCompressionCodec::GZIP(Default::default()),
            ParquetCompression::Lz4 => ParquetCompressionCodec::LZ4,
            ParquetCompression::Zstd => ParquetCompressionCodec::ZSTD(Default::default()),
        }
    }
}

/// Options for Parquet export.
#[derive(Debug, Clone)]
pub struct ParquetExportOptions {
    /// Number of rows per batch (default: 1024)
    pub batch_size: usize,
    /// Compression type (default: Snappy)
    pub compression: ParquetCompression,
    /// Whether CSV has column names header row (default: true)
    pub with_column_names: bool,
    /// Column separator for CSV parsing (default: ',')
    pub column_separator: char,
    /// Column delimiter for CSV parsing (default: '"')
    pub column_delimiter: char,
    /// NULL marker of the CSV-bytes entry points (default: an empty field is NULL).
    ///
    /// The transport export ignores it and reads an empty field as NULL, because the driver
    /// passes no NULL clause to the EXPORT statement.
    pub null_value: Option<String>,
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

impl Default for ParquetExportOptions {
    fn default() -> Self {
        Self {
            batch_size: 1024,
            compression: ParquetCompression::default(),
            with_column_names: true,
            column_separator: ',',
            column_delimiter: '"',
            null_value: None,
            host: String::new(),
            port: 0,
            use_tls: false,
        }
    }
}

impl ParquetExportOptions {
    /// Create a new `ParquetExportOptions` with default values.
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
    pub fn with_compression(mut self, compression: ParquetCompression) -> Self {
        self.compression = compression;
        self
    }

    #[must_use]
    pub fn with_column_names(mut self, with_column_names: bool) -> Self {
        self.with_column_names = with_column_names;
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

    #[must_use]
    pub fn with_null_value(mut self, null_value: impl Into<String>) -> Self {
        self.null_value = Some(null_value.into());
        self
    }

    /// Set the Exasol host for HTTP transport connection.
    ///
    /// This should be the same host as used for the WebSocket connection.
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

    /// Sets whether to use TLS for the HTTP transport tunnel.
    #[must_use]
    pub fn use_tls(mut self, v: bool) -> Self {
        self.use_tls = v;
        self
    }
}

/// Convert caller-supplied CSV bytes to a Parquet file.
///
/// The CSV is parsed against `schema` and written to `file_path`. Fields are kept verbatim,
/// without trimming. An empty field is NULL when `options.null_value` is unset. Otherwise only a
/// field equal to `null_value` is NULL. The file is created before the CSV is converted, so a
/// conversion error leaves it behind.
///
/// # Arguments
///
/// * `csv_data` - CSV data as bytes (simulating what would come from HTTP transport)
/// * `schema` - Arrow schema for the data
/// * `file_path` - Path to write the Parquet file
/// * `options` - Export options
///
/// # Returns
///
/// The number of rows exported.
///
/// # Errors
///
/// Returns `ParquetExportError` if:
/// - CSV parsing fails
/// - Arrow conversion fails
/// - Parquet writing fails
/// - IO operations fail
pub async fn export_to_parquet(
    csv_data: &[u8],
    schema: Arc<Schema>,
    file_path: &Path,
    options: ParquetExportOptions,
) -> Result<u64, ParquetExportError> {
    let file = std::fs::File::create(file_path)?;
    export_to_parquet_stream(csv_data, schema, file, options).await
}

/// Convert caller-supplied CSV bytes to a Parquet stream.
///
/// This function writes to any writer implementing `Write`, such as a network stream or an
/// in-memory buffer. It does not trim fields: whitespace around a value is kept, and a typed
/// field with surrounding whitespace fails the conversion. An empty field is NULL when
/// `options.null_value` is unset. Otherwise only a field equal to `null_value` is NULL.
///
/// # Arguments
///
/// * `csv_data` - CSV data as bytes (simulating what would come from HTTP transport)
/// * `schema` - Arrow schema for the data
/// * `writer` - Any type implementing `Write`
/// * `options` - Export options
///
/// # Returns
///
/// The number of rows exported.
///
/// # Errors
///
/// Returns `ParquetExportError` if:
/// - CSV parsing fails
/// - Arrow conversion fails
/// - Parquet writing fails
pub async fn export_to_parquet_stream<W: Write + Send>(
    csv_data: &[u8],
    schema: Arc<Schema>,
    writer: W,
    options: ParquetExportOptions,
) -> Result<u64, ParquetExportError> {
    let batches = csv_to_record_batches(csv_data, &schema, &options)?;
    let total_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();

    write_parquet(writer, &schema, &batches, options.compression)?;

    Ok(total_rows)
}

/// Writes `batches` as one Parquet file to `writer`; an empty list yields a file with no rows.
fn write_parquet<W: Write + Send>(
    writer: W,
    schema: &Arc<Schema>,
    batches: &[RecordBatch],
    compression: ParquetCompression,
) -> Result<(), ParquetExportError> {
    let props = WriterProperties::builder()
        .set_compression(compression.to_codec())
        .set_encoding(Encoding::PLAIN)
        .build();

    let mut parquet_writer = ArrowWriter::try_new(writer, Arc::clone(schema), Some(props))?;
    for batch in batches {
        parquet_writer.write(batch)?;
    }
    parquet_writer.close()?;

    Ok(())
}

/// Convert CSV data to Arrow RecordBatches.
///
/// Fields are kept verbatim, including surrounding whitespace. A quoted field may contain the
/// column separator, the column delimiter, and line breaks. An empty field is NULL when
/// `options.null_value` is unset. Otherwise only a field equal to `null_value` is NULL.
///
/// # Arguments
///
/// * `csv_data` - CSV data as bytes
/// * `schema` - Arrow schema for the data
/// * `options` - Export options containing CSV parsing settings
///
/// # Returns
///
/// A vector of RecordBatches, holding one empty batch when the input has no data rows.
///
/// # Errors
///
/// Returns `ParquetExportError::CsvParse` when the input is not UTF-8, ends inside a quoted
/// field, or has a row with the wrong number of fields or a value its column cannot hold. Its
/// `row` is the 0-based index of the data row in the whole input, header excluded.
pub fn csv_to_record_batches(
    csv_data: &[u8],
    schema: &Schema,
    options: &ParquetExportOptions,
) -> Result<Vec<RecordBatch>, ParquetExportError> {
    let csv_str = std::str::from_utf8(csv_data).map_err(|e| ParquetExportError::CsvParse {
        row: 0,
        message: format!("Invalid UTF-8: {}", e),
    })?;

    let header_rows = usize::from(options.with_column_names);
    let mut rows =
        crate::export::csv::csv_rows(csv_str, options.column_separator, options.column_delimiter);
    if options.with_column_names {
        if let Some(Err(e)) = rows.next() {
            return Err(csv_parse_error(e, header_rows));
        }
    }
    let data_rows = rows.map(|row| row.map_err(|e| csv_parse_error(e, header_rows)));

    rows_to_record_batches(
        data_rows,
        &Arc::new(schema.clone()),
        options.null_value.as_deref(),
        options.batch_size,
    )
}

fn csv_parse_error(
    error: crate::export::csv::ExportError,
    header_rows: usize,
) -> ParquetExportError {
    match error {
        crate::export::csv::ExportError::CsvParseError { row, message } => {
            ParquetExportError::CsvParse {
                row: row.saturating_sub(header_rows),
                message,
            }
        }
        other => ParquetExportError::Arrow(other.to_string()),
    }
}

/// Convert parsed rows into RecordBatches of at most `batch_size` rows.
///
/// Pulls one chunk of rows at a time and drops it before pulling the next, so memory holds one
/// chunk of parsed fields. A field equal to `null_value` is NULL, and with no `null_value` an
/// empty field is NULL. Every other field is kept verbatim.
///
/// A returned `CsvParse` error carries the 0-based index of the failing row in `rows`.
fn rows_to_record_batches(
    rows: impl IntoIterator<Item = Result<Vec<String>, ParquetExportError>>,
    schema: &Arc<Schema>,
    null_value: Option<&str>,
    batch_size: usize,
) -> Result<Vec<RecordBatch>, ParquetExportError> {
    let mut rows = rows.into_iter();
    let num_columns = schema.fields().len();
    let mut batches = Vec::new();
    let mut first_row_of_chunk = 0;

    loop {
        let mut chunk: Vec<Vec<String>> = Vec::new();
        for row in rows.by_ref().take(batch_size.max(1)) {
            let row = row?;
            if row.len() != num_columns {
                return Err(ParquetExportError::CsvParse {
                    row: first_row_of_chunk + chunk.len(),
                    message: format!("Expected {} columns, found {}", num_columns, row.len()),
                });
            }
            chunk.push(row);
        }
        if chunk.is_empty() {
            break;
        }

        let batch = chunk_to_record_batch(&chunk, schema, null_value).map_err(|e| match e {
            ParquetExportError::CsvParse { row, message } => ParquetExportError::CsvParse {
                row: first_row_of_chunk + row,
                message,
            },
            other => other,
        })?;
        batches.push(batch);
        first_row_of_chunk += chunk.len();
    }

    if batches.is_empty() {
        batches.push(RecordBatch::new_empty(Arc::clone(schema)));
    }
    Ok(batches)
}

fn chunk_to_record_batch(
    rows: &[Vec<String>],
    schema: &Arc<Schema>,
    null_value: Option<&str>,
) -> Result<RecordBatch, ParquetExportError> {
    let arrays = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(col_idx, field)| {
            let column_values: Vec<Option<&str>> = rows
                .iter()
                .map(|row| field_value(&row[col_idx], null_value))
                .collect();
            build_array_from_csv_column(&column_values, field, col_idx)
        })
        .collect::<Result<Vec<ArrayRef>, _>>()?;

    Ok(RecordBatch::try_new(Arc::clone(schema), arrays)?)
}

fn field_value<'a>(field: &'a str, null_value: Option<&str>) -> Option<&'a str> {
    let is_null = match null_value {
        Some(marker) => field == marker,
        None => field.is_empty(),
    };
    (!is_null).then_some(field)
}

/// Build an Arrow array from CSV column values.
fn build_array_from_csv_column(
    values: &[Option<&str>],
    field: &Field,
    col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    match field.data_type() {
        DataType::Boolean => build_boolean_array_from_csv(values, col_idx),
        DataType::Utf8 => build_string_array_from_csv(values, col_idx),
        DataType::Float64 => build_float64_array_from_csv(values, col_idx),
        DataType::Decimal128(precision, scale) => {
            build_decimal128_array_from_csv(values, *precision, *scale, col_idx)
        }
        DataType::Date32 => build_date32_array_from_csv(values, col_idx),
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            build_timestamp_array_from_csv(values, tz.clone(), col_idx)
        }
        DataType::Int64 => build_int64_array_from_csv(values, col_idx),
        _ => Err(ParquetExportError::Schema(format!(
            "Unsupported data type for Parquet export: {:?}",
            field.data_type()
        ))),
    }
}

/// Build a Boolean array from CSV string values.
fn build_boolean_array_from_csv(
    values: &[Option<&str>],
    col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    let mut builder = BooleanBuilder::with_capacity(values.len());

    for (row_idx, value) in values.iter().enumerate() {
        match value {
            None => builder.append_null(),
            Some(s) => {
                let lower = s.to_lowercase();
                let bool_val = match lower.as_str() {
                    "true" | "1" | "yes" | "t" | "y" => true,
                    "false" | "0" | "no" | "f" | "n" => false,
                    _ => {
                        return Err(ParquetExportError::CsvParse {
                            row: row_idx,
                            message: format!("Invalid boolean value at column {}: {}", col_idx, s),
                        })
                    }
                };
                builder.append_value(bool_val);
            }
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Build a String array from CSV string values.
fn build_string_array_from_csv(
    values: &[Option<&str>],
    _col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    let mut builder = StringBuilder::with_capacity(values.len(), values.len() * 32);

    for value in values {
        match value {
            None => builder.append_null(),
            Some(s) => builder.append_value(s),
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Build a Float64 array from CSV string values.
fn build_float64_array_from_csv(
    values: &[Option<&str>],
    col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    let mut builder = Float64Builder::with_capacity(values.len());

    for (row_idx, value) in values.iter().enumerate() {
        match value {
            None => builder.append_null(),
            Some(s) => {
                let float_val = s.parse::<f64>().map_err(|_| ParquetExportError::CsvParse {
                    row: row_idx,
                    message: format!("Invalid float value at column {}: {}", col_idx, s),
                })?;
                builder.append_value(float_val);
            }
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Build a Decimal128 array from CSV string values.
fn build_decimal128_array_from_csv(
    values: &[Option<&str>],
    precision: u8,
    scale: i8,
    col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    let mut builder = Decimal128Builder::with_capacity(values.len())
        .with_precision_and_scale(precision, scale)
        .map_err(|e| ParquetExportError::Arrow(e.to_string()))?;

    for (row_idx, value) in values.iter().enumerate() {
        match value {
            None => builder.append_null(),
            Some(s) => {
                let decimal_val = parse_decimal_to_i128(s, scale, row_idx, col_idx)?;
                builder.append_value(decimal_val);
            }
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Parse a decimal string to i128.
fn parse_decimal_to_i128(
    value_str: &str,
    scale: i8,
    row_idx: usize,
    col_idx: usize,
) -> Result<i128, ParquetExportError> {
    parse_decimal_to_i128_impl(value_str, scale).map_err(|e| ParquetExportError::CsvParse {
        row: row_idx,
        message: format!("at column {}: {}", col_idx, e),
    })
}

/// Build a Date32 array from CSV string values.
fn build_date32_array_from_csv(
    values: &[Option<&str>],
    col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    let mut builder = Date32Builder::with_capacity(values.len());

    for (row_idx, value) in values.iter().enumerate() {
        match value {
            None => builder.append_null(),
            Some(s) => {
                let days = parse_date_to_days(s, row_idx, col_idx)?;
                builder.append_value(days);
            }
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Parse a date string to days since Unix epoch.
fn parse_date_to_days(
    date_str: &str,
    row_idx: usize,
    col_idx: usize,
) -> Result<i32, ParquetExportError> {
    parse_date_to_days_impl(date_str).map_err(|e| ParquetExportError::CsvParse {
        row: row_idx,
        message: format!("at column {}: {}", col_idx, e),
    })
}

/// Build a Timestamp array from CSV string values.
fn build_timestamp_array_from_csv(
    values: &[Option<&str>],
    _tz: Option<Arc<str>>,
    col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    let mut builder = TimestampMicrosecondBuilder::with_capacity(values.len());

    for (row_idx, value) in values.iter().enumerate() {
        match value {
            None => builder.append_null(),
            Some(s) => {
                let micros = parse_timestamp_to_micros(s, row_idx, col_idx)?;
                builder.append_value(micros);
            }
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Parse a timestamp string to microseconds since Unix epoch.
fn parse_timestamp_to_micros(
    timestamp_str: &str,
    row_idx: usize,
    col_idx: usize,
) -> Result<i64, ParquetExportError> {
    parse_timestamp_to_micros_impl(timestamp_str).map_err(|e| ParquetExportError::CsvParse {
        row: row_idx,
        message: format!("at column {}: {}", col_idx, e),
    })
}

/// Build an Int64 array from CSV string values.
fn build_int64_array_from_csv(
    values: &[Option<&str>],
    col_idx: usize,
) -> Result<ArrayRef, ParquetExportError> {
    use arrow::array::Int64Builder;

    let mut builder = Int64Builder::with_capacity(values.len());

    for (row_idx, value) in values.iter().enumerate() {
        match value {
            None => builder.append_null(),
            Some(s) => {
                let int_val = s.parse::<i64>().map_err(|_| ParquetExportError::CsvParse {
                    row: row_idx,
                    message: format!("Invalid integer value at column {}: {}", col_idx, s),
                })?;
                builder.append_value(int_val);
            }
        }
    }

    Ok(Arc::new(builder.finish()))
}

/// Create an Arrow schema from Exasol column types.
///
/// This function maps Exasol types to the Arrow types a Parquet export writes. INTERVAL YEAR TO
/// MONTH, INTERVAL DAY TO SECOND, GEOMETRY, and HASHTYPE map to `Utf8`, which holds the CSV
/// text Exasol writes for them. TIMESTAMP WITH LOCAL TIME ZONE maps to
/// `Timestamp(Microsecond, None)`, which holds the session-local wall-clock value.
pub fn exasol_types_to_arrow_schema(
    column_names: &[String],
    column_types: &[ExasolType],
) -> Result<Schema, ParquetExportError> {
    if column_names.len() != column_types.len() {
        return Err(ParquetExportError::Schema(format!(
            "Column names count ({}) doesn't match types count ({})",
            column_names.len(),
            column_types.len()
        )));
    }

    let fields: Result<Vec<Field>, ParquetExportError> = column_names
        .iter()
        .zip(column_types.iter())
        .map(|(name, exasol_type)| {
            let arrow_type = exasol_type_to_arrow(exasol_type)?;
            Ok(Field::new(name, arrow_type, true)) // All columns nullable
        })
        .collect();

    Ok(Schema::new(fields?))
}

/// Convert an Exasol type to the Arrow type its CSV text converts to.
fn exasol_type_to_arrow(exasol_type: &ExasolType) -> Result<DataType, ParquetExportError> {
    match exasol_type {
        // No typed conversion exists for these, so the CSV text is kept as Utf8.
        ExasolType::IntervalYearToMonth
        | ExasolType::IntervalDayToSecond { .. }
        | ExasolType::Geometry { .. }
        | ExasolType::Hashtype { .. } => Ok(DataType::Utf8),
        // Exasol writes the session-local wall-clock value, so no UTC label applies.
        ExasolType::Timestamp {
            with_local_time_zone: true,
        } => Ok(DataType::Timestamp(TimeUnit::Microsecond, None)),
        _ => exasol_type_to_arrow_impl(exasol_type).map_err(ParquetExportError::Schema),
    }
}

// =============================================================================
// Transport-integrated export functions
// =============================================================================

use crate::export::csv::ExportError;
use crate::query::export::ExportSource;
use crate::transport::TransportProtocol;

/// Exports data from an Exasol table or query to a Parquet file via transport.
///
/// The export runs in three steps:
/// 1. Prepare the source's SELECT statement and derive the Arrow schema from its result-set
///    metadata, so the file carries the source's column names and types. The prepared statement
///    is closed before the EXPORT statement runs.
/// 2. Run the EXPORT through the HTTP transport and collect its CSV rows.
/// 3. Convert the rows to typed batches, then create the file and write them.
///
/// An empty export writes a file that carries the schema and no rows. The file is created only
/// after every batch converts, and a failed write removes it again.
///
/// An empty field is NULL and every other field is kept verbatim, so
/// [`ParquetExportOptions::null_value`] has no effect here. Typed columns need Exasol's default
/// session formats for numbers, dates, and timestamps.
///
/// # Arguments
///
/// * `transport` - Transport for executing SQL
/// * `source` - The data source (table or query); it must produce a result set
/// * `file_path` - Path to write the Parquet file
/// * `options` - Export options
///
/// # Returns
///
/// The number of rows exported.
///
/// # Errors
///
/// Returns `ExportError::SqlExecutionError` when the source cannot be prepared, produces no
/// result set, or has a column type with no Arrow mapping. Returns
/// `ExportError::TransportError` when closing the prepared statement fails. Returns
/// `ExportError::CsvParseError` when a value does not match its column type, and also, with
/// row 0, when the Parquet writer fails. Returns `ExportError::IoError` when the output file
/// cannot be created.
pub async fn export_to_parquet_via_transport<T: TransportProtocol + ?Sized>(
    transport: &mut T,
    source: ExportSource,
    file_path: &Path,
    options: ParquetExportOptions,
) -> Result<u64, ExportError> {
    use crate::export::csv::{export_to_list, shared_csv_export_options, SharedCsvExportParams};

    let schema = export_schema(transport, &source).await?;

    let csv_options = shared_csv_export_options(SharedCsvExportParams {
        column_separator: options.column_separator,
        column_delimiter: options.column_delimiter,
        host: &options.host,
        port: options.port,
        use_tls: options.use_tls,
    });
    let rows = export_to_list(transport, source, csv_options).await?;

    let batches =
        rows_to_record_batches(rows.into_iter().map(Ok), &schema, None, options.batch_size)
            .map_err(export_error)?;
    let total_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();

    write_output_file(file_path, |file| {
        write_parquet(file, &schema, &batches, options.compression)
    })
    .map_err(export_error)?;

    Ok(total_rows)
}

/// Derives the Arrow schema of `source` from the result-set metadata of its prepared SELECT.
///
/// The prepared statement is closed before returning, also when the source is rejected. A
/// rejection takes precedence over a close failure.
async fn export_schema<T: TransportProtocol + ?Sized>(
    transport: &mut T,
    source: &ExportSource,
) -> Result<Arc<Schema>, ExportError> {
    let handle = transport
        .create_prepared_statement(&source.select_statement())
        .await
        .map_err(|e| ExportError::SqlExecutionError {
            message: format!("Failed to prepare the export source to read its columns: {e}"),
        })?;

    let schema = schema_of_result_columns(&handle.result_columns);
    let closed = transport.close_prepared_statement(&handle).await;

    let schema = schema?;
    closed?;
    Ok(Arc::new(schema))
}

fn schema_of_result_columns(
    columns: &[crate::transport::messages::ColumnInfo],
) -> Result<Schema, ExportError> {
    if columns.is_empty() {
        return Err(ExportError::SqlExecutionError {
            message: "The export source produces no result set; export a table or a SELECT query"
                .to_string(),
        });
    }

    let names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();
    let types = columns
        .iter()
        .map(|column| {
            crate::query::results::exasol_type_of(&column.data_type).map_err(|_| {
                ExportError::SqlExecutionError {
                    message: format!(
                        "Cannot export column {} of Exasol type {}: no Arrow type maps to it",
                        column.name, column.data_type.type_name
                    ),
                }
            })
        })
        .collect::<Result<Vec<ExasolType>, _>>()?;

    exasol_types_to_arrow_schema(&names, &types).map_err(|e| ExportError::SqlExecutionError {
        message: format!("Cannot derive the export schema: {e}"),
    })
}

/// Creates `file_path`, runs `write` on it, and removes the file again when `write` fails.
///
/// The original error is returned even when the removal fails.
fn write_output_file<F>(file_path: &Path, write: F) -> Result<(), ParquetExportError>
where
    F: FnOnce(std::fs::File) -> Result<(), ParquetExportError>,
{
    let file = std::fs::File::create(file_path)?;
    write(file).inspect_err(|_| {
        let _ = std::fs::remove_file(file_path);
    })
}

fn export_error(error: ParquetExportError) -> ExportError {
    match error {
        ParquetExportError::Io(e) => ExportError::IoError(e),
        ParquetExportError::CsvParse { row, message } => {
            ExportError::CsvParseError { row, message }
        }
        other => ExportError::CsvParseError {
            row: 0,
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::protocol::QueryResult;
    use crate::transport::test_support::{FakeExasolServer, MockTransport};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    // ==========================================================================
    // Tests for ParquetCompression
    // ==========================================================================

    #[test]
    fn test_parquet_compression_default() {
        let compression = ParquetCompression::default();
        assert_eq!(compression, ParquetCompression::Snappy);
    }

    #[test]
    fn test_parquet_compression_to_codec() {
        assert!(matches!(
            ParquetCompression::None.to_codec(),
            ParquetCompressionCodec::UNCOMPRESSED
        ));
        assert!(matches!(
            ParquetCompression::Snappy.to_codec(),
            ParquetCompressionCodec::SNAPPY
        ));
        assert!(matches!(
            ParquetCompression::Gzip.to_codec(),
            ParquetCompressionCodec::GZIP(_)
        ));
        assert!(matches!(
            ParquetCompression::Lz4.to_codec(),
            ParquetCompressionCodec::LZ4
        ));
        assert!(matches!(
            ParquetCompression::Zstd.to_codec(),
            ParquetCompressionCodec::ZSTD(_)
        ));
    }

    // ==========================================================================
    // Tests for ParquetExportOptions
    // ==========================================================================

    #[test]
    fn test_parquet_export_options_default() {
        let options = ParquetExportOptions::default();
        assert_eq!(options.batch_size, 1024);
        assert_eq!(options.compression, ParquetCompression::Snappy);
        assert!(options.with_column_names);
        assert_eq!(options.column_separator, ',');
        assert_eq!(options.column_delimiter, '"');
        assert!(options.null_value.is_none());
        assert_eq!(options.host, "");
        assert_eq!(options.port, 0);
        assert!(!options.use_tls);
    }

    #[test]
    fn test_parquet_export_options_builder() {
        let options = ParquetExportOptions::new()
            .with_batch_size(2048)
            .with_compression(ParquetCompression::Gzip)
            .with_column_names(false)
            .with_column_separator(';')
            .with_column_delimiter('\'')
            .with_null_value("\\N")
            .exasol_host("exasol.example.com")
            .exasol_port(8563)
            .use_tls(true);

        assert_eq!(options.batch_size, 2048);
        assert_eq!(options.compression, ParquetCompression::Gzip);
        assert!(!options.with_column_names);
        assert_eq!(options.column_separator, ';');
        assert_eq!(options.column_delimiter, '\'');
        assert_eq!(options.null_value, Some("\\N".to_string()));
        assert_eq!(options.host, "exasol.example.com");
        assert_eq!(options.port, 8563);
        assert!(options.use_tls);
    }

    #[test]
    fn test_parquet_export_options_use_tls_builder() {
        assert!(ParquetExportOptions::default().use_tls(true).use_tls);
        assert!(!ParquetExportOptions::default().use_tls(false).use_tls);
    }

    // ==========================================================================
    // Tests for CSV parsing
    // ==========================================================================

    fn text_column(batch: &RecordBatch, column: usize) -> Vec<Option<String>> {
        batch
            .column(column)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("the column must be Utf8")
            .iter()
            .map(|value| value.map(str::to_string))
            .collect()
    }

    fn id_name_schema() -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ])
    }

    fn id_name_flag_schema() -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
            Field::new("flag", DataType::Boolean, true),
        ])
    }

    fn headerless() -> ParquetExportOptions {
        ParquetExportOptions::default().with_column_names(false)
    }

    #[test]
    fn test_csv_to_record_batches_splits_simple_fields() {
        let batches =
            csv_to_record_batches(b"1,Alice,true", &id_name_flag_schema(), &headerless()).unwrap();

        assert_eq!(batches[0].num_rows(), 1);
        assert_eq!(text_column(&batches[0], 1), vec![Some("Alice".to_string())]);
    }

    #[test]
    fn test_csv_to_record_batches_keeps_separator_inside_quotes() {
        let csv = b"1,\"Hello, World\",true";
        let batches = csv_to_record_batches(csv, &id_name_flag_schema(), &headerless()).unwrap();

        assert_eq!(
            text_column(&batches[0], 1),
            vec![Some("Hello, World".to_string())]
        );
    }

    #[test]
    fn test_csv_to_record_batches_unescapes_doubled_delimiters() {
        let csv = b"1,\"Say \"\"Hello\"\"\",true";
        let batches = csv_to_record_batches(csv, &id_name_flag_schema(), &headerless()).unwrap();

        assert_eq!(
            text_column(&batches[0], 1),
            vec![Some("Say \"Hello\"".to_string())]
        );
    }

    #[test]
    fn test_csv_to_record_batches_reads_empty_field_as_null_without_null_value() {
        let batches =
            csv_to_record_batches(b"1,,true", &id_name_flag_schema(), &headerless()).unwrap();

        assert_eq!(text_column(&batches[0], 1), vec![None]);
    }

    #[test]
    fn test_csv_to_record_batches_reads_null_value_marker_as_null() {
        let options = headerless().with_null_value("\\N");
        let batches =
            csv_to_record_batches(b"1,\\N,true", &id_name_flag_schema(), &options).unwrap();

        assert_eq!(text_column(&batches[0], 1), vec![None]);
    }

    #[test]
    fn test_csv_to_record_batches_rejects_wrong_column_count() {
        let err =
            csv_to_record_batches(b"1,Alice", &id_name_flag_schema(), &headerless()).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 0, .. }),
            "got: {err}"
        );
        assert!(
            err.to_string().contains("Expected 3 columns, found 2"),
            "got: {err}"
        );
    }

    /// Scenario: CSV-bytes export accepts line breaks inside quoted fields
    #[test]
    fn test_csv_to_record_batches_accepts_line_breaks_in_quoted_fields() {
        let csv = b"1,\"line one\nline two\"\n2,\"a, b\"\n";

        let batches = csv_to_record_batches(csv, &id_name_schema(), &headerless()).unwrap();

        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
        assert_eq!(
            text_column(&batches[0], 1),
            vec![
                Some("line one\nline two".to_string()),
                Some("a, b".to_string())
            ]
        );
    }

    /// Scenario: CSV-bytes export keeps field whitespace
    #[test]
    fn test_csv_to_record_batches_reports_the_data_row_of_a_bad_value_in_a_later_batch() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, true)]);
        let options = ParquetExportOptions::default().with_batch_size(2);

        let err = csv_to_record_batches(b"id\n1\n2\n3\nx\n5", &schema, &options).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 3, .. }),
            "got: {err}"
        );
    }

    #[test]
    fn test_csv_to_record_batches_converts_every_row_when_batch_size_is_zero() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, true)]);
        let options = headerless().with_batch_size(0);

        let batches = csv_to_record_batches(b"1\n2\n3", &schema, &options).unwrap();

        assert_eq!(batches.len(), 3);
        assert!(batches.iter().all(|batch| batch.num_rows() == 1));
    }

    #[test]
    fn test_csv_to_record_batches_reports_the_data_row_of_a_short_row_in_a_later_batch() {
        let options = headerless().with_batch_size(2);

        let err =
            csv_to_record_batches(b"1,a\n2,b\n3,c\n4\n", &id_name_schema(), &options).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 3, .. }),
            "got: {err}"
        );
    }

    #[test]
    fn test_csv_to_record_batches_reports_the_data_row_of_an_unclosed_quote() {
        let schema = Schema::new(vec![Field::new("name", DataType::Utf8, true)]);
        let options = ParquetExportOptions::default();

        let err = csv_to_record_batches(b"name\nx\n\"open", &schema, &options).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 1, .. }),
            "got: {err}"
        );
        assert!(err.to_string().contains("Unclosed quote"), "got: {err}");
    }

    #[test]
    fn test_csv_to_record_batches_reports_an_unclosed_quote_in_the_header() {
        let schema = Schema::new(vec![Field::new("name", DataType::Utf8, true)]);
        let options = ParquetExportOptions::default();

        let err = csv_to_record_batches(b"\"name", &schema, &options).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 0, .. }),
            "got: {err}"
        );
    }

    // ==========================================================================
    // Tests for CSV to RecordBatch conversion
    // ==========================================================================

    #[test]
    fn test_csv_to_record_batches_empty() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, true)]);
        let options = ParquetExportOptions::default();
        let batches = csv_to_record_batches(b"", &schema, &options).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 0);
    }

    #[test]
    fn test_csv_to_record_batches_header_only() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, true)]);
        let options = ParquetExportOptions::default();
        let batches = csv_to_record_batches(b"id", &schema, &options).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 0);
    }

    #[test]
    fn test_csv_to_record_batches_simple() {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ]);
        let options = ParquetExportOptions::default();
        let csv_data = b"id,name\n1,Alice\n2,Bob";
        let batches = csv_to_record_batches(csv_data, &schema, &options).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[0].num_columns(), 2);
    }

    #[test]
    fn test_csv_to_record_batches_multiple_batches() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, true)]);
        let options = ParquetExportOptions {
            batch_size: 2,
            ..Default::default()
        };
        let csv_data = b"id\n1\n2\n3\n4\n5";
        let batches = csv_to_record_batches(csv_data, &schema, &options).unwrap();
        assert_eq!(batches.len(), 3); // 2 + 2 + 1
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[1].num_rows(), 2);
        assert_eq!(batches[2].num_rows(), 1);
    }

    // ==========================================================================
    // Tests for array building
    // ==========================================================================

    #[test]
    fn test_build_boolean_array_from_csv() {
        let values = vec![Some("true"), Some("false"), None, Some("1"), Some("0")];
        let array = build_boolean_array_from_csv(&values, 0).unwrap();
        assert_eq!(array.len(), 5);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_boolean_array_from_csv_invalid() {
        let values = vec![Some("invalid")];
        let result = build_boolean_array_from_csv(&values, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_string_array_from_csv() {
        let values = vec![Some("hello"), Some("world"), None];
        let array = build_string_array_from_csv(&values, 0).unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_float64_array_from_csv() {
        let values = vec![Some("1.5"), Some("2.7"), None];
        let array = build_float64_array_from_csv(&values, 0).unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_float64_array_from_csv_invalid() {
        let values = vec![Some("not_a_number")];
        let result = build_float64_array_from_csv(&values, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_decimal128_array_from_csv() {
        let values = vec![Some("123.45"), Some("678.90"), None];
        let array = build_decimal128_array_from_csv(&values, 10, 2, 0).unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_date32_array_from_csv() {
        let values = vec![Some("2024-01-15"), Some("2023-06-20"), None];
        let array = build_date32_array_from_csv(&values, 0).unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_timestamp_array_from_csv() {
        let values = vec![
            Some("2024-01-15 10:30:00"),
            Some("2023-06-20 14:45:30.123456"),
            None,
        ];
        let array = build_timestamp_array_from_csv(&values, None, 0).unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_int64_array_from_csv() {
        let values = vec![Some("1"), Some("2"), None, Some("-100")];
        let array = build_int64_array_from_csv(&values, 0).unwrap();
        assert_eq!(array.len(), 4);
        assert_eq!(array.null_count(), 1);
    }

    #[test]
    fn test_build_int64_array_from_csv_invalid() {
        let values = vec![Some("not_a_number")];
        let result = build_int64_array_from_csv(&values, 0);
        assert!(result.is_err());
    }

    // ==========================================================================
    // Tests for date parsing
    // ==========================================================================

    #[test]
    fn test_parse_date_to_days_epoch() {
        let days = parse_date_to_days("1970-01-01", 0, 0).unwrap();
        assert_eq!(days, 0);
    }

    #[test]
    fn test_parse_date_to_days_after_epoch() {
        let days = parse_date_to_days("1970-01-02", 0, 0).unwrap();
        assert_eq!(days, 1);
    }

    #[test]
    fn test_parse_date_to_days_before_epoch() {
        let days = parse_date_to_days("1969-12-31", 0, 0).unwrap();
        assert_eq!(days, -1);
    }

    #[test]
    fn test_parse_date_to_days_invalid_format() {
        let result = parse_date_to_days("2024/01/15", 0, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_month() {
        let result = parse_date_to_days("2024-13-01", 0, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_leap_year() {
        // 2024 is a leap year
        let march_1_2024 = parse_date_to_days("2024-03-01", 0, 0).unwrap();
        let feb_28_2024 = parse_date_to_days("2024-02-28", 0, 0).unwrap();
        assert_eq!(march_1_2024 - feb_28_2024, 2); // Feb 29 exists
    }

    // ==========================================================================
    // Tests for timestamp parsing
    // ==========================================================================

    #[test]
    fn test_parse_timestamp_to_micros_epoch() {
        let micros = parse_timestamp_to_micros("1970-01-01 00:00:00", 0, 0).unwrap();
        assert_eq!(micros, 0);
    }

    #[test]
    fn test_parse_timestamp_to_micros_one_second() {
        let micros = parse_timestamp_to_micros("1970-01-01 00:00:01", 0, 0).unwrap();
        assert_eq!(micros, 1_000_000);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_fraction() {
        let micros = parse_timestamp_to_micros("1970-01-01 00:00:00.123456", 0, 0).unwrap();
        assert_eq!(micros, 123_456);
    }

    #[test]
    fn test_parse_timestamp_to_micros_date_only() {
        let micros = parse_timestamp_to_micros("1970-01-02", 0, 0).unwrap();
        assert_eq!(micros, 86400 * 1_000_000);
    }

    // ==========================================================================
    // Tests for decimal parsing
    // ==========================================================================

    #[test]
    fn test_parse_decimal_to_i128_integer() {
        let result = parse_decimal_to_i128("123", 2, 0, 0).unwrap();
        assert_eq!(result, 12300);
    }

    #[test]
    fn test_parse_decimal_to_i128_with_fraction() {
        let result = parse_decimal_to_i128("123.45", 2, 0, 0).unwrap();
        assert_eq!(result, 12345);
    }

    #[test]
    fn test_parse_decimal_to_i128_negative() {
        let result = parse_decimal_to_i128("-123.45", 2, 0, 0).unwrap();
        assert_eq!(result, -12345);
    }

    #[test]
    fn test_parse_decimal_to_i128_invalid_format() {
        let result = parse_decimal_to_i128("1.2.3", 2, 0, 0);
        assert!(result.is_err());
    }

    // ==========================================================================
    // Tests for schema conversion
    // ==========================================================================

    #[test]
    fn test_exasol_types_to_arrow_schema() {
        let names = vec!["id".to_string(), "name".to_string(), "active".to_string()];
        let types = vec![
            ExasolType::Decimal {
                precision: 18,
                scale: 0,
            },
            ExasolType::Varchar { size: 100 },
            ExasolType::Boolean,
        ];
        let schema = exasol_types_to_arrow_schema(&names, &types).unwrap();
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(schema.field(2).name(), "active");
    }

    #[test]
    fn test_exasol_types_to_arrow_schema_mismatched_lengths() {
        let names = vec!["id".to_string()];
        let types = vec![ExasolType::Boolean, ExasolType::Double];
        let result = exasol_types_to_arrow_schema(&names, &types);
        assert!(result.is_err());
    }

    #[test]
    fn test_exasol_type_to_arrow_boolean() {
        let result = exasol_type_to_arrow(&ExasolType::Boolean).unwrap();
        assert_eq!(result, DataType::Boolean);
    }

    #[test]
    fn test_exasol_type_to_arrow_varchar() {
        let result = exasol_type_to_arrow(&ExasolType::Varchar { size: 100 }).unwrap();
        assert_eq!(result, DataType::Utf8);
    }

    #[test]
    fn test_exasol_type_to_arrow_decimal() {
        let result = exasol_type_to_arrow(&ExasolType::Decimal {
            precision: 18,
            scale: 2,
        })
        .unwrap();
        assert_eq!(result, DataType::Decimal128(18, 2));
    }

    #[test]
    fn test_exasol_type_to_arrow_double() {
        let result = exasol_type_to_arrow(&ExasolType::Double).unwrap();
        assert_eq!(result, DataType::Float64);
    }

    #[test]
    fn test_exasol_type_to_arrow_date() {
        let result = exasol_type_to_arrow(&ExasolType::Date).unwrap();
        assert_eq!(result, DataType::Date32);
    }

    #[test]
    fn test_exasol_type_to_arrow_timestamp_without_tz() {
        let result = exasol_type_to_arrow(&ExasolType::Timestamp {
            with_local_time_zone: false,
        })
        .unwrap();
        assert_eq!(result, DataType::Timestamp(TimeUnit::Microsecond, None));
    }

    /// Scenario: Date and time types mapping
    #[test]
    fn test_exasol_types_to_arrow_schema_maps_timestamp_with_local_time_zone_without_time_zone() {
        let schema = exasol_types_to_arrow_schema(
            &["ts_ltz".to_string()],
            &[ExasolType::Timestamp {
                with_local_time_zone: true,
            }],
        )
        .unwrap();

        assert_eq!(
            schema.field(0).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None)
        );
    }

    // ==========================================================================
    // Integration tests for Parquet export
    // ==========================================================================

    #[tokio::test]
    async fn test_export_to_parquet_stream_simple() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ]));
        let options = ParquetExportOptions::default();
        let csv_data = b"id,name\n1,Alice\n2,Bob\n3,Charlie";

        let mut buffer = Vec::new();
        let rows = export_to_parquet_stream(csv_data, schema, &mut buffer, options)
            .await
            .unwrap();

        assert_eq!(rows, 3);
        assert!(!buffer.is_empty());
        // Verify it's a valid Parquet file (magic bytes: PAR1)
        assert_eq!(&buffer[0..4], b"PAR1");
    }

    #[tokio::test]
    async fn test_export_to_parquet_stream_with_nulls() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ]));
        let options = ParquetExportOptions::default();
        let csv_data = b"id,name\n1,Alice\n2,\n3,Charlie";

        let mut buffer = Vec::new();
        let rows = export_to_parquet_stream(csv_data, schema, &mut buffer, options)
            .await
            .unwrap();

        assert_eq!(rows, 3);
        assert!(!buffer.is_empty());
    }

    #[tokio::test]
    async fn test_export_to_parquet_stream_empty() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let options = ParquetExportOptions::default();
        let csv_data = b"id";

        let mut buffer = Vec::new();
        let rows = export_to_parquet_stream(csv_data, schema, &mut buffer, options)
            .await
            .unwrap();

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn test_export_to_parquet_stream_all_types() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("bool_col", DataType::Boolean, true),
            Field::new("int_col", DataType::Int64, true),
            Field::new("float_col", DataType::Float64, true),
            Field::new("str_col", DataType::Utf8, true),
            Field::new("date_col", DataType::Date32, true),
            Field::new(
                "ts_col",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                true,
            ),
        ]));
        let options = ParquetExportOptions::default();
        let csv_data = b"bool_col,int_col,float_col,str_col,date_col,ts_col\ntrue,1,1.5,hello,2024-01-15,2024-01-15 10:30:00\nfalse,2,2.5,world,2024-06-20,2024-06-20 14:45:30.123456";

        let mut buffer = Vec::new();
        let rows = export_to_parquet_stream(csv_data, schema, &mut buffer, options)
            .await
            .unwrap();

        assert_eq!(rows, 2);
        assert!(!buffer.is_empty());
    }

    #[tokio::test]
    async fn test_export_to_parquet_stream_with_compression() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));

        // Test all compression types
        for compression in [
            ParquetCompression::None,
            ParquetCompression::Snappy,
            ParquetCompression::Gzip,
            ParquetCompression::Lz4,
            ParquetCompression::Zstd,
        ] {
            let options = ParquetExportOptions {
                compression,
                ..Default::default()
            };
            let csv_data = b"id\n1\n2\n3";

            let mut buffer = Vec::new();
            let rows = export_to_parquet_stream(csv_data, schema.clone(), &mut buffer, options)
                .await
                .unwrap();

            assert_eq!(rows, 3);
            assert!(!buffer.is_empty());
            // Verify it's a valid Parquet file
            assert_eq!(&buffer[0..4], b"PAR1");
        }
    }

    #[tokio::test]
    async fn test_export_to_parquet_file() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.parquet");

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ]));
        let options = ParquetExportOptions::default();
        let csv_data = b"id,name\n1,Alice\n2,Bob";

        let rows = export_to_parquet(csv_data, schema, &file_path, options)
            .await
            .unwrap();

        assert_eq!(rows, 2);
        assert!(file_path.exists());

        // Verify file content starts with PAR1
        let content = fs::read(&file_path).unwrap();
        assert_eq!(&content[0..4], b"PAR1");
    }

    #[test]
    fn test_from_arrow_error_wraps_message() {
        let arrow_err = arrow::error::ArrowError::ComputeError("bad compute".to_string());

        let err: ParquetExportError = arrow_err.into();

        assert!(matches!(err, ParquetExportError::Arrow(_)), "got: {err}");
        assert!(err.to_string().contains("bad compute"), "got: {err}");
    }

    #[test]
    fn test_from_parquet_error_wraps_message() {
        let parquet_err = parquet::errors::ParquetError::General("bad footer".to_string());

        let err: ParquetExportError = parquet_err.into();

        assert!(matches!(err, ParquetExportError::Parquet(_)), "got: {err}");
        assert!(err.to_string().contains("bad footer"), "got: {err}");
    }

    #[test]
    fn test_csv_to_record_batches_rejects_invalid_utf8() {
        let schema = Schema::new(vec![Field::new("name", DataType::Utf8, true)]);
        let options = ParquetExportOptions::default().with_column_names(false);

        let err = csv_to_record_batches(&[0xF0, 0x28, 0x8C, 0x28], &schema, &options).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 0, .. }),
            "got: {err}"
        );
        assert!(err.to_string().contains("Invalid UTF-8"), "got: {err}");
    }

    #[test]
    fn test_build_array_from_csv_column_rejects_unsupported_type() {
        let field = Field::new("b", DataType::Binary, true);

        let err = build_array_from_csv_column(&[Some("x")], &field, 0).unwrap_err();

        assert!(matches!(err, ParquetExportError::Schema(_)), "got: {err}");
        assert!(
            err.to_string()
                .contains("Unsupported data type for Parquet export"),
            "got: {err}"
        );
    }

    #[test]
    fn test_csv_to_record_batches_dispatches_every_supported_type() {
        use arrow::array::{
            BooleanArray, Date32Array, Decimal128Array, Float64Array, Int64Array, StringArray,
            TimestampMicrosecondArray,
        };

        let schema = Schema::new(vec![
            Field::new("flag", DataType::Boolean, true),
            Field::new("label", DataType::Utf8, true),
            Field::new("ratio", DataType::Float64, true),
            Field::new("amount", DataType::Decimal128(10, 2), true),
            Field::new("day", DataType::Date32, true),
            Field::new(
                "moment",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                true,
            ),
            Field::new("count", DataType::Int64, true),
        ]);
        let options = ParquetExportOptions::default().with_column_names(false);
        let csv = b"true,hello,1.5,123.45,1970-01-02,1970-01-01 00:00:01,42\n";

        let batches = csv_to_record_batches(csv, &schema, &options).expect("batches");

        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1);
        assert!(batch
            .column(0)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap()
            .value(0));
        assert_eq!(
            batch
                .column(1)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "hello"
        );
        assert_eq!(
            batch
                .column(2)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(0),
            1.5
        );
        assert_eq!(
            batch
                .column(3)
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap()
                .value(0),
            12345
        );
        assert_eq!(
            batch
                .column(4)
                .as_any()
                .downcast_ref::<Date32Array>()
                .unwrap()
                .value(0),
            1
        );
        assert_eq!(
            batch
                .column(5)
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap()
                .value(0),
            1_000_000
        );
        assert_eq!(
            batch
                .column(6)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            42
        );
    }

    #[test]
    fn test_parse_timestamp_to_micros_reports_row_and_column() {
        let err = parse_timestamp_to_micros("not-a-timestamp", 7, 3).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 7, .. }),
            "got: {err}"
        );
        assert!(err.to_string().contains("at column 3"), "got: {err}");
    }

    #[test]
    fn test_parse_date_to_days_reports_row_and_column() {
        let err = parse_date_to_days("not-a-date", 4, 2).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 4, .. }),
            "got: {err}"
        );
        assert!(err.to_string().contains("at column 2"), "got: {err}");
    }

    #[test]
    fn test_parse_decimal_to_i128_reports_row_and_column() {
        let err = parse_decimal_to_i128("not-a-decimal", 2, 5, 1).unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 5, .. }),
            "got: {err}"
        );
        assert!(err.to_string().contains("at column 1"), "got: {err}");
    }

    /// Scenario: Columns without a typed CSV conversion export as text
    /// Scenario: Date and time types mapping
    #[test]
    fn test_exasol_types_to_arrow_schema_maps_text_only_types_to_utf8() {
        let names: Vec<String> = ["iym", "ids", "g", "h"].map(String::from).to_vec();
        let types = [
            ExasolType::IntervalYearToMonth,
            ExasolType::IntervalDayToSecond { precision: 3 },
            ExasolType::Geometry { srid: Some(4326) },
            ExasolType::Hashtype { byte_size: 16 },
        ];

        let schema = exasol_types_to_arrow_schema(&names, &types).unwrap();

        assert!(schema
            .fields()
            .iter()
            .all(|field| field.data_type() == &DataType::Utf8));
    }

    async fn stream_to_batches(
        csv: &[u8],
        schema: Schema,
        options: ParquetExportOptions,
    ) -> Result<Vec<RecordBatch>, ParquetExportError> {
        let mut buffer = Vec::new();
        export_to_parquet_stream(csv, Arc::new(schema), &mut buffer, options).await?;
        Ok(
            ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(buffer))
                .expect("the stream export must write a readable Parquet file")
                .build()
                .expect("the Parquet reader must accept the written schema")
                .map(|batch| batch.expect("every written row group must decode"))
                .collect(),
        )
    }

    /// Scenario: CSV-bytes export keeps field whitespace
    #[tokio::test]
    async fn test_export_to_parquet_stream_keeps_field_whitespace() {
        let csv = b"id,name\n1,\"  padded  \"\n2,   \n3,\n";

        let batches = stream_to_batches(csv, id_name_schema(), ParquetExportOptions::default())
            .await
            .unwrap();

        assert_eq!(
            text_column(&batches[0], 1),
            vec![
                Some("  padded  ".to_string()),
                Some("   ".to_string()),
                None
            ]
        );

        let err = stream_to_batches(
            b"id,name\n1,x\n 7,y\n",
            id_name_schema(),
            ParquetExportOptions::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, ParquetExportError::CsvParse { row: 1, .. }),
            "got: {err}"
        );
    }

    /// Scenario: CSV-bytes export reads only the null_value marker as NULL
    #[tokio::test]
    async fn test_export_to_parquet_stream_reads_null_value_marker_as_null() {
        let options = ParquetExportOptions::default().with_null_value("NULL");

        let batches = stream_to_batches(b"id,name\n1,NULL\n2,\n", id_name_schema(), options)
            .await
            .unwrap();

        assert_eq!(text_column(&batches[0], 1), vec![None, Some(String::new())]);
    }

    // ==========================================================================
    // Tests for export_to_parquet_via_transport
    // ==========================================================================

    use crate::transport::messages::{ColumnInfo, DataType as TransportDataType};
    use crate::transport::protocol::PreparedStatementHandle;
    use mockall::predicate::eq;
    use mockall::Sequence;

    fn tunnel_options(server: &FakeExasolServer) -> ParquetExportOptions {
        ParquetExportOptions::default()
            .exasol_host(&server.host)
            .exasol_port(server.port)
            .use_tls(false)
    }

    fn column(name: &str, data_type: TransportDataType) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type,
        }
    }

    fn unknown_type(type_name: &str) -> TransportDataType {
        TransportDataType {
            type_name: type_name.to_string(),
            ..TransportDataType::boolean()
        }
    }

    fn id_name_columns() -> Vec<ColumnInfo> {
        vec![
            column("ID", TransportDataType::decimal(18, 0)),
            column("NAME", TransportDataType::varchar(100)),
        ]
    }

    fn handle_describing(columns: Vec<ColumnInfo>) -> PreparedStatementHandle {
        PreparedStatementHandle::new(7, 0, vec![], vec![]).with_result_columns(columns)
    }

    fn users_source() -> ExportSource {
        ExportSource::Table {
            schema: Some("S".to_string()),
            name: "USERS".to_string(),
            columns: vec![],
        }
    }

    /// A transport whose prepare reports `columns`, whose close succeeds, and whose EXPORT
    /// statement succeeds.
    fn describing_transport(columns: Vec<ColumnInfo>) -> MockTransport {
        let mut transport = MockTransport::new();
        transport
            .expect_create_prepared_statement()
            .returning(move |_| Ok(handle_describing(columns.clone())));
        transport
            .expect_close_prepared_statement()
            .returning(|_| Ok(()));
        transport
            .expect_execute_query()
            .returning(|_| Ok(QueryResult::row_count(2)));
        transport
    }

    fn read_back(file_path: &Path) -> Vec<RecordBatch> {
        let file = std::fs::File::open(file_path).expect("the export must have created the file");
        ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("the export must have written a readable Parquet file")
            .build()
            .expect("the Parquet reader must accept the written schema")
            .map(|batch| batch.expect("every written row group must decode"))
            .collect()
    }

    fn read_schema(file_path: &Path) -> Arc<Schema> {
        let file = std::fs::File::open(file_path).expect("the export must have created the file");
        ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("the export must have written a readable Parquet file")
            .schema()
            .clone()
    }

    #[tokio::test]
    async fn test_export_schema_names_fields_after_result_columns_and_closes_the_statement() {
        let source = users_source();
        let mut transport = MockTransport::new();
        transport
            .expect_create_prepared_statement()
            .with(eq(source.select_statement()))
            .times(1)
            .returning(|_| Ok(handle_describing(id_name_columns())));
        transport
            .expect_close_prepared_statement()
            .times(1)
            .returning(|_| Ok(()));

        let schema = export_schema(&mut transport, &source).await.unwrap();

        assert_eq!(
            schema.as_ref(),
            &Schema::new(vec![
                Field::new("ID", DataType::Decimal128(18, 0), true),
                Field::new("NAME", DataType::Utf8, true),
            ])
        );
    }

    #[tokio::test]
    async fn test_export_schema_names_column_and_type_it_cannot_map_and_closes_the_statement() {
        let mut transport = MockTransport::new();
        transport.expect_create_prepared_statement().returning(|_| {
            Ok(handle_describing(vec![column(
                "MYSTERY",
                unknown_type("FOO"),
            )]))
        });
        transport
            .expect_close_prepared_statement()
            .times(1)
            .returning(|_| Ok(()));

        let err = export_schema(&mut transport, &users_source())
            .await
            .unwrap_err();

        assert!(
            matches!(&err, ExportError::SqlExecutionError { message }
                if message.contains("MYSTERY") && message.contains("FOO")),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_export_schema_returns_close_failure_as_transport_error() {
        let mut transport = MockTransport::new();
        transport
            .expect_create_prepared_statement()
            .returning(|_| Ok(handle_describing(id_name_columns())));
        transport.expect_close_prepared_statement().returning(|_| {
            Err(crate::error::TransportError::SendError(
                "closed".to_string(),
            ))
        });

        let err = export_schema(&mut transport, &users_source())
            .await
            .unwrap_err();

        assert!(matches!(err, ExportError::TransportError(_)), "got: {err}");
    }

    #[tokio::test]
    async fn test_export_schema_returns_rejection_when_close_also_fails() {
        let mut transport = MockTransport::new();
        transport
            .expect_create_prepared_statement()
            .returning(|_| Ok(handle_describing(vec![])));
        transport.expect_close_prepared_statement().returning(|_| {
            Err(crate::error::TransportError::SendError(
                "closed".to_string(),
            ))
        });

        let err = export_schema(&mut transport, &users_source())
            .await
            .unwrap_err();

        assert!(
            matches!(err, ExportError::SqlExecutionError { .. }),
            "got: {err}"
        );
    }

    #[test]
    fn test_write_output_file_removes_the_file_when_the_write_step_fails() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("partial.parquet");

        let err = write_output_file(&file_path, |mut file| {
            file.write_all(b"PAR1 truncated")?;
            Err(ParquetExportError::Parquet("close failed".to_string()))
        })
        .unwrap_err();

        assert!(
            matches!(&err, ParquetExportError::Parquet(message) if message == "close failed"),
            "got: {err}"
        );
        assert!(
            !file_path.exists(),
            "a failed write must not leave a partial file behind"
        );
    }

    #[test]
    fn test_write_output_file_keeps_the_file_when_the_write_step_succeeds() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("complete.parquet");

        write_output_file(&file_path, |mut file| Ok(file.write_all(b"bytes")?)).unwrap();

        assert_eq!(std::fs::read(&file_path).unwrap(), b"bytes");
    }

    #[tokio::test]
    async fn test_export_to_parquet_via_transport_writes_named_typed_fields_and_quoted_rows() {
        let server = FakeExasolServer::serving_csv("1,\"Smith, John\"\n2,bob\n").await;
        let mut transport = describing_transport(id_name_columns());
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("users.parquet");

        let rows_written = export_to_parquet_via_transport(
            &mut transport,
            users_source(),
            &file_path,
            tunnel_options(&server),
        )
        .await
        .expect("a completed tunnel export must produce a Parquet file");

        assert_eq!(rows_written, 2);
        let batches = read_back(&file_path);
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].schema().as_ref(),
            &Schema::new(vec![
                Field::new("ID", DataType::Decimal128(18, 0), true),
                Field::new("NAME", DataType::Utf8, true),
            ])
        );
        assert_eq!(
            text_column(&batches[0], 1),
            vec![Some("Smith, John".to_string()), Some("bob".to_string())]
        );
    }

    #[tokio::test]
    async fn test_export_to_parquet_via_transport_ignores_null_value_for_the_tunnel_rows() {
        let server = FakeExasolServer::serving_csv("1,NULL\n2,\n").await;
        let mut transport = describing_transport(id_name_columns());
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("users.parquet");

        export_to_parquet_via_transport(
            &mut transport,
            users_source(),
            &file_path,
            tunnel_options(&server).with_null_value("NULL"),
        )
        .await
        .unwrap();

        assert_eq!(
            text_column(&read_back(&file_path)[0], 1),
            vec![Some("NULL".to_string()), None]
        );
    }

    /// An EXPORT that matches no rows still carries the source's schema.
    #[tokio::test]
    async fn test_export_to_parquet_via_transport_writes_a_schema_only_file_for_an_empty_export() {
        let server = FakeExasolServer::serving_csv("").await;
        let mut transport = describing_transport(id_name_columns());
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("users.parquet");

        let rows_written = export_to_parquet_via_transport(
            &mut transport,
            users_source(),
            &file_path,
            tunnel_options(&server),
        )
        .await
        .expect("an empty export is not a failure");

        assert_eq!(rows_written, 0);
        let schema = read_schema(&file_path);
        assert_eq!(schema.field(0).name(), "ID");
        assert_eq!(schema.field(1).data_type(), &DataType::Utf8);
        assert_eq!(
            read_back(&file_path)
                .iter()
                .map(RecordBatch::num_rows)
                .sum::<usize>(),
            0
        );
    }

    #[tokio::test]
    async fn test_export_to_parquet_via_transport_leaves_no_file_when_a_value_does_not_convert() {
        let server = FakeExasolServer::serving_csv("not-a-number,bob\n").await;
        let mut transport = describing_transport(id_name_columns());
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("users.parquet");

        let err = export_to_parquet_via_transport(
            &mut transport,
            users_source(),
            &file_path,
            tunnel_options(&server),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, ExportError::CsvParseError { row: 0, .. }),
            "got: {err}"
        );
        assert!(!file_path.exists());
    }

    /// Scenario: Export releases the schema prepared statement before the EXPORT statement runs
    #[tokio::test]
    async fn test_export_to_parquet_via_transport_closes_the_schema_statement_before_the_export() {
        let server = FakeExasolServer::serving_csv("1,alice\n").await;
        let mut sequence = Sequence::new();
        let mut transport = MockTransport::new();
        transport
            .expect_create_prepared_statement()
            .once()
            .in_sequence(&mut sequence)
            .returning(|_| Ok(handle_describing(id_name_columns())));
        transport
            .expect_close_prepared_statement()
            .once()
            .in_sequence(&mut sequence)
            .returning(|_| Ok(()));
        transport
            .expect_execute_query()
            .once()
            .in_sequence(&mut sequence)
            .returning(|_| Ok(QueryResult::row_count(1)));
        let directory = tempfile::TempDir::new().expect("temp dir");

        export_to_parquet_via_transport(
            &mut transport,
            users_source(),
            &directory.path().join("users.parquet"),
            tunnel_options(&server),
        )
        .await
        .unwrap();
    }

    /// Scenario: Export releases the schema prepared statement before the EXPORT statement runs
    #[tokio::test]
    async fn test_export_to_parquet_via_transport_closes_the_schema_statement_when_the_source_has_no_result_set(
    ) {
        let mut transport = MockTransport::new();
        transport
            .expect_create_prepared_statement()
            .once()
            .returning(|_| Ok(handle_describing(vec![])));
        transport
            .expect_close_prepared_statement()
            .once()
            .returning(|_| Ok(()));
        transport.expect_execute_query().never();
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("users.parquet");

        let err = export_to_parquet_via_transport(
            &mut transport,
            ExportSource::Query {
                sql: "CREATE TABLE S.T2 (X DECIMAL(1,0))".to_string(),
            },
            &file_path,
            ParquetExportOptions::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, ExportError::SqlExecutionError { message } if message.contains("no result set")),
            "got: {err}"
        );
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn test_export_to_parquet_via_transport_maps_prepare_failure_without_running_the_export()
    {
        let mut transport = MockTransport::new();
        transport.expect_create_prepared_statement().returning(|_| {
            Err(crate::error::TransportError::ProtocolError(
                "syntax error".to_string(),
            ))
        });
        transport.expect_close_prepared_statement().never();
        transport.expect_execute_query().never();
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("users.parquet");

        let err = export_to_parquet_via_transport(
            &mut transport,
            users_source(),
            &file_path,
            ParquetExportOptions::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, ExportError::SqlExecutionError { message } if message.contains("syntax error")),
            "got: {err}"
        );
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn test_export_to_parquet_via_transport_returns_io_error_when_the_file_cannot_be_created()
    {
        let server = FakeExasolServer::serving_csv("1,alice\n").await;
        let mut transport = describing_transport(id_name_columns());
        let directory = tempfile::TempDir::new().expect("temp dir");
        let file_path = directory.path().join("missing").join("users.parquet");

        let err = export_to_parquet_via_transport(
            &mut transport,
            users_source(),
            &file_path,
            tunnel_options(&server),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ExportError::IoError(_)), "got: {err}");
    }

    #[test]
    fn test_export_error_maps_a_parquet_writer_failure_to_csv_parse_error_at_row_zero() {
        let err = export_error(ParquetExportError::Parquet("close failed".to_string()));

        assert!(
            matches!(&err, ExportError::CsvParseError { row: 0, message } if message.contains("close failed")),
            "got: {err}"
        );
    }
}
