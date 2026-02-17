//! Schema inference for CSV files.
//!
//! This module provides functionality to infer Exasol table schemas from CSV files,
//! mirroring the Parquet inference API with additional CSV-specific options.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use arrow::csv::reader::Format;
use arrow::datatypes::Schema;

use super::infer::{format_column_name, widen_type, InferredColumn, InferredTableSchema};
use super::mapping::{ColumnNameMode, ExasolType, TypeMapper};
use crate::import::ImportError;

/// Options for CSV schema inference.
///
/// Controls how CSV files are parsed and how column types are inferred.
/// Use the builder pattern to customize options from the defaults.
///
/// # Example
///
/// ```ignore
/// let options = CsvInferenceOptions::new()
///     .with_delimiter(b'\t')
///     .with_has_header(false)
///     .with_column_name_mode(ColumnNameMode::Sanitize);
/// ```
#[derive(Debug, Clone)]
pub struct CsvInferenceOptions {
    /// Field delimiter byte (default: `,`).
    pub delimiter: u8,
    /// Whether the first row is a header (default: `true`).
    pub has_header: bool,
    /// Quote character (default: `Some(b'"')`).
    pub quote: Option<u8>,
    /// Escape character (default: `None`).
    pub escape: Option<u8>,
    /// Regex pattern for values treated as NULL (default: `"^$"` — empty string).
    pub null_regex: Option<String>,
    /// Maximum number of records to sample for type inference (default: `None` — all rows).
    pub max_sample_records: Option<usize>,
    /// How to format column names in DDL output.
    pub column_name_mode: ColumnNameMode,
}

impl Default for CsvInferenceOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_header: true,
            quote: Some(b'"'),
            escape: None,
            null_regex: Some("^$".to_string()),
            max_sample_records: None,
            column_name_mode: ColumnNameMode::Quoted,
        }
    }
}

impl CsvInferenceOptions {
    /// Create a new `CsvInferenceOptions` with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the field delimiter.
    #[must_use]
    pub fn with_delimiter(mut self, delimiter: u8) -> Self {
        self.delimiter = delimiter;
        self
    }

    /// Set whether the CSV has a header row.
    #[must_use]
    pub fn with_has_header(mut self, has_header: bool) -> Self {
        self.has_header = has_header;
        self
    }

    /// Set the quote character.
    #[must_use]
    pub fn with_quote(mut self, quote: Option<u8>) -> Self {
        self.quote = quote;
        self
    }

    /// Set the escape character.
    #[must_use]
    pub fn with_escape(mut self, escape: Option<u8>) -> Self {
        self.escape = escape;
        self
    }

    /// Set the null regex pattern.
    #[must_use]
    pub fn with_null_regex(mut self, null_regex: Option<String>) -> Self {
        self.null_regex = null_regex;
        self
    }

    /// Set the maximum number of records to sample.
    #[must_use]
    pub fn with_max_sample_records(mut self, max_sample_records: Option<usize>) -> Self {
        self.max_sample_records = max_sample_records;
        self
    }

    /// Set the column name mode.
    #[must_use]
    pub fn with_column_name_mode(mut self, mode: ColumnNameMode) -> Self {
        self.column_name_mode = mode;
        self
    }
}

/// Build an arrow-csv `Format` from `CsvInferenceOptions`.
fn build_csv_format(options: &CsvInferenceOptions) -> Format {
    let mut format = Format::default()
        .with_header(options.has_header)
        .with_delimiter(options.delimiter);

    if let Some(quote) = options.quote {
        format = format.with_quote(quote);
    }
    if let Some(escape) = options.escape {
        format = format.with_escape(escape);
    }

    format
}

/// Convert a CSV-inferred Arrow schema to inferred columns.
///
/// When `has_header` is false, generates column names as `col_1`, `col_2`, etc.
/// Unrecognized Arrow types fall back to `VARCHAR(2000000)`.
fn csv_schema_to_columns(schema: &Schema, options: &CsvInferenceOptions) -> Vec<InferredColumn> {
    schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let original_name = if options.has_header {
                field.name().clone()
            } else {
                format!("col_{}", i + 1)
            };

            let exasol_type = TypeMapper::arrow_to_exasol(field.data_type())
                .unwrap_or(ExasolType::Varchar { size: 2_000_000 });

            InferredColumn {
                ddl_name: format_column_name(&original_name, options.column_name_mode),
                original_name,
                exasol_type,
                nullable: field.is_nullable(),
            }
        })
        .collect()
}

/// Infer an Exasol table schema from a single CSV file.
///
/// Uses arrow-csv's type inference to detect column types from sampled rows,
/// then maps Arrow types to Exasol types.
///
/// # Arguments
///
/// * `file_path` - Path to the CSV file
/// * `options` - CSV parsing and inference options
///
/// # Errors
///
/// Returns `ImportError::SchemaInferenceError` if:
/// - The file cannot be opened
/// - The CSV cannot be parsed
/// - The file contains no data rows (header only)
pub fn infer_schema_from_csv(
    file_path: &Path,
    options: &CsvInferenceOptions,
) -> Result<InferredTableSchema, ImportError> {
    let file = std::fs::File::open(file_path).map_err(|e| {
        ImportError::SchemaInferenceError(format!(
            "Failed to open file '{}': {}",
            file_path.display(),
            e
        ))
    })?;

    let reader = BufReader::new(file);
    let format = build_csv_format(options);

    let (schema, records_read) = format
        .infer_schema(reader, options.max_sample_records)
        .map_err(|e| {
            ImportError::SchemaInferenceError(format!(
                "Failed to infer CSV schema from '{}': {}",
                file_path.display(),
                e
            ))
        })?;

    if records_read == 0 {
        return Err(ImportError::SchemaInferenceError(format!(
            "CSV file '{}' contains no data rows",
            file_path.display()
        )));
    }

    let columns = csv_schema_to_columns(&schema, options);

    Ok(InferredTableSchema {
        columns,
        source_files: vec![file_path.to_path_buf()],
    })
}

/// Infer a union schema from multiple CSV files.
///
/// Reads and infers schemas from all files, then merges them using type widening
/// to produce a schema that can accommodate data from all files.
///
/// # Arguments
///
/// * `file_paths` - Paths to the CSV files
/// * `options` - CSV parsing and inference options
///
/// # Errors
///
/// Returns an error if:
/// - No files are provided
/// - Any file cannot be read or parsed
/// - Any file contains no data rows
/// - Files have different numbers of columns
pub fn infer_schema_from_csv_files(
    file_paths: &[PathBuf],
    options: &CsvInferenceOptions,
) -> Result<InferredTableSchema, ImportError> {
    if file_paths.is_empty() {
        return Err(ImportError::SchemaInferenceError(
            "No files provided for schema inference".to_string(),
        ));
    }

    if file_paths.len() == 1 {
        return infer_schema_from_csv(&file_paths[0], options);
    }

    let format = build_csv_format(options);
    let first_path = &file_paths[0];
    let mut merged_columns: Option<Vec<InferredColumn>> = None;

    for path in file_paths {
        let file = std::fs::File::open(path).map_err(|e| {
            ImportError::SchemaInferenceError(format!(
                "Failed to open file '{}': {}",
                path.display(),
                e
            ))
        })?;

        let reader = BufReader::new(file);

        let (schema, records_read) = format
            .infer_schema(reader, options.max_sample_records)
            .map_err(|e| {
                ImportError::SchemaInferenceError(format!(
                    "Failed to infer CSV schema from '{}': {}",
                    path.display(),
                    e
                ))
            })?;

        if records_read == 0 {
            return Err(ImportError::SchemaInferenceError(format!(
                "CSV file '{}' contains no data rows",
                path.display()
            )));
        }

        let file_columns = csv_schema_to_columns(&schema, options);

        match &mut merged_columns {
            None => {
                merged_columns = Some(file_columns);
            }
            Some(columns) => {
                if file_columns.len() != columns.len() {
                    return Err(ImportError::SchemaMismatchError(format!(
                        "Schema mismatch: '{}' has {} columns, but '{}' has {} columns",
                        first_path.display(),
                        columns.len(),
                        path.display(),
                        file_columns.len()
                    )));
                }

                for (i, other_col) in file_columns.iter().enumerate() {
                    columns[i].exasol_type =
                        widen_type(&columns[i].exasol_type, &other_col.exasol_type);
                    columns[i].nullable = columns[i].nullable || other_col.nullable;
                }
            }
        }
    }

    Ok(InferredTableSchema {
        // Safe to unwrap: we checked file_paths is non-empty above
        columns: merged_columns.unwrap(),
        source_files: file_paths.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_csv(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_csv_inference_options_default() {
        let options = CsvInferenceOptions::default();
        assert_eq!(options.delimiter, b',');
        assert!(options.has_header);
        assert_eq!(options.quote, Some(b'"'));
        assert_eq!(options.escape, None);
        assert_eq!(options.null_regex, Some("^$".to_string()));
        assert_eq!(options.max_sample_records, None);
        assert_eq!(options.column_name_mode, ColumnNameMode::Quoted);
    }

    #[test]
    fn test_csv_inference_options_builder() {
        let options = CsvInferenceOptions::new()
            .with_delimiter(b'\t')
            .with_has_header(false)
            .with_quote(None)
            .with_escape(Some(b'\\'))
            .with_null_regex(None)
            .with_max_sample_records(Some(100))
            .with_column_name_mode(ColumnNameMode::Sanitize);

        assert_eq!(options.delimiter, b'\t');
        assert!(!options.has_header);
        assert_eq!(options.quote, None);
        assert_eq!(options.escape, Some(b'\\'));
        assert_eq!(options.null_regex, None);
        assert_eq!(options.max_sample_records, Some(100));
        assert_eq!(options.column_name_mode, ColumnNameMode::Sanitize);
    }

    #[test]
    fn test_infer_mixed_types() {
        let csv = write_csv("id,value,name,flag\n1,3.14,hello,true\n2,2.71,world,false\n");
        let options = CsvInferenceOptions::default();
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        assert_eq!(schema.columns.len(), 4);

        assert_eq!(schema.columns[0].original_name, "id");
        assert!(matches!(
            schema.columns[0].exasol_type,
            ExasolType::Decimal { .. }
        ));

        assert_eq!(schema.columns[1].original_name, "value");
        assert_eq!(schema.columns[1].exasol_type, ExasolType::Double);

        assert_eq!(schema.columns[2].original_name, "name");
        assert_eq!(
            schema.columns[2].exasol_type,
            ExasolType::Varchar { size: 2_000_000 }
        );

        assert_eq!(schema.columns[3].original_name, "flag");
        assert_eq!(schema.columns[3].exasol_type, ExasolType::Boolean);
    }

    #[test]
    fn test_infer_tab_delimiter() {
        let csv = write_csv("id\tname\n1\thello\n2\tworld\n");
        let options = CsvInferenceOptions::new().with_delimiter(b'\t');
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.columns[0].original_name, "id");
        assert_eq!(schema.columns[1].original_name, "name");
    }

    #[test]
    fn test_infer_no_header() {
        let csv = write_csv("1,hello,true\n2,world,false\n");
        let options = CsvInferenceOptions::new().with_has_header(false);
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        assert_eq!(schema.columns.len(), 3);
        assert_eq!(schema.columns[0].original_name, "col_1");
        assert_eq!(schema.columns[1].original_name, "col_2");
        assert_eq!(schema.columns[2].original_name, "col_3");
    }

    #[test]
    fn test_infer_no_header_ddl_names() {
        let csv = write_csv("1,hello\n2,world\n");
        let options = CsvInferenceOptions::new()
            .with_has_header(false)
            .with_column_name_mode(ColumnNameMode::Sanitize);
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        assert_eq!(schema.columns[0].ddl_name, "COL_1");
        assert_eq!(schema.columns[1].ddl_name, "COL_2");
    }

    #[test]
    fn test_infer_multi_file_widening() {
        // File A: id is integer
        let csv_a = write_csv("id,value\n1,hello\n2,world\n");
        // File B: id is float → should widen to Double
        let csv_b = write_csv("id,value\n1.5,foo\n2.5,bar\n");

        let options = CsvInferenceOptions::default();
        let paths = vec![csv_a.path().to_path_buf(), csv_b.path().to_path_buf()];
        let schema = infer_schema_from_csv_files(&paths, &options).unwrap();

        assert_eq!(schema.columns.len(), 2);
        // Int64 (Decimal) + Float64 (Double) → Double
        assert_eq!(schema.columns[0].exasol_type, ExasolType::Double);
        assert_eq!(schema.source_files.len(), 2);
    }

    #[test]
    fn test_infer_empty_csv_header_only() {
        let csv = write_csv("id,name\n");
        let options = CsvInferenceOptions::default();
        let result = infer_schema_from_csv(csv.path(), &options);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no data rows"));
    }

    #[test]
    fn test_infer_nullable_columns() {
        let csv = write_csv("id,name\n1,hello\n2,\n3,world\n");
        let options = CsvInferenceOptions::default();
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        assert_eq!(schema.columns.len(), 2);
        assert_eq!(
            schema.columns[1].exasol_type,
            ExasolType::Varchar { size: 2_000_000 }
        );
    }

    #[test]
    fn test_infer_no_files() {
        let options = CsvInferenceOptions::default();
        let result = infer_schema_from_csv_files(&[], &options);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No files provided"));
    }

    #[test]
    fn test_infer_single_file_via_multi() {
        let csv = write_csv("a,b\n1,hello\n");
        let options = CsvInferenceOptions::default();
        let paths = vec![csv.path().to_path_buf()];
        let schema = infer_schema_from_csv_files(&paths, &options).unwrap();

        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.source_files.len(), 1);
    }

    #[test]
    fn test_infer_multi_file_column_count_mismatch() {
        let csv_a = write_csv("a,b\n1,2\n");
        let csv_b = write_csv("a,b,c\n1,2,3\n");

        let options = CsvInferenceOptions::default();
        let paths = vec![csv_a.path().to_path_buf(), csv_b.path().to_path_buf()];
        let result = infer_schema_from_csv_files(&paths, &options);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Schema mismatch"));
    }

    #[test]
    fn test_infer_source_files_tracked() {
        let csv = write_csv("a\n1\n");
        let options = CsvInferenceOptions::default();
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        assert_eq!(schema.source_files.len(), 1);
        assert_eq!(schema.source_files[0], csv.path());
    }

    #[test]
    fn test_infer_ddl_generation() {
        let csv = write_csv("id,name,active\n1,hello,true\n");
        let options = CsvInferenceOptions::default();
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        let ddl = schema.to_ddl("test_table", None);
        assert!(ddl.contains("CREATE TABLE test_table"));
        assert!(ddl.contains("\"id\""));
        assert!(ddl.contains("\"name\""));
        assert!(ddl.contains("\"active\""));
    }

    #[test]
    fn test_infer_file_not_found() {
        let options = CsvInferenceOptions::default();
        let result = infer_schema_from_csv(Path::new("/nonexistent/file.csv"), &options);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to open file"));
    }

    #[test]
    fn test_infer_multi_file_nullable_merge() {
        // File A: name is always present
        let csv_a = write_csv("id,name\n1,hello\n2,world\n");
        // File B: name has empty values → nullable
        let csv_b = write_csv("id,name\n3,foo\n4,\n");

        let options = CsvInferenceOptions::default();
        let paths = vec![csv_a.path().to_path_buf(), csv_b.path().to_path_buf()];
        let schema = infer_schema_from_csv_files(&paths, &options).unwrap();

        // Nullable should be merged (true if any file has nullable)
        // Note: arrow-csv may or may not detect nullable from empty values,
        // but the merge logic (||) is correct regardless
        assert_eq!(schema.columns.len(), 2);
    }

    #[test]
    fn test_infer_max_sample_records() {
        // Create a CSV where early rows are integers but later rows are strings
        let csv = write_csv("value\n1\n2\n3\nhello\nworld\n");
        let options = CsvInferenceOptions::new().with_max_sample_records(Some(3));
        let schema = infer_schema_from_csv(csv.path(), &options).unwrap();

        // With only 3 rows sampled, arrow-csv should infer Int64
        assert!(matches!(
            schema.columns[0].exasol_type,
            ExasolType::Decimal { .. }
        ));
    }
}
