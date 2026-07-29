//! Arrow RecordBatch import functionality.
//!
//! This module provides utilities for importing Arrow RecordBatches and Arrow IPC
//! files/streams into Exasol tables by converting them to CSV format on-the-fly.
//!
//! # Overview
//!
//! Arrow data is converted to CSV format using [`ArrowToCsvWriter`] which:
//! - Handles all Arrow data types (Int, Float, String, Date, Timestamp, Decimal, etc.)
//! - Properly escapes CSV special characters
//! - Handles NULL values
//! - Supports configurable CSV format options
//!
//! # Example
//!

use super::ImportError;
use arrow::array::cast::AsArray;
use arrow::array::types::{
    Date32Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type,
    TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
    TimestampSecondType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow::array::{Array, RecordBatch};
use arrow::datatypes::DataType;
use std::fmt::Write as FmtWrite;
use std::io::Write;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

/// Options for CSV output when converting Arrow data.
#[derive(Debug, Clone)]
pub struct CsvWriterOptions {
    /// Column separator character (default: ',')
    pub column_separator: char,
    /// Column delimiter character for quoting (default: '"')
    pub column_delimiter: char,
    /// Row separator (default: '\n')
    pub row_separator: &'static str,
    /// String to represent NULL values (default: empty string)
    pub null_value: String,
    /// Whether to write column headers (default: false)
    pub write_header: bool,
    /// Date format string (default: "%Y-%m-%d")
    pub date_format: String,
    /// Timestamp format string (default: "%Y-%m-%d %H:%M:%S%.6f")
    pub timestamp_format: String,
}

impl Default for CsvWriterOptions {
    fn default() -> Self {
        Self {
            column_separator: ',',
            column_delimiter: '"',
            row_separator: "\n",
            null_value: String::new(),
            write_header: false,
            date_format: "%Y-%m-%d".to_string(),
            timestamp_format: "%Y-%m-%d %H:%M:%S%.6f".to_string(),
        }
    }
}

/// Options for Arrow import operations.
#[derive(Debug, Clone)]
pub struct ArrowImportOptions {
    /// Target schema name (optional)
    pub schema: Option<String>,
    /// Columns to import (optional, imports all if None)
    pub columns: Option<Vec<String>>,
    /// Batch size for chunked processing (default: 10000)
    pub batch_size: usize,
    /// CSV writer options
    pub csv_options: CsvWriterOptions,
    /// Whether to use TLS for the HTTP transport tunnel
    pub use_tls: bool,
    /// Exasol host for HTTP transport connection.
    /// This is typically the same host as the WebSocket connection.
    pub host: String,
    /// Exasol port for HTTP transport connection.
    /// This is typically the same port as the WebSocket connection.
    pub port: u16,
}

impl Default for ArrowImportOptions {
    fn default() -> Self {
        Self {
            schema: None,
            columns: None,
            batch_size: 10000,
            csv_options: CsvWriterOptions::default(),
            use_tls: false,
            host: String::new(),
            port: 0,
        }
    }
}

impl ArrowImportOptions {
    /// Create new options with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    #[must_use]
    pub fn columns(mut self, columns: Vec<String>) -> Self {
        self.columns = Some(columns);
        self
    }

    #[must_use]
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    #[must_use]
    pub fn null_value(mut self, value: impl Into<String>) -> Self {
        self.csv_options.null_value = value.into();
        self
    }

    #[must_use]
    pub fn column_separator(mut self, sep: char) -> Self {
        self.csv_options.column_separator = sep;
        self
    }

    #[must_use]
    pub fn column_delimiter(mut self, delim: char) -> Self {
        self.csv_options.column_delimiter = delim;
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
}

/// Arrow to CSV writer that converts RecordBatches to CSV format.
///
/// This writer handles:
/// - All Arrow numeric types (Int8-64, UInt8-64, Float32/64)
/// - String types (Utf8, LargeUtf8)
/// - Boolean types
/// - Date types (Date32)
/// - Timestamp types (with various precisions)
/// - Decimal128 types
/// - NULL handling
/// - Proper CSV escaping
pub struct ArrowToCsvWriter<W: AsyncWrite + Unpin> {
    writer: W,
    options: CsvWriterOptions,
    rows_written: usize,
}

impl<W: AsyncWrite + Unpin> ArrowToCsvWriter<W> {
    /// Create a new Arrow to CSV writer.
    pub fn new(writer: W, options: CsvWriterOptions) -> Self {
        Self {
            writer,
            options,
            rows_written: 0,
        }
    }

    /// Write a RecordBatch to CSV format.
    ///
    /// Returns the number of bytes written.
    pub async fn write_batch(&mut self, batch: &RecordBatch) -> Result<usize, ImportError> {
        let mut bytes_written = 0;
        let num_rows = batch.num_rows();

        // Write header if requested and this is the first batch
        if self.options.write_header && self.rows_written == 0 {
            let header = format_header(&self.options, batch);
            self.writer
                .write_all(header.as_bytes())
                .await
                .map_err(|e| ImportError::CsvWriteError(e.to_string()))?;
            bytes_written += header.len();
        }

        for row_idx in 0..num_rows {
            let row_str = format_row(&self.options, batch, row_idx)?;

            self.writer
                .write_all(row_str.as_bytes())
                .await
                .map_err(|e| ImportError::CsvWriteError(e.to_string()))?;
            bytes_written += row_str.len();
        }

        self.rows_written += num_rows;
        Ok(bytes_written)
    }

    /// Finish writing and flush the output.
    pub async fn finish(mut self) -> Result<usize, ImportError> {
        self.writer
            .flush()
            .await
            .map_err(|e| ImportError::CsvWriteError(e.to_string()))?;
        Ok(self.rows_written)
    }

    /// Get the number of rows written so far.
    #[must_use]
    pub fn rows_written(&self) -> usize {
        self.rows_written
    }
}

/// Synchronous Arrow to CSV converter for testing and simple use cases.
pub struct SyncArrowToCsvWriter<W: Write> {
    writer: W,
    options: CsvWriterOptions,
    rows_written: usize,
}

impl<W: Write> SyncArrowToCsvWriter<W> {
    /// Create a new synchronous Arrow to CSV writer.
    pub fn new(writer: W, options: CsvWriterOptions) -> Self {
        Self {
            writer,
            options,
            rows_written: 0,
        }
    }

    /// Write a RecordBatch to CSV format synchronously.
    pub fn write_batch(&mut self, batch: &RecordBatch) -> Result<usize, ImportError> {
        let mut bytes_written = 0;
        let num_rows = batch.num_rows();

        // Write header if requested and this is the first batch
        if self.options.write_header && self.rows_written == 0 {
            let header = format_header(&self.options, batch);
            self.writer
                .write_all(header.as_bytes())
                .map_err(|e| ImportError::CsvWriteError(e.to_string()))?;
            bytes_written += header.len();
        }

        for row_idx in 0..num_rows {
            let row_str = format_row(&self.options, batch, row_idx)?;

            self.writer
                .write_all(row_str.as_bytes())
                .map_err(|e| ImportError::CsvWriteError(e.to_string()))?;
            bytes_written += row_str.len();
        }

        self.rows_written += num_rows;
        Ok(bytes_written)
    }

    /// Finish writing and flush the output.
    pub fn finish(mut self) -> Result<usize, ImportError> {
        self.writer
            .flush()
            .map_err(|e| ImportError::CsvWriteError(e.to_string()))?;
        Ok(self.rows_written)
    }
}

/// Format the header row from a batch schema, terminated by the row separator.
fn format_header(options: &CsvWriterOptions, batch: &RecordBatch) -> String {
    let schema = batch.schema();
    let mut header = String::new();

    for (idx, field) in schema.fields().iter().enumerate() {
        if idx > 0 {
            header.push(options.column_separator);
        }
        header.push_str(&escape_string(options, field.name()));
    }
    header.push_str(options.row_separator);

    header
}

/// Format one batch row as a separator-joined CSV line, terminated by the row separator.
fn format_row(
    options: &CsvWriterOptions,
    batch: &RecordBatch,
    row_idx: usize,
) -> Result<String, ImportError> {
    let mut row_str = String::with_capacity(256);

    for col_idx in 0..batch.num_columns() {
        if col_idx > 0 {
            row_str.push(options.column_separator);
        }
        row_str.push_str(&format_value(options, batch.column(col_idx), row_idx)?);
    }

    row_str.push_str(options.row_separator);

    Ok(row_str)
}

/// Format a value from an Arrow array at the given row index.
fn format_value(
    options: &CsvWriterOptions,
    array: &dyn Array,
    row_idx: usize,
) -> Result<String, ImportError> {
    if array.is_null(row_idx) {
        return Ok(options.null_value.clone());
    }

    let data_type = array.data_type();
    match data_type {
        DataType::Boolean => {
            let arr = array.as_boolean();
            Ok(if arr.value(row_idx) { "true" } else { "false" }.to_string())
        }
        DataType::Int8 => Ok(array.as_primitive::<Int8Type>().value(row_idx).to_string()),
        DataType::Int16 => Ok(array.as_primitive::<Int16Type>().value(row_idx).to_string()),
        DataType::Int32 => Ok(array.as_primitive::<Int32Type>().value(row_idx).to_string()),
        DataType::Int64 => Ok(array.as_primitive::<Int64Type>().value(row_idx).to_string()),
        DataType::UInt8 => Ok(array.as_primitive::<UInt8Type>().value(row_idx).to_string()),
        DataType::UInt16 => Ok(array
            .as_primitive::<UInt16Type>()
            .value(row_idx)
            .to_string()),
        DataType::UInt32 => Ok(array
            .as_primitive::<UInt32Type>()
            .value(row_idx)
            .to_string()),
        DataType::UInt64 => Ok(array
            .as_primitive::<UInt64Type>()
            .value(row_idx)
            .to_string()),
        DataType::Float32 => {
            let val = array.as_primitive::<Float32Type>().value(row_idx);
            Ok(format_float(val as f64))
        }
        DataType::Float64 => {
            let val = array.as_primitive::<Float64Type>().value(row_idx);
            Ok(format_float(val))
        }
        DataType::Utf8 => {
            let arr = array.as_string::<i32>();
            Ok(escape_string(options, arr.value(row_idx)))
        }
        DataType::LargeUtf8 => {
            let arr = array.as_string::<i64>();
            Ok(escape_string(options, arr.value(row_idx)))
        }
        DataType::Date32 => {
            let val = array.as_primitive::<Date32Type>().value(row_idx);
            Ok(format_date32(val))
        }
        DataType::Timestamp(unit, _tz) => Ok(format_timestamp(array, row_idx, unit)),
        DataType::Decimal128(_precision, scale) => format_decimal_value(array, row_idx, *scale),
        DataType::Binary => {
            let arr = array.as_binary::<i32>();
            Ok(hex::encode(arr.value(row_idx)))
        }
        DataType::LargeBinary => {
            let arr = array.as_binary::<i64>();
            Ok(hex::encode(arr.value(row_idx)))
        }
        other => Err(ImportError::ConversionError(format!(
            "Unsupported Arrow type for CSV conversion: {:?}",
            other
        ))),
    }
}

/// Format a timestamp value, normalizing every Arrow time unit to microseconds.
fn format_timestamp(
    array: &dyn Array,
    row_idx: usize,
    unit: &arrow::datatypes::TimeUnit,
) -> String {
    use arrow::datatypes::TimeUnit;

    let micros = match unit {
        TimeUnit::Second => {
            let val = array.as_primitive::<TimestampSecondType>().value(row_idx);
            val * 1_000_000
        }
        TimeUnit::Millisecond => {
            let val = array
                .as_primitive::<TimestampMillisecondType>()
                .value(row_idx);
            val * 1_000
        }
        TimeUnit::Microsecond => array
            .as_primitive::<TimestampMicrosecondType>()
            .value(row_idx),
        TimeUnit::Nanosecond => {
            let val = array
                .as_primitive::<TimestampNanosecondType>()
                .value(row_idx);
            val / 1_000
        }
    };

    format_timestamp_micros(micros)
}

/// Format a Decimal128 array element using the column's declared scale.
fn format_decimal_value(
    array: &dyn Array,
    row_idx: usize,
    scale: i8,
) -> Result<String, ImportError> {
    let arr = array
        .as_any()
        .downcast_ref::<arrow::array::Decimal128Array>()
        .ok_or_else(|| ImportError::ConversionError("Expected Decimal128Array".to_string()))?;

    Ok(format_decimal128(arr.value(row_idx), scale))
}

/// Escape a string value for CSV.
///
/// Quotes the string if it contains the separator, delimiter, or newlines.
/// Doubles any delimiter characters inside the string.
fn escape_string(options: &CsvWriterOptions, s: &str) -> String {
    let sep = options.column_separator;
    let delim = options.column_delimiter;

    let needs_quoting =
        s.contains(sep) || s.contains(delim) || s.contains('\n') || s.contains('\r');

    if !needs_quoting {
        return s.to_string();
    }

    let mut result = String::with_capacity(s.len() + 4);
    result.push(delim);
    for c in s.chars() {
        if c == delim {
            result.push(delim);
        }
        result.push(c);
    }
    result.push(delim);
    result
}

/// Format a floating-point value for CSV.
fn format_float(val: f64) -> String {
    if val.is_nan() {
        "NaN".to_string()
    } else if val.is_infinite() {
        if val.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    } else {
        // Use default Rust formatting which handles precision well
        val.to_string()
    }
}

/// Format a Date32 value (days since epoch) to YYYY-MM-DD.
fn format_date32(days: i32) -> String {
    // Days since Unix epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// Convert days since epoch to year, month, day.
fn days_to_ymd(days: i32) -> (i32, u32, u32) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Format a timestamp in microseconds since epoch to YYYY-MM-DD HH:MM:SS.ffffff.
fn format_timestamp_micros(micros: i64) -> String {
    let total_seconds = micros / 1_000_000;
    let frac_micros = (micros % 1_000_000).unsigned_abs();

    let days = (total_seconds / 86400) as i32;
    let day_seconds = (total_seconds % 86400).unsigned_abs() as u32;

    let (year, month, day) = days_to_ymd(days);
    let hours = day_seconds / 3600;
    let minutes = (day_seconds % 3600) / 60;
    let seconds = day_seconds % 60;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
        year, month, day, hours, minutes, seconds, frac_micros
    )
}

/// Format a Decimal128 value to a string with proper scale.
fn format_decimal128(value: i128, scale: i8) -> String {
    if scale <= 0 {
        // No decimal point needed, value is integer
        let multiplier = 10_i128.pow((-scale) as u32);
        return (value * multiplier).to_string();
    }

    let scale = scale as u32;
    let divisor = 10_i128.pow(scale);

    let is_negative = value < 0;
    let abs_value = value.unsigned_abs();

    let integer_part = abs_value / divisor as u128;
    let fractional_part = abs_value % divisor as u128;

    let mut result = String::new();
    if is_negative {
        result.push('-');
    }
    write!(
        result,
        "{}.{:0width$}",
        integer_part,
        fractional_part,
        width = scale as usize
    )
    .unwrap();

    result
}

/// Convert a RecordBatch to CSV bytes.
///
/// This is a convenience function for converting a single RecordBatch to CSV format
/// in memory.
pub fn record_batch_to_csv(
    batch: &RecordBatch,
    options: CsvWriterOptions,
) -> Result<Vec<u8>, ImportError> {
    let mut buffer = Vec::new();
    let mut writer = SyncArrowToCsvWriter::new(&mut buffer, options);
    writer.write_batch(batch)?;
    writer.finish()?;
    Ok(buffer)
}

// ============================================================================
// Import Functions (placeholder implementations for session integration)
// ============================================================================

/// Import a single RecordBatch into an Exasol table.
///
/// This function converts the RecordBatch to CSV format and streams it to Exasol
/// using the HTTP transport protocol.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements. Takes SQL string and returns row count.
/// * `table` - The target table name
/// * `batch` - The RecordBatch to import
/// * `options` - Import options
///
/// # Returns
///
/// The number of rows imported.
///
/// # Example
///
pub async fn import_from_record_batch<F, Fut>(
    execute_sql: F,
    table: &str,
    batch: &RecordBatch,
    options: ArrowImportOptions,
) -> Result<u64, ImportError>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<u64, String>>,
{
    // Convert RecordBatch to CSV bytes
    let csv_bytes = record_batch_to_csv(batch, options.csv_options.clone())?;

    // Use the CSV import stream function to send the data
    super::csv::import_from_stream(
        execute_sql,
        table,
        std::io::Cursor::new(csv_bytes),
        csv_import_options(&options),
    )
    .await
}

/// Translate Arrow import options into the CSV import options of the transport path.
///
/// Arrow data is always converted to UTF-8 CSV with LF rows and no compression,
/// so only the caller-visible options carry over.
fn csv_import_options(options: &ArrowImportOptions) -> super::csv::CsvImportOptions {
    super::csv::CsvImportOptions {
        encoding: "UTF-8".to_string(),
        column_separator: options.csv_options.column_separator,
        column_delimiter: options.csv_options.column_delimiter,
        row_separator: crate::query::import::RowSeparator::LF,
        skip_rows: 0,
        null_value: if options.csv_options.null_value.is_empty() {
            None
        } else {
            Some(options.csv_options.null_value.clone())
        },
        trim_mode: crate::query::import::TrimMode::None,
        compression: crate::query::import::Compression::None,
        reject_limit: None,
        use_tls: options.use_tls,
        schema: options.schema.clone(),
        columns: options.columns.clone(),
        host: options.host.clone(),
        port: options.port,
    }
}

/// Import multiple RecordBatches from an iterator into an Exasol table.
///
/// This function converts each RecordBatch to CSV format and streams it to Exasol
/// using the HTTP transport protocol. All batches are concatenated into a single
/// CSV stream.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements. Takes SQL string and returns row count.
/// * `table` - The target table name
/// * `batches` - An iterator of RecordBatches to import
/// * `options` - Import options
///
/// # Returns
///
/// The number of rows imported.
///
/// # Example
///
pub async fn import_from_record_batches<I, F, Fut>(
    execute_sql: F,
    table: &str,
    batches: I,
    options: ArrowImportOptions,
) -> Result<u64, ImportError>
where
    I: IntoIterator<Item = RecordBatch>,
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<u64, String>>,
{
    // Convert all batches to CSV and concatenate
    let mut all_csv_bytes = Vec::new();
    let mut writer = SyncArrowToCsvWriter::new(&mut all_csv_bytes, options.csv_options.clone());

    for batch in batches {
        writer.write_batch(&batch)?;
    }
    writer.finish()?;

    // Use the CSV import stream function to send the data
    super::csv::import_from_stream(
        execute_sql,
        table,
        std::io::Cursor::new(all_csv_bytes),
        csv_import_options(&options),
    )
    .await
}

/// Import from an Arrow IPC file/stream into an Exasol table.
///
/// This function reads Arrow IPC format data, converts each RecordBatch to CSV,
/// and streams it to Exasol using the HTTP transport protocol.
///
/// # Arguments
///
/// * `execute_sql` - Function to execute SQL statements. Takes SQL string and returns row count.
/// * `table` - The target table name
/// * `reader` - An async reader containing Arrow IPC data
/// * `options` - Import options
///
/// # Returns
///
/// The number of rows imported.
///
/// # Example
///
pub async fn import_from_arrow_ipc<R, F, Fut>(
    execute_sql: F,
    table: &str,
    mut reader: R,
    options: ArrowImportOptions,
) -> Result<u64, ImportError>
where
    R: AsyncRead + Unpin + Send,
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<u64, String>>,
{
    use tokio::io::AsyncReadExt;

    // Read all IPC data into memory
    let mut buffer = Vec::new();
    reader
        .read_to_end(&mut buffer)
        .await
        .map_err(ImportError::IoError)?;

    // Parse as Arrow IPC
    let cursor = std::io::Cursor::new(buffer);
    let ipc_reader = arrow::ipc::reader::FileReader::try_new(cursor, None)
        .map_err(|e| ImportError::ArrowIpcError(e.to_string()))?;

    // Convert all batches to CSV
    let mut all_csv_bytes = Vec::new();
    let mut writer = SyncArrowToCsvWriter::new(&mut all_csv_bytes, options.csv_options.clone());

    for batch_result in ipc_reader {
        let batch = batch_result.map_err(|e| ImportError::ArrowIpcError(e.to_string()))?;
        writer.write_batch(&batch)?;
    }
    writer.finish()?;

    // Use the CSV import stream function to send the data
    super::csv::import_from_stream(
        execute_sql,
        table,
        std::io::Cursor::new(all_csv_bytes),
        csv_import_options(&options),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{
        ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array,
        Float64Array, Int32Array, Int64Array, StringArray, TimestampMicrosecondArray,
    };
    use arrow::datatypes::{Field, Schema};
    use std::sync::Arc;

    fn create_test_batch() -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("value", DataType::Float64, true),
        ]);

        let id_array = Int32Array::from(vec![1, 2, 3]);
        let name_array = StringArray::from(vec![Some("Alice"), Some("Bob"), None]);
        let value_array = Float64Array::from(vec![Some(1.5), Some(2.5), Some(3.5)]);

        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(id_array) as ArrayRef,
                Arc::new(name_array) as ArrayRef,
                Arc::new(value_array) as ArrayRef,
            ],
        )
        .unwrap()
    }

    #[test]
    fn test_csv_writer_options_default() {
        let options = CsvWriterOptions::default();
        assert_eq!(options.column_separator, ',');
        assert_eq!(options.column_delimiter, '"');
        assert_eq!(options.row_separator, "\n");
        assert_eq!(options.null_value, "");
        assert!(!options.write_header);
    }

    #[test]
    fn test_arrow_import_options_default() {
        let options = ArrowImportOptions::default();
        assert!(options.schema.is_none());
        assert!(options.columns.is_none());
        assert_eq!(options.batch_size, 10000);
        assert!(!options.use_tls);
        assert_eq!(options.host, "");
        assert_eq!(options.port, 0);
    }

    #[test]
    fn test_arrow_import_options_builder() {
        let options = ArrowImportOptions::new()
            .schema("myschema")
            .columns(vec!["col1".to_string(), "col2".to_string()])
            .batch_size(5000)
            .null_value("NULL")
            .column_separator(';')
            .use_tls(true)
            .exasol_host("exasol.example.com")
            .exasol_port(8563);

        assert_eq!(options.schema, Some("myschema".to_string()));
        assert_eq!(
            options.columns,
            Some(vec!["col1".to_string(), "col2".to_string()])
        );
        assert_eq!(options.batch_size, 5000);
        assert_eq!(options.csv_options.null_value, "NULL");
        assert_eq!(options.csv_options.column_separator, ';');
        assert!(options.use_tls);
        assert_eq!(options.host, "exasol.example.com");
        assert_eq!(options.port, 8563);
    }

    #[test]
    fn test_arrow_import_options_use_tls_builder() {
        assert!(ArrowImportOptions::default().use_tls(true).use_tls);
        assert!(!ArrowImportOptions::default().use_tls(false).use_tls);
    }

    #[test]
    fn test_record_batch_to_csv_basic() {
        let batch = create_test_batch();
        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("1,Alice,1.5"));
        assert!(csv_str.contains("2,Bob,2.5"));
        assert!(csv_str.contains("3,,3.5"));
    }

    #[test]
    fn test_record_batch_to_csv_with_header() {
        let batch = create_test_batch();
        let options = CsvWriterOptions {
            write_header: true,
            ..Default::default()
        };
        let result = record_batch_to_csv(&batch, options).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.starts_with("id,name,value\n"));
    }

    #[test]
    fn test_record_batch_to_csv_with_custom_separator() {
        let batch = create_test_batch();
        let options = CsvWriterOptions {
            column_separator: ';',
            ..Default::default()
        };
        let result = record_batch_to_csv(&batch, options).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("1;Alice;1.5"));
    }

    #[test]
    fn test_record_batch_to_csv_with_null_value() {
        let batch = create_test_batch();
        let options = CsvWriterOptions {
            null_value: "NULL".to_string(),
            ..Default::default()
        };
        let result = record_batch_to_csv(&batch, options).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("3,NULL,3.5"));
    }

    #[test]
    fn test_escape_string_no_special_chars() {
        let escaped = escape_string(&CsvWriterOptions::default(), "hello");
        assert_eq!(escaped, "hello");
    }

    #[test]
    fn test_escape_string_with_separator() {
        let escaped = escape_string(&CsvWriterOptions::default(), "hello,world");
        assert_eq!(escaped, "\"hello,world\"");
    }

    #[test]
    fn test_escape_string_with_delimiter() {
        let escaped = escape_string(&CsvWriterOptions::default(), "say \"hello\"");
        assert_eq!(escaped, "\"say \"\"hello\"\"\"");
    }

    #[test]
    fn test_escape_string_with_newline() {
        let escaped = escape_string(&CsvWriterOptions::default(), "line1\nline2");
        assert_eq!(escaped, "\"line1\nline2\"");
    }

    #[test]
    fn test_format_float_normal() {
        assert_eq!(format_float(1.5), "1.5");
        assert_eq!(format_float(-2.5), "-2.5");
        assert_eq!(format_float(0.0), "0");
    }

    #[test]
    fn test_format_float_special() {
        assert_eq!(format_float(f64::NAN), "NaN");
        assert_eq!(format_float(f64::INFINITY), "Infinity");
        assert_eq!(format_float(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn test_format_date32() {
        assert_eq!(format_date32(0), "1970-01-01");
        assert_eq!(format_date32(1), "1970-01-02");
        assert_eq!(format_date32(365), "1971-01-01");
        assert_eq!(format_date32(-1), "1969-12-31");
    }

    #[test]
    fn test_format_timestamp_micros() {
        assert_eq!(format_timestamp_micros(0), "1970-01-01 00:00:00.000000");
        assert_eq!(
            format_timestamp_micros(1_000_000),
            "1970-01-01 00:00:01.000000"
        );
        assert_eq!(
            format_timestamp_micros(86_400_000_000),
            "1970-01-02 00:00:00.000000"
        );
        assert_eq!(
            format_timestamp_micros(123456),
            "1970-01-01 00:00:00.123456"
        );
    }

    #[test]
    fn test_format_decimal128() {
        assert_eq!(format_decimal128(12345, 2), "123.45");
        assert_eq!(format_decimal128(-12345, 2), "-123.45");
        assert_eq!(format_decimal128(100, 2), "1.00");
        assert_eq!(format_decimal128(1, 2), "0.01");
        assert_eq!(format_decimal128(12345, 0), "12345");
        assert_eq!(format_decimal128(12345, -2), "1234500");
    }

    #[test]
    fn test_boolean_array_conversion() {
        let schema = Schema::new(vec![Field::new("flag", DataType::Boolean, true)]);
        let arr = BooleanArray::from(vec![Some(true), Some(false), None]);
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("true"));
        assert!(csv_str.contains("false"));
    }

    #[test]
    fn test_int32_array_conversion() {
        let schema = Schema::new(vec![Field::new("num", DataType::Int32, true)]);
        let arr = Int32Array::from(vec![Some(42), Some(-100), None]);
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("42"));
        assert!(csv_str.contains("-100"));
    }

    #[test]
    fn test_int64_array_conversion() {
        let schema = Schema::new(vec![Field::new("big_num", DataType::Int64, false)]);
        let arr = Int64Array::from(vec![9_223_372_036_854_775_807i64, -1]);
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("9223372036854775807"));
        assert!(csv_str.contains("-1"));
    }

    #[test]
    fn test_float32_array_conversion() {
        let schema = Schema::new(vec![Field::new("val", DataType::Float32, true)]);
        let arr = Float32Array::from(vec![Some(1.5f32), Some(f32::INFINITY), None]);
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("1.5"));
        assert!(csv_str.contains("Infinity"));
    }

    #[test]
    fn test_date32_array_conversion() {
        let schema = Schema::new(vec![Field::new("date", DataType::Date32, false)]);
        let arr = Date32Array::from(vec![0, 365, 19724]); // 1970-01-01, 1971-01-01, 2024-01-01
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("1970-01-01"));
        assert!(csv_str.contains("1971-01-01"));
    }

    #[test]
    fn test_timestamp_array_conversion() {
        let schema = Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            false,
        )]);
        let arr = TimestampMicrosecondArray::from(vec![0, 1_000_000, 86_400_000_000]);
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("1970-01-01 00:00:00.000000"));
        assert!(csv_str.contains("1970-01-01 00:00:01.000000"));
        assert!(csv_str.contains("1970-01-02 00:00:00.000000"));
    }

    #[test]
    fn test_decimal128_array_conversion() {
        let schema = Schema::new(vec![Field::new(
            "price",
            DataType::Decimal128(10, 2),
            false,
        )]);
        let arr = Decimal128Array::from(vec![12345i128, -9999, 100])
            .with_precision_and_scale(10, 2)
            .unwrap();
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("123.45"));
        assert!(csv_str.contains("-99.99"));
        assert!(csv_str.contains("1.00"));
    }

    #[test]
    fn test_binary_array_conversion() {
        let schema = Schema::new(vec![Field::new("data", DataType::Binary, false)]);
        let arr = BinaryArray::from(vec![b"Hello".as_slice(), b"\xDE\xAD\xBE\xEF".as_slice()]);
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(arr) as ArrayRef]).unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("48656c6c6f")); // "Hello" in hex
        assert!(csv_str.contains("deadbeef"));
    }

    #[test]
    fn test_multiple_batches() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));

        let batch1 = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef],
        )
        .unwrap();
        let batch2 = RecordBatch::try_new(
            schema,
            vec![Arc::new(Int32Array::from(vec![3, 4])) as ArrayRef],
        )
        .unwrap();

        let mut buffer = Vec::new();
        let mut writer = SyncArrowToCsvWriter::new(&mut buffer, CsvWriterOptions::default());

        writer.write_batch(&batch1).unwrap();
        writer.write_batch(&batch2).unwrap();
        let total = writer.finish().unwrap();

        assert_eq!(total, 4);
        let csv_str = String::from_utf8(buffer).unwrap();
        assert!(csv_str.contains("1\n"));
        assert!(csv_str.contains("2\n"));
        assert!(csv_str.contains("3\n"));
        assert!(csv_str.contains("4\n"));
    }

    /// Test that record_batch_to_csv correctly validates Arrow type conversions.
    /// This tests the conversion logic that import_from_record_batch uses internally.
    #[test]
    fn test_record_batch_csv_conversion_for_import() {
        let batch = create_test_batch();
        let options = CsvWriterOptions::default();

        // Verify that the batch can be converted to CSV without errors
        let result = record_batch_to_csv(&batch, options);
        assert!(result.is_ok());

        let csv_bytes = result.unwrap();
        let csv_str = String::from_utf8(csv_bytes).unwrap();

        // Verify all rows are present
        assert!(csv_str.contains("1,Alice,1.5"));
        assert!(csv_str.contains("2,Bob,2.5"));
        // Row 3 has NULL name
        assert!(csv_str.contains("3,,3.5"));
    }

    /// Test that multiple batches can be combined into a single CSV stream.
    /// This tests the conversion logic that import_from_record_batches uses internally.
    #[test]
    fn test_multiple_record_batches_csv_conversion_for_import() {
        let batch1 = create_test_batch();
        let batch2 = create_test_batch();
        let options = CsvWriterOptions::default();

        // Convert batches using the same pattern as import_from_record_batches
        let mut all_csv_bytes = Vec::new();
        let mut writer = SyncArrowToCsvWriter::new(&mut all_csv_bytes, options);

        writer.write_batch(&batch1).unwrap();
        writer.write_batch(&batch2).unwrap();
        let total_rows = writer.finish().unwrap();

        assert_eq!(total_rows, 6);

        let csv_str = String::from_utf8(all_csv_bytes).unwrap();
        // Both batches should be present (6 rows total = 2 x 3 rows)
        let lines: Vec<&str> = csv_str.lines().collect();
        assert_eq!(lines.len(), 6);
    }

    // ========================================================================
    // Task 7.4: Arrow import unit tests
    // ========================================================================

    // 7.4.1: Test all Arrow type conversions
    // ----------------------------------------

    #[test]
    fn test_all_integer_types_conversion() {
        use arrow::array::{
            Int16Array, Int8Array, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
        };

        let schema = Schema::new(vec![
            Field::new("i8", DataType::Int8, true),
            Field::new("i16", DataType::Int16, true),
            Field::new("i32", DataType::Int32, true),
            Field::new("i64", DataType::Int64, true),
            Field::new("u8", DataType::UInt8, true),
            Field::new("u16", DataType::UInt16, true),
            Field::new("u32", DataType::UInt32, true),
            Field::new("u64", DataType::UInt64, true),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Int8Array::from(vec![Some(i8::MIN), Some(i8::MAX), None])) as ArrayRef,
                Arc::new(Int16Array::from(vec![Some(i16::MIN), Some(i16::MAX), None])) as ArrayRef,
                Arc::new(Int32Array::from(vec![Some(i32::MIN), Some(i32::MAX), None])) as ArrayRef,
                Arc::new(Int64Array::from(vec![Some(i64::MIN), Some(i64::MAX), None])) as ArrayRef,
                Arc::new(UInt8Array::from(vec![Some(u8::MIN), Some(u8::MAX), None])) as ArrayRef,
                Arc::new(UInt16Array::from(vec![
                    Some(u16::MIN),
                    Some(u16::MAX),
                    None,
                ])) as ArrayRef,
                Arc::new(UInt32Array::from(vec![
                    Some(u32::MIN),
                    Some(u32::MAX),
                    None,
                ])) as ArrayRef,
                Arc::new(UInt64Array::from(vec![
                    Some(u64::MIN),
                    Some(u64::MAX),
                    None,
                ])) as ArrayRef,
            ],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Check min values row
        assert!(csv_str.contains("-128,-32768,-2147483648,-9223372036854775808,0,0,0,0"));
        // Check max values row
        assert!(csv_str.contains(
            "127,32767,2147483647,9223372036854775807,255,65535,4294967295,18446744073709551615"
        ));
    }

    #[test]
    fn test_all_float_types_conversion() {
        let schema = Schema::new(vec![
            Field::new("f32", DataType::Float32, true),
            Field::new("f64", DataType::Float64, true),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Float32Array::from(vec![
                    Some(1.5f32),
                    Some(-2.5f32),
                    Some(f32::INFINITY),
                    Some(f32::NEG_INFINITY),
                    Some(f32::NAN),
                    None,
                ])) as ArrayRef,
                Arc::new(Float64Array::from(vec![
                    Some(1.5f64),
                    Some(-2.5f64),
                    Some(f64::INFINITY),
                    Some(f64::NEG_INFINITY),
                    Some(f64::NAN),
                    None,
                ])) as ArrayRef,
            ],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("1.5,1.5"));
        assert!(csv_str.contains("-2.5,-2.5"));
        assert!(csv_str.contains("Infinity,Infinity"));
        assert!(csv_str.contains("-Infinity,-Infinity"));
        assert!(csv_str.contains("NaN,NaN"));
    }

    #[test]
    fn test_all_timestamp_units_conversion() {
        use arrow::array::{
            TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
        };

        let schema = Schema::new(vec![
            Field::new(
                "ts_sec",
                DataType::Timestamp(arrow::datatypes::TimeUnit::Second, None),
                false,
            ),
            Field::new(
                "ts_ms",
                DataType::Timestamp(arrow::datatypes::TimeUnit::Millisecond, None),
                false,
            ),
            Field::new(
                "ts_us",
                DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
                false,
            ),
            Field::new(
                "ts_ns",
                DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                false,
            ),
        ]);

        // All represent the same timestamp: 1970-01-01 00:00:01.000000
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(TimestampSecondArray::from(vec![1i64])) as ArrayRef,
                Arc::new(TimestampMillisecondArray::from(vec![1000i64])) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(vec![1_000_000i64])) as ArrayRef,
                Arc::new(TimestampNanosecondArray::from(vec![1_000_000_000i64])) as ArrayRef,
            ],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // All should produce the same timestamp string
        let expected = "1970-01-01 00:00:01.000000";
        let row = csv_str.lines().next().unwrap();
        let parts: Vec<&str> = row.split(',').collect();
        assert_eq!(parts.len(), 4);
        for part in parts {
            assert_eq!(part, expected);
        }
    }

    #[test]
    fn test_string_types_conversion() {
        use arrow::array::LargeStringArray;

        let schema = Schema::new(vec![
            Field::new("utf8", DataType::Utf8, true),
            Field::new("large_utf8", DataType::LargeUtf8, true),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec![Some("hello"), Some("world"), None])) as ArrayRef,
                Arc::new(LargeStringArray::from(vec![
                    Some("large"),
                    Some("string"),
                    None,
                ])) as ArrayRef,
            ],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("hello,large"));
        assert!(csv_str.contains("world,string"));
    }

    #[test]
    fn test_binary_types_conversion() {
        use arrow::array::LargeBinaryArray;

        let schema = Schema::new(vec![
            Field::new("bin", DataType::Binary, false),
            Field::new("large_bin", DataType::LargeBinary, false),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(BinaryArray::from(vec![
                    b"abc".as_slice(),
                    b"\x00\xff".as_slice(),
                ])) as ArrayRef,
                Arc::new(LargeBinaryArray::from(vec![
                    b"def".as_slice(),
                    b"\x01\x02".as_slice(),
                ])) as ArrayRef,
            ],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Binary is hex-encoded
        assert!(csv_str.contains("616263,646566")); // "abc" and "def" in hex
        assert!(csv_str.contains("00ff,0102")); // binary bytes in hex
    }

    // 7.4.2: Test NULL value handling
    // ----------------------------------------

    #[test]
    fn test_null_values_default_representation() {
        let schema = Schema::new(vec![
            Field::new("str", DataType::Utf8, true),
            Field::new("int", DataType::Int32, true),
            Field::new("float", DataType::Float64, true),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec![
                    None as Option<&str>,
                    Some("value"),
                    None,
                ])) as ArrayRef,
                Arc::new(Int32Array::from(vec![None, Some(42), None])) as ArrayRef,
                Arc::new(Float64Array::from(vec![None, Some(3.125), None])) as ArrayRef,
            ],
        )
        .unwrap();

        // Default: null_value is empty string
        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Rows with all NULLs should be ",,\n"
        assert!(csv_str.contains(",,"));
        // Row with values should be "value,42,3.125"
        assert!(csv_str.contains("value,42,3.125"));
    }

    #[test]
    fn test_null_values_custom_representation() {
        let schema = Schema::new(vec![
            Field::new("str", DataType::Utf8, true),
            Field::new("int", DataType::Int32, true),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(StringArray::from(vec![None as Option<&str>, Some("value")])) as ArrayRef,
                Arc::new(Int32Array::from(vec![None, Some(42)])) as ArrayRef,
            ],
        )
        .unwrap();

        let options = CsvWriterOptions {
            null_value: "\\N".to_string(),
            ..Default::default()
        };
        let result = record_batch_to_csv(&batch, options).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // NULLs should be represented as "\N"
        assert!(csv_str.contains("\\N,\\N"));
        assert!(csv_str.contains("value,42"));
    }

    #[test]
    fn test_null_values_in_nested_data() {
        // Test NULL handling in various array types
        let schema = Schema::new(vec![
            Field::new("bool", DataType::Boolean, true),
            Field::new("date", DataType::Date32, true),
            Field::new("decimal", DataType::Decimal128(10, 2), true),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(BooleanArray::from(vec![Some(true), None, Some(false)])) as ArrayRef,
                Arc::new(Date32Array::from(vec![Some(0), None, Some(365)])) as ArrayRef,
                Arc::new(
                    Decimal128Array::from(vec![Some(12345i128), None, Some(100)])
                        .with_precision_and_scale(10, 2)
                        .unwrap(),
                ) as ArrayRef,
            ],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        assert!(csv_str.contains("true,1970-01-01,123.45"));
        assert!(csv_str.contains(",,")); // Row with all NULLs
        assert!(csv_str.contains("false,1971-01-01,1.00"));
    }

    // 7.4.3: Test special character escaping
    // ----------------------------------------

    #[test]
    fn test_escape_comma_in_string() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["hello,world"])) as ArrayRef],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Comma-containing strings should be quoted
        assert!(csv_str.contains("\"hello,world\""));
    }

    #[test]
    fn test_escape_quotes_in_string() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["say \"hello\""])) as ArrayRef],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Quotes should be doubled inside quoted strings
        assert!(csv_str.contains("\"say \"\"hello\"\"\""));
    }

    #[test]
    fn test_escape_newlines_in_string() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["line1\nline2", "line1\r\nline2"])) as ArrayRef],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Newline-containing strings should be quoted
        assert!(csv_str.contains("\"line1\nline2\""));
        assert!(csv_str.contains("\"line1\r\nline2\""));
    }

    #[test]
    fn test_escape_carriage_return_in_string() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["text\rwith\rCR"])) as ArrayRef],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Carriage return should trigger quoting
        assert!(csv_str.contains("\"text\rwith\rCR\""));
    }

    #[test]
    fn test_escape_multiple_special_chars() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec![
                "combo: \"quoted\", newline\n, and comma",
            ])) as ArrayRef],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Complex string with multiple special chars
        assert!(csv_str.contains("\"combo: \"\"quoted\"\", newline\n, and comma\""));
    }

    #[test]
    fn test_custom_separator_escaping() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["value;with;semicolons"])) as ArrayRef],
        )
        .unwrap();

        // Use semicolon as separator
        let options = CsvWriterOptions {
            column_separator: ';',
            ..Default::default()
        };
        let result = record_batch_to_csv(&batch, options).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // String containing semicolons should be quoted when separator is semicolon
        assert!(csv_str.contains("\"value;with;semicolons\""));
    }

    #[test]
    fn test_custom_delimiter_escaping() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["value'with'quotes"])) as ArrayRef],
        )
        .unwrap();

        // Use single quote as delimiter
        let options = CsvWriterOptions {
            column_delimiter: '\'',
            ..Default::default()
        };
        let result = record_batch_to_csv(&batch, options).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // String containing single quotes should be escaped when delimiter is single quote
        assert!(csv_str.contains("'value''with''quotes'"));
    }

    #[test]
    fn test_no_escaping_for_clean_string() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["clean simple text"])) as ArrayRef],
        )
        .unwrap();

        let result = record_batch_to_csv(&batch, CsvWriterOptions::default()).unwrap();
        let csv_str = String::from_utf8(result).unwrap();

        // Clean strings should not be quoted
        assert!(csv_str.contains("clean simple text"));
        assert!(!csv_str.contains("\"clean simple text\""));
    }

    #[test]
    fn test_days_to_ymd_edge_cases() {
        // Test boundary dates
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));

        // Leap year date - 2000-02-29 is day 11016 from epoch
        let (y, m, d) = days_to_ymd(11016);
        assert_eq!((y, m, d), (2000, 2, 29));

        // 2000-03-01 is day 11017 from epoch
        let (y, m, d) = days_to_ymd(11017);
        assert_eq!((y, m, d), (2000, 3, 1));

        // End of 1999 - December 31, 1999 is day 10956 from epoch
        let (y, m, d) = days_to_ymd(10956);
        assert_eq!((y, m, d), (1999, 12, 31));
    }

    #[test]
    fn test_days_to_ymd_before_epoch() {
        assert_eq!(days_to_ymd(-1), (1969, 12, 31));
        assert_eq!(days_to_ymd(-365), (1969, 1, 1));
        // 1900-01-01 is 25567 days before the epoch; 1900 is not a leap year.
        assert_eq!(days_to_ymd(-25567), (1900, 1, 1));
        // Crosses the negative-era branch of the Hinnant algorithm.
        assert_eq!(days_to_ymd(-719_468), (0, 3, 1));
        assert_eq!(days_to_ymd(-719_469), (0, 2, 29));
        // Before the year-zero era boundary, where the era divisor turns negative.
        assert_eq!(days_to_ymd(-800_000), (-221, 9, 4));
    }

    #[test]
    fn test_format_date32_before_epoch() {
        assert_eq!(format_date32(-1), "1969-12-31");
        assert_eq!(format_date32(-25567), "1900-01-01");
    }

    #[test]
    fn test_format_timestamp_micros_truncates_negative_day_offset() {
        // Characterization: integer division truncates toward zero, so a
        // pre-epoch timestamp keeps the epoch date and an absolute time of day.
        // The parquet import path (chrono-based) renders these differently.
        assert_eq!(
            format_timestamp_micros(-1_000_000),
            "1970-01-01 00:00:01.000000"
        );
        assert_eq!(
            format_timestamp_micros(-86_400_000_000),
            "1969-12-31 00:00:00.000000"
        );
    }

    #[test]
    fn test_format_value_rejects_unsupported_type() {
        let array = arrow::array::Time32SecondArray::from(vec![1]);
        let options = CsvWriterOptions::default();

        let err = format_value(&options, &array, 0).unwrap_err();

        assert!(
            err.to_string()
                .contains("Unsupported Arrow type for CSV conversion"),
            "got: {err}"
        );
    }

    #[test]
    fn test_format_value_null_uses_configured_null_marker() {
        let array = arrow::array::StringArray::from(vec![None::<&str>]);
        let options = CsvWriterOptions {
            null_value: "\\N".to_string(),
            ..Default::default()
        };

        assert_eq!(format_value(&options, &array, 0).unwrap(), "\\N");
    }

    #[test]
    fn test_format_header_escapes_names_containing_the_separator() {
        let schema = Schema::new(vec![
            Field::new("a,b", DataType::Int32, false),
            Field::new("plain", DataType::Int32, false),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Int32Array::from(vec![1])),
                Arc::new(Int32Array::from(vec![2])),
            ],
        )
        .unwrap();

        let header = format_header(&CsvWriterOptions::default(), &batch);

        assert_eq!(header, "\"a,b\",plain\n");
    }

    #[test]
    fn test_format_row_joins_columns_and_terminates_with_row_separator() {
        let batch = create_test_batch();

        let row = format_row(&CsvWriterOptions::default(), &batch, 0).unwrap();

        assert_eq!(row, "1,Alice,1.5\n");
    }

    #[test]
    fn test_csv_import_options_maps_empty_null_value_to_none() {
        let options = ArrowImportOptions::new()
            .exasol_host("db.example.com")
            .exasol_port(8563)
            .use_tls(true)
            .column_separator(';')
            .column_delimiter('\'');

        let csv_options = csv_import_options(&options);

        assert_eq!(csv_options.encoding, "UTF-8");
        assert_eq!(csv_options.column_separator, ';');
        assert_eq!(csv_options.column_delimiter, '\'');
        assert_eq!(csv_options.null_value, None);
        assert_eq!(csv_options.skip_rows, 0);
        assert!(csv_options.use_tls);
        assert_eq!(csv_options.host, "db.example.com");
        assert_eq!(csv_options.port, 8563);
        assert_eq!(csv_options.schema, None);
        assert_eq!(csv_options.columns, None);
    }

    #[test]
    fn test_csv_import_options_carries_null_value_schema_and_columns() {
        let options = ArrowImportOptions::new()
            .null_value("NULL")
            .schema("staging")
            .columns(vec!["a".to_string(), "b".to_string()]);

        let csv_options = csv_import_options(&options);

        assert_eq!(csv_options.null_value, Some("NULL".to_string()));
        assert_eq!(csv_options.schema, Some("staging".to_string()));
        assert_eq!(
            csv_options.columns,
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[tokio::test]
    async fn test_async_writer_writes_rows_and_reports_byte_count() {
        let mut buffer = Vec::new();
        let mut writer = ArrowToCsvWriter::new(&mut buffer, CsvWriterOptions::default());

        let bytes = writer.write_batch(&create_test_batch()).await.unwrap();

        assert_eq!(writer.rows_written(), 3);
        assert_eq!(bytes, buffer.len());
        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "1,Alice,1.5\n2,Bob,2.5\n3,,3.5\n"
        );
    }

    #[tokio::test]
    async fn test_async_writer_writes_header_only_before_the_first_batch() {
        let options = CsvWriterOptions {
            write_header: true,
            ..Default::default()
        };
        let mut buffer = Vec::new();
        let mut writer = ArrowToCsvWriter::new(&mut buffer, options);

        writer.write_batch(&create_test_batch()).await.unwrap();
        writer.write_batch(&create_test_batch()).await.unwrap();
        let rows = writer.finish().await.unwrap();

        let csv = String::from_utf8(buffer).unwrap();
        assert_eq!(rows, 6);
        assert_eq!(csv.matches("id,name,value\n").count(), 1);
        assert!(csv.starts_with("id,name,value\n1,Alice,1.5\n"));
    }

    #[tokio::test]
    async fn test_async_writer_formats_every_supported_arrow_type() {
        use arrow::array::{
            BinaryArray, Decimal128Array, LargeBinaryArray, LargeStringArray,
            TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt16Array,
            UInt32Array, UInt64Array, UInt8Array,
        };
        use arrow::datatypes::TimeUnit;

        let schema = Schema::new(vec![
            Field::new("b", DataType::Boolean, false),
            Field::new("i8", DataType::Int8, false),
            Field::new("i16", DataType::Int16, false),
            Field::new("i64", DataType::Int64, false),
            Field::new("u8", DataType::UInt8, false),
            Field::new("u16", DataType::UInt16, false),
            Field::new("u32", DataType::UInt32, false),
            Field::new("u64", DataType::UInt64, false),
            Field::new("f32", DataType::Float32, false),
            Field::new("f64", DataType::Float64, false),
            Field::new("s", DataType::Utf8, false),
            Field::new("ls", DataType::LargeUtf8, false),
            Field::new("d", DataType::Date32, false),
            Field::new("ts_s", DataType::Timestamp(TimeUnit::Second, None), false),
            Field::new(
                "ts_ms",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
            Field::new(
                "ts_us",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new(
                "ts_ns",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                false,
            ),
            Field::new("dec", DataType::Decimal128(10, 2), false),
            Field::new("bin", DataType::Binary, false),
            Field::new("lbin", DataType::LargeBinary, false),
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(arrow::array::BooleanArray::from(vec![false])),
                Arc::new(arrow::array::Int8Array::from(vec![-8])),
                Arc::new(arrow::array::Int16Array::from(vec![-16])),
                Arc::new(arrow::array::Int64Array::from(vec![-64])),
                Arc::new(UInt8Array::from(vec![8u8])),
                Arc::new(UInt16Array::from(vec![16u16])),
                Arc::new(UInt32Array::from(vec![32u32])),
                Arc::new(UInt64Array::from(vec![64u64])),
                Arc::new(arrow::array::Float32Array::from(vec![1.5f32])),
                Arc::new(arrow::array::Float64Array::from(vec![f64::NAN])),
                Arc::new(arrow::array::StringArray::from(vec!["a,b"])),
                Arc::new(LargeStringArray::from(vec!["large"])),
                Arc::new(arrow::array::Date32Array::from(vec![-1])),
                Arc::new(TimestampSecondArray::from(vec![1])),
                Arc::new(TimestampMillisecondArray::from(vec![1_500])),
                Arc::new(arrow::array::TimestampMicrosecondArray::from(vec![
                    1_000_123,
                ])),
                Arc::new(TimestampNanosecondArray::from(vec![1_000_123_999])),
                Arc::new(
                    Decimal128Array::from(vec![-12345i128])
                        .with_precision_and_scale(10, 2)
                        .unwrap(),
                ),
                Arc::new(BinaryArray::from(vec![&[0xDEu8, 0xADu8][..]])),
                Arc::new(LargeBinaryArray::from(vec![&[0xBEu8, 0xEFu8][..]])),
            ],
        )
        .unwrap();

        let mut buffer = Vec::new();
        let mut writer = ArrowToCsvWriter::new(&mut buffer, CsvWriterOptions::default());
        writer.write_batch(&batch).await.unwrap();

        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            concat!(
                "false,-8,-16,-64,8,16,32,64,1.5,NaN,\"a,b\",large,1969-12-31,",
                "1970-01-01 00:00:01.000000,1970-01-01 00:00:01.500000,",
                "1970-01-01 00:00:01.000123,1970-01-01 00:00:01.000123,",
                "-123.45,dead,beef\n"
            )
        );
    }

    #[tokio::test]
    async fn test_async_writer_propagates_unsupported_type_error() {
        let schema = Schema::new(vec![Field::new(
            "t",
            DataType::Time32(arrow::datatypes::TimeUnit::Second),
            false,
        )]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(arrow::array::Time32SecondArray::from(vec![1]))],
        )
        .unwrap();

        let mut buffer = Vec::new();
        let mut writer = ArrowToCsvWriter::new(&mut buffer, CsvWriterOptions::default());

        let err = writer.write_batch(&batch).await.unwrap_err();

        assert!(
            err.to_string().contains("Unsupported Arrow type"),
            "got: {err}"
        );
        assert_eq!(writer.rows_written(), 0);
    }
}
