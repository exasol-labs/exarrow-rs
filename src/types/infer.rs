//! Schema inference and DDL generation for Parquet files.
//!
//! This module provides functionality to infer Exasol table schemas from Parquet files
//! and generate CREATE TABLE DDL statements automatically.

use std::path::{Path, PathBuf};

use arrow::datatypes::Schema;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use super::mapping::{ColumnNameMode, ExasolType, TypeMapper};
use crate::import::ImportError;

/// An inferred column definition for DDL generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferredColumn {
    /// Original column name from the source schema.
    pub original_name: String,
    /// Column name formatted for DDL (quoted or sanitized).
    pub ddl_name: String,
    /// Inferred Exasol data type.
    pub exasol_type: ExasolType,
    /// Whether the column allows NULL values.
    pub nullable: bool,
}

/// An inferred table schema from one or more source files.
#[derive(Debug, Clone)]
pub struct InferredTableSchema {
    /// Column definitions in order.
    pub columns: Vec<InferredColumn>,
    /// Source files used for inference (for error context).
    pub source_files: Vec<PathBuf>,
}

impl InferredTableSchema {
    /// Generate a CREATE TABLE DDL statement from this schema.
    ///
    /// # Arguments
    ///
    /// * `table_name` - The name of the table to create
    /// * `schema_name` - Optional schema/database name prefix
    ///
    /// # Returns
    ///
    /// A complete CREATE TABLE DDL statement as a string.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ddl = schema.to_ddl("my_table", Some("my_schema"));
    /// // CREATE TABLE my_schema.my_table (
    /// //     "id" DECIMAL(18,0),
    /// //     "name" VARCHAR(2000000),
    /// //     ...
    /// // );
    /// ```
    ///
    /// Note: Table and schema names are NOT quoted to match how IMPORT statements
    /// reference tables. In Exasol, unquoted identifiers are converted to uppercase.
    /// Column names are quoted/sanitized according to the `column_name_mode` used
    /// during schema inference.
    #[must_use]
    pub fn to_ddl(&self, table_name: &str, schema_name: Option<&str>) -> String {
        // Don't quote table/schema names - they should match IMPORT statement format
        // Exasol converts unquoted identifiers to uppercase
        let table_ref = if let Some(schema) = schema_name {
            format!("{schema}.{table_name}")
        } else {
            table_name.to_string()
        };

        let column_defs: Vec<String> = self
            .columns
            .iter()
            .map(|col| format!("    {} {}", col.ddl_name, col.exasol_type.to_ddl_type()))
            .collect();

        format!(
            "CREATE TABLE {} (\n{}\n);",
            table_ref,
            column_defs.join(",\n")
        )
    }
}

/// Sanitize a column name to be a valid unquoted Exasol identifier.
///
/// This function:
/// - Converts the name to uppercase
/// - Replaces invalid identifier characters with underscores
/// - Prefixes names starting with digits with an underscore
///
/// # Arguments
///
/// * `name` - The original column name
///
/// # Returns
///
/// A sanitized column name suitable for use as an unquoted identifier.
///
/// # Example
///
/// ```ignore
/// assert_eq!(sanitize_column_name("my Column"), "MY_COLUMN");
/// assert_eq!(sanitize_column_name("123abc"), "_123ABC");
/// ```
#[must_use]
pub fn sanitize_column_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());

    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' {
            result.push(c.to_ascii_uppercase());
        } else {
            result.push('_');
        }
        // If first character is a digit, we'll prefix with underscore later
        if i == 0 && c.is_ascii_digit() {
            // Mark for prefix - we handle this after the loop
        }
    }

    // Prefix with underscore if name starts with a digit
    if result.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        result = format!("_{result}");
    }

    // Handle empty result
    if result.is_empty() {
        return "_".to_string();
    }

    result
}

/// Quote a column name for use in Exasol DDL.
///
/// This function wraps the name in double quotes and escapes
/// any internal double quotes by doubling them.
///
/// # Arguments
///
/// * `name` - The original column name
///
/// # Returns
///
/// A quoted column name suitable for DDL statements.
///
/// # Example
///
/// ```ignore
/// assert_eq!(quote_column_name("my Column"), "\"my Column\"");
/// assert_eq!(quote_column_name("col\"name"), "\"col\"\"name\"");
/// ```
#[must_use]
pub fn quote_column_name(name: &str) -> String {
    let escaped = name.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// Quote an identifier (table or schema name) for Exasol DDL.
///
/// Same escaping rules as `quote_column_name`.
#[must_use]
pub fn quote_identifier(name: &str) -> String {
    quote_column_name(name)
}

/// Format a column name according to the specified mode.
#[must_use]
pub fn format_column_name(name: &str, mode: ColumnNameMode) -> String {
    match mode {
        ColumnNameMode::Quoted => quote_column_name(name),
        ColumnNameMode::Sanitize => sanitize_column_name(name),
    }
}

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

/// Widen two Exasol types to a common supertype.
///
/// Type widening rules:
/// - Identical types remain unchanged
/// - DECIMAL: max(precision), max(scale)
/// - VARCHAR: max(size)
/// - DECIMAL + DOUBLE -> DOUBLE
/// - Incompatible types -> VARCHAR(2000000) as fallback
///
/// # Arguments
///
/// * `a` - First type
/// * `b` - Second type
///
/// # Returns
///
/// A type that can represent values from both input types.
#[must_use]
pub fn widen_type(a: &ExasolType, b: &ExasolType) -> ExasolType {
    if a == b {
        return a.clone();
    }

    match (a, b) {
        // Decimal widening: max precision and scale
        (
            ExasolType::Decimal {
                precision: p1,
                scale: s1,
            },
            ExasolType::Decimal {
                precision: p2,
                scale: s2,
            },
        ) => {
            let max_precision = (*p1).max(*p2).min(36); // Cap at Exasol's max precision
            let max_scale = (*s1).max(*s2);
            ExasolType::Decimal {
                precision: max_precision,
                scale: max_scale,
            }
        }

        // VARCHAR widening: max size
        (ExasolType::Varchar { size: s1 }, ExasolType::Varchar { size: s2 }) => {
            ExasolType::Varchar {
                size: (*s1).max(*s2).min(2_000_000),
            }
        }

        // CHAR widening: max size
        (ExasolType::Char { size: s1 }, ExasolType::Char { size: s2 }) => ExasolType::Char {
            size: (*s1).max(*s2).min(2_000),
        },

        // CHAR + VARCHAR -> VARCHAR
        (ExasolType::Char { size: s1 }, ExasolType::Varchar { size: s2 })
        | (ExasolType::Varchar { size: s2 }, ExasolType::Char { size: s1 }) => {
            ExasolType::Varchar {
                size: (*s1).max(*s2).min(2_000_000),
            }
        }

        // Decimal + Double -> Double
        (ExasolType::Decimal { .. }, ExasolType::Double)
        | (ExasolType::Double, ExasolType::Decimal { .. }) => ExasolType::Double,

        // Timestamp widening: prefer with timezone
        (
            ExasolType::Timestamp {
                with_local_time_zone: tz1,
            },
            ExasolType::Timestamp {
                with_local_time_zone: tz2,
            },
        ) => ExasolType::Timestamp {
            with_local_time_zone: *tz1 || *tz2,
        },

        // Interval widening
        (
            ExasolType::IntervalDayToSecond { precision: p1 },
            ExasolType::IntervalDayToSecond { precision: p2 },
        ) => ExasolType::IntervalDayToSecond {
            precision: (*p1).max(*p2),
        },

        // Incompatible types: fall back to VARCHAR
        _ => ExasolType::Varchar { size: 2_000_000 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_column_name_simple() {
        assert_eq!(sanitize_column_name("name"), "NAME");
        assert_eq!(sanitize_column_name("MyColumn"), "MYCOLUMN");
    }

    #[test]
    fn test_sanitize_column_name_with_spaces() {
        assert_eq!(sanitize_column_name("my column"), "MY_COLUMN");
        assert_eq!(sanitize_column_name("first name"), "FIRST_NAME");
    }

    #[test]
    fn test_sanitize_column_name_special_chars() {
        assert_eq!(sanitize_column_name("col@#$%"), "COL____");
        assert_eq!(sanitize_column_name("a-b-c"), "A_B_C");
    }

    #[test]
    fn test_sanitize_column_name_starts_with_digit() {
        assert_eq!(sanitize_column_name("123abc"), "_123ABC");
        assert_eq!(sanitize_column_name("1st_column"), "_1ST_COLUMN");
    }

    #[test]
    fn test_sanitize_column_name_empty() {
        assert_eq!(sanitize_column_name(""), "_");
    }

    #[test]
    fn test_sanitize_column_name_all_special() {
        assert_eq!(sanitize_column_name("@#$"), "___");
    }

    #[test]
    fn test_quote_column_name_simple() {
        assert_eq!(quote_column_name("name"), "\"name\"");
        assert_eq!(quote_column_name("MyColumn"), "\"MyColumn\"");
    }

    #[test]
    fn test_quote_column_name_with_spaces() {
        assert_eq!(quote_column_name("my column"), "\"my column\"");
    }

    #[test]
    fn test_quote_column_name_with_quotes() {
        assert_eq!(quote_column_name("col\"name"), "\"col\"\"name\"");
        assert_eq!(quote_column_name("\"quoted\""), "\"\"\"quoted\"\"\"");
    }

    #[test]
    fn test_format_column_name_quoted_mode() {
        assert_eq!(
            format_column_name("my Column", ColumnNameMode::Quoted),
            "\"my Column\""
        );
    }

    #[test]
    fn test_format_column_name_sanitize_mode() {
        assert_eq!(
            format_column_name("my Column", ColumnNameMode::Sanitize),
            "MY_COLUMN"
        );
    }

    #[test]
    fn test_widen_type_identical() {
        let t = ExasolType::Boolean;
        assert_eq!(widen_type(&t, &t), ExasolType::Boolean);
    }

    #[test]
    fn test_widen_type_decimal() {
        let t1 = ExasolType::Decimal {
            precision: 10,
            scale: 2,
        };
        let t2 = ExasolType::Decimal {
            precision: 15,
            scale: 4,
        };
        assert_eq!(
            widen_type(&t1, &t2),
            ExasolType::Decimal {
                precision: 15,
                scale: 4
            }
        );
    }

    #[test]
    fn test_widen_type_varchar() {
        let t1 = ExasolType::Varchar { size: 100 };
        let t2 = ExasolType::Varchar { size: 500 };
        assert_eq!(widen_type(&t1, &t2), ExasolType::Varchar { size: 500 });
    }

    #[test]
    fn test_widen_type_char_varchar() {
        let t1 = ExasolType::Char { size: 50 };
        let t2 = ExasolType::Varchar { size: 100 };
        assert_eq!(widen_type(&t1, &t2), ExasolType::Varchar { size: 100 });
    }

    #[test]
    fn test_widen_type_decimal_double() {
        let t1 = ExasolType::Decimal {
            precision: 18,
            scale: 2,
        };
        let t2 = ExasolType::Double;
        assert_eq!(widen_type(&t1, &t2), ExasolType::Double);
        assert_eq!(widen_type(&t2, &t1), ExasolType::Double);
    }

    #[test]
    fn test_widen_type_incompatible() {
        let t1 = ExasolType::Boolean;
        let t2 = ExasolType::Date;
        assert_eq!(
            widen_type(&t1, &t2),
            ExasolType::Varchar { size: 2_000_000 }
        );
    }

    #[test]
    fn test_widen_type_timestamp() {
        let t1 = ExasolType::Timestamp {
            with_local_time_zone: false,
        };
        let t2 = ExasolType::Timestamp {
            with_local_time_zone: true,
        };
        assert_eq!(
            widen_type(&t1, &t2),
            ExasolType::Timestamp {
                with_local_time_zone: true
            }
        );
    }

    #[test]
    fn test_inferred_table_schema_to_ddl_basic() {
        let schema = InferredTableSchema {
            columns: vec![
                InferredColumn {
                    original_name: "id".to_string(),
                    ddl_name: "\"id\"".to_string(),
                    exasol_type: ExasolType::Decimal {
                        precision: 18,
                        scale: 0,
                    },
                    nullable: false,
                },
                InferredColumn {
                    original_name: "name".to_string(),
                    ddl_name: "\"name\"".to_string(),
                    exasol_type: ExasolType::Varchar { size: 100 },
                    nullable: true,
                },
            ],
            source_files: vec![],
        };

        let ddl = schema.to_ddl("my_table", None);
        // Table name is NOT quoted (matches IMPORT statement format)
        assert!(ddl.contains("CREATE TABLE my_table"));
        // Column names ARE quoted (based on ColumnNameMode)
        assert!(ddl.contains("\"id\" DECIMAL(18,0)"));
        assert!(ddl.contains("\"name\" VARCHAR(100)"));
    }

    #[test]
    fn test_inferred_table_schema_to_ddl_with_schema() {
        let schema = InferredTableSchema {
            columns: vec![InferredColumn {
                original_name: "col".to_string(),
                ddl_name: "\"col\"".to_string(),
                exasol_type: ExasolType::Boolean,
                nullable: false,
            }],
            source_files: vec![],
        };

        let ddl = schema.to_ddl("my_table", Some("my_schema"));
        // Table and schema names are NOT quoted (matches IMPORT statement format)
        assert!(ddl.contains("CREATE TABLE my_schema.my_table"));
    }

    // Note: Integration tests for infer_schema_from_parquet would require
    // creating actual Parquet files, which is better suited for the
    // integration test suite.
}
