//! Schema inference from Parquet files.
//!
//! Reads only the Parquet footer, so inference does not load the data.

use std::path::{Path, PathBuf};

use arrow::datatypes::Schema;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use super::infer::{format_column_name, widen_type, InferredColumn, InferredTableSchema};
use super::mapping::{ColumnNameMode, TypeMapper};
use crate::import::ImportError;

/// Infer an Exasol table schema from a single Parquet file.
///
/// This function reads only the Parquet file metadata (not the actual data)
/// to determine the schema.
///
/// # Arguments
///
/// * `file_path` - Path to the Parquet file
/// * `column_name_mode` - How to handle column names in DDL
///
/// # Returns
///
/// An `InferredTableSchema` containing column definitions.
///
/// # Errors
///
/// Returns `ImportError::SchemaInferenceError` if:
/// - The file cannot be opened
/// - The file is not a valid Parquet file
/// - A column type cannot be mapped to Exasol
pub fn infer_schema_from_parquet(
    file_path: &Path,
    column_name_mode: ColumnNameMode,
) -> Result<InferredTableSchema, ImportError> {
    let file = std::fs::File::open(file_path).map_err(|e| {
        ImportError::SchemaInferenceError(format!(
            "Failed to open file '{}': {}",
            file_path.display(),
            e
        ))
    })?;

    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
        ImportError::SchemaInferenceError(format!(
            "Failed to read Parquet metadata from '{}': {}",
            file_path.display(),
            e
        ))
    })?;

    let arrow_schema = builder.schema();

    let columns = arrow_schema_to_columns(arrow_schema, column_name_mode)?;

    Ok(InferredTableSchema {
        columns,
        source_files: vec![file_path.to_path_buf()],
    })
}

/// Infer a union schema from multiple Parquet files.
///
/// This function reads metadata from all files and computes a schema
/// that can accommodate data from all of them using type widening.
///
/// # Arguments
///
/// * `file_paths` - Paths to the Parquet files
/// * `column_name_mode` - How to handle column names in DDL
///
/// # Returns
///
/// An `InferredTableSchema` with widened types.
///
/// # Errors
///
/// Returns `ImportError::SchemaInferenceError` if:
/// - Any file cannot be read
/// - Schemas have incompatible column counts
/// - A type cannot be mapped
pub fn infer_schema_from_parquet_files(
    file_paths: &[PathBuf],
    column_name_mode: ColumnNameMode,
) -> Result<InferredTableSchema, ImportError> {
    if file_paths.is_empty() {
        return Err(ImportError::SchemaInferenceError(
            "No files provided for schema inference".to_string(),
        ));
    }

    if file_paths.len() == 1 {
        return infer_schema_from_parquet(&file_paths[0], column_name_mode);
    }

    // Read schemas from all files
    let mut schemas: Vec<(PathBuf, Schema)> = Vec::with_capacity(file_paths.len());

    for path in file_paths {
        let file = std::fs::File::open(path).map_err(|e| {
            ImportError::SchemaInferenceError(format!(
                "Failed to open file '{}': {}",
                path.display(),
                e
            ))
        })?;

        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            ImportError::SchemaInferenceError(format!(
                "Failed to read Parquet metadata from '{}': {}",
                path.display(),
                e
            ))
        })?;

        schemas.push((path.clone(), builder.schema().as_ref().clone()));
    }

    // Start with the first schema
    let (first_path, first_schema) = &schemas[0];
    let mut columns = arrow_schema_to_columns(first_schema, column_name_mode)?;

    // Merge with remaining schemas
    for (path, schema) in schemas.iter().skip(1) {
        if schema.fields().len() != columns.len() {
            return Err(ImportError::SchemaMismatchError(format!(
                "Schema mismatch: '{}' has {} columns, but '{}' has {} columns",
                first_path.display(),
                columns.len(),
                path.display(),
                schema.fields().len()
            )));
        }

        for (i, field) in schema.fields().iter().enumerate() {
            let other_type = TypeMapper::arrow_to_exasol(field.data_type()).map_err(|e| {
                ImportError::SchemaInferenceError(format!(
                    "Failed to map type for column '{}' in '{}': {}",
                    field.name(),
                    path.display(),
                    e
                ))
            })?;

            columns[i].exasol_type = widen_type(&columns[i].exasol_type, &other_type);
            columns[i].nullable = columns[i].nullable || field.is_nullable();
        }
    }

    Ok(InferredTableSchema {
        columns,
        source_files: file_paths.to_vec(),
    })
}

/// Convert an Arrow schema to a list of inferred columns.
fn arrow_schema_to_columns(
    schema: &Schema,
    column_name_mode: ColumnNameMode,
) -> Result<Vec<InferredColumn>, ImportError> {
    let mut columns = Vec::with_capacity(schema.fields().len());

    for field in schema.fields() {
        let exasol_type = TypeMapper::arrow_to_exasol(field.data_type()).map_err(|e| {
            ImportError::SchemaInferenceError(format!(
                "Failed to map type for column '{}': {}",
                field.name(),
                e
            ))
        })?;

        columns.push(InferredColumn {
            original_name: field.name().clone(),
            ddl_name: format_column_name(field.name(), column_name_mode),
            exasol_type,
            nullable: field.is_nullable(),
        });
    }

    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ExasolType;

    #[test]
    fn test_infer_schema_from_parquet_files_rejects_empty_slice() {
        let err = infer_schema_from_parquet_files(&[], ColumnNameMode::Quoted).unwrap_err();

        assert!(
            matches!(err, ImportError::SchemaInferenceError(_)),
            "got: {err}"
        );
        assert!(
            err.to_string()
                .contains("No files provided for schema inference"),
            "got: {err}"
        );
    }

    fn write_parquet(path: &Path, fields: Vec<arrow::datatypes::Field>) {
        use arrow::array::{ArrayRef, Int32Array, Int64Array, StringArray};
        use arrow::datatypes::{DataType, Schema};
        use arrow::record_batch::RecordBatch;
        use parquet::arrow::ArrowWriter;
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(fields));
        let columns: Vec<ArrayRef> = schema
            .fields()
            .iter()
            .map(|field| match field.data_type() {
                DataType::Int32 => Arc::new(Int32Array::from(vec![1])) as ArrayRef,
                DataType::Int64 => Arc::new(Int64Array::from(vec![1i64])) as ArrayRef,
                _ => Arc::new(StringArray::from(vec!["x"])) as ArrayRef,
            })
            .collect();

        let batch = RecordBatch::try_new(Arc::clone(&schema), columns).expect("batch");
        let file = std::fs::File::create(path).expect("create file");
        let mut writer = ArrowWriter::try_new(file, schema, None).expect("writer");
        writer.write(&batch).expect("write");
        writer.close().expect("close");
    }

    #[test]
    fn test_infer_schema_from_parquet_maps_columns_and_nullability() {
        use arrow::datatypes::{DataType, Field};

        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("data.parquet");
        write_parquet(
            &path,
            vec![
                Field::new("id", DataType::Int64, false),
                Field::new("my name", DataType::Utf8, true),
            ],
        );

        let inferred =
            infer_schema_from_parquet(&path, ColumnNameMode::Sanitize).expect("inference");

        assert_eq!(inferred.source_files, vec![path.clone()]);
        assert_eq!(inferred.columns.len(), 2);
        assert_eq!(inferred.columns[0].original_name, "id");
        assert_eq!(inferred.columns[0].ddl_name, "ID");
        assert_eq!(
            inferred.columns[0].exasol_type,
            ExasolType::Decimal {
                precision: 36,
                scale: 0
            }
        );
        assert!(!inferred.columns[0].nullable);
        assert_eq!(inferred.columns[1].ddl_name, "MY_NAME");
        assert!(inferred.columns[1].nullable);
    }

    #[test]
    fn test_infer_schema_from_parquet_reports_missing_file() {
        let err = infer_schema_from_parquet(
            Path::new("/nonexistent/dir/missing.parquet"),
            ColumnNameMode::Quoted,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("Failed to open file"),
            "got: {err}"
        );
    }

    #[test]
    fn test_infer_schema_from_parquet_reports_unreadable_metadata() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("broken.parquet");
        std::fs::write(&path, b"not parquet at all").expect("write file");

        let err = infer_schema_from_parquet(&path, ColumnNameMode::Quoted).unwrap_err();

        assert!(
            err.to_string().contains("Failed to read Parquet metadata"),
            "got: {err}"
        );
    }

    #[test]
    fn test_infer_schema_from_parquet_files_delegates_for_a_single_file() {
        use arrow::datatypes::{DataType, Field};

        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("one.parquet");
        write_parquet(&path, vec![Field::new("id", DataType::Int32, false)]);

        let inferred =
            infer_schema_from_parquet_files(std::slice::from_ref(&path), ColumnNameMode::Quoted)
                .expect("inference");

        assert_eq!(inferred.source_files, vec![path]);
        assert_eq!(inferred.columns[0].ddl_name, "\"id\"");
    }

    #[test]
    fn test_infer_schema_from_parquet_files_widens_types_and_nullability() {
        use arrow::datatypes::{DataType, Field};

        let dir = tempfile::TempDir::new().expect("temp dir");
        let first = dir.path().join("first.parquet");
        let second = dir.path().join("second.parquet");
        write_parquet(&first, vec![Field::new("id", DataType::Int32, false)]);
        write_parquet(&second, vec![Field::new("id", DataType::Int64, true)]);

        let inferred = infer_schema_from_parquet_files(
            &[first.clone(), second.clone()],
            ColumnNameMode::Quoted,
        )
        .expect("inference");

        assert_eq!(inferred.source_files, vec![first, second]);
        assert_eq!(
            inferred.columns[0].exasol_type,
            ExasolType::Decimal {
                precision: 36,
                scale: 0
            }
        );
        assert!(inferred.columns[0].nullable);
    }

    #[test]
    fn test_infer_schema_from_parquet_files_rejects_differing_column_counts() {
        use arrow::datatypes::{DataType, Field};

        let dir = tempfile::TempDir::new().expect("temp dir");
        let first = dir.path().join("first.parquet");
        let second = dir.path().join("second.parquet");
        write_parquet(&first, vec![Field::new("id", DataType::Int32, false)]);
        write_parquet(
            &second,
            vec![
                Field::new("id", DataType::Int32, false),
                Field::new("extra", DataType::Utf8, true),
            ],
        );

        let err =
            infer_schema_from_parquet_files(&[first, second], ColumnNameMode::Quoted).unwrap_err();

        assert!(
            matches!(err, ImportError::SchemaMismatchError(_)),
            "got: {err}"
        );
        assert!(err.to_string().contains("has 1 columns"), "got: {err}");
        assert!(err.to_string().contains("has 2 columns"), "got: {err}");
    }

    #[test]
    fn test_infer_schema_from_parquet_files_reports_missing_file() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let existing = dir.path().join("one.parquet");
        write_parquet(
            &existing,
            vec![arrow::datatypes::Field::new(
                "id",
                arrow::datatypes::DataType::Int32,
                false,
            )],
        );

        let err = infer_schema_from_parquet_files(
            &[existing, PathBuf::from("/nonexistent/dir/missing.parquet")],
            ColumnNameMode::Quoted,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("Failed to open file"),
            "got: {err}"
        );
    }

    #[test]
    fn test_infer_schema_from_parquet_files_reports_unreadable_metadata() {
        use arrow::datatypes::{DataType, Field};

        let dir = tempfile::TempDir::new().expect("temp dir");
        let first = dir.path().join("first.parquet");
        let broken = dir.path().join("broken.parquet");
        write_parquet(&first, vec![Field::new("id", DataType::Int32, false)]);
        std::fs::write(&broken, b"not parquet at all").expect("write file");

        let err =
            infer_schema_from_parquet_files(&[first, broken], ColumnNameMode::Quoted).unwrap_err();

        assert!(
            err.to_string().contains("Failed to read Parquet metadata"),
            "got: {err}"
        );
    }

    #[test]
    fn test_infer_schema_from_parquet_files_reports_unmappable_type_in_a_later_file() {
        use arrow::array::{ArrayRef, Time32SecondArray};
        use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
        use arrow::record_batch::RecordBatch;
        use parquet::arrow::ArrowWriter;
        use std::sync::Arc;

        let dir = tempfile::TempDir::new().expect("temp dir");
        let first = dir.path().join("first.parquet");
        write_parquet(&first, vec![Field::new("t", DataType::Int32, false)]);

        let second = dir.path().join("second.parquet");
        let schema = Arc::new(Schema::new(vec![Field::new(
            "t",
            DataType::Time32(TimeUnit::Second),
            false,
        )]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Time32SecondArray::from(vec![1])) as ArrayRef],
        )
        .expect("batch");
        let file = std::fs::File::create(&second).expect("create file");
        let mut writer = ArrowWriter::try_new(file, schema, None).expect("writer");
        writer.write(&batch).expect("write");
        writer.close().expect("close");

        let err =
            infer_schema_from_parquet_files(&[first, second], ColumnNameMode::Quoted).unwrap_err();

        assert!(
            err.to_string()
                .contains("Failed to map type for column 't'"),
            "got: {err}"
        );
    }

    #[test]
    fn test_infer_schema_rejects_unmappable_arrow_type() {
        use arrow::datatypes::{DataType, Field, Schema, TimeUnit};

        let schema = Schema::new(vec![Field::new(
            "t",
            DataType::Time32(TimeUnit::Second),
            false,
        )]);

        let err = arrow_schema_to_columns(&schema, ColumnNameMode::Quoted).unwrap_err();

        assert!(
            err.to_string()
                .contains("Failed to map type for column 't'"),
            "got: {err}"
        );
    }
}
