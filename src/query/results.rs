//! Result set handling and iteration.
//!
//! This module provides types for handling query results, including
//! streaming result sets and metadata.

use crate::error::{ConversionError, QueryError};
use crate::transport::messages::{ColumnInfo, ResultData, ResultPayload, ResultSetHandle};
use crate::transport::protocol::QueryResult as TransportQueryResult;
use crate::transport::TransportProtocol;
use crate::types::TypeMapper;
use arrow::array::{
    new_empty_array, Array, BooleanArray, BooleanBuilder, Decimal128Array, Decimal128Builder,
    PrimitiveArray, PrimitiveBuilder, RecordBatch, StringArray, StringBuilder,
};
use arrow::datatypes::{
    ArrowPrimitiveType, DataType, Date32Type, Field, Float64Type, Int32Type, Int64Type, Schema,
    TimestampMicrosecondType,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 3600;
const SECONDS_PER_DAY: i64 = 86400;
const MICROS_PER_SECOND: i64 = 1_000_000;
const MICROS_FRACTION_DIGITS: usize = 6;

/// Metadata about a query execution.
#[derive(Debug, Clone)]
pub struct QueryMetadata {
    /// Schema of the result set
    pub schema: Arc<Schema>,
    /// Total number of rows (if known)
    pub total_rows: Option<i64>,
    /// Number of columns
    pub column_count: usize,
    /// Execution time in milliseconds (if available)
    pub execution_time_ms: Option<u64>,
}

impl QueryMetadata {
    /// Create metadata from schema and row count.
    pub fn new(schema: Arc<Schema>, total_rows: Option<i64>) -> Self {
        Self {
            column_count: schema.fields().len(),
            schema,
            total_rows,
            execution_time_ms: None,
        }
    }

    /// Set execution time.
    pub fn with_execution_time(mut self, execution_time_ms: u64) -> Self {
        self.execution_time_ms = Some(execution_time_ms);
        self
    }

    /// Get column names.
    pub fn column_names(&self) -> Vec<&str> {
        self.schema
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect()
    }

    /// Get column types.
    pub fn column_types(&self) -> Vec<&arrow::datatypes::DataType> {
        self.schema.fields().iter().map(|f| f.data_type()).collect()
    }
}

/// Query result set that can be either a row count or streaming data.
pub struct ResultSet {
    /// Result type
    inner: ResultSetInner,
    /// Transport reference for fetching more data
    transport: Arc<Mutex<dyn TransportProtocol>>,
}

impl std::fmt::Debug for ResultSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultSet")
            .field("inner", &self.inner)
            .field("transport", &"<TransportProtocol>")
            .finish()
    }
}

#[derive(Debug)]
enum ResultSetInner {
    /// Row count result (INSERT, UPDATE, DELETE)
    RowCount { count: i64 },
    /// Streaming result set (SELECT)
    Stream {
        /// Result set handle for fetching
        handle: Option<ResultSetHandle>,
        /// Query metadata
        metadata: QueryMetadata,
        /// Buffered batches
        batches: Vec<RecordBatch>,
        /// Whether all data has been fetched
        complete: bool,
    },
}

impl ResultSet {
    /// Create a result set from a transport query result.
    pub(crate) fn from_transport_result(
        result: TransportQueryResult,
        transport: Arc<Mutex<dyn TransportProtocol>>,
    ) -> Result<Self, QueryError> {
        match result {
            TransportQueryResult::RowCount { count } => Ok(Self {
                inner: ResultSetInner::RowCount { count },
                transport,
            }),
            TransportQueryResult::ResultSet { handle, data } => {
                let schema = Self::build_schema(&data.columns)
                    .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

                let metadata = QueryMetadata::new(Arc::clone(&schema), Some(data.total_rows));

                let num_rows_received = data.data.num_rows() as i64;

                // A result set always carries column metadata, even with zero rows.
                // Emit one (possibly empty) batch built from that schema so consumers
                // that read the schema from the first batch (e.g. ADBC's
                // RecordBatchReader, used by dbt Fusion for `WHERE FALSE LIMIT 0`
                // schema probes) still receive the columns. Returning an empty Vec
                // here would drop the schema for zero-row results.
                let batch = if data.data.is_empty() {
                    Self::empty_record_batch(&schema)
                } else {
                    Self::payload_to_record_batch(&data, &schema)
                }
                .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;
                let batches = vec![batch];

                let complete = handle.is_none() || data.total_rows == num_rows_received;

                Ok(Self {
                    inner: ResultSetInner::Stream {
                        handle, // Already Option<ResultSetHandle>
                        metadata,
                        batches,
                        complete,
                    },
                    transport,
                })
            }
        }
    }

    /// Get the row count if this is a row count result.
    pub fn row_count(&self) -> Option<i64> {
        match &self.inner {
            ResultSetInner::RowCount { count } => Some(*count),
            _ => None,
        }
    }

    /// Get the metadata if this is a streaming result.
    pub fn metadata(&self) -> Option<&QueryMetadata> {
        match &self.inner {
            ResultSetInner::Stream { metadata, .. } => Some(metadata),
            _ => None,
        }
    }

    /// Check if this is a streaming result set.
    pub fn is_stream(&self) -> bool {
        matches!(&self.inner, ResultSetInner::Stream { .. })
    }

    /// Convert to an iterator over RecordBatches.
    ///
    /// # Errors
    /// Returns `QueryError::NoResultSet` if this is not a streaming result.
    pub fn into_iterator(self) -> Result<ResultSetIterator, QueryError> {
        match self.inner {
            ResultSetInner::Stream {
                handle,
                metadata,
                batches,
                complete,
            } => Ok(ResultSetIterator {
                handle,
                transport: self.transport,
                metadata,
                batches,
                current_index: 0,
                complete,
            }),
            ResultSetInner::RowCount { .. } => Err(QueryError::NoResultSet(
                "Cannot iterate over row count result".to_string(),
            )),
        }
    }

    /// Fetch all remaining batches into memory.
    ///
    /// # Errors
    /// Returns `QueryError` if fetching fails or if this is not a streaming result.
    pub async fn fetch_all(mut self) -> Result<Vec<RecordBatch>, QueryError> {
        let ResultSetInner::Stream {
            handle,
            metadata,
            batches,
            complete,
        } = &mut self.inner
        else {
            return Err(QueryError::NoResultSet(
                "Cannot fetch batches from row count result".to_string(),
            ));
        };

        if !*complete {
            if let Some(handle_val) = *handle {
                *batches = Self::paginate_remaining(
                    &self.transport,
                    handle_val,
                    metadata,
                    batches.clone(),
                )
                .await?;
                *complete = true;
            }
        }
        let all_batches = batches.clone();

        // Close the result set handle on the server to release resources
        if let Some(handle_val) = handle.take() {
            let mut transport = self.transport.lock().await;
            // Ignore close errors - we've already fetched the data
            let _ = transport.close_result_set(handle_val).await;
        }

        Ok(all_batches)
    }

    /// Fetch every remaining page of a result set, appending to `collected`.
    ///
    /// Pagination ends either when the transport returns an empty page or when
    /// the row count known from the query metadata has been reached. The
    /// per-page `total_rows` reported by the transport is deliberately ignored:
    /// it describes that page, so trusting it would end pagination early.
    async fn paginate_remaining(
        transport: &Arc<Mutex<dyn TransportProtocol>>,
        handle: ResultSetHandle,
        metadata: &QueryMetadata,
        mut collected: Vec<RecordBatch>,
    ) -> Result<Vec<RecordBatch>, QueryError> {
        let known_total = metadata.total_rows.unwrap_or(0);

        loop {
            // The guard deliberately spans the whole iteration, matching the
            // locking window the transport has always been given.
            let mut locked = transport.lock().await;
            let result_data = locked
                .fetch_results(handle)
                .await
                .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

            if result_data.data.is_empty() {
                return Ok(collected);
            }

            collected.push(
                Self::payload_to_record_batch(&result_data, &metadata.schema)
                    .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?,
            );

            if known_total > 0 && Self::row_count_of(&collected) >= known_total as usize {
                return Ok(collected);
            }
        }
    }

    fn row_count_of(batches: &[RecordBatch]) -> usize {
        batches.iter().map(|b| b.num_rows()).sum()
    }

    /// Build Arrow schema from column information.
    fn build_schema(columns: &[ColumnInfo]) -> Result<Arc<Schema>, ConversionError> {
        let fields: Result<Vec<Field>, ConversionError> = columns
            .iter()
            .map(|col| {
                // Parse Exasol type from DataType
                let arrow_type = Self::exasol_datatype_to_arrow(&col.data_type)?;

                // All fields are nullable by default in Exasol
                Ok(Field::new(&col.name, arrow_type, true))
            })
            .collect();

        Ok(Arc::new(Schema::new(fields?)))
    }

    /// Convert Exasol DataType to Arrow DataType.
    fn exasol_datatype_to_arrow(
        data_type: &crate::transport::messages::DataType,
    ) -> Result<arrow::datatypes::DataType, ConversionError> {
        use crate::types::ExasolType;

        let exasol_type = match data_type.type_name.as_str() {
            "BOOLEAN" => ExasolType::Boolean,
            "CHAR" => ExasolType::Char {
                size: data_type.size.unwrap_or(1) as usize,
            },
            "VARCHAR" => ExasolType::Varchar {
                size: data_type.size.unwrap_or(2000000) as usize,
            },
            "DECIMAL" => ExasolType::Decimal {
                precision: data_type.precision.unwrap_or(18) as u8,
                scale: data_type.scale.unwrap_or(0) as i8,
            },
            "DOUBLE" => ExasolType::Double,
            "DATE" => ExasolType::Date,
            "TIMESTAMP" => ExasolType::Timestamp {
                with_local_time_zone: data_type.with_local_time_zone.unwrap_or(false),
            },
            "TIMESTAMP WITH LOCAL TIME ZONE" => ExasolType::Timestamp {
                with_local_time_zone: true,
            },
            "INTERVAL YEAR TO MONTH" => ExasolType::IntervalYearToMonth,
            "INTERVAL DAY TO SECOND" => ExasolType::IntervalDayToSecond {
                precision: data_type.fraction.unwrap_or(3) as u8,
            },
            "GEOMETRY" => ExasolType::Geometry { srid: None },
            "HASHTYPE" => ExasolType::Hashtype { byte_size: 16 },
            _ => {
                return Err(ConversionError::UnsupportedType {
                    exasol_type: data_type.type_name.clone(),
                })
            }
        };

        TypeMapper::exasol_to_arrow(&exasol_type, true)
    }

    /// Convert a ResultData payload to RecordBatch, handling both JSON and Arrow variants.
    fn payload_to_record_batch(
        data: &ResultData,
        schema: &Arc<Schema>,
    ) -> Result<RecordBatch, ConversionError> {
        match &data.data {
            ResultPayload::Arrow(batch) => Ok(batch.clone()),
            ResultPayload::Json(_) => Self::column_major_to_record_batch(data, schema),
        }
    }

    /// Build a zero-row RecordBatch that carries the given schema.
    ///
    /// Used for empty result sets so the column schema is still conveyed to
    /// consumers that read it from the first batch (the schema itself comes
    /// from the result set's column metadata, which is present even with no
    /// rows).
    fn empty_record_batch(schema: &Arc<Schema>) -> Result<RecordBatch, ConversionError> {
        let arrays: Vec<Arc<dyn Array>> = schema
            .fields()
            .iter()
            .map(|field| new_empty_array(field.data_type()))
            .collect();
        RecordBatch::try_new(Arc::clone(schema), arrays)
            .map_err(|e| ConversionError::ArrowError(e.to_string()))
    }

    /// Convert row-major JSON result data to RecordBatch.
    fn column_major_to_record_batch(
        data: &ResultData,
        schema: &Arc<Schema>,
    ) -> Result<RecordBatch, ConversionError> {
        let rows = match &data.data {
            ResultPayload::Json(rows) => rows,
            ResultPayload::Arrow(batch) => return Ok(batch.clone()),
        };

        if rows.is_empty() {
            return Self::empty_record_batch(schema);
        }

        let arrays: Vec<Arc<dyn Array>> = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(column_index, field)| {
                let values = Self::column_of(rows, column_index);
                Self::json_column_to_array(field.data_type(), &values)
            })
            .collect::<Result<_, _>>()?;

        RecordBatch::try_new(Arc::clone(schema), arrays)
            .map_err(|e| ConversionError::ArrowError(e.to_string()))
    }

    /// Slice one column out of row-major JSON rows, padding short rows with NULL.
    fn column_of(rows: &[Vec<Value>], column_index: usize) -> Vec<&Value> {
        rows.iter()
            .map(|row| row.get(column_index).unwrap_or(&Value::Null))
            .collect()
    }

    /// Build the Arrow array for one result column from its JSON values.
    ///
    /// A value that does not fit the target Arrow type becomes NULL instead of
    /// failing the batch: Exasol renders the same logical type differently
    /// depending on transport and column width, so a single unexpected shape
    /// must not discard an otherwise valid result set. Only a structurally
    /// impossible column (an out-of-range DECIMAL precision/scale, or a decimal
    /// string that cannot be scaled) is reported as an error.
    fn json_column_to_array(
        data_type: &DataType,
        values: &[&Value],
    ) -> Result<Arc<dyn Array>, ConversionError> {
        Ok(match data_type {
            DataType::Boolean => Arc::new(Self::json_to_boolean_array(values)),
            DataType::Int32 => Arc::new(Self::json_to_primitive_array::<Int32Type, _>(
                values,
                |value| value.as_i64().map(|i| i as i32),
            )),
            DataType::Int64 => Arc::new(Self::json_to_primitive_array::<Int64Type, _>(
                values,
                |value| value.as_i64(),
            )),
            DataType::Float64 => Arc::new(Self::json_to_primitive_array::<Float64Type, _>(
                values,
                |value| value.as_f64(),
            )),
            DataType::Utf8 => Arc::new(Self::json_to_string_array(values, Self::render_as_text)),
            DataType::Decimal128(precision, scale) => {
                Arc::new(Self::json_to_decimal_array(values, *precision, *scale)?)
            }
            DataType::Date32 => Arc::new(Self::json_to_primitive_array::<Date32Type, _>(
                values,
                |value| {
                    value
                        .as_str()
                        .and_then(|s| Self::parse_date_to_days(s).ok())
                },
            )),
            DataType::Timestamp(_, timezone) => {
                let array =
                    Self::json_to_primitive_array::<TimestampMicrosecondType, _>(values, |value| {
                        value
                            .as_str()
                            .and_then(|s| Self::parse_timestamp_to_micros(s).ok())
                    });
                match timezone {
                    Some(_) => Arc::new(array.with_timezone("UTC")),
                    None => Arc::new(array),
                }
            }
            // Every remaining Arrow type is rendered as its JSON text form.
            _ => Arc::new(Self::json_to_string_array(values, Value::to_string)),
        })
    }

    /// Build a primitive Arrow array, treating any value `extract` rejects as NULL.
    fn json_to_primitive_array<T, F>(values: &[&Value], extract: F) -> PrimitiveArray<T>
    where
        T: ArrowPrimitiveType,
        F: Fn(&Value) -> Option<T::Native>,
    {
        let mut builder = PrimitiveBuilder::<T>::with_capacity(values.len());
        for value in values {
            builder.append_option(extract(value));
        }
        builder.finish()
    }

    /// Build a boolean Arrow array, treating any non-boolean value as NULL.
    fn json_to_boolean_array(values: &[&Value]) -> BooleanArray {
        let mut builder = BooleanBuilder::with_capacity(values.len());
        for value in values {
            builder.append_option(value.as_bool());
        }
        builder.finish()
    }

    /// Build a UTF-8 Arrow array, rendering every non-NULL value via `render`.
    fn json_to_string_array<F>(values: &[&Value], render: F) -> StringArray
    where
        F: Fn(&Value) -> String,
    {
        let mut builder = StringBuilder::new();
        for value in values {
            if value.is_null() {
                builder.append_null();
            } else {
                builder.append_value(render(value));
            }
        }
        builder.finish()
    }

    /// Render a value destined for a VARCHAR/CHAR column.
    ///
    /// JSON strings are taken verbatim; anything else falls back to its JSON
    /// text form so that a mistyped column still round-trips its content.
    fn render_as_text(value: &Value) -> String {
        value
            .as_str()
            .map_or_else(|| value.to_string(), String::from)
    }

    /// Build a DECIMAL Arrow array at the column's declared precision and scale.
    fn json_to_decimal_array(
        values: &[&Value],
        precision: u8,
        scale: i8,
    ) -> Result<Decimal128Array, ConversionError> {
        let mut builder = Decimal128Builder::new()
            .with_precision_and_scale(precision, scale)
            .map_err(|e| ConversionError::ArrowError(e.to_string()))?;

        for value in values {
            builder.append_option(Self::scale_to_decimal(value, scale)?);
        }

        Ok(builder.finish())
    }

    /// Scale a single JSON value into the i128 representation of a DECIMAL column.
    ///
    /// Exasol sends decimals as strings when they exceed the JSON number range
    /// and as numbers otherwise, so all three shapes are accepted; a value of
    /// any other shape becomes NULL.
    fn scale_to_decimal(value: &Value, scale: i8) -> Result<Option<i128>, ConversionError> {
        if let Some(text) = value.as_str() {
            return Self::parse_string_to_decimal(text, scale).map(Some);
        }
        if let Some(integer) = value.as_i64() {
            return Ok(Some((integer * 10i64.pow(scale as u32)) as i128));
        }
        if let Some(float) = value.as_f64() {
            return Ok(Some((float * 10f64.powi(scale as i32)) as i128));
        }
        Ok(None)
    }

    /// Parse a date string "YYYY-MM-DD" to days since Unix epoch (1970-01-01).
    fn parse_date_to_days(date_str: &str) -> Result<i32, ()> {
        let parts: Vec<&str> = date_str.split('-').collect();
        if parts.len() != 3 {
            return Err(());
        }

        let year: i32 = parts[0].parse().map_err(|_| ())?;
        let month: u32 = parts[1].parse().map_err(|_| ())?;
        let day: u32 = parts[2].parse().map_err(|_| ())?;

        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return Err(());
        }

        // Calculate days since Unix epoch
        let days_from_year =
            (year - 1970) * 365 + (year - 1969) / 4 - (year - 1901) / 100 + (year - 1601) / 400;
        let days_from_month = match month {
            1 => 0,
            2 => 31,
            3 => 59,
            4 => 90,
            5 => 120,
            6 => 151,
            7 => 181,
            8 => 212,
            9 => 243,
            10 => 273,
            11 => 304,
            12 => 334,
            _ => return Err(()),
        };

        // Add leap day if after February and leap year
        let is_leap_year = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let leap_adjustment = if month > 2 && is_leap_year { 1 } else { 0 };

        Ok(days_from_year + days_from_month + day as i32 - 1 + leap_adjustment)
    }

    /// Parse a timestamp string to microseconds since Unix epoch.
    ///
    /// Accepts `YYYY-MM-DD`, `YYYY-MM-DD HH:MM`, `YYYY-MM-DD HH:MM:SS` and
    /// `YYYY-MM-DD HH:MM:SS.ffffff`. Anything after the time component is
    /// ignored; a time component that is not `HH:MM`-shaped contributes nothing.
    fn parse_timestamp_to_micros(timestamp_str: &str) -> Result<i64, ()> {
        // `str::split` always yields at least one element, so the date part is
        // never absent and needs no emptiness guard.
        let parts: Vec<&str> = timestamp_str.split(' ').collect();

        let days = Self::parse_date_to_days(parts[0])?;
        let mut micros = days as i64 * SECONDS_PER_DAY * MICROS_PER_SECOND;

        if let Some(time_str) = parts.get(1) {
            micros += Self::parse_time_of_day_to_micros(time_str)?;
        }

        Ok(micros)
    }

    /// Parse the `HH:MM[:SS[.ffffff]]` part of a timestamp into microseconds.
    fn parse_time_of_day_to_micros(time_str: &str) -> Result<i64, ()> {
        let time_parts: Vec<&str> = time_str.split(':').collect();
        let (Some(hours_str), Some(minutes_str)) = (time_parts.first(), time_parts.get(1)) else {
            return Ok(0);
        };

        let hours: i64 = hours_str.parse().map_err(|_| ())?;
        let minutes: i64 = minutes_str.parse().map_err(|_| ())?;
        let mut micros = hours * SECONDS_PER_HOUR * MICROS_PER_SECOND
            + minutes * SECONDS_PER_MINUTE * MICROS_PER_SECOND;

        if let Some(seconds_str) = time_parts.get(2) {
            micros += Self::parse_seconds_to_micros(seconds_str)?;
        }

        Ok(micros)
    }

    /// Parse the `SS[.ffffff]` part of a timestamp into microseconds.
    fn parse_seconds_to_micros(seconds_str: &str) -> Result<i64, ()> {
        let seconds_parts: Vec<&str> = seconds_str.split('.').collect();
        let seconds: i64 = seconds_parts[0].parse().map_err(|_| ())?;
        let fraction = seconds_parts
            .get(1)
            .map_or(0, |frac| Self::fractional_seconds_to_micros(frac));

        Ok(seconds * MICROS_PER_SECOND + fraction)
    }

    /// Interpret the digits after the decimal point as a microsecond fraction.
    ///
    /// Shorter fractions are right-padded, longer ones truncated to microsecond
    /// resolution; anything unparsable contributes nothing.
    fn fractional_seconds_to_micros(fraction: &str) -> i64 {
        if fraction.len() <= MICROS_FRACTION_DIGITS {
            let padding = MICROS_FRACTION_DIGITS - fraction.len();
            let padded = format!("{}{}", fraction, "0".repeat(padding));
            padded.parse::<i64>().unwrap_or(0)
        } else {
            fraction[..MICROS_FRACTION_DIGITS]
                .parse::<i64>()
                .unwrap_or(0)
        }
    }

    /// Parse a decimal string to i128 scaled value.
    ///
    /// Handles formats like "123", "123.45", "-123.45"
    fn parse_string_to_decimal(s: &str, scale: i8) -> Result<i128, ConversionError> {
        // Handle empty string
        if s.is_empty() {
            return Err(ConversionError::InvalidFormat(
                "Empty decimal string".to_string(),
            ));
        }

        // Split on decimal point
        let parts: Vec<&str> = s.split('.').collect();

        let (integer_part, decimal_part) = match parts.len() {
            1 => (parts[0], ""),
            2 => (parts[0], parts[1]),
            _ => {
                return Err(ConversionError::InvalidFormat(format!(
                    "Invalid decimal format: {}",
                    s
                )));
            }
        };

        // Parse the integer part
        let mut result: i128 = integer_part.parse().map_err(|_| {
            ConversionError::InvalidFormat(format!("Invalid integer part: {}", integer_part))
        })?;

        // Scale up by 10^scale
        result = result
            .checked_mul(10_i128.pow(scale as u32))
            .ok_or_else(|| ConversionError::InvalidFormat("Decimal overflow".to_string()))?;

        // Add the decimal part
        if !decimal_part.is_empty() {
            let decimal_digits = decimal_part.len().min(scale as usize);
            let decimal_value: i128 = decimal_part[..decimal_digits].parse().map_err(|_| {
                ConversionError::InvalidFormat(format!("Invalid decimal part: {}", decimal_part))
            })?;

            // Scale the decimal part appropriately
            let scale_diff = scale as usize - decimal_digits;
            let scaled_decimal = decimal_value * 10_i128.pow(scale_diff as u32);

            result = result
                .checked_add(if integer_part.starts_with('-') {
                    -scaled_decimal
                } else {
                    scaled_decimal
                })
                .ok_or_else(|| ConversionError::InvalidFormat("Decimal overflow".to_string()))?;
        }

        Ok(result)
    }
}

/// Iterator over RecordBatches from a result set.
///
/// Lazily fetches data from the transport as needed.
pub struct ResultSetIterator {
    /// Result set handle
    handle: Option<ResultSetHandle>,
    /// Transport reference
    transport: Arc<Mutex<dyn TransportProtocol>>,
    /// Query metadata
    metadata: QueryMetadata,
    /// Buffered batches
    batches: Vec<RecordBatch>,
    /// Current iteration index
    current_index: usize,
    /// Whether all data has been fetched
    complete: bool,
}

impl ResultSetIterator {
    /// Get the query metadata.
    pub fn metadata(&self) -> &QueryMetadata {
        &self.metadata
    }

    /// Fetch the next batch from the transport.
    async fn fetch_next_batch(&mut self) -> Result<Option<RecordBatch>, QueryError> {
        if self.complete {
            return Ok(None);
        }

        let handle = match self.handle {
            Some(h) => h,
            None => {
                self.complete = true;
                return Ok(None);
            }
        };

        let mut transport = self.transport.lock().await;
        let result_data = transport
            .fetch_results(handle)
            .await
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        if result_data.data.is_empty() {
            self.complete = true;
            return Ok(None);
        }

        let batch = ResultSet::payload_to_record_batch(&result_data, &self.metadata.schema)
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        Ok(Some(batch))
    }

    /// Get the next batch synchronously (blocking).
    ///
    /// This is useful for implementing sync Iterator trait.
    pub fn next_batch(&mut self) -> Option<Result<RecordBatch, QueryError>> {
        // Return buffered batch if available
        if self.current_index < self.batches.len() {
            let batch = self.batches[self.current_index].clone();
            self.current_index += 1;
            return Some(Ok(batch));
        }

        // Need to fetch more data
        if self.complete {
            return None;
        }

        // Use block_on for async fetch (not ideal, but works for Phase 1)
        let runtime = tokio::runtime::Handle::try_current();
        if let Ok(handle) = runtime {
            let result = handle.block_on(self.fetch_next_batch());
            match result {
                Ok(Some(batch)) => {
                    self.batches.push(batch.clone());
                    self.current_index += 1;
                    Some(Ok(batch))
                }
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        } else {
            // No runtime available
            Some(Err(QueryError::InvalidState(
                "No async runtime available".to_string(),
            )))
        }
    }

    /// Close the result set and release resources.
    pub async fn close(mut self) -> Result<(), QueryError> {
        if let Some(handle) = self.handle.take() {
            let mut transport = self.transport.lock().await;
            transport
                .close_result_set(handle)
                .await
                .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;
        }
        Ok(())
    }
}

// Implement Iterator for ResultSetIterator
impl Iterator for ResultSetIterator {
    type Item = Result<RecordBatch, QueryError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_batch()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TransportError;
    use crate::transport::messages::{ColumnInfo, DataType, ResultPayload};
    use crate::transport::test_support::MockTransport;
    use arrow::array::Array;
    use mockall::predicate::eq;
    use serde_json::json;

    // =========================================================================
    // Shared test fixtures
    // =========================================================================

    /// An Exasol type descriptor carrying no modifiers.
    fn plain_type(type_name: &str) -> DataType {
        DataType {
            type_name: type_name.to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        }
    }

    fn decimal_column(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: DataType {
                precision: Some(18),
                scale: Some(0),
                ..plain_type("DECIMAL")
            },
        }
    }

    fn varchar_column(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: DataType {
                size: Some(100),
                character_set: Some("UTF8".to_string()),
                ..plain_type("VARCHAR")
            },
        }
    }

    fn boolean_column(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: plain_type("BOOLEAN"),
        }
    }

    /// A transport that serves one further page holding `values`, then reports
    /// exhaustion with an empty page.
    fn transport_serving_one_more_page(values: &'static [i64]) -> MockTransport {
        let mut transport = MockTransport::new();
        let mut call = 0;
        transport
            .expect_fetch_results()
            .times(2)
            .returning(move |_| {
                call += 1;
                Ok(if call == 1 {
                    single_column_result_data(values, 0)
                } else {
                    single_column_result_data(&[], 0)
                })
            });
        transport
    }

    fn single_column_result_data(values: &[i64], total_rows: i64) -> ResultData {
        ResultData {
            columns: vec![decimal_column("id")],
            data: ResultPayload::Json(values.iter().map(|v| vec![json!(v)]).collect()),
            total_rows,
        }
    }

    fn streaming_result_set(
        transport: MockTransport,
        values: &[i64],
        total_rows: i64,
        handle: Option<ResultSetHandle>,
    ) -> ResultSet {
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(transport));
        ResultSet::from_transport_result(
            TransportQueryResult::ResultSet {
                handle,
                data: single_column_result_data(values, total_rows),
            },
            transport,
        )
        .unwrap()
    }

    fn streaming_iterator(
        transport: MockTransport,
        values: &[i64],
        total_rows: i64,
        handle: Option<ResultSetHandle>,
    ) -> ResultSetIterator {
        streaming_result_set(transport, values, total_rows, handle)
            .into_iterator()
            .unwrap()
    }

    fn entered_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime")
    }

    #[tokio::test]
    async fn test_result_set_row_count() {
        let mock_transport = MockTransport::new();
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(mock_transport));

        let result = TransportQueryResult::RowCount { count: 42 };
        let result_set = ResultSet::from_transport_result(result, transport).unwrap();

        assert_eq!(result_set.row_count(), Some(42));
        assert!(!result_set.is_stream());
        assert!(result_set.metadata().is_none());
    }

    #[tokio::test]
    async fn test_result_set_stream() {
        let mock_transport = MockTransport::new();
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(mock_transport));

        // Row-major data format:
        // Row 0: [1, "Alice"]
        // Row 1: [2, "Bob"]
        let data = ResultData {
            columns: vec![decimal_column("id"), varchar_column("name")],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(1), serde_json::json!("Alice")],
                vec![serde_json::json!(2), serde_json::json!("Bob")],
            ]),
            total_rows: 2,
        };

        let result = TransportQueryResult::ResultSet {
            handle: Some(ResultSetHandle::new(1)),
            data,
        };

        let result_set = ResultSet::from_transport_result(result, transport).unwrap();

        assert!(result_set.row_count().is_none());
        assert!(result_set.is_stream());

        let metadata = result_set.metadata().unwrap();
        assert_eq!(metadata.column_count, 2);
        assert_eq!(metadata.total_rows, Some(2));
        assert_eq!(metadata.column_names(), vec!["id", "name"]);
    }

    #[tokio::test]
    async fn test_empty_result_set_preserves_schema() {
        // A zero-row result set must still convey its columns: ADBC consumers
        // read the schema from the first batch, so an empty Vec would drop it
        // (regression test for `WHERE FALSE LIMIT 0` schema probes).
        let mock_transport = MockTransport::new();
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(mock_transport));

        let data = ResultData {
            columns: vec![decimal_column("id"), varchar_column("name")],
            data: ResultPayload::Json(vec![]),
            total_rows: 0,
        };

        let result = TransportQueryResult::ResultSet { handle: None, data };
        let result_set = ResultSet::from_transport_result(result, transport).unwrap();

        let batches = result_set.fetch_all().await.unwrap();
        assert_eq!(
            batches.len(),
            1,
            "empty result set must yield one schema-carrying batch"
        );
        assert_eq!(batches[0].num_rows(), 0);
        assert_eq!(batches[0].num_columns(), 2);
        assert_eq!(
            batches[0]
                .schema()
                .fields()
                .iter()
                .map(|f| f.name().clone())
                .collect::<Vec<_>>(),
            vec!["id", "name"]
        );
    }

    #[tokio::test]
    async fn test_result_set_to_record_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", arrow::datatypes::DataType::Decimal128(18, 0), true),
            Field::new("name", arrow::datatypes::DataType::Utf8, true),
            Field::new("active", arrow::datatypes::DataType::Boolean, true),
        ]));

        // Row-major data format:
        // Row 0: [1, "Alice", true]
        // Row 1: [2, "Bob", false]
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![
                    serde_json::json!(1),
                    serde_json::json!("Alice"),
                    serde_json::json!(true),
                ],
                vec![
                    serde_json::json!(2),
                    serde_json::json!("Bob"),
                    serde_json::json!(false),
                ],
            ]),
            total_rows: 2,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);
    }

    #[tokio::test]
    async fn test_single_row_single_column() {
        // Test case for: SELECT 42 AS answer
        let schema = Arc::new(Schema::new(vec![Field::new(
            "answer",
            arrow::datatypes::DataType::Decimal128(18, 0),
            true,
        )]));

        // Row-major: one row with one value
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(42)], // Row 0: [42]
            ]),
            total_rows: 1,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 1, "Should have exactly 1 row");
        assert_eq!(batch.num_columns(), 1, "Should have exactly 1 column");
    }

    #[tokio::test]
    async fn test_single_row_two_columns() {
        // Test case for: SELECT 42 AS answer, 'hello' AS greeting
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "answer",
                arrow::datatypes::DataType::Decimal128(18, 0),
                true,
            ),
            Field::new("greeting", arrow::datatypes::DataType::Utf8, true),
        ]));

        // Row-major: one row with two values
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(42), serde_json::json!("hello")], // Row 0: [42, "hello"]
            ]),
            total_rows: 1,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 1, "Should have exactly 1 row");
        assert_eq!(batch.num_columns(), 2, "Should have exactly 2 columns");
    }

    #[tokio::test]
    async fn test_ten_rows_two_columns() {
        // Test case for: SELECT LEVEL AS id, 'Row ' || LEVEL AS label FROM DUAL CONNECT BY LEVEL <= 10
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", arrow::datatypes::DataType::Decimal128(18, 0), true),
            Field::new("label", arrow::datatypes::DataType::Utf8, true),
        ]));

        // Row-major: ten rows, each with two values
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(1), serde_json::json!("Row 1")],
                vec![serde_json::json!(2), serde_json::json!("Row 2")],
                vec![serde_json::json!(3), serde_json::json!("Row 3")],
                vec![serde_json::json!(4), serde_json::json!("Row 4")],
                vec![serde_json::json!(5), serde_json::json!("Row 5")],
                vec![serde_json::json!(6), serde_json::json!("Row 6")],
                vec![serde_json::json!(7), serde_json::json!("Row 7")],
                vec![serde_json::json!(8), serde_json::json!("Row 8")],
                vec![serde_json::json!(9), serde_json::json!("Row 9")],
                vec![serde_json::json!(10), serde_json::json!("Row 10")],
            ]),
            total_rows: 10,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 10, "Should have exactly 10 rows");
        assert_eq!(batch.num_columns(), 2, "Should have exactly 2 columns");
    }

    #[tokio::test]
    async fn test_schema_building() {
        let columns = vec![
            decimal_column("id"),
            varchar_column("name"),
            ColumnInfo {
                name: "created_at".to_string(),
                data_type: DataType {
                    with_local_time_zone: Some(false),
                    ..plain_type("TIMESTAMP")
                },
            },
        ];

        let schema = ResultSet::build_schema(&columns).unwrap();

        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(schema.field(2).name(), "created_at");
    }

    #[test]
    fn test_query_metadata() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", arrow::datatypes::DataType::Int64, false),
            Field::new("name", arrow::datatypes::DataType::Utf8, true),
        ]));

        let metadata = QueryMetadata::new(Arc::clone(&schema), Some(100)).with_execution_time(250);

        assert_eq!(metadata.column_count, 2);
        assert_eq!(metadata.total_rows, Some(100));
        assert_eq!(metadata.execution_time_ms, Some(250));
        assert_eq!(metadata.column_names(), vec!["id", "name"]);
    }

    // =========================================================================
    // Tests for QueryMetadata::column_types
    // =========================================================================

    #[test]
    fn test_query_metadata_column_types_returns_correct_types() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", arrow::datatypes::DataType::Int64, false),
            Field::new("name", arrow::datatypes::DataType::Utf8, true),
            Field::new("active", arrow::datatypes::DataType::Boolean, true),
        ]));

        let metadata = QueryMetadata::new(Arc::clone(&schema), Some(100));
        let types = metadata.column_types();

        assert_eq!(types.len(), 3);
        assert_eq!(types[0], &arrow::datatypes::DataType::Int64);
        assert_eq!(types[1], &arrow::datatypes::DataType::Utf8);
        assert_eq!(types[2], &arrow::datatypes::DataType::Boolean);
    }

    #[test]
    fn test_query_metadata_column_types_empty_schema() {
        let fields: Vec<Field> = vec![];
        let schema = Arc::new(Schema::new(fields));

        let metadata = QueryMetadata::new(Arc::clone(&schema), None);
        let types = metadata.column_types();

        assert!(types.is_empty());
    }

    // =========================================================================
    // Tests for ResultSet::parse_date_to_days
    // =========================================================================

    #[test]
    fn test_parse_date_to_days_unix_epoch() {
        let result = ResultSet::parse_date_to_days("1970-01-01").unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_parse_date_to_days_after_epoch() {
        let result = ResultSet::parse_date_to_days("1970-01-02").unwrap();
        assert_eq!(result, 1);
    }

    #[test]
    fn test_parse_date_to_days_year_2000() {
        // 2000-01-01 is 10957 days after Unix epoch
        let result = ResultSet::parse_date_to_days("2000-01-01").unwrap();
        assert_eq!(result, 10957);
    }

    #[test]
    fn test_parse_date_to_days_leap_year() {
        // 2000-03-01 should include Feb 29 (leap year)
        let result = ResultSet::parse_date_to_days("2000-03-01").unwrap();
        // 2000-01-01 = 10957, Jan has 31 days, Feb has 29 days (leap year)
        // 10957 + 31 + 29 = 11017
        assert_eq!(result, 11017);
    }

    #[test]
    fn test_parse_date_to_days_non_leap_year() {
        // 2001-03-01 should NOT include Feb 29 (non-leap year)
        let result = ResultSet::parse_date_to_days("2001-03-01").unwrap();
        // 2001-01-01 = 11323, Jan has 31 days, Feb has 28 days
        // 11323 + 31 + 28 = 11382
        assert_eq!(result, 11382);
    }

    #[test]
    fn test_parse_date_to_days_before_epoch() {
        // 1969-12-31 is -1 day before Unix epoch
        let result = ResultSet::parse_date_to_days("1969-12-31").unwrap();
        assert_eq!(result, -1);
    }

    #[test]
    fn test_parse_date_to_days_invalid_format_wrong_separator() {
        let result = ResultSet::parse_date_to_days("2000/01/01");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_format_missing_parts() {
        let result = ResultSet::parse_date_to_days("2000-01");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_month_zero() {
        let result = ResultSet::parse_date_to_days("2000-00-01");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_month_thirteen() {
        let result = ResultSet::parse_date_to_days("2000-13-01");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_day_zero() {
        let result = ResultSet::parse_date_to_days("2000-01-00");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_day_thirty_two() {
        let result = ResultSet::parse_date_to_days("2000-01-32");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_date_to_days_invalid_non_numeric() {
        let result = ResultSet::parse_date_to_days("YYYY-MM-DD");
        assert!(result.is_err());
    }

    // =========================================================================
    // Tests for ResultSet::parse_timestamp_to_micros
    // =========================================================================

    #[test]
    fn test_parse_timestamp_to_micros_date_only() {
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01").unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_time() {
        // 1970-01-01 01:00:00 = 1 hour = 3600 * 1_000_000 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 01:00:00").unwrap();
        assert_eq!(result, 3600 * 1_000_000);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_minutes() {
        // 1970-01-01 00:30:00 = 30 minutes = 30 * 60 * 1_000_000 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 00:30:00").unwrap();
        assert_eq!(result, 30 * 60 * 1_000_000);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_seconds() {
        // 1970-01-01 00:00:45 = 45 seconds = 45 * 1_000_000 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:45").unwrap();
        assert_eq!(result, 45 * 1_000_000);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_fractional_seconds_3_digits() {
        // 1970-01-01 00:00:00.123 = 123000 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:00.123").unwrap();
        assert_eq!(result, 123000);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_fractional_seconds_6_digits() {
        // 1970-01-01 00:00:00.123456 = 123456 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:00.123456").unwrap();
        assert_eq!(result, 123456);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_fractional_seconds_more_than_6_digits() {
        // 1970-01-01 00:00:00.1234567 should truncate to 123456 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:00.1234567").unwrap();
        assert_eq!(result, 123456);
    }

    #[test]
    fn test_parse_timestamp_to_micros_with_fractional_seconds_1_digit() {
        // 1970-01-01 00:00:00.1 = 100000 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:00.1").unwrap();
        assert_eq!(result, 100000);
    }

    #[test]
    fn test_parse_timestamp_to_micros_complex_timestamp() {
        // 2000-06-15 12:30:45.500
        // days = 11124 (from previous calculation for 2000-06-15)
        // time = 12*3600 + 30*60 + 45 seconds = 45045 seconds
        // micros from time = 45045 * 1_000_000 + 500000
        let result = ResultSet::parse_timestamp_to_micros("2000-06-15 12:30:45.500").unwrap();

        // Calculate expected value
        let days_micros: i64 =
            ResultSet::parse_date_to_days("2000-06-15").unwrap() as i64 * 86400 * 1_000_000;
        let time_micros: i64 = (12 * 3600 + 30 * 60 + 45) * 1_000_000 + 500000;
        assert_eq!(result, days_micros + time_micros);
    }

    #[test]
    fn test_parse_timestamp_to_micros_empty_string() {
        let result = ResultSet::parse_timestamp_to_micros("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_timestamp_to_micros_hours_and_minutes_only() {
        // 1970-01-01 01:30 (without seconds)
        // hours + minutes = 1*3600 + 30*60 = 5400 seconds = 5400 * 1_000_000 microseconds
        let result = ResultSet::parse_timestamp_to_micros("1970-01-01 01:30").unwrap();
        assert_eq!(result, 5400 * 1_000_000);
    }

    // =========================================================================
    // Tests for ResultSet::parse_string_to_decimal
    // =========================================================================

    #[test]
    fn test_parse_string_to_decimal_integer() {
        let result = ResultSet::parse_string_to_decimal("123", 2).unwrap();
        // 123 with scale 2 = 12300
        assert_eq!(result, 12300);
    }

    #[test]
    fn test_parse_string_to_decimal_with_decimal_point() {
        let result = ResultSet::parse_string_to_decimal("123.45", 2).unwrap();
        // 123.45 with scale 2 = 12345
        assert_eq!(result, 12345);
    }

    #[test]
    fn test_parse_string_to_decimal_negative() {
        let result = ResultSet::parse_string_to_decimal("-123.45", 2).unwrap();
        // -123.45 with scale 2 = -12345
        assert_eq!(result, -12345);
    }

    #[test]
    fn test_parse_string_to_decimal_zero_scale() {
        let result = ResultSet::parse_string_to_decimal("123", 0).unwrap();
        assert_eq!(result, 123);
    }

    #[test]
    fn test_parse_string_to_decimal_high_scale() {
        let result = ResultSet::parse_string_to_decimal("1.5", 6).unwrap();
        // 1.5 with scale 6 = 1500000
        assert_eq!(result, 1500000);
    }

    #[test]
    fn test_parse_string_to_decimal_truncates_extra_decimals() {
        // When decimal part has more digits than scale
        let result = ResultSet::parse_string_to_decimal("1.123456", 3).unwrap();
        // 1.123 with scale 3 = 1123 (truncates extra digits)
        assert_eq!(result, 1123);
    }

    #[test]
    fn test_parse_string_to_decimal_zero() {
        let result = ResultSet::parse_string_to_decimal("0", 2).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_parse_string_to_decimal_negative_zero() {
        let result = ResultSet::parse_string_to_decimal("-0", 2).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_parse_string_to_decimal_empty_string() {
        let result = ResultSet::parse_string_to_decimal("", 2);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_string_to_decimal_multiple_decimal_points() {
        let result = ResultSet::parse_string_to_decimal("1.2.3", 2);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_string_to_decimal_non_numeric() {
        let result = ResultSet::parse_string_to_decimal("abc", 2);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_string_to_decimal_large_value() {
        let result = ResultSet::parse_string_to_decimal("999999999999999999", 0).unwrap();
        assert_eq!(result, 999999999999999999_i128);
    }

    // =========================================================================
    // Tests for ResultSet::exasol_datatype_to_arrow
    // =========================================================================

    #[test]
    fn test_exasol_datatype_to_arrow_boolean() {
        let data_type = plain_type("BOOLEAN");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Boolean);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_char() {
        let data_type = DataType {
            size: Some(10),
            ..plain_type("CHAR")
        };

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Utf8);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_char_default_size() {
        let data_type = plain_type("CHAR");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Utf8);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_varchar() {
        let data_type = DataType {
            size: Some(100),
            ..plain_type("VARCHAR")
        };

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Utf8);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_varchar_default_size() {
        let data_type = plain_type("VARCHAR");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Utf8);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_decimal() {
        let data_type = DataType {
            precision: Some(18),
            scale: Some(2),
            ..plain_type("DECIMAL")
        };

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Decimal128(18, 2));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_decimal_default_precision_scale() {
        let data_type = plain_type("DECIMAL");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Decimal128(18, 0));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_double() {
        let data_type = plain_type("DOUBLE");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Float64);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_date() {
        let data_type = plain_type("DATE");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Date32);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_timestamp_without_tz() {
        let data_type = DataType {
            with_local_time_zone: Some(false),
            ..plain_type("TIMESTAMP")
        };

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert!(matches!(
            result,
            arrow::datatypes::DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None)
        ));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_timestamp_with_tz() {
        let data_type = DataType {
            with_local_time_zone: Some(true),
            ..plain_type("TIMESTAMP")
        };

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert!(matches!(
            result,
            arrow::datatypes::DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, Some(_))
        ));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_timestamp_default_tz() {
        let data_type = plain_type("TIMESTAMP");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert!(matches!(
            result,
            arrow::datatypes::DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None)
        ));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_interval_year_to_month() {
        let data_type = plain_type("INTERVAL YEAR TO MONTH");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert!(matches!(
            result,
            arrow::datatypes::DataType::Interval(arrow::datatypes::IntervalUnit::MonthDayNano)
        ));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_interval_day_to_second() {
        let data_type = DataType {
            fraction: Some(6),
            ..plain_type("INTERVAL DAY TO SECOND")
        };

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert!(matches!(
            result,
            arrow::datatypes::DataType::Interval(arrow::datatypes::IntervalUnit::MonthDayNano)
        ));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_interval_day_to_second_default_fraction() {
        let data_type = plain_type("INTERVAL DAY TO SECOND");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert!(matches!(
            result,
            arrow::datatypes::DataType::Interval(arrow::datatypes::IntervalUnit::MonthDayNano)
        ));
    }

    #[test]
    fn test_exasol_datatype_to_arrow_geometry() {
        let data_type = plain_type("GEOMETRY");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Binary);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_hashtype() {
        let data_type = plain_type("HASHTYPE");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();
        assert_eq!(result, arrow::datatypes::DataType::Binary);
    }

    #[test]
    fn test_exasol_datatype_to_arrow_unsupported_type() {
        let data_type = plain_type("UNKNOWN_TYPE");

        let result = ResultSet::exasol_datatype_to_arrow(&data_type);
        assert!(result.is_err());
    }

    // =========================================================================
    // Tests for ResultSet::column_major_to_record_batch with various types
    // =========================================================================

    #[tokio::test]
    async fn test_column_major_to_record_batch_with_int32() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            arrow::datatypes::DataType::Int32,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(1)],
                vec![serde_json::json!(2)],
                vec![serde_json::json!(null)],
                vec![serde_json::json!(4)],
            ]),
            total_rows: 4,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 4);
        assert_eq!(batch.num_columns(), 1);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .unwrap();
        assert_eq!(array.value(0), 1);
        assert_eq!(array.value(1), 2);
        assert!(array.is_null(2));
        assert_eq!(array.value(3), 4);
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_with_int64() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            arrow::datatypes::DataType::Int64,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(9223372036854775807_i64)], // Max i64
                vec![serde_json::json!(-9223372036854775808_i64)], // Min i64
                vec![serde_json::json!(null)],
            ]),
            total_rows: 3,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 3);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        assert_eq!(array.value(0), 9223372036854775807_i64);
        assert_eq!(array.value(1), -9223372036854775808_i64);
        assert!(array.is_null(2));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_with_float64() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            arrow::datatypes::DataType::Float64,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(1.23456)],
                vec![serde_json::json!(-9.87654)],
                vec![serde_json::json!(null)],
                vec![serde_json::json!(0.0)],
            ]),
            total_rows: 4,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 4);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Float64Array>()
            .unwrap();
        assert!((array.value(0) - 1.23456).abs() < 0.00001);
        assert!((array.value(1) - (-9.87654)).abs() < 0.00001);
        assert!(array.is_null(2));
        assert!((array.value(3) - 0.0).abs() < 0.00001);
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_with_boolean() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "flag",
            arrow::datatypes::DataType::Boolean,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(true)],
                vec![serde_json::json!(false)],
                vec![serde_json::json!(null)],
            ]),
            total_rows: 3,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 3);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::BooleanArray>()
            .unwrap();
        assert!(array.value(0));
        assert!(!array.value(1));
        assert!(array.is_null(2));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_with_date32() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "date",
            arrow::datatypes::DataType::Date32,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("1970-01-01")],
                vec![serde_json::json!("2000-01-01")],
                vec![serde_json::json!(null)],
            ]),
            total_rows: 3,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 3);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Date32Array>()
            .unwrap();
        assert_eq!(array.value(0), 0); // Unix epoch
        assert_eq!(array.value(1), 10957); // 2000-01-01
        assert!(array.is_null(2));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_with_timestamp() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "timestamp",
            arrow::datatypes::DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("1970-01-01 00:00:00")],
                vec![serde_json::json!("1970-01-01 01:00:00.123456")],
                vec![serde_json::json!(null)],
            ]),
            total_rows: 3,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 3);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::TimestampMicrosecondArray>()
            .unwrap();
        assert_eq!(array.value(0), 0);
        // 1 hour + 123456 microseconds
        assert_eq!(array.value(1), 3600 * 1_000_000 + 123456);
        assert!(array.is_null(2));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_empty_data() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", arrow::datatypes::DataType::Int64, true),
            Field::new("name", arrow::datatypes::DataType::Utf8, true),
        ]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![]),
            total_rows: 0,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 2);
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_with_utf8_string() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "text",
            arrow::datatypes::DataType::Utf8,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("hello")],
                vec![serde_json::json!("world")],
                vec![serde_json::json!(null)],
                vec![serde_json::json!("")], // Empty string
            ]),
            total_rows: 4,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 4);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();
        assert_eq!(array.value(0), "hello");
        assert_eq!(array.value(1), "world");
        assert!(array.is_null(2));
        assert_eq!(array.value(3), "");
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_utf8_from_non_string_value() {
        // Test that non-string JSON values are converted to string
        let schema = Arc::new(Schema::new(vec![Field::new(
            "text",
            arrow::datatypes::DataType::Utf8,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(123)], // Number should be converted to "123"
            ]),
            total_rows: 1,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();
        assert_eq!(array.value(0), "123");
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_decimal_from_string() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            arrow::datatypes::DataType::Decimal128(18, 2),
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("123.45")],
                vec![serde_json::json!("-67.89")],
                vec![serde_json::json!(null)],
            ]),
            total_rows: 3,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 3);

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Decimal128Array>()
            .unwrap();
        assert_eq!(array.value(0), 12345); // 123.45 with scale 2
        assert_eq!(array.value(1), -6789); // -67.89 with scale 2
        assert!(array.is_null(2));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_decimal_from_integer() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            arrow::datatypes::DataType::Decimal128(18, 2),
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(100)], // Integer should be scaled
            ]),
            total_rows: 1,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Decimal128Array>()
            .unwrap();
        assert_eq!(array.value(0), 10000); // 100 with scale 2 = 10000
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_decimal_from_float() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            arrow::datatypes::DataType::Decimal128(18, 2),
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!(99.99)], // Float should be scaled
            ]),
            total_rows: 1,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Decimal128Array>()
            .unwrap();
        assert_eq!(array.value(0), 9999); // 99.99 with scale 2 = 9999
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_invalid_date_becomes_null() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "date",
            arrow::datatypes::DataType::Date32,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("invalid-date")],
                vec![serde_json::json!(123)], // Non-string should become null
            ]),
            total_rows: 2,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Date32Array>()
            .unwrap();
        // Invalid date formats become null
        assert!(array.is_null(0));
        assert!(array.is_null(1));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_invalid_timestamp_becomes_null() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "timestamp",
            arrow::datatypes::DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("not-a-timestamp")],
                vec![serde_json::json!(12345)], // Non-string should become null
            ]),
            total_rows: 2,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::TimestampMicrosecondArray>()
            .unwrap();
        assert!(array.is_null(0));
        assert!(array.is_null(1));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_boolean_null_from_non_bool() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "flag",
            arrow::datatypes::DataType::Boolean,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("not-a-bool")], // String is not a bool
                vec![serde_json::json!(123)],          // Number is not a bool
            ]),
            total_rows: 2,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::BooleanArray>()
            .unwrap();
        // Non-bool values become null
        assert!(array.is_null(0));
        assert!(array.is_null(1));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_int32_null_from_non_int() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            arrow::datatypes::DataType::Int32,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("not-an-int")], // String is not parseable as int
            ]),
            total_rows: 1,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .unwrap();
        assert!(array.is_null(0));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_float64_null_from_non_float() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            arrow::datatypes::DataType::Float64,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![serde_json::json!("not-a-float")], // String is not parseable as float
            ]),
            total_rows: 1,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let array = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Float64Array>()
            .unwrap();
        assert!(array.is_null(0));
    }

    #[tokio::test]
    async fn test_column_major_to_record_batch_unsupported_type_returns_error() {
        // Test that unsupported types (like LargeUtf8) result in an ArrowError
        // because the fallback creates a StringArray which doesn't match the schema
        let schema = Arc::new(Schema::new(vec![Field::new(
            "large_text",
            arrow::datatypes::DataType::LargeUtf8,
            true,
        )]));

        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![vec![serde_json::json!("test_data")]]),
            total_rows: 1,
        };

        let result = ResultSet::column_major_to_record_batch(&data, &schema);

        // Unsupported types cause an error because the fallback creates a StringArray
        // which doesn't match the expected LargeUtf8 type in the schema
        assert!(result.is_err());
    }

    // =========================================================================
    // Tests for ResultSetIterator::metadata
    // =========================================================================

    #[tokio::test]
    async fn test_result_set_iterator_metadata() {
        let mock_transport = MockTransport::new();
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(mock_transport));

        let data = ResultData {
            columns: vec![decimal_column("id")],
            data: ResultPayload::Json(vec![vec![serde_json::json!(1)]]),
            total_rows: 1,
        };

        let result = TransportQueryResult::ResultSet {
            handle: Some(ResultSetHandle::new(1)),
            data,
        };

        let result_set = ResultSet::from_transport_result(result, transport).unwrap();
        let iterator = result_set.into_iterator().unwrap();

        let metadata = iterator.metadata();
        assert_eq!(metadata.column_count, 1);
        assert_eq!(metadata.total_rows, Some(1));
        assert_eq!(metadata.column_names(), vec!["id"]);
    }

    #[tokio::test]
    async fn test_result_set_iterator_metadata_multiple_columns() {
        let mock_transport = MockTransport::new();
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(mock_transport));

        let data = ResultData {
            columns: vec![
                decimal_column("id"),
                varchar_column("name"),
                boolean_column("active"),
            ],
            data: ResultPayload::Json(vec![vec![
                serde_json::json!(1),
                serde_json::json!("Alice"),
                serde_json::json!(true),
            ]]),
            total_rows: 1,
        };

        let result = TransportQueryResult::ResultSet {
            handle: Some(ResultSetHandle::new(1)),
            data,
        };

        let result_set = ResultSet::from_transport_result(result, transport).unwrap();
        let iterator = result_set.into_iterator().unwrap();

        let metadata = iterator.metadata();
        assert_eq!(metadata.column_count, 3);
        assert_eq!(metadata.column_names(), vec!["id", "name", "active"]);

        let types = metadata.column_types();
        assert_eq!(types.len(), 3);
        assert!(matches!(
            types[0],
            arrow::datatypes::DataType::Decimal128(18, 0)
        ));
        assert!(matches!(types[1], arrow::datatypes::DataType::Utf8));
        assert!(matches!(types[2], arrow::datatypes::DataType::Boolean));
    }

    // =========================================================================
    // Tests for ResultSet::fetch_all pagination
    // =========================================================================

    #[tokio::test]
    async fn test_fetch_all_paginates_until_empty_payload() {
        let handle = ResultSetHandle::new(7);
        let mut transport = transport_serving_one_more_page(&[2]);
        transport
            .expect_close_result_set()
            .with(eq(handle))
            .times(1)
            .returning(|_| Ok(()));

        let batches = streaming_result_set(transport, &[1], 0, Some(handle))
            .fetch_all()
            .await
            .unwrap();

        assert_eq!(batches.len(), 2);
        assert_eq!(ResultSet::row_count_of(&batches), 2);
    }

    #[tokio::test]
    async fn test_fetch_all_stops_once_known_total_is_reached() {
        let handle = ResultSetHandle::new(1);
        let mut transport = MockTransport::new();
        // Each page reports its own `total_rows`; only the metadata total (3)
        // may end pagination, so exactly one extra fetch must happen.
        transport
            .expect_fetch_results()
            .times(1)
            .returning(|_| Ok(single_column_result_data(&[2, 3], 2)));
        transport
            .expect_close_result_set()
            .times(1)
            .returning(|_| Ok(()));

        let batches = streaming_result_set(transport, &[1], 3, Some(handle))
            .fetch_all()
            .await
            .unwrap();

        assert_eq!(batches.len(), 2);
        assert_eq!(ResultSet::row_count_of(&batches), 3);
    }

    #[tokio::test]
    async fn test_fetch_all_skips_pagination_when_first_page_is_complete() {
        let handle = ResultSetHandle::new(3);
        let mut transport = MockTransport::new();
        transport.expect_fetch_results().times(0);
        transport
            .expect_close_result_set()
            .with(eq(handle))
            .times(1)
            .returning(|_| Ok(()));

        let batches = streaming_result_set(transport, &[1, 2], 2, Some(handle))
            .fetch_all()
            .await
            .unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(ResultSet::row_count_of(&batches), 2);
    }

    #[tokio::test]
    async fn test_fetch_all_without_handle_returns_buffered_batches_only() {
        let mut transport = MockTransport::new();
        transport.expect_fetch_results().times(0);
        transport.expect_close_result_set().times(0);

        let batches = streaming_result_set(transport, &[1], 5, None)
            .fetch_all()
            .await
            .unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(ResultSet::row_count_of(&batches), 1);
    }

    #[tokio::test]
    async fn test_fetch_all_propagates_fetch_error() {
        let mut transport = MockTransport::new();
        transport.expect_fetch_results().returning(|_| {
            Err(TransportError::ProtocolError(
                "page fetch failed".to_string(),
            ))
        });

        let err = streaming_result_set(transport, &[1], 0, Some(ResultSetHandle::new(1)))
            .fetch_all()
            .await
            .unwrap_err();

        assert!(matches!(err, QueryError::ExecutionFailed(_)));
        assert!(err.to_string().contains("page fetch failed"));
    }

    #[tokio::test]
    async fn test_fetch_all_ignores_close_result_set_error() {
        let mut transport = MockTransport::new();
        transport
            .expect_close_result_set()
            .times(1)
            .returning(|_| Err(TransportError::IoError("socket gone".to_string())));

        let batches = streaming_result_set(transport, &[1], 1, Some(ResultSetHandle::new(1)))
            .fetch_all()
            .await
            .unwrap();

        assert_eq!(batches.len(), 1);
    }

    #[tokio::test]
    async fn test_fetch_all_on_row_count_result_reports_no_result_set() {
        let transport: Arc<Mutex<dyn TransportProtocol>> =
            Arc::new(Mutex::new(MockTransport::new()));
        let result_set = ResultSet::from_transport_result(
            TransportQueryResult::RowCount { count: 5 },
            transport,
        )
        .unwrap();

        let err = result_set.fetch_all().await.unwrap_err();

        assert!(matches!(err, QueryError::NoResultSet(_)));
        assert!(err.to_string().contains("Cannot fetch batches"));
    }

    #[test]
    fn test_into_iterator_on_row_count_result_reports_no_result_set() {
        let transport: Arc<Mutex<dyn TransportProtocol>> =
            Arc::new(Mutex::new(MockTransport::new()));
        let result_set = ResultSet::from_transport_result(
            TransportQueryResult::RowCount { count: 5 },
            transport,
        )
        .unwrap();

        let err = match result_set.into_iterator() {
            Err(err) => err,
            Ok(_) => panic!("row count result must not yield an iterator"),
        };

        assert!(matches!(err, QueryError::NoResultSet(_)));
        assert!(err.to_string().contains("Cannot iterate"));
    }

    // =========================================================================
    // Tests for Debug on ResultSet
    // =========================================================================

    #[test]
    fn test_result_set_debug_redacts_transport() {
        let rendered = format!(
            "{:?}",
            streaming_result_set(MockTransport::new(), &[1], 1, None)
        );

        assert!(rendered.starts_with("ResultSet {"), "got: {}", rendered);
        assert!(rendered.contains("Stream"));
        assert!(rendered.contains("<TransportProtocol>"));
    }

    #[test]
    fn test_result_set_debug_shows_row_count_variant() {
        let transport: Arc<Mutex<dyn TransportProtocol>> =
            Arc::new(Mutex::new(MockTransport::new()));
        let result_set = ResultSet::from_transport_result(
            TransportQueryResult::RowCount { count: 42 },
            transport,
        )
        .unwrap();

        let rendered = format!("{:?}", result_set);

        assert!(rendered.contains("RowCount"));
        assert!(rendered.contains("42"));
        assert!(rendered.contains("<TransportProtocol>"));
    }

    // =========================================================================
    // Tests for ResultSetIterator batch fetching
    // =========================================================================

    #[test]
    fn test_next_batch_serves_buffered_page_then_fetches_the_next() {
        let runtime = entered_runtime();
        let _guard = runtime.enter();

        let transport = transport_serving_one_more_page(&[2]);

        let mut iterator = streaming_iterator(transport, &[1], 0, Some(ResultSetHandle::new(1)));

        assert_eq!(iterator.next_batch().unwrap().unwrap().num_rows(), 1);
        assert_eq!(iterator.next_batch().unwrap().unwrap().num_rows(), 1);
        assert!(iterator.next_batch().is_none());
        // Once exhausted the iterator must not fetch again.
        assert!(iterator.next_batch().is_none());
    }

    #[test]
    fn test_next_batch_propagates_fetch_error() {
        let runtime = entered_runtime();
        let _guard = runtime.enter();

        let mut transport = MockTransport::new();
        transport
            .expect_fetch_results()
            .times(1)
            .returning(|_| Err(TransportError::ReceiveError("no page".to_string())));

        let mut iterator = streaming_iterator(transport, &[1], 0, Some(ResultSetHandle::new(1)));
        assert_eq!(iterator.next_batch().unwrap().unwrap().num_rows(), 1);

        let err = iterator.next_batch().unwrap().unwrap_err();

        assert!(matches!(err, QueryError::ExecutionFailed(_)));
        assert!(err.to_string().contains("no page"));
    }

    #[tokio::test]
    async fn test_fetch_next_batch_short_circuits_once_the_stream_is_complete() {
        let mut transport = MockTransport::new();
        transport.expect_fetch_results().times(0);
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(transport));

        let mut iterator = ResultSetIterator {
            handle: Some(ResultSetHandle::new(1)),
            transport,
            metadata: QueryMetadata::new(
                ResultSet::build_schema(&[decimal_column("id")]).unwrap(),
                Some(1),
            ),
            batches: Vec::new(),
            current_index: 0,
            complete: true,
        };

        assert!(iterator.fetch_next_batch().await.unwrap().is_none());
    }

    #[test]
    fn test_next_batch_reports_exhausted_when_handle_was_already_released() {
        let runtime = entered_runtime();
        let _guard = runtime.enter();

        let mut transport = MockTransport::new();
        transport.expect_fetch_results().times(0);
        let transport: Arc<Mutex<dyn TransportProtocol>> = Arc::new(Mutex::new(transport));

        let mut iterator = ResultSetIterator {
            handle: None,
            transport,
            metadata: QueryMetadata::new(
                ResultSet::build_schema(&[decimal_column("id")]).unwrap(),
                Some(1),
            ),
            batches: Vec::new(),
            current_index: 0,
            complete: false,
        };

        assert!(iterator.next_batch().is_none());
    }

    #[test]
    fn test_next_batch_without_async_runtime_reports_invalid_state() {
        let mut transport = MockTransport::new();
        transport.expect_fetch_results().times(0);

        let mut iterator = streaming_iterator(transport, &[1], 0, Some(ResultSetHandle::new(1)));
        assert_eq!(iterator.next_batch().unwrap().unwrap().num_rows(), 1);

        let err = iterator.next_batch().unwrap().unwrap_err();

        assert!(matches!(err, QueryError::InvalidState(_)));
        assert!(err.to_string().contains("No async runtime available"));
    }

    #[test]
    fn test_iterator_trait_yields_every_page_until_exhausted() {
        let runtime = entered_runtime();
        let _guard = runtime.enter();

        let mut transport = MockTransport::new();
        transport
            .expect_fetch_results()
            .times(1)
            .returning(|_| Ok(single_column_result_data(&[], 0)));

        let iterator = streaming_iterator(transport, &[1, 2], 0, Some(ResultSetHandle::new(1)));
        let batches: Vec<RecordBatch> = iterator.collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(ResultSet::row_count_of(&batches), 2);
    }

    // =========================================================================
    // Tests for ResultSetIterator::close
    // =========================================================================

    #[tokio::test]
    async fn test_iterator_close_releases_the_handle() {
        let handle = ResultSetHandle::new(9);
        let mut transport = MockTransport::new();
        transport
            .expect_close_result_set()
            .with(eq(handle))
            .times(1)
            .returning(|_| Ok(()));

        streaming_iterator(transport, &[1], 1, Some(handle))
            .close()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_iterator_close_without_handle_is_a_no_op() {
        let mut transport = MockTransport::new();
        transport.expect_close_result_set().times(0);

        streaming_iterator(transport, &[1], 1, None)
            .close()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_iterator_close_propagates_transport_error() {
        let mut transport = MockTransport::new();
        transport
            .expect_close_result_set()
            .times(1)
            .returning(|_| Err(TransportError::IoError("closed twice".to_string())));

        let err = streaming_iterator(transport, &[1], 1, Some(ResultSetHandle::new(1)))
            .close()
            .await
            .unwrap_err();

        assert!(matches!(err, QueryError::ExecutionFailed(_)));
        assert!(err.to_string().contains("closed twice"));
    }

    // =========================================================================
    // Tests for Arrow payload pass-through
    // =========================================================================

    fn int64_batch(values: Vec<i64>) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "id",
            arrow::datatypes::DataType::Int64,
            true,
        )]));
        RecordBatch::try_new(
            schema,
            vec![Arc::new(arrow::array::Int64Array::from(values))],
        )
        .unwrap()
    }

    #[test]
    fn test_column_major_to_record_batch_returns_arrow_payload_unchanged() {
        let batch = int64_batch(vec![1, 2, 3]);
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Arrow(batch.clone()),
            total_rows: 3,
        };

        let converted = ResultSet::column_major_to_record_batch(&data, &batch.schema()).unwrap();

        assert_eq!(converted.num_rows(), 3);
        assert_eq!(converted.schema(), batch.schema());
    }

    #[test]
    fn test_payload_to_record_batch_returns_arrow_payload_unchanged() {
        let batch = int64_batch(vec![7]);
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Arrow(batch.clone()),
            total_rows: 1,
        };

        let converted = ResultSet::payload_to_record_batch(&data, &batch.schema()).unwrap();

        assert_eq!(converted.num_rows(), 1);
    }

    // =========================================================================
    // Tests for the parse_date_to_days month table
    // =========================================================================

    #[test]
    fn test_parse_date_to_days_covers_every_month_of_a_common_year() {
        // 1970 is a common year, so the first of each month lands exactly on
        // the cumulative day offsets of the month table.
        let first_of_month_offsets = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

        for (index, expected) in first_of_month_offsets.iter().enumerate() {
            let date = format!("1970-{:02}-01", index + 1);
            assert_eq!(
                ResultSet::parse_date_to_days(&date),
                Ok(*expected),
                "unexpected day offset for {}",
                date
            );
        }
    }

    #[test]
    fn test_parse_date_to_days_adds_the_leap_day_only_after_february() {
        // 1972 is a leap year: February has 29 days, so March 1st sits 29 days
        // after February 1st (28 in a common year).
        let february = ResultSet::parse_date_to_days("1972-02-01").unwrap();
        let march = ResultSet::parse_date_to_days("1972-03-01").unwrap();

        assert_eq!(march - february, 29);
    }

    #[test]
    fn test_parse_timestamp_to_micros_always_has_a_date_part_to_parse() {
        // `str::split` always yields at least one element, so the date part is
        // never absent: an empty timestamp reaches (and fails) date parsing.
        assert_eq!("".split(' ').count(), 1);
        assert!(ResultSet::parse_date_to_days("").is_err());
        assert!(ResultSet::parse_timestamp_to_micros("").is_err());
    }

    #[test]
    fn test_parse_timestamp_to_micros_ignores_a_trailing_third_space_part() {
        // Only the first two space-separated parts are interpreted; anything
        // after the time is discarded.
        assert_eq!(
            ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:01 UTC"),
            Ok(1_000_000)
        );
    }

    #[test]
    fn test_parse_timestamp_to_micros_empty_fractional_part_contributes_nothing() {
        assert_eq!(
            ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:01."),
            Ok(1_000_000)
        );
    }

    #[test]
    fn test_parse_timestamp_to_micros_rejects_non_numeric_time_parts() {
        assert!(ResultSet::parse_timestamp_to_micros("1970-01-01 aa:00").is_err());
        assert!(ResultSet::parse_timestamp_to_micros("1970-01-01 00:bb").is_err());
        assert!(ResultSet::parse_timestamp_to_micros("1970-01-01 00:00:cc").is_err());
    }

    // =========================================================================
    // Remaining type-mapping and parsing branches
    // =========================================================================

    #[test]
    fn test_exasol_datatype_to_arrow_named_timestamp_with_local_time_zone() {
        // Exasol may spell the type out instead of setting the flag; both must
        // map to a timezone-carrying Arrow timestamp.
        let data_type = plain_type("TIMESTAMP WITH LOCAL TIME ZONE");

        let arrow_type = ResultSet::exasol_datatype_to_arrow(&data_type).unwrap();

        assert!(matches!(
            arrow_type,
            arrow::datatypes::DataType::Timestamp(_, Some(_))
        ));
    }

    #[test]
    fn test_column_major_to_record_batch_stamps_utc_on_zoned_timestamps() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            arrow::datatypes::DataType::Timestamp(
                arrow::datatypes::TimeUnit::Microsecond,
                Some("UTC".into()),
            ),
            true,
        )]));
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![vec![json!("1970-01-01 00:00:01")], vec![Value::Null]]),
            total_rows: 2,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::TimestampMicrosecondArray>()
            .expect("timestamp column");
        assert_eq!(column.value(0), 1_000_000);
        assert!(column.is_null(1));
        assert_eq!(
            batch.schema().field(0).data_type(),
            &arrow::datatypes::DataType::Timestamp(
                arrow::datatypes::TimeUnit::Microsecond,
                Some("UTC".into())
            )
        );
    }

    #[test]
    fn test_column_major_to_record_batch_decimal_from_unsupported_shape_is_null() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            arrow::datatypes::DataType::Decimal128(18, 2),
            true,
        )]));
        let data = ResultData {
            columns: vec![],
            data: ResultPayload::Json(vec![
                vec![Value::Null],
                vec![json!(true)],
                vec![json!([1, 2])],
            ]),
            total_rows: 3,
        };

        let batch = ResultSet::column_major_to_record_batch(&data, &schema).unwrap();

        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.column(0).null_count(), 3);
    }

    #[test]
    fn test_parse_timestamp_to_micros_time_part_without_a_colon_adds_nothing() {
        assert_eq!(
            ResultSet::parse_timestamp_to_micros("1970-01-02 12"),
            Ok(SECONDS_PER_DAY * MICROS_PER_SECOND)
        );
    }

    #[test]
    fn test_parse_string_to_decimal_rejects_a_non_numeric_fractional_part() {
        let err = ResultSet::parse_string_to_decimal("1.ab", 2).unwrap_err();

        assert!(matches!(err, ConversionError::InvalidFormat(_)));
        assert_eq!(
            err.to_string(),
            "Invalid data format: Invalid decimal part: ab"
        );
    }

    #[test]
    fn test_query_metadata_with_execution_time() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "id",
            arrow::datatypes::DataType::Int64,
            true,
        )]));

        let metadata = QueryMetadata::new(schema, Some(3)).with_execution_time(17);

        assert_eq!(metadata.execution_time_ms, Some(17));
        assert_eq!(metadata.total_rows, Some(3));
        assert_eq!(metadata.column_count, 1);
    }
}
