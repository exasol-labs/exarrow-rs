//! Inferred table schemas and DDL generation.
//!
//! This module holds the inferred schema types, column name formatting, and type
//! widening that CSV and Parquet schema inference share.

use std::path::PathBuf;

use super::mapping::{ColumnNameMode, ExasolType};

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

    #[test]
    fn test_quote_identifier_escapes_embedded_quotes() {
        assert_eq!(quote_identifier("my_table"), "\"my_table\"");
        assert_eq!(quote_identifier("tab\"le"), "\"tab\"\"le\"");
    }

    #[test]
    fn test_widen_type_char_keeps_the_larger_size() {
        let widened = widen_type(
            &ExasolType::Char { size: 10 },
            &ExasolType::Char { size: 40 },
        );

        assert_eq!(widened, ExasolType::Char { size: 40 });
    }

    #[test]
    fn test_widen_type_char_caps_at_exasol_char_limit() {
        let widened = widen_type(
            &ExasolType::Char { size: 1_000 },
            &ExasolType::Char { size: 5_000 },
        );

        assert_eq!(widened, ExasolType::Char { size: 2_000 });
    }

    #[test]
    fn test_widen_type_varchar_before_char_still_yields_varchar() {
        let widened = widen_type(
            &ExasolType::Varchar { size: 100 },
            &ExasolType::Char { size: 250 },
        );

        assert_eq!(widened, ExasolType::Varchar { size: 250 });
    }

    #[test]
    fn test_widen_type_varchar_char_caps_at_exasol_varchar_limit() {
        let widened = widen_type(
            &ExasolType::Varchar { size: 3_000_000 },
            &ExasolType::Char { size: 10 },
        );

        assert_eq!(widened, ExasolType::Varchar { size: 2_000_000 });
    }

    #[test]
    fn test_widen_type_interval_day_to_second_keeps_the_larger_precision() {
        let widened = widen_type(
            &ExasolType::IntervalDayToSecond { precision: 3 },
            &ExasolType::IntervalDayToSecond { precision: 6 },
        );

        assert_eq!(widened, ExasolType::IntervalDayToSecond { precision: 6 });
    }
}
