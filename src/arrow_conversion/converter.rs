//! Main converter for transforming Exasol WebSocket JSON responses to Arrow RecordBatch.
//!
//! This module provides the core conversion logic that takes row-major JSON data
//! and converts it to Arrow format.
//!
//! **Note**: Data is expected in row-major format (`data[row_idx][col_idx]`) after
//! the streaming deserializer transposes Exasol's column-major wire format.

use crate::error::ConversionError;
use crate::transport::messages::{ColumnInfo, ResultData, ResultPayload};
use crate::types::{ExasolType, SchemaBuilder};
use arrow::array::RecordBatch;
use arrow::datatypes::Schema;
use serde_json::Value;
use std::sync::Arc;

use super::builders::build_array;

/// Converter for transforming Exasol result data to Arrow RecordBatch.
pub struct ArrowConverter {
    schema: Arc<Schema>,
    column_types: Vec<ExasolType>,
}

impl ArrowConverter {
    /// Create a new Arrow converter from Exasol column metadata.
    ///
    /// # Arguments
    /// * `columns` - Column metadata from Exasol result set
    ///
    /// # Returns
    /// A new `ArrowConverter` instance
    ///
    /// # Errors
    /// Returns `ConversionError` if the schema cannot be built
    pub fn new(columns: &[ColumnInfo]) -> Result<Self, ConversionError> {
        // Extract column metadata and build schema
        let column_metadata: Result<Vec<_>, ConversionError> = columns
            .iter()
            .map(|col| {
                let exasol_type = parse_exasol_type(&col.data_type)?;
                Ok::<(String, ExasolType), ConversionError>((col.name.clone(), exasol_type))
            })
            .collect();

        let column_metadata = column_metadata?;

        // Build Arrow schema
        let mut schema_builder = SchemaBuilder::new();
        let mut column_types = Vec::with_capacity(column_metadata.len());

        for (name, exasol_type) in column_metadata {
            schema_builder = schema_builder.add_column(crate::types::ColumnMetadata {
                name,
                data_type: exasol_type.clone(),
                nullable: true, // Exasol columns are nullable by default
            });
            column_types.push(exasol_type);
        }

        let schema = Arc::new(schema_builder.build()?);

        Ok(Self {
            schema,
            column_types,
        })
    }

    /// Get the Arrow schema for this converter.
    pub fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.schema)
    }

    /// Convert Exasol result data to an Arrow RecordBatch.
    ///
    /// Data is expected in row-major format where `data[row_idx][col_idx]` contains
    /// the value at that position. This method extracts column values from rows
    /// and converts to Arrow format.
    ///
    /// # Arguments
    /// * `result_data` - The result data from Exasol WebSocket response
    ///
    /// # Returns
    /// An Arrow `RecordBatch` containing the converted data
    ///
    /// # Errors
    /// Returns `ConversionError` if:
    /// - The data doesn't match the schema
    /// - Type conversion fails for any value
    /// - Numeric overflow occurs
    /// - UTF-8 validation fails for strings
    pub fn convert_to_record_batch(
        &self,
        result_data: &ResultData,
    ) -> Result<RecordBatch, ConversionError> {
        // Native transport payloads arrive as Arrow already
        let rows = match &result_data.data {
            ResultPayload::Arrow(batch) => return Ok(batch.clone()),
            ResultPayload::Json(rows) => rows,
        };

        if rows.is_empty() {
            return Ok(RecordBatch::new_empty(Arc::clone(&self.schema)));
        }

        self.check_row_width(rows)?;

        let column_values: Vec<Vec<&Value>> = (0..self.column_types.len())
            .map(|col_idx| {
                rows.iter()
                    .map(|row| row.get(col_idx).unwrap_or(&Value::Null))
                    .collect()
            })
            .collect();

        self.build_record_batch(&column_values)
    }

    /// Convert Exasol result data to an Arrow RecordBatch, consuming the input.
    ///
    /// This is an optimized version that takes ownership of the result data,
    /// allowing values to be moved instead of cloned during conversion.
    ///
    /// # Arguments
    /// * `result_data` - The result data from Exasol WebSocket response (consumed)
    ///
    /// # Returns
    /// An Arrow `RecordBatch` containing the converted data
    ///
    /// # Errors
    /// Returns `ConversionError` if:
    /// - The data doesn't match the schema
    /// - Type conversion fails for any value
    /// - Numeric overflow occurs
    /// - UTF-8 validation fails for strings
    pub fn convert_to_record_batch_owned(
        &self,
        result_data: ResultData,
    ) -> Result<RecordBatch, ConversionError> {
        // Native transport payloads arrive as Arrow already
        let mut rows = match result_data.data {
            ResultPayload::Arrow(batch) => return Ok(batch),
            ResultPayload::Json(rows) => rows,
        };

        if rows.is_empty() {
            return Ok(RecordBatch::new_empty(Arc::clone(&self.schema)));
        }

        self.check_row_width(&rows)?;

        let num_columns = self.column_types.len();
        let num_rows = rows.len();
        let mut columns: Vec<Vec<Value>> = (0..num_columns)
            .map(|_| Vec::with_capacity(num_rows))
            .collect();

        for mut row in rows.drain(..) {
            for (col_idx, value) in row.drain(..).enumerate() {
                if col_idx < num_columns {
                    columns[col_idx].push(value);
                }
            }
        }

        let column_values: Vec<Vec<&Value>> = columns
            .iter()
            .map(|column| column.iter().collect())
            .collect();

        self.build_record_batch(&column_values)
    }

    /// Reject data whose first row does not have one value per schema column.
    ///
    /// Exasol sends uniform rows, so the first row is a sufficient witness of
    /// the result set's width; checking every row would cost a full scan for a
    /// condition the server never produces.
    fn check_row_width(&self, rows: &[Vec<Value>]) -> Result<(), ConversionError> {
        match rows.first() {
            Some(first_row) if first_row.len() != self.column_types.len() => {
                Err(ConversionError::SchemaMismatch(format!(
                    "Data has {} columns, expected {}",
                    first_row.len(),
                    self.column_types.len()
                )))
            }
            _ => Ok(()),
        }
    }

    /// Build one Arrow array per schema column and assemble them into a batch.
    ///
    /// `column_values` must be indexed by column position, in schema order.
    fn build_record_batch(
        &self,
        column_values: &[Vec<&Value>],
    ) -> Result<RecordBatch, ConversionError> {
        let arrays: Result<Vec<_>, _> = self
            .column_types
            .iter()
            .enumerate()
            .map(|(col_idx, exasol_type)| {
                build_array(exasol_type, &column_values[col_idx], col_idx)
            })
            .collect();

        RecordBatch::try_new(Arc::clone(&self.schema), arrays?)
            .map_err(|e| ConversionError::ArrowError(e.to_string()))
    }

    /// Convert multiple result chunks to RecordBatches.
    ///
    /// This is useful when fetching large result sets in multiple chunks.
    ///
    /// # Arguments
    /// * `result_data_chunks` - Multiple result data chunks
    ///
    /// # Returns
    /// A vector of `RecordBatch` instances
    pub fn convert_chunks(
        &self,
        result_data_chunks: &[ResultData],
    ) -> Result<Vec<RecordBatch>, ConversionError> {
        result_data_chunks
            .iter()
            .map(|chunk| self.convert_to_record_batch(chunk))
            .collect()
    }

    /// Convert multiple result chunks to RecordBatches, consuming the input.
    ///
    /// This is an optimized version that takes ownership of the chunks to avoid
    /// cloning values during conversion.
    ///
    /// # Arguments
    /// * `result_data_chunks` - Multiple result data chunks (consumed)
    ///
    /// # Returns
    /// A vector of `RecordBatch` instances
    pub fn convert_chunks_owned(
        &self,
        result_data_chunks: Vec<ResultData>,
    ) -> Result<Vec<RecordBatch>, ConversionError> {
        result_data_chunks
            .into_iter()
            .map(|chunk| self.convert_to_record_batch_owned(chunk))
            .collect()
    }
}

/// Parse Exasol DataType from WebSocket message format to ExasolType enum.
fn parse_exasol_type(
    data_type: &crate::transport::messages::DataType,
) -> Result<ExasolType, ConversionError> {
    match data_type.type_name.as_str() {
        "BOOLEAN" => Ok(ExasolType::Boolean),

        "CHAR" => {
            let size = required_attribute(data_type.size, "CHAR", "size")? as usize;
            Ok(ExasolType::Char { size })
        }

        "VARCHAR" => {
            let size = required_attribute(data_type.size, "VARCHAR", "size")? as usize;
            Ok(ExasolType::Varchar { size })
        }

        "DECIMAL" => {
            let precision = required_attribute(data_type.precision, "DECIMAL", "precision")? as u8;
            let scale = required_attribute(data_type.scale, "DECIMAL", "scale")? as i8;
            Ok(ExasolType::Decimal { precision, scale })
        }

        "DOUBLE" => Ok(ExasolType::Double),

        "DATE" => Ok(ExasolType::Date),

        "TIMESTAMP" => {
            let with_local_time_zone = data_type.with_local_time_zone.unwrap_or(false);
            Ok(ExasolType::Timestamp {
                with_local_time_zone,
            })
        }

        "TIMESTAMP WITH LOCAL TIME ZONE" => Ok(ExasolType::Timestamp {
            with_local_time_zone: true,
        }),

        "INTERVAL YEAR TO MONTH" => Ok(ExasolType::IntervalYearToMonth),

        "INTERVAL DAY TO SECOND" => {
            let precision = data_type.fraction.unwrap_or(3) as u8;
            Ok(ExasolType::IntervalDayToSecond { precision })
        }

        "GEOMETRY" => Ok(ExasolType::Geometry { srid: None }),

        "HASHTYPE" => {
            let byte_size = data_type.size.unwrap_or(16) as usize;
            Ok(ExasolType::Hashtype { byte_size })
        }

        unknown => Err(ConversionError::UnsupportedType {
            exasol_type: unknown.to_string(),
        }),
    }
}

/// Unwrap a type attribute Exasol must send for the given type name.
fn required_attribute<T>(
    value: Option<T>,
    type_name: &str,
    attribute: &str,
) -> Result<T, ConversionError> {
    value.ok_or_else(|| {
        ConversionError::InvalidFormat(format!("{} type missing {}", type_name, attribute))
    })
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::transport::messages::{DataType, ResultPayload};
    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType as ArrowDataType, Field};
    use serde_json::json;

    /// A WebSocket `DataType` carrying only a type name, as Exasol sends for
    /// types whose shape needs no extra metadata.
    fn scalar_type(type_name: &str) -> DataType {
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

    fn sized_type(type_name: &str, size: i64) -> DataType {
        DataType {
            size: Some(size),
            ..scalar_type(type_name)
        }
    }

    fn decimal_type(precision: i32, scale: i32) -> DataType {
        DataType {
            precision: Some(precision),
            scale: Some(scale),
            ..scalar_type("DECIMAL")
        }
    }

    fn column(name: &str, data_type: DataType) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type,
        }
    }

    fn json_result(columns: &[ColumnInfo], rows: Vec<Vec<Value>>) -> ResultData {
        ResultData {
            columns: columns.to_vec(),
            total_rows: rows.len() as i64,
            data: ResultPayload::Json(rows),
        }
    }

    fn arrow_result(columns: &[ColumnInfo], batch: RecordBatch) -> ResultData {
        ResultData {
            columns: columns.to_vec(),
            total_rows: batch.num_rows() as i64,
            data: ResultPayload::Arrow(batch),
        }
    }

    /// Two result chunks of two rows each, as a paged result set arrives.
    fn two_chunks(columns: &[ColumnInfo]) -> Vec<ResultData> {
        vec![
            json_result(
                columns,
                vec![
                    vec![json!(1), json!("Alice"), json!(true)],
                    vec![json!(2), json!("Bob"), json!(false)],
                ],
            ),
            json_result(
                columns,
                vec![
                    vec![json!(3), json!("Charlie"), json!(true)],
                    vec![json!(4), json!("Dave"), json!(false)],
                ],
            ),
        ]
    }

    fn invalid_format_message(error: ConversionError) -> String {
        match error {
            ConversionError::InvalidFormat(message) => message,
            other => panic!("Expected InvalidFormat, got {:?}", other),
        }
    }

    fn schema_mismatch_message(error: ConversionError) -> String {
        match error {
            ConversionError::SchemaMismatch(message) => message,
            other => panic!("Expected SchemaMismatch, got {:?}", other),
        }
    }

    fn native_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "n",
            ArrowDataType::Int32,
            false,
        )]));
        RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1, 2, 3]))]).unwrap()
    }

    fn create_test_columns() -> Vec<ColumnInfo> {
        vec![
            column("id", decimal_type(18, 0)),
            column(
                "name",
                DataType {
                    character_set: Some("UTF8".to_string()),
                    ..sized_type("VARCHAR", 100)
                },
            ),
            column("active", scalar_type("BOOLEAN")),
        ]
    }

    fn three_sample_rows() -> Vec<Vec<Value>> {
        vec![
            vec![json!(1), json!("Alice"), json!(true)],
            vec![json!(2), json!("Bob"), json!(false)],
            vec![json!(3), json!("Charlie"), json!(true)],
        ]
    }

    fn three_rows_with_nulls() -> Vec<Vec<Value>> {
        vec![
            vec![json!(1), json!("Alice"), json!(true)],
            vec![json!(2), json!(null), json!(false)],
            vec![json!(null), json!("Charlie"), json!(null)],
        ]
    }

    fn mixed_type_columns() -> Vec<ColumnInfo> {
        vec![
            column("bool_col", scalar_type("BOOLEAN")),
            column("decimal_col", decimal_type(10, 2)),
            column("double_col", scalar_type("DOUBLE")),
            column("date_col", scalar_type("DATE")),
        ]
    }

    fn mixed_type_rows() -> Vec<Vec<Value>> {
        vec![
            vec![
                json!(true),
                json!("123.45"),
                json!(std::f64::consts::PI),
                json!("2024-01-15"),
            ],
            vec![
                json!(false),
                json!("678.90"),
                json!(std::f64::consts::E),
                json!("2024-02-20"),
            ],
        ]
    }

    #[test]
    fn test_arrow_converter_creation() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let schema = converter.schema();
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(schema.field(2).name(), "active");
    }

    #[test]
    fn test_arrow_converter_creation_rejects_unsupported_column_type() {
        let columns = vec![column("mystery", scalar_type("QUANTUM"))];
        let result = ArrowConverter::new(&columns);
        assert!(matches!(
            result,
            Err(ConversionError::UnsupportedType { .. })
        ));
    }

    #[test]
    fn test_convert_empty_result() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, vec![]);

        let batch = converter.convert_to_record_batch(&result_data).unwrap();
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 3);
    }

    #[test]
    fn test_convert_simple_result() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, three_sample_rows());

        let batch = converter.convert_to_record_batch(&result_data).unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 3);
    }

    #[test]
    fn test_convert_simple_result_owned() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, three_sample_rows());

        let batch = converter
            .convert_to_record_batch_owned(result_data)
            .unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 3);
    }

    #[test]
    fn test_convert_with_nulls() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, three_rows_with_nulls());

        let batch = converter.convert_to_record_batch(&result_data).unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 3);

        assert_eq!(batch.column(0).null_count(), 1);
        assert_eq!(batch.column(1).null_count(), 1);
        assert_eq!(batch.column(2).null_count(), 1);
    }

    #[test]
    fn test_convert_with_nulls_owned() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, three_rows_with_nulls());

        let batch = converter
            .convert_to_record_batch_owned(result_data)
            .unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 3);

        assert_eq!(batch.column(0).null_count(), 1);
        assert_eq!(batch.column(1).null_count(), 1);
        assert_eq!(batch.column(2).null_count(), 1);
    }

    #[test]
    fn test_convert_multiple_chunks() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let chunks = two_chunks(&columns);

        let batches = converter.convert_chunks(&chunks).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[1].num_rows(), 2);
    }

    #[test]
    fn test_convert_multiple_chunks_owned() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let chunks = two_chunks(&columns);

        let batches = converter.convert_chunks_owned(chunks).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[1].num_rows(), 2);
    }

    #[test]
    fn test_convert_chunks_propagates_conversion_error() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let chunks = vec![json_result(&columns, vec![vec![json!(1), json!("Alice")]])];

        assert!(matches!(
            converter.convert_chunks(&chunks).unwrap_err(),
            ConversionError::SchemaMismatch(_)
        ));
    }

    #[test]
    fn test_convert_chunks_owned_propagates_conversion_error() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let chunks = vec![json_result(&columns, vec![vec![json!(1), json!("Alice")]])];

        assert!(matches!(
            converter.convert_chunks_owned(chunks).unwrap_err(),
            ConversionError::SchemaMismatch(_)
        ));
    }

    #[test]
    fn test_schema_mismatch_error() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        // Wrong number of columns in data (row has only 2 values instead of 3)
        let result_data = json_result(&columns, vec![vec![json!(1), json!("Alice")]]);

        let message =
            schema_mismatch_message(converter.convert_to_record_batch(&result_data).unwrap_err());
        assert_eq!(message, "Data has 2 columns, expected 3");
    }

    #[test]
    fn test_schema_mismatch_error_owned() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, vec![vec![json!(1), json!("Alice")]]);

        let message = schema_mismatch_message(
            converter
                .convert_to_record_batch_owned(result_data)
                .unwrap_err(),
        );
        assert_eq!(message, "Data has 2 columns, expected 3");
    }

    #[test]
    fn test_convert_pads_short_trailing_row_with_nulls() {
        // Only the first row's width is validated; a later short row is padded
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(
            &columns,
            vec![
                vec![json!(1), json!("Alice"), json!(true)],
                vec![json!(2), json!("Bob")],
            ],
        );

        let batch = converter.convert_to_record_batch(&result_data).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.column(2).null_count(), 1);
    }

    #[test]
    fn test_convert_owned_rejects_short_trailing_row() {
        // The owned path builds ragged columns instead of padding, which Arrow rejects
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(
            &columns,
            vec![
                vec![json!(1), json!("Alice"), json!(true)],
                vec![json!(2), json!("Bob")],
            ],
        );

        let error = converter
            .convert_to_record_batch_owned(result_data)
            .unwrap_err();
        assert!(matches!(error, ConversionError::ArrowError(_)));
    }

    #[test]
    fn test_convert_owned_drops_extra_values_in_trailing_row() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(
            &columns,
            vec![
                vec![json!(1), json!("Alice"), json!(true)],
                vec![json!(2), json!("Bob"), json!(false), json!("surplus")],
            ],
        );

        let batch = converter
            .convert_to_record_batch_owned(result_data)
            .unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);
    }

    #[test]
    fn test_convert_returns_arrow_payload_unchanged() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = arrow_result(&columns, native_batch());

        let batch = converter.convert_to_record_batch(&result_data).unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 1);
        assert_eq!(batch.schema().field(0).name(), "n");
    }

    #[test]
    fn test_convert_owned_returns_arrow_payload_unchanged() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = arrow_result(&columns, native_batch());

        let batch = converter
            .convert_to_record_batch_owned(result_data)
            .unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 1);
        assert_eq!(batch.schema().field(0).name(), "n");
    }

    #[test]
    fn test_convert_empty_arrow_payload_is_returned_unchanged() {
        // An empty Arrow batch keeps its own schema instead of the converter's
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let empty = RecordBatch::new_empty(native_batch().schema());
        let result_data = arrow_result(&columns, empty);

        let batch = converter.convert_to_record_batch(&result_data).unwrap();
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 1);
    }

    #[test]
    fn test_parse_all_exasol_types() {
        let test_cases = vec![
            scalar_type("BOOLEAN"),
            sized_type("CHAR", 10),
            sized_type("VARCHAR", 100),
            decimal_type(18, 2),
            scalar_type("DOUBLE"),
            scalar_type("DATE"),
            DataType {
                with_local_time_zone: Some(false),
                ..scalar_type("TIMESTAMP")
            },
        ];

        for data_type in test_cases {
            let result = parse_exasol_type(&data_type);
            assert!(result.is_ok(), "Failed to parse: {:?}", data_type);
        }
    }

    #[test]
    fn test_parse_exasol_type_char_uses_declared_size() {
        assert_eq!(
            parse_exasol_type(&sized_type("CHAR", 10)).unwrap(),
            ExasolType::Char { size: 10 }
        );
    }

    #[test]
    fn test_parse_exasol_type_char_without_size_is_rejected() {
        let message = invalid_format_message(parse_exasol_type(&scalar_type("CHAR")).unwrap_err());
        assert_eq!(message, "CHAR type missing size");
    }

    #[test]
    fn test_parse_exasol_type_varchar_uses_declared_size() {
        assert_eq!(
            parse_exasol_type(&sized_type("VARCHAR", 100)).unwrap(),
            ExasolType::Varchar { size: 100 }
        );
    }

    #[test]
    fn test_parse_exasol_type_varchar_without_size_is_rejected() {
        let message =
            invalid_format_message(parse_exasol_type(&scalar_type("VARCHAR")).unwrap_err());
        assert_eq!(message, "VARCHAR type missing size");
    }

    #[test]
    fn test_parse_exasol_type_decimal_uses_precision_and_scale() {
        assert_eq!(
            parse_exasol_type(&decimal_type(18, 2)).unwrap(),
            ExasolType::Decimal {
                precision: 18,
                scale: 2
            }
        );
    }

    #[test]
    fn test_parse_exasol_type_decimal_without_precision_is_rejected() {
        let data_type = DataType {
            scale: Some(2),
            ..scalar_type("DECIMAL")
        };
        let message = invalid_format_message(parse_exasol_type(&data_type).unwrap_err());
        assert_eq!(message, "DECIMAL type missing precision");
    }

    #[test]
    fn test_parse_exasol_type_decimal_without_scale_is_rejected() {
        let data_type = DataType {
            precision: Some(18),
            ..scalar_type("DECIMAL")
        };
        let message = invalid_format_message(parse_exasol_type(&data_type).unwrap_err());
        assert_eq!(message, "DECIMAL type missing scale");
    }

    #[test]
    fn test_parse_exasol_type_boolean() {
        assert_eq!(
            parse_exasol_type(&scalar_type("BOOLEAN")).unwrap(),
            ExasolType::Boolean
        );
    }

    #[test]
    fn test_parse_exasol_type_double() {
        assert_eq!(
            parse_exasol_type(&scalar_type("DOUBLE")).unwrap(),
            ExasolType::Double
        );
    }

    #[test]
    fn test_parse_exasol_type_date() {
        assert_eq!(
            parse_exasol_type(&scalar_type("DATE")).unwrap(),
            ExasolType::Date
        );
    }

    #[test]
    fn test_parse_exasol_type_interval_year_to_month() {
        assert_eq!(
            parse_exasol_type(&scalar_type("INTERVAL YEAR TO MONTH")).unwrap(),
            ExasolType::IntervalYearToMonth
        );
    }

    #[test]
    fn test_parse_exasol_type_interval_day_to_second_uses_fraction_as_precision() {
        let data_type = DataType {
            fraction: Some(6),
            ..scalar_type("INTERVAL DAY TO SECOND")
        };
        assert_eq!(
            parse_exasol_type(&data_type).unwrap(),
            ExasolType::IntervalDayToSecond { precision: 6 }
        );
    }

    #[test]
    fn test_parse_exasol_type_interval_day_to_second_defaults_precision() {
        assert_eq!(
            parse_exasol_type(&scalar_type("INTERVAL DAY TO SECOND")).unwrap(),
            ExasolType::IntervalDayToSecond { precision: 3 }
        );
    }

    #[test]
    fn test_parse_exasol_type_geometry_has_no_srid() {
        assert_eq!(
            parse_exasol_type(&scalar_type("GEOMETRY")).unwrap(),
            ExasolType::Geometry { srid: None }
        );
    }

    #[test]
    fn test_parse_exasol_type_hashtype_uses_declared_size() {
        assert_eq!(
            parse_exasol_type(&sized_type("HASHTYPE", 32)).unwrap(),
            ExasolType::Hashtype { byte_size: 32 }
        );
    }

    #[test]
    fn test_parse_exasol_type_hashtype_defaults_byte_size() {
        assert_eq!(
            parse_exasol_type(&scalar_type("HASHTYPE")).unwrap(),
            ExasolType::Hashtype { byte_size: 16 }
        );
    }

    #[test]
    fn test_unsupported_type_error() {
        let data_type = scalar_type("UNKNOWN_TYPE");

        let result = parse_exasol_type(&data_type);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConversionError::UnsupportedType { .. }
        ));
    }

    #[test]
    fn test_all_data_types_conversion() {
        let columns = mixed_type_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, mixed_type_rows());

        let batch = converter.convert_to_record_batch(&result_data).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 4);
    }

    #[test]
    fn test_all_data_types_conversion_owned() {
        let columns = mixed_type_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, mixed_type_rows());

        let batch = converter
            .convert_to_record_batch_owned(result_data)
            .unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 4);
    }

    #[test]
    fn test_convert_empty_result_owned() {
        let columns = create_test_columns();
        let converter = ArrowConverter::new(&columns).unwrap();

        let result_data = json_result(&columns, vec![]);

        let batch = converter
            .convert_to_record_batch_owned(result_data)
            .unwrap();
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 3);
    }

    #[test]
    fn test_parse_timestamp_with_local_time_zone() {
        // When the WebSocket API sends type_name "TIMESTAMP WITH LOCAL TIME ZONE",
        // it should be parsed as ExasolType::Timestamp { with_local_time_zone: true }
        let result = parse_exasol_type(&scalar_type("TIMESTAMP WITH LOCAL TIME ZONE")).unwrap();
        assert_eq!(
            result,
            ExasolType::Timestamp {
                with_local_time_zone: true
            }
        );
    }

    #[test]
    fn test_parse_timestamp_without_local_time_zone() {
        // Regular TIMESTAMP without withLocalTimeZone property should default to false
        let result = parse_exasol_type(&scalar_type("TIMESTAMP")).unwrap();
        assert_eq!(
            result,
            ExasolType::Timestamp {
                with_local_time_zone: false
            }
        );
    }

    #[test]
    fn test_parse_timestamp_with_local_time_zone_property() {
        // TIMESTAMP with withLocalTimeZone=true property should also work
        let data_type = DataType {
            with_local_time_zone: Some(true),
            ..scalar_type("TIMESTAMP")
        };
        let result = parse_exasol_type(&data_type).unwrap();
        assert_eq!(
            result,
            ExasolType::Timestamp {
                with_local_time_zone: true
            }
        );
    }
}
