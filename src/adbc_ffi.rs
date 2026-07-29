//! ADBC FFI-compatible trait implementations.
//!
//! This module provides wrapper types that implement the `adbc_core` traits,
//! enabling the exarrow-rs driver to be exported as a C-compatible shared library.
//!
//! The wrappers bridge the async Rust implementation to the synchronous ADBC C API
//! using a tokio runtime for async-to-sync conversion.
//!
//! # FFI Export
//!
//! When built with `--features ffi`, the library exports a C-compatible shared library
//! with the entry point `ExarrowDriverInit`. This can be loaded by ADBC driver managers.
//!
//! ## Building the FFI Library
//!
//!
//! ## Exported Symbol
//!
//! - **Function name**: `ExarrowDriverInit`
//! - **Library location**:
//!   - macOS: `target/release/libexarrow_rs.dylib`
//!   - Linux: `target/release/libexarrow_rs.so`
//!   - Windows: `target/release/exarrow_rs.dll`
//!
//! ## Connection URI Format
//!
//! The driver accepts URIs in the following format:
//!
//!
//! Examples:
//! - `exasol://sys:exasol@localhost:8563`
//! - `exasol://admin:secret@192.168.1.100:8563/my_schema`
//! - `exasol://sys:exasol@exasol-server.example.com:8563`
//!
//! ## Using with ADBC Driver Manager (Rust)
//!
//!
//! ## Using with ADBC Driver Manager (Python)
//!
//!
//! ## Supported ADBC Features
//!
//! | Feature | Status |
//! |---------|--------|
//! | `execute` (SELECT queries) | Supported |
//! | `execute_update` (INSERT/UPDATE/DELETE) | Supported |
//! | `get_info` | Supported |
//! | `get_table_types` | Supported |
//! | `commit` / `rollback` | Supported |
//! | `get_objects` | Supported |
//! | `get_table_schema` | Supported |
//! | `get_parameter_schema` | Supported |
//! | `bulk_ingestion` | Supported |
//! | Partitioned results | Not supported |
//! | Substrait plans | Not supported |
//!
//! # Running Integration Tests
//!
//! To test the FFI driver with the ADBC driver manager:
//!

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use adbc_core::error::{Error as AdbcError, Result as AdbcResult, Status as AdbcStatus};
use adbc_core::options::{
    InfoCode, ObjectDepth, OptionConnection, OptionDatabase, OptionStatement, OptionValue,
};
use adbc_core::{Optionable, PartitionedResult};
use arrow::array::{Array, RecordBatch, RecordBatchReader};
use arrow::compute::concat_batches;
use arrow::datatypes::Schema;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

use crate::adbc::Connection as ExaConnection;
use crate::error::{ExasolError, QueryError};
use crate::query::prepared::PreparedStatement;
use crate::query::Parameter;
use crate::transport::messages::DataType as TransportDataType;
use crate::types::{ExasolType, TypeMapper};

/// Exasol's maximum `VARCHAR` length, and the width assumed for an unsized
/// `VARCHAR`, `CLOB`, or `LONG VARCHAR`.
const MAX_VARCHAR_SIZE: usize = 2_000_000;

/// Precision and scale assumed for a `DECIMAL`/`NUMERIC` with no parameters,
/// and for the integer aliases Exasol reports as `DECIMAL`.
const DEFAULT_DECIMAL_PRECISION: u8 = 18;
const DEFAULT_DECIMAL_SCALE: i8 = 0;

/// Fractional-seconds precision assumed for a bare `INTERVAL DAY TO SECOND`.
const DEFAULT_INTERVAL_PRECISION: i64 = 3;

/// Byte width assumed for a `HASHTYPE` with no explicit size.
const DEFAULT_HASHTYPE_BYTES: i64 = 16;

/// Global tokio runtime for async-to-sync bridging.
///
/// Uses 2 worker threads so the I/O reactor remains available
/// even when one worker is parked in `block_on` during import.
fn get_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for ADBC FFI")
    })
}

/// Convert an ExasolError to an ADBC Error.
fn to_adbc_error(err: impl std::error::Error) -> AdbcError {
    AdbcError::with_message_and_status(err.to_string(), AdbcStatus::Internal)
}

/// Column metadata for GetObjects result: (schema, table) -> Vec<(col_name, ordinal, type_name)>.
type ColumnMetadataMap = std::collections::HashMap<(String, String), Vec<(String, i32, String)>>;

/// Quote an Exasol identifier with double quotes and escape embedded double quotes.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Build a fully-qualified, properly-quoted table name.
///
/// If `schema` is provided, it is quoted separately and prepended.
/// If `schema` is None but `table` contains a dot, the first dot is treated
/// as a schema/table separator so that `"SCHEMA.TABLE"` becomes `"SCHEMA"."TABLE"`
/// rather than a single quoted identifier containing a literal dot.
fn build_qualified_table_name(schema: Option<&str>, table: &str) -> String {
    if let Some(schema) = schema {
        format!("{}.{}", quote_identifier(schema), quote_identifier(table))
    } else if let Some((schema_part, table_part)) = table.split_once('.') {
        format!(
            "{}.{}",
            quote_identifier(schema_part),
            quote_identifier(table_part)
        )
    } else {
        quote_identifier(table)
    }
}

/// Generate a CREATE TABLE DDL statement from an Arrow Schema.
///
/// Maps each Arrow field to an Exasol type and respects nullability constraints.
/// The `table_name` should already be a properly quoted/qualified identifier.
pub(crate) fn generate_create_table_ddl(table_name: &str, schema: &Schema) -> AdbcResult<String> {
    let mut columns = Vec::new();

    for field in schema.fields() {
        let exasol_type = TypeMapper::arrow_to_exasol(field.data_type()).map_err(|e| {
            AdbcError::with_message_and_status(
                format!(
                    "Cannot map Arrow type {:?} for column '{}': {}",
                    field.data_type(),
                    field.name(),
                    e
                ),
                AdbcStatus::InvalidArguments,
            )
        })?;

        let ddl_type = exasol_type.to_ddl_type();
        let escaped_name = format!("\"{}\"", field.name().replace('"', "\"\""));

        let col_def = if !field.is_nullable() {
            format!("{} {} NOT NULL", escaped_name, ddl_type)
        } else {
            format!("{} {}", escaped_name, ddl_type)
        };
        columns.push(col_def);
    }

    Ok(format!(
        "CREATE TABLE {} ({})",
        table_name,
        columns.join(", ")
    ))
}

/// Parse an Exasol type name string from system views into an ExasolType.
///
/// Handles formats like "DECIMAL(18,0)", "VARCHAR(100)", "BOOLEAN", "TIMESTAMP",
/// "TIMESTAMP WITH LOCAL TIME ZONE", "DATE", "DOUBLE", etc.
fn parse_exasol_type_string(type_str: &str) -> AdbcResult<ExasolType> {
    let type_upper = type_str.trim().to_uppercase();

    parse_unparameterized_type(&type_upper)
        .or_else(|| parse_large_character_type(&type_upper))
        .or_else(|| parse_temporal_type(&type_upper))
        .or_else(|| parse_sized_type(&type_upper))
        .ok_or_else(|| {
            AdbcError::with_message_and_status(
                format!("Unknown Exasol type: {}", type_str),
                AdbcStatus::Internal,
            )
        })
}

/// Types spelled as a bare name, carrying no parenthesized parameter.
///
/// The integer aliases all collapse onto Exasol's own `DECIMAL(18,0)`
/// representation, which is what the server reports for them.
fn parse_unparameterized_type(type_upper: &str) -> Option<ExasolType> {
    match type_upper {
        "BOOLEAN" => Some(ExasolType::Boolean),
        "DATE" => Some(ExasolType::Date),
        "DOUBLE" | "DOUBLE PRECISION" | "FLOAT" | "REAL" => Some(ExasolType::Double),
        "BIGINT" | "INT" | "INTEGER" | "SMALLINT" | "TINYINT" => Some(ExasolType::Decimal {
            precision: DEFAULT_DECIMAL_PRECISION,
            scale: DEFAULT_DECIMAL_SCALE,
        }),
        _ => None,
    }
}

/// `CLOB` and its `LONG VARCHAR` alias.
///
/// Both denote a maximum-size VARCHAR, so any length in parentheses is
/// deliberately ignored rather than narrowing the type.
fn parse_large_character_type(type_upper: &str) -> Option<ExasolType> {
    if type_upper == "CLOB" || type_upper.starts_with("LONG VARCHAR") {
        return Some(ExasolType::Varchar {
            size: MAX_VARCHAR_SIZE,
        });
    }
    None
}

/// The temporal families, whose names are prefixes followed by an optional
/// precision and, for timestamps, an optional time-zone qualifier.
fn parse_temporal_type(type_upper: &str) -> Option<ExasolType> {
    if type_upper.starts_with("TIMESTAMP") {
        return Some(ExasolType::Timestamp {
            with_local_time_zone: type_upper.ends_with("WITH LOCAL TIME ZONE"),
        });
    }
    if type_upper.starts_with("INTERVAL YEAR") && type_upper.contains("TO MONTH") {
        return Some(ExasolType::IntervalYearToMonth);
    }
    if type_upper.starts_with("INTERVAL DAY") && type_upper.contains("TO SECOND") {
        return Some(ExasolType::IntervalDayToSecond {
            precision: extract_last_param(type_upper).unwrap_or(DEFAULT_INTERVAL_PRECISION) as u8,
        });
    }
    None
}

/// The families whose size, precision, or SRID rides in parentheses.
///
/// `CHAR` is tested before `VARCHAR` and excludes `CHAR VARYING`, which is an
/// alias for `VARCHAR` rather than a fixed-width `CHAR`.
fn parse_sized_type(type_upper: &str) -> Option<ExasolType> {
    if type_upper.starts_with("GEOMETRY") {
        return Some(ExasolType::Geometry {
            srid: extract_single_param(type_upper).map(|srid| srid as i32),
        });
    }
    if type_upper.starts_with("HASHTYPE") {
        return Some(ExasolType::Hashtype {
            byte_size: extract_single_param(type_upper).unwrap_or(DEFAULT_HASHTYPE_BYTES) as usize,
        });
    }
    if type_upper.starts_with("CHAR") && !type_upper.starts_with("CHAR VARYING") {
        return Some(ExasolType::Char {
            size: extract_single_param(type_upper).unwrap_or(1) as usize,
        });
    }
    if type_upper.starts_with("VARCHAR") || type_upper.starts_with("CHAR VARYING") {
        return Some(ExasolType::Varchar {
            size: extract_single_param(type_upper).unwrap_or(MAX_VARCHAR_SIZE as i64) as usize,
        });
    }
    if type_upper.starts_with("DECIMAL") || type_upper.starts_with("NUMERIC") {
        let (precision, scale) = extract_two_params(type_upper).unwrap_or((
            DEFAULT_DECIMAL_PRECISION as i64,
            DEFAULT_DECIMAL_SCALE as i64,
        ));
        return Some(ExasolType::Decimal {
            precision: precision as u8,
            scale: scale as i8,
        });
    }
    None
}

fn extract_single_param(s: &str) -> Option<i64> {
    let start = s.find('(')?;
    let end = s.find(')')?;
    s[start + 1..end].trim().parse().ok()
}

fn extract_last_param(s: &str) -> Option<i64> {
    let start = s.rfind('(')?;
    let end = s.rfind(')')?;
    if start < end {
        s[start + 1..end].trim().parse().ok()
    } else {
        None
    }
}

fn extract_two_params(s: &str) -> Option<(i64, i64)> {
    let start = s.find('(')?;
    let end = s.find(')')?;
    let inner = &s[start + 1..end];
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() == 2 {
        let a = parts[0].trim().parse().ok()?;
        let b = parts[1].trim().parse().ok()?;
        Some((a, b))
    } else if parts.len() == 1 {
        let a = parts[0].trim().parse().ok()?;
        Some((a, 0))
    } else {
        None
    }
}

/// Convert a transport DataType to an ExasolType for Arrow conversion.
fn transport_datatype_to_exasol(dt: &TransportDataType) -> AdbcResult<ExasolType> {
    let type_upper = dt.type_name.to_uppercase();

    match type_upper.as_str() {
        "BOOLEAN" => Ok(ExasolType::Boolean),
        "DATE" => Ok(ExasolType::Date),
        "DOUBLE" | "DOUBLE PRECISION" => Ok(ExasolType::Double),
        "TIMESTAMP" => Ok(ExasolType::Timestamp {
            with_local_time_zone: dt.with_local_time_zone.unwrap_or(false),
        }),
        "TIMESTAMP WITH LOCAL TIME ZONE" => Ok(ExasolType::Timestamp {
            with_local_time_zone: true,
        }),
        "CHAR" => Ok(ExasolType::Char {
            size: dt.size.unwrap_or(1) as usize,
        }),
        "VARCHAR" => Ok(ExasolType::Varchar {
            size: dt.size.unwrap_or(2000000) as usize,
        }),
        "DECIMAL" => {
            let precision = dt.precision.unwrap_or(18) as u8;
            let scale = dt.scale.unwrap_or(0) as i8;
            Ok(ExasolType::Decimal { precision, scale })
        }
        "GEOMETRY" => Ok(ExasolType::Geometry { srid: None }),
        "HASHTYPE" => Ok(ExasolType::Hashtype {
            byte_size: dt.size.unwrap_or(16) as usize,
        }),
        "INTERVAL DAY TO SECOND" => Ok(ExasolType::IntervalDayToSecond {
            precision: dt.fraction.unwrap_or(3) as u8,
        }),
        "INTERVAL YEAR TO MONTH" => Ok(ExasolType::IntervalYearToMonth),
        _ => Err(AdbcError::with_message_and_status(
            format!("Unknown transport type: {}", dt.type_name),
            AdbcStatus::Internal,
        )),
    }
}

/// Build the Arrow schema for the ADBC GetObjects result.
/// A nullable `List` of the given struct layout, the shape every nesting level
/// of the `GetObjects` result uses.
fn list_of_structs(fields: arrow::datatypes::Fields) -> arrow::datatypes::DataType {
    use arrow::datatypes::{DataType, Field};
    DataType::List(Arc::new(Field::new("item", DataType::Struct(fields), true)))
}

/// The `table_columns` struct layout of the `GetObjects` result.
///
/// This function and its three siblings are the single owner of the result's
/// nested shape: both the schema handed to the caller and the Arrow builders
/// that populate it derive from these, so the two cannot drift apart.
fn get_objects_column_fields() -> arrow::datatypes::Fields {
    use arrow::datatypes::{DataType, Field, Fields};
    Fields::from(vec![
        Field::new("column_name", DataType::Utf8, false),
        Field::new("ordinal_position", DataType::Int32, false),
        Field::new("xdbc_type_name", DataType::Utf8, true),
    ])
}

/// The `table_constraints` struct layout. Exasol reports no constraints here,
/// so the list is always emitted as null.
fn get_objects_constraint_fields() -> arrow::datatypes::Fields {
    use arrow::datatypes::{DataType, Field, Fields};
    Fields::from(vec![
        Field::new("constraint_name", DataType::Utf8, true),
        Field::new("constraint_type", DataType::Utf8, false),
    ])
}

/// The `db_schema_tables` struct layout of the `GetObjects` result.
fn get_objects_table_fields() -> arrow::datatypes::Fields {
    use arrow::datatypes::{DataType, Field, Fields};
    Fields::from(vec![
        Field::new("table_name", DataType::Utf8, false),
        Field::new("table_type", DataType::Utf8, false),
        Field::new(
            "table_columns",
            list_of_structs(get_objects_column_fields()),
            true,
        ),
        Field::new(
            "table_constraints",
            list_of_structs(get_objects_constraint_fields()),
            true,
        ),
    ])
}

/// The `catalog_db_schemas` struct layout of the `GetObjects` result.
fn get_objects_schema_fields() -> arrow::datatypes::Fields {
    use arrow::datatypes::{DataType, Field, Fields};
    Fields::from(vec![
        Field::new("db_schema_name", DataType::Utf8, true),
        Field::new(
            "db_schema_tables",
            list_of_structs(get_objects_table_fields()),
            true,
        ),
    ])
}

fn build_get_objects_schema() -> Schema {
    use arrow::datatypes::{DataType, Field};

    Schema::new(vec![
        Field::new("catalog_name", DataType::Utf8, false),
        Field::new(
            "catalog_db_schemas",
            list_of_structs(get_objects_schema_fields()),
            true,
        ),
    ])
}

/// Tables discovered by `GetObjects`, grouped by owning schema:
/// schema -> [(table name, table type)].
type TablesBySchema = std::collections::HashMap<String, Vec<(String, String)>>;

/// Statement option naming the table a bulk ingest writes into.
const INGEST_TARGET_TABLE_OPTION: &str = "adbc.ingest.target_table";

/// Bind row `row_idx` of `batch` as the statement's positional parameters.
fn bind_row_as_parameters(
    prepared: &mut PreparedStatement,
    batch: &RecordBatch,
    row_idx: usize,
) -> AdbcResult<()> {
    for col_idx in 0..batch.num_columns() {
        let param = arrow_value_to_parameter(batch.column(col_idx).as_ref(), row_idx)?;
        prepared.bind(col_idx, param).map_err(to_adbc_error)?;
    }
    Ok(())
}

/// Wrap `batches` in a reader, taking the schema from the first batch and
/// falling back to an empty schema when there are none.
fn reader_over_batches(batches: Vec<RecordBatch>) -> Box<dyn RecordBatchReader + Send> {
    let schema = batches
        .first()
        .map_or_else(|| Arc::new(Schema::empty()), |batch| batch.schema());
    Box::new(VecRecordBatchReader::new(schema, batches))
}

/// Require a string option value, reporting `requirement` when it is not one.
fn require_string_option(value: OptionValue, requirement: &str) -> AdbcResult<String> {
    match value {
        OptionValue::String(text) => Ok(text),
        _ => Err(AdbcError::with_message_and_status(
            requirement,
            AdbcStatus::InvalidArguments,
        )),
    }
}

/// Whether a catalog filter names Exasol's single catalog, `EXA`.
///
/// An absent filter matches, matching ADBC's "no filter" semantics.
fn catalog_is_exasol(catalog: Option<&str>) -> bool {
    catalog.is_none_or(|name| name.eq_ignore_ascii_case("EXA"))
}

/// Run `sql` on the connection from this synchronous FFI thread and collect
/// every result batch.
fn query_batches(conn: &Arc<Mutex<ExaConnection>>, sql: &str) -> AdbcResult<Vec<RecordBatch>> {
    get_runtime()
        .block_on(async {
            let mut conn = conn.lock().await;
            conn.query(sql).await
        })
        .map_err(to_adbc_error)
}

/// A `LIKE` predicate on `column`, with single quotes in `pattern` escaped so a
/// quote in a caller-supplied filter cannot terminate the literal.
fn like_condition(column: &str, pattern: &str) -> String {
    format!("{} LIKE '{}'", column, pattern.replace('\'', "''"))
}

/// Append a `WHERE` clause joining `conditions` with `AND`, or nothing at all
/// when no condition applies.
fn append_where_clause(sql: &mut String, conditions: &[String]) {
    if conditions.is_empty() {
        return;
    }
    sql.push_str(" WHERE ");
    sql.push_str(&conditions.join(" AND "));
}

/// The schema-listing query behind `GetObjects`.
fn get_objects_schemas_query(db_schema: Option<&str>) -> String {
    let mut sql = "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS".to_string();
    let conditions: Vec<String> = db_schema
        .map(|pattern| vec![like_condition("SCHEMA_NAME", pattern)])
        .unwrap_or_default();
    append_where_clause(&mut sql, &conditions);
    sql.push_str(" ORDER BY SCHEMA_NAME");
    sql
}

/// The table-listing query behind `GetObjects`.
///
/// Tables and views are always both in scope; an explicit `table_type` filter
/// narrows that further rather than replacing it.
fn get_objects_tables_query(
    db_schema: Option<&str>,
    table_name: Option<&str>,
    table_type: Option<&[&str]>,
) -> String {
    let mut sql = "SELECT OBJECT_NAME, OBJECT_TYPE, ROOT_NAME FROM SYS.EXA_ALL_OBJECTS".to_string();
    let mut conditions = vec!["OBJECT_TYPE IN ('TABLE', 'VIEW')".to_string()];
    if let Some(pattern) = db_schema {
        conditions.push(like_condition("ROOT_NAME", pattern));
    }
    if let Some(pattern) = table_name {
        conditions.push(like_condition("OBJECT_NAME", pattern));
    }
    if let Some(types) = table_type {
        let quoted: Vec<String> = types
            .iter()
            .map(|t| format!("'{}'", t.replace('\'', "''")))
            .collect();
        conditions.push(format!("OBJECT_TYPE IN ({})", quoted.join(",")));
    }
    append_where_clause(&mut sql, &conditions);
    sql.push_str(" ORDER BY ROOT_NAME, OBJECT_NAME");
    sql
}

/// The column-listing query behind `GetObjects`.
fn get_objects_columns_query(
    db_schema: Option<&str>,
    table_name: Option<&str>,
    column_name: Option<&str>,
) -> String {
    let mut sql = "SELECT COLUMN_NAME, COLUMN_ORDINAL_POSITION, COLUMN_TYPE, COLUMN_SCHEMA, COLUMN_TABLE FROM SYS.EXA_ALL_COLUMNS".to_string();
    let mut conditions = Vec::new();
    if let Some(pattern) = db_schema {
        conditions.push(like_condition("COLUMN_SCHEMA", pattern));
    }
    if let Some(pattern) = table_name {
        conditions.push(like_condition("COLUMN_TABLE", pattern));
    }
    if let Some(pattern) = column_name {
        conditions.push(like_condition("COLUMN_NAME", pattern));
    }
    append_where_clause(&mut sql, &conditions);
    sql.push_str(" ORDER BY COLUMN_SCHEMA, COLUMN_TABLE, COLUMN_ORDINAL_POSITION");
    sql
}

/// The column-metadata query behind `GetTableSchema`.
///
/// Unlike `GetObjects` this matches names exactly rather than by pattern, since
/// the caller names one specific table.
fn table_schema_query(db_schema: Option<&str>, table_name: &str) -> String {
    let mut sql = format!(
        "SELECT COLUMN_NAME, COLUMN_TYPE, COLUMN_MAXSIZE, COLUMN_NUM_PREC, COLUMN_NUM_SCALE, COLUMN_IS_NULLABLE FROM SYS.EXA_ALL_COLUMNS WHERE COLUMN_TABLE = '{}'",
        table_name.replace('\'', "''")
    );
    if let Some(schema) = db_schema {
        sql.push_str(&format!(
            " AND COLUMN_SCHEMA = '{}'",
            schema.replace('\'', "''")
        ));
    }
    sql.push_str(" ORDER BY COLUMN_ORDINAL_POSITION");
    sql
}

/// Translate the column-metadata result into Arrow fields.
///
/// An empty result means the table does not exist; the caller turns that into a
/// not-found error, since an existing table always has at least one column.
fn table_schema_fields_from(batches: &[RecordBatch]) -> AdbcResult<Vec<arrow::datatypes::Field>> {
    use arrow::datatypes::Field;

    let mut fields = Vec::new();
    for batch in batches {
        let names = string_column(batch, 0, "COLUMN_NAME")?;
        let types = string_column(batch, 1, "COLUMN_TYPE")?;
        let nullable_flags = batch.column(5);

        for row in 0..batch.num_rows() {
            let nullable = get_nullable_value(nullable_flags.as_ref(), row);
            let exasol_type = parse_exasol_type_string(types.value(row))?;
            let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, nullable).map_err(|e| {
                AdbcError::with_message_and_status(e.to_string(), AdbcStatus::Internal)
            })?;
            fields.push(Field::new(names.value(row), arrow_type, nullable));
        }
    }
    Ok(fields)
}

/// Borrow column `index` of `batch` as a string array.
///
/// `field` names the column in the error, so a system view that unexpectedly
/// reports a non-text column says which one.
fn string_column<'a>(
    batch: &'a RecordBatch,
    index: usize,
    field: &str,
) -> AdbcResult<&'a arrow::array::StringArray> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .ok_or_else(|| {
            AdbcError::with_message_and_status(
                format!("Expected string column for {}", field),
                AdbcStatus::Internal,
            )
        })
}

/// Collect the non-null schema names from the schema-listing result.
fn schema_names_from(batches: &[RecordBatch]) -> AdbcResult<Vec<String>> {
    let mut names = Vec::new();
    for batch in batches {
        let column = string_column(batch, 0, "SCHEMA_NAME")?;
        for row in 0..column.len() {
            if !column.is_null(row) {
                names.push(column.value(row).to_string());
            }
        }
    }
    Ok(names)
}

/// Group the table-listing result by owning schema.
fn tables_by_schema_from(batches: &[RecordBatch]) -> AdbcResult<TablesBySchema> {
    let mut tables = TablesBySchema::new();
    for batch in batches {
        let names = string_column(batch, 0, "OBJECT_NAME")?;
        let types = string_column(batch, 1, "OBJECT_TYPE")?;
        let schemas = string_column(batch, 2, "ROOT_NAME")?;
        for row in 0..batch.num_rows() {
            tables
                .entry(schemas.value(row).to_string())
                .or_default()
                .push((names.value(row).to_string(), types.value(row).to_string()));
        }
    }
    Ok(tables)
}

/// Group the column-listing result by owning (schema, table).
///
/// `COLUMN_ORDINAL_POSITION` arrives as any of several numeric types; a value
/// that cannot be read falls back to the row's own 1-based position.
fn columns_by_table_from(batches: &[RecordBatch]) -> AdbcResult<ColumnMetadataMap> {
    let mut columns = ColumnMetadataMap::new();
    for batch in batches {
        let names = string_column(batch, 0, "COLUMN_NAME")?;
        let ordinals = batch.column(1);
        let types = string_column(batch, 2, "COLUMN_TYPE")?;
        let schemas = string_column(batch, 3, "COLUMN_SCHEMA")?;
        let tables = string_column(batch, 4, "COLUMN_TABLE")?;
        for row in 0..batch.num_rows() {
            let ordinal = get_int_value(ordinals.as_ref(), row).unwrap_or(row as i32 + 1);
            columns
                .entry((
                    schemas.value(row).to_string(),
                    tables.value(row).to_string(),
                ))
                .or_default()
                .push((
                    names.value(row).to_string(),
                    ordinal,
                    types.value(row).to_string(),
                ));
        }
    }
    Ok(columns)
}

/// Which nesting levels of the `GetObjects` result the requested depth fills in.
///
/// ADBC's depth is cumulative — asking for columns also asks for the tables and
/// schemas that contain them — so this resolves the one enum into the three
/// independent questions the builder actually asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObjectDepthLevels {
    schemas: bool,
    tables: bool,
    columns: bool,
}

impl From<ObjectDepth> for ObjectDepthLevels {
    fn from(depth: ObjectDepth) -> Self {
        Self {
            schemas: matches!(
                depth,
                ObjectDepth::Schemas
                    | ObjectDepth::Tables
                    | ObjectDepth::Columns
                    | ObjectDepth::All
            ),
            tables: matches!(
                depth,
                ObjectDepth::Tables | ObjectDepth::Columns | ObjectDepth::All
            ),
            columns: matches!(depth, ObjectDepth::Columns | ObjectDepth::All),
        }
    }
}

/// Report a missing or mistyped Arrow builder for `field`.
fn builder_error(field: &str) -> AdbcError {
    AdbcError::with_message_and_status(
        format!("Failed to access Arrow builder for field '{field}'"),
        AdbcStatus::Internal,
    )
}

/// Borrow the nested struct builder a `ListBuilder` appends into.
fn list_struct_builder<'a>(
    list: &'a mut arrow::array::builder::ListBuilder<Box<dyn arrow::array::builder::ArrayBuilder>>,
    field: &str,
) -> AdbcResult<&'a mut arrow::array::builder::StructBuilder> {
    list.values()
        .as_any_mut()
        .downcast_mut::<arrow::array::builder::StructBuilder>()
        .ok_or_else(|| builder_error(field))
}

/// Append one string field of a struct builder.
fn append_string_field(
    target: &mut arrow::array::builder::StructBuilder,
    index: usize,
    field: &str,
    value: &str,
) -> AdbcResult<()> {
    target
        .field_builder::<arrow::array::builder::StringBuilder>(index)
        .ok_or_else(|| builder_error(field))?
        .append_value(value);
    Ok(())
}

/// Borrow one nested list field of a struct builder.
fn list_field<'a>(
    target: &'a mut arrow::array::builder::StructBuilder,
    index: usize,
    field: &str,
) -> AdbcResult<
    &'a mut arrow::array::builder::ListBuilder<Box<dyn arrow::array::builder::ArrayBuilder>>,
> {
    target
        .field_builder::<arrow::array::builder::ListBuilder<Box<dyn arrow::array::builder::ArrayBuilder>>>(
            index,
        )
        .ok_or_else(|| builder_error(field))
}

/// Append every column of one table into that table's `table_columns` list.
fn append_table_columns(
    columns_list: &mut arrow::array::builder::ListBuilder<
        Box<dyn arrow::array::builder::ArrayBuilder>,
    >,
    table_columns: &[(String, i32, String)],
) -> AdbcResult<()> {
    for (column_name, ordinal, type_name) in table_columns {
        let entry = list_struct_builder(columns_list, "table_columns downcast")?;
        append_string_field(entry, 0, "column_name", column_name)?;
        entry
            .field_builder::<arrow::array::builder::Int32Builder>(1)
            .ok_or_else(|| builder_error("ordinal_position"))?
            .append_value(*ordinal);
        append_string_field(entry, 2, "xdbc_type_name", type_name)?;
        entry.append(true);
    }
    Ok(())
}

/// Append one table entry, including its columns when the depth asks for them.
///
/// Exasol reports no constraints through this API, so `table_constraints` is
/// always appended as null.
fn append_table_entry(
    tables_list: &mut arrow::array::builder::ListBuilder<
        Box<dyn arrow::array::builder::ArrayBuilder>,
    >,
    table_name: &str,
    table_type: &str,
    table_columns: Option<&Vec<(String, i32, String)>>,
    include_columns: bool,
) -> AdbcResult<()> {
    let entry = list_struct_builder(tables_list, "db_schema_tables downcast")?;
    append_string_field(entry, 0, "table_name", table_name)?;
    append_string_field(entry, 1, "table_type", table_type)?;

    let columns_list = list_field(entry, 2, "table_columns")?;
    if include_columns {
        append_table_columns(
            columns_list,
            table_columns.map_or(&[][..], |cols| &cols[..]),
        )?;
        columns_list.append(true);
    } else {
        columns_list.append(false);
    }

    list_field(entry, 3, "table_constraints")?.append(false);
    entry.append(true);
    Ok(())
}

/// Append one schema entry, including its tables when the depth asks for them.
fn append_schema_entry(
    schema_list: &mut arrow::array::builder::ListBuilder<arrow::array::builder::StructBuilder>,
    schema_name: &str,
    tables: &TablesBySchema,
    columns: &ColumnMetadataMap,
    levels: ObjectDepthLevels,
) -> AdbcResult<()> {
    let entry = schema_list.values();
    entry
        .field_builder::<arrow::array::builder::StringBuilder>(0)
        .ok_or_else(|| builder_error("db_schema_name"))?
        .append_value(schema_name);

    let tables_list = list_field(entry, 1, "db_schema_tables")?;
    if !levels.tables {
        tables_list.append(false);
        entry.append(true);
        return Ok(());
    }

    for (table_name, table_type) in tables.get(schema_name).map_or(&[][..], |t| &t[..]) {
        let table_columns = columns.get(&(schema_name.to_string(), table_name.clone()));
        append_table_entry(
            tables_list,
            table_name,
            table_type,
            table_columns,
            levels.columns,
        )?;
    }
    tables_list.append(true);
    entry.append(true);
    Ok(())
}

/// Assemble the single-row `GetObjects` batch from the gathered metadata.
///
/// Exasol exposes exactly one catalog, `EXA`, so the batch always holds one row
/// whose nested schema list is null when the depth stops short of schemas.
fn build_get_objects_batch(
    schemas: &[String],
    tables: &TablesBySchema,
    columns: &ColumnMetadataMap,
    levels: ObjectDepthLevels,
) -> AdbcResult<RecordBatch> {
    use arrow::array::builder::{ListBuilder, StringBuilder, StructBuilder};

    let mut catalog_name_builder = StringBuilder::new();
    catalog_name_builder.append_value("EXA");

    let mut schema_list_builder =
        ListBuilder::new(StructBuilder::from_fields(get_objects_schema_fields(), 0));

    if levels.schemas {
        for schema_name in schemas {
            append_schema_entry(
                &mut schema_list_builder,
                schema_name,
                tables,
                columns,
                levels,
            )?;
        }
        schema_list_builder.append(true);
    } else {
        schema_list_builder.append(false);
    }

    RecordBatch::try_new(
        Arc::new(build_get_objects_schema()),
        vec![
            Arc::new(catalog_name_builder.finish()),
            Arc::new(schema_list_builder.finish()),
        ],
    )
    .map_err(|e| AdbcError::with_message_and_status(e.to_string(), AdbcStatus::Internal))
}

/// Read the value at `idx` from a primitive Arrow array, mapping both a NULL
/// and a column that is not of the requested type to `None`.
fn primitive_value_at<T: arrow::datatypes::ArrowPrimitiveType>(
    array: &dyn arrow::array::Array,
    idx: usize,
) -> Option<T::Native> {
    let typed = array
        .as_any()
        .downcast_ref::<arrow::array::PrimitiveArray<T>>()?;
    if typed.is_null(idx) {
        return None;
    }
    Some(typed.value(idx))
}

/// Extract an i32 value from an Arrow array at the given index.
/// Handles various numeric array types that Exasol might return.
fn get_int_value(array: &dyn arrow::array::Array, idx: usize) -> Option<i32> {
    use arrow::datatypes::{DataType, Decimal128Type, Float64Type, Int32Type, Int64Type};

    match array.data_type() {
        DataType::Int32 => primitive_value_at::<Int32Type>(array, idx),
        DataType::Int64 => primitive_value_at::<Int64Type>(array, idx).map(|value| value as i32),
        DataType::Float64 => {
            primitive_value_at::<Float64Type>(array, idx).map(|value| value as i32)
        }
        DataType::Decimal128(_, _) => {
            primitive_value_at::<Decimal128Type>(array, idx).map(|value| value as i32)
        }
        _ => None,
    }
}

/// Extract a nullable boolean from a column value.
/// Exasol returns COLUMN_IS_NULLABLE as a BOOLEAN or string.
///
/// Nullable is the safe default: a NULL flag, an unexpected column type, and a
/// text flag that spells no affirmative all report nullable, because wrongly
/// emitting `NOT NULL` in generated DDL rejects data the source accepted.
fn get_nullable_value(array: &dyn arrow::array::Array, idx: usize) -> bool {
    use arrow::array::{BooleanArray, StringArray};
    use arrow::datatypes::DataType;

    match array.data_type() {
        DataType::Boolean => array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .filter(|flags| !flags.is_null(idx))
            .map(|flags| flags.value(idx))
            .unwrap_or(true),
        DataType::Utf8 => array
            .as_any()
            .downcast_ref::<StringArray>()
            .filter(|flags| !flags.is_null(idx))
            .map(|flags| spells_affirmative(flags.value(idx)))
            .unwrap_or(true),
        _ => true,
    }
}

/// Whether a system-view text flag spells an affirmative.
fn spells_affirmative(flag: &str) -> bool {
    matches!(flag.to_uppercase().as_str(), "TRUE" | "YES" | "1")
}

// -----------------------------------------------------------------------------
// FFI Driver
// -----------------------------------------------------------------------------

/// FFI-compatible ADBC Driver wrapper.
///
/// This wraps the internal `Driver` type to implement `adbc_core::Driver`.
/// The driver is loaded via the `ExarrowDriverInit` entry point.
///
/// # Example
///
#[derive(Debug, Default)]
pub struct FfiDriver;

impl adbc_core::Driver for FfiDriver {
    type DatabaseType = FfiDatabase;

    fn new_database(&mut self) -> AdbcResult<Self::DatabaseType> {
        Ok(FfiDatabase::new())
    }

    fn new_database_with_opts(
        &mut self,
        opts: impl IntoIterator<Item = (OptionDatabase, OptionValue)>,
    ) -> AdbcResult<Self::DatabaseType> {
        let mut db = FfiDatabase::new();
        for (key, value) in opts {
            db.set_option(key, value)?;
        }
        Ok(db)
    }
}

// -----------------------------------------------------------------------------
// FFI Database
// -----------------------------------------------------------------------------

/// FFI-compatible ADBC Database wrapper.
///
/// This stores connection options and creates connections on demand.
/// The primary option is `OptionDatabase::Uri` which should contain
/// the Exasol connection string.
///
/// # Connection URI Format
///
///
/// # Example
///
pub struct FfiDatabase {
    /// URI for the connection (exasol://user:pass@host:port/schema)
    uri: Option<String>,
    /// Username override
    username: Option<String>,
    /// Password override
    password: Option<String>,
    /// Custom options
    options: std::collections::HashMap<String, OptionValue>,
}

impl FfiDatabase {
    fn new() -> Self {
        Self {
            uri: None,
            username: None,
            password: None,
            options: std::collections::HashMap::new(),
        }
    }

    /// Build connection parameters from stored options.
    /// This rebuilds the URI with any override credentials.
    fn build_connection_uri(&self) -> AdbcResult<String> {
        let uri = self.uri.as_ref().ok_or_else(|| {
            AdbcError::with_message_and_status(
                "Database URI not set. Set adbc.exasol.uri option.",
                AdbcStatus::InvalidState,
            )
        })?;

        // If no overrides, use URI as-is
        if self.username.is_none() && self.password.is_none() {
            return Ok(uri.clone());
        }

        // Parse the URI and rebuild with overrides
        // URI format: exasol://[user[:pass]@]host[:port][/schema][?params]
        let uri_str = uri.as_str();
        if !uri_str.starts_with("exasol://") {
            return Err(AdbcError::with_message_and_status(
                "URI must start with exasol://",
                AdbcStatus::InvalidArguments,
            ));
        }

        let after_scheme = &uri_str[9..]; // Skip "exasol://"

        // Find @, /, ? positions
        let at_pos = after_scheme.rfind('@');
        let (host_part, orig_user, orig_pass) = if let Some(at) = at_pos {
            let auth_part = &after_scheme[..at];
            let host_part = &after_scheme[at + 1..];
            let (user, pass) = if let Some(colon) = auth_part.find(':') {
                (&auth_part[..colon], Some(&auth_part[colon + 1..]))
            } else {
                (auth_part, None)
            };
            (host_part, Some(user), pass)
        } else {
            (after_scheme, None, None)
        };

        // Use overrides or originals
        let user = self.username.as_deref().or(orig_user).unwrap_or("sys");
        let pass = self.password.as_deref().or(orig_pass).unwrap_or("");

        // Rebuild URI
        if pass.is_empty() {
            Ok(format!("exasol://{}@{}", user, host_part))
        } else {
            Ok(format!("exasol://{}:{}@{}", user, pass, host_part))
        }
    }
}

impl Optionable for FfiDatabase {
    type Option = OptionDatabase;

    fn set_option(&mut self, key: Self::Option, value: OptionValue) -> AdbcResult<()> {
        match key {
            OptionDatabase::Uri => {
                if let OptionValue::String(s) = value {
                    self.uri = Some(s);
                } else {
                    return Err(AdbcError::with_message_and_status(
                        "URI must be a string",
                        AdbcStatus::InvalidArguments,
                    ));
                }
            }
            OptionDatabase::Username => {
                if let OptionValue::String(s) = value {
                    self.username = Some(s);
                } else {
                    return Err(AdbcError::with_message_and_status(
                        "Username must be a string",
                        AdbcStatus::InvalidArguments,
                    ));
                }
            }
            OptionDatabase::Password => {
                if let OptionValue::String(s) = value {
                    self.password = Some(s);
                } else {
                    return Err(AdbcError::with_message_and_status(
                        "Password must be a string",
                        AdbcStatus::InvalidArguments,
                    ));
                }
            }
            OptionDatabase::Other(key) => {
                self.options.insert(key, value);
            }
            _ => {
                // Handle any future additions to the enum
                return Err(AdbcError::with_message_and_status(
                    "Unsupported database option",
                    AdbcStatus::NotImplemented,
                ));
            }
        }
        Ok(())
    }

    fn get_option_string(&self, key: Self::Option) -> AdbcResult<String> {
        match key {
            OptionDatabase::Uri => self.uri.clone().ok_or_else(|| {
                AdbcError::with_message_and_status("URI not set", AdbcStatus::NotFound)
            }),
            OptionDatabase::Username => self.username.clone().ok_or_else(|| {
                AdbcError::with_message_and_status("Username not set", AdbcStatus::NotFound)
            }),
            OptionDatabase::Password => Err(AdbcError::with_message_and_status(
                "Password cannot be retrieved",
                AdbcStatus::InvalidArguments,
            )),
            OptionDatabase::Other(key) => {
                if let Some(OptionValue::String(s)) = self.options.get(&key) {
                    Ok(s.clone())
                } else {
                    Err(AdbcError::with_message_and_status(
                        format!("Option {} not found or not a string", key),
                        AdbcStatus::NotFound,
                    ))
                }
            }
            _ => Err(AdbcError::with_message_and_status(
                "Option not found",
                AdbcStatus::NotFound,
            )),
        }
    }

    fn get_option_bytes(&self, key: Self::Option) -> AdbcResult<Vec<u8>> {
        if let OptionDatabase::Other(key) = key {
            if let Some(OptionValue::Bytes(b)) = self.options.get(&key) {
                return Ok(b.clone());
            }
        }
        Err(AdbcError::with_message_and_status(
            "Option not found or not bytes",
            AdbcStatus::NotFound,
        ))
    }

    fn get_option_int(&self, key: Self::Option) -> AdbcResult<i64> {
        if let OptionDatabase::Other(key) = key {
            if let Some(OptionValue::Int(i)) = self.options.get(&key) {
                return Ok(*i);
            }
        }
        Err(AdbcError::with_message_and_status(
            "Option not found or not an integer",
            AdbcStatus::NotFound,
        ))
    }

    fn get_option_double(&self, key: Self::Option) -> AdbcResult<f64> {
        if let OptionDatabase::Other(key) = key {
            if let Some(OptionValue::Double(d)) = self.options.get(&key) {
                return Ok(*d);
            }
        }
        Err(AdbcError::with_message_and_status(
            "Option not found or not a double",
            AdbcStatus::NotFound,
        ))
    }
}

impl adbc_core::Database for FfiDatabase {
    type ConnectionType = FfiConnection;

    fn new_connection(&self) -> AdbcResult<Self::ConnectionType> {
        let uri = self.build_connection_uri()?;
        Ok(FfiConnection::new(uri))
    }

    fn new_connection_with_opts(
        &self,
        opts: impl IntoIterator<Item = (OptionConnection, OptionValue)>,
    ) -> AdbcResult<Self::ConnectionType> {
        let uri = self.build_connection_uri()?;
        let mut conn = FfiConnection::new(uri);
        for (key, value) in opts {
            conn.set_option(key, value)?;
        }
        Ok(conn)
    }
}

// -----------------------------------------------------------------------------
// FFI Connection
// -----------------------------------------------------------------------------

/// FFI-compatible ADBC Connection wrapper.
///
/// Represents an active connection to an Exasol database. The connection
/// is lazily established on first use (when creating a statement or
/// performing a transaction operation).
pub struct FfiConnection {
    /// Connection URI
    uri: String,
    /// The actual connection (lazily initialized), shared with statements
    inner: Option<Arc<Mutex<ExaConnection>>>,
    /// Pre-init options
    options: std::collections::HashMap<String, OptionValue>,
    /// Auto-commit mode
    auto_commit: bool,
    /// Current schema
    current_schema: Option<String>,
}

impl FfiConnection {
    fn new(uri: String) -> Self {
        Self {
            uri,
            inner: None,
            options: std::collections::HashMap::new(),
            auto_commit: true,
            current_schema: None,
        }
    }

    /// Borrow the already-established connection.
    ///
    /// Unlike [`ensure_connected`](Self::ensure_connected) this never dials: the
    /// read-only metadata calls take `&self` and so cannot store a new
    /// connection, and must report the unconnected state instead.
    fn require_connection(&self) -> AdbcResult<Arc<Mutex<ExaConnection>>> {
        self.inner.as_ref().map(Arc::clone).ok_or_else(|| {
            AdbcError::with_message_and_status(
                "Connection not established. Call new_statement() or connect first.",
                AdbcStatus::InvalidState,
            )
        })
    }

    /// Switch autocommit on or off, matching the server-side transaction to it.
    ///
    /// Turning autocommit off opens a transaction so following statements are
    /// staged rather than committed one by one; turning it back on commits
    /// whatever that transaction had accumulated. Re-asserting the value already
    /// in force changes nothing and never dials the server.
    fn apply_auto_commit(&mut self, enabled: bool) -> AdbcResult<()> {
        let was_enabled = self.auto_commit;
        self.auto_commit = enabled;

        if was_enabled == enabled {
            return Ok(());
        }
        if enabled {
            return self.commit_open_transaction();
        }
        self.open_transaction()
    }

    /// Begin a server-side transaction to stage subsequent statements.
    fn open_transaction(&mut self) -> AdbcResult<()> {
        let conn_arc = self.ensure_connected()?;
        get_runtime()
            .block_on(async {
                let mut conn = conn_arc.lock().await;
                conn.begin_transaction().await
            })
            .map_err(to_adbc_error)
    }

    /// Commit the staged transaction, if one is open.
    fn commit_open_transaction(&mut self) -> AdbcResult<()> {
        let conn_arc = self.ensure_connected()?;
        get_runtime()
            .block_on(async {
                let mut conn = conn_arc.lock().await;
                if conn.in_transaction() {
                    conn.commit().await?;
                }
                Ok::<(), QueryError>(())
            })
            .map_err(to_adbc_error)
    }

    /// Ensure the connection is established.
    fn ensure_connected(&mut self) -> AdbcResult<Arc<Mutex<ExaConnection>>> {
        if self.inner.is_none() {
            let uri = self.uri.clone();
            let conn = get_runtime()
                .block_on(async {
                    let params: crate::connection::ConnectionParams = uri.parse()?;
                    ExaConnection::from_params(params).await
                })
                .map_err(to_adbc_error)?;
            self.inner = Some(Arc::new(Mutex::new(conn)));
        }
        Ok(Arc::clone(self.inner.as_ref().unwrap()))
    }
}

impl Drop for FfiConnection {
    fn drop(&mut self) {
        if let Some(conn) = self.inner.take() {
            // Attempt graceful shutdown: close WebSocket and session
            // to prevent lingering I/O tasks on the static runtime.
            let _ = get_runtime().block_on(async {
                let conn = conn.lock().await;
                conn.shutdown().await
            });
        }
    }
}

impl Optionable for FfiConnection {
    type Option = OptionConnection;

    fn set_option(&mut self, key: Self::Option, value: OptionValue) -> AdbcResult<()> {
        match key {
            OptionConnection::AutoCommit => {
                let requested = require_string_option(
                    value,
                    "AutoCommit must be a string ('true' or 'false')",
                )?;
                self.apply_auto_commit(requested == "true" || requested == "1")
            }
            OptionConnection::CurrentSchema => {
                self.current_schema = Some(require_string_option(
                    value,
                    "CurrentSchema must be a string",
                )?);
                Ok(())
            }
            OptionConnection::ReadOnly | OptionConnection::IsolationLevel => {
                self.options.insert(key.as_ref().to_string(), value);
                Ok(())
            }
            OptionConnection::CurrentCatalog => Err(AdbcError::with_message_and_status(
                "Exasol does not support catalogs",
                AdbcStatus::NotImplemented,
            )),
            OptionConnection::Other(key) => {
                self.options.insert(key, value);
                Ok(())
            }
            _ => Err(AdbcError::with_message_and_status(
                "Unsupported connection option",
                AdbcStatus::NotImplemented,
            )),
        }
    }

    fn get_option_string(&self, key: Self::Option) -> AdbcResult<String> {
        match key {
            OptionConnection::AutoCommit => {
                Ok(if self.auto_commit { "true" } else { "false" }.to_string())
            }
            OptionConnection::CurrentCatalog => Ok("EXA".to_string()),
            OptionConnection::CurrentSchema => self.current_schema.clone().ok_or_else(|| {
                AdbcError::with_message_and_status("CurrentSchema not set", AdbcStatus::NotFound)
            }),
            OptionConnection::Other(key) => {
                if let Some(OptionValue::String(s)) = self.options.get(&key) {
                    Ok(s.clone())
                } else {
                    Err(AdbcError::with_message_and_status(
                        format!("Option {} not found", key),
                        AdbcStatus::NotFound,
                    ))
                }
            }
            _ => Err(AdbcError::with_message_and_status(
                "Option not found",
                AdbcStatus::NotFound,
            )),
        }
    }

    fn get_option_bytes(&self, key: Self::Option) -> AdbcResult<Vec<u8>> {
        if let OptionConnection::Other(key) = key {
            if let Some(OptionValue::Bytes(b)) = self.options.get(&key) {
                return Ok(b.clone());
            }
        }
        Err(AdbcError::with_message_and_status(
            "Option not found or not bytes",
            AdbcStatus::NotFound,
        ))
    }

    fn get_option_int(&self, key: Self::Option) -> AdbcResult<i64> {
        if let OptionConnection::Other(key) = key {
            if let Some(OptionValue::Int(i)) = self.options.get(&key) {
                return Ok(*i);
            }
        }
        Err(AdbcError::with_message_and_status(
            "Option not found or not an integer",
            AdbcStatus::NotFound,
        ))
    }

    fn get_option_double(&self, key: Self::Option) -> AdbcResult<f64> {
        if let OptionConnection::Other(key) = key {
            if let Some(OptionValue::Double(d)) = self.options.get(&key) {
                return Ok(*d);
            }
        }
        Err(AdbcError::with_message_and_status(
            "Option not found or not a double",
            AdbcStatus::NotFound,
        ))
    }
}

/// A simple RecordBatchReader implementation that yields batches from a Vec.
struct VecRecordBatchReader {
    schema: Arc<Schema>,
    batches: std::vec::IntoIter<RecordBatch>,
}

impl VecRecordBatchReader {
    fn new(schema: Arc<Schema>, batches: Vec<RecordBatch>) -> Self {
        Self {
            schema,
            batches: batches.into_iter(),
        }
    }

    fn empty(schema: Arc<Schema>) -> Self {
        Self::new(schema, vec![])
    }
}

impl Iterator for VecRecordBatchReader {
    type Item = Result<RecordBatch, arrow::error::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.batches.next().map(Ok)
    }
}

impl RecordBatchReader for VecRecordBatchReader {
    fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.schema)
    }
}

impl adbc_core::Connection for FfiConnection {
    type StatementType = FfiStatement;

    fn new_statement(&mut self) -> AdbcResult<Self::StatementType> {
        let conn = self.ensure_connected()?;
        let mut stmt = FfiStatement::with_connection(Some(conn), self.uri.clone());
        stmt.auto_commit = self.auto_commit;
        Ok(stmt)
    }

    fn cancel(&mut self) -> AdbcResult<()> {
        // Cancel is not directly supported in our async implementation
        Err(AdbcError::with_message_and_status(
            "Cancel not implemented",
            AdbcStatus::NotImplemented,
        ))
    }

    fn get_info(
        &self,
        _codes: Option<HashSet<InfoCode>>,
    ) -> AdbcResult<Box<dyn RecordBatchReader + Send>> {
        // Return driver info as a RecordBatch
        use arrow::array::builder::{StringBuilder, UInt32Builder};
        use arrow::datatypes::{DataType, Field};

        // Build info schema
        let schema = Arc::new(Schema::new(vec![
            Field::new("info_name", DataType::UInt32, false),
            Field::new("info_value", DataType::Utf8, true),
        ]));

        // Build info data
        let mut name_builder = UInt32Builder::new();
        let mut value_builder = StringBuilder::new();

        // Add driver info
        name_builder.append_value(0); // VendorName
        value_builder.append_value("Exasol");

        name_builder.append_value(100); // DriverName
        value_builder.append_value("exarrow-rs");

        name_builder.append_value(101); // DriverVersion
        value_builder.append_value(env!("CARGO_PKG_VERSION"));

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(name_builder.finish()),
                Arc::new(value_builder.finish()),
            ],
        )
        .map_err(|e| AdbcError::with_message_and_status(e.to_string(), AdbcStatus::Internal))?;

        Ok(Box::new(VecRecordBatchReader::new(schema, vec![batch])))
    }

    fn get_objects(
        &self,
        depth: ObjectDepth,
        catalog: Option<&str>,
        db_schema: Option<&str>,
        table_name: Option<&str>,
        table_type: Option<Vec<&str>>,
        column_name: Option<&str>,
    ) -> AdbcResult<Box<dyn RecordBatchReader + Send>> {
        // Exasol exposes exactly one catalog, so any other catalog filter
        // matches nothing and yields an empty — but correctly shaped — result.
        if !catalog_is_exasol(catalog) {
            return Ok(Box::new(VecRecordBatchReader::empty(Arc::new(
                build_get_objects_schema(),
            ))));
        }

        let conn_arc = self.require_connection()?;
        let levels = ObjectDepthLevels::from(depth);

        let schemas = if levels.schemas {
            schema_names_from(&query_batches(
                &conn_arc,
                &get_objects_schemas_query(db_schema),
            )?)?
        } else {
            Vec::new()
        };

        let tables = if levels.tables {
            let sql = get_objects_tables_query(db_schema, table_name, table_type.as_deref());
            tables_by_schema_from(&query_batches(&conn_arc, &sql)?)?
        } else {
            TablesBySchema::new()
        };

        let columns = if levels.columns {
            let sql = get_objects_columns_query(db_schema, table_name, column_name);
            columns_by_table_from(&query_batches(&conn_arc, &sql)?)?
        } else {
            ColumnMetadataMap::new()
        };

        let batch = build_get_objects_batch(&schemas, &tables, &columns, levels)?;
        let schema = batch.schema();
        Ok(Box::new(VecRecordBatchReader::new(schema, vec![batch])))
    }

    fn get_table_schema(
        &self,
        _catalog: Option<&str>,
        db_schema: Option<&str>,
        table_name: &str,
    ) -> AdbcResult<Schema> {
        let conn_arc = self.require_connection()?;
        let sql = table_schema_query(db_schema, table_name);
        let batches = query_batches(&conn_arc, &sql)?;

        let fields = table_schema_fields_from(&batches)?;
        if fields.is_empty() {
            return Err(AdbcError::with_message_and_status(
                format!("Table '{}' not found", table_name),
                AdbcStatus::NotFound,
            ));
        }

        Ok(Schema::new(fields))
    }

    fn get_table_types(&self) -> AdbcResult<Box<dyn RecordBatchReader + Send>> {
        use arrow::array::builder::StringBuilder;
        use arrow::datatypes::{DataType, Field};

        let schema = Arc::new(Schema::new(vec![Field::new(
            "table_type",
            DataType::Utf8,
            false,
        )]));

        let mut builder = StringBuilder::new();
        builder.append_value("TABLE");
        builder.append_value("VIEW");
        builder.append_value("SYSTEM TABLE");

        let batch = RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(builder.finish())])
            .map_err(|e| AdbcError::with_message_and_status(e.to_string(), AdbcStatus::Internal))?;

        Ok(Box::new(VecRecordBatchReader::new(schema, vec![batch])))
    }

    fn get_statistic_names(&self) -> AdbcResult<Box<dyn RecordBatchReader + Send>> {
        use arrow::datatypes::{DataType, Field};
        let schema = Arc::new(Schema::new(vec![
            Field::new("statistic_name", DataType::Utf8, false),
            Field::new("statistic_key", DataType::Int16, false),
        ]));
        Ok(Box::new(VecRecordBatchReader::empty(schema)))
    }

    fn get_statistics(
        &self,
        _catalog: Option<&str>,
        _db_schema: Option<&str>,
        _table_name: Option<&str>,
        _approximate: bool,
    ) -> AdbcResult<Box<dyn RecordBatchReader + Send>> {
        Err::<Box<dyn RecordBatchReader + Send>, _>(AdbcError::with_message_and_status(
            "get_statistics not yet implemented",
            AdbcStatus::NotImplemented,
        ))
    }

    fn commit(&mut self) -> AdbcResult<()> {
        if self.auto_commit {
            return Err(AdbcError::with_message_and_status(
                "Cannot commit when autocommit is enabled",
                AdbcStatus::InvalidState,
            ));
        }
        let conn_arc = self.ensure_connected()?;
        get_runtime()
            .block_on(async {
                let mut conn = conn_arc.lock().await;
                conn.commit().await
            })
            .map_err(to_adbc_error)
    }

    fn rollback(&mut self) -> AdbcResult<()> {
        if self.auto_commit {
            return Err(AdbcError::with_message_and_status(
                "Cannot rollback when autocommit is enabled",
                AdbcStatus::InvalidState,
            ));
        }
        let conn_arc = self.ensure_connected()?;
        get_runtime()
            .block_on(async {
                let mut conn = conn_arc.lock().await;
                conn.rollback().await
            })
            .map_err(to_adbc_error)
    }

    fn read_partition(
        &self,
        _partition: impl AsRef<[u8]>,
    ) -> AdbcResult<Box<dyn RecordBatchReader + Send>> {
        Err::<Box<dyn RecordBatchReader + Send>, _>(AdbcError::with_message_and_status(
            "Partitioned results not supported",
            AdbcStatus::NotImplemented,
        ))
    }
}

// -----------------------------------------------------------------------------
// Arrow-to-Parameter Conversion
// -----------------------------------------------------------------------------

/// Extract a value from an Arrow array at a given row index and convert it to
/// a `Parameter` enum suitable for Exasol prepared statement binding.
///
/// Returns `Parameter::Null` for null values. Handles all Arrow types that map
/// to Exasol types: Int16/32/64, Float32/64, Utf8/LargeUtf8, Binary/LargeBinary,
/// Boolean, Date32, Timestamp (all units), and Decimal128.
fn arrow_value_to_parameter(array: &dyn Array, row: usize) -> AdbcResult<Parameter> {
    use arrow::array::{
        BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array, Float64Array,
        Int16Array, Int32Array, Int64Array, LargeBinaryArray, LargeStringArray, StringArray,
        TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
        TimestampSecondArray,
    };
    use arrow::datatypes::DataType;

    if array.is_null(row) {
        return Ok(Parameter::Null);
    }

    let dt = array.data_type();
    match dt {
        DataType::Boolean => {
            let arr = array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("boolean downcast");
            Ok(Parameter::Boolean(arr.value(row)))
        }
        DataType::Int16 => {
            let arr = array
                .as_any()
                .downcast_ref::<Int16Array>()
                .expect("int16 downcast");
            Ok(Parameter::Integer(arr.value(row) as i64))
        }
        DataType::Int32 => {
            let arr = array
                .as_any()
                .downcast_ref::<Int32Array>()
                .expect("int32 downcast");
            Ok(Parameter::Integer(arr.value(row) as i64))
        }
        DataType::Int64 => {
            let arr = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("int64 downcast");
            Ok(Parameter::Integer(arr.value(row)))
        }
        DataType::Float32 => {
            let arr = array
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("float32 downcast");
            Ok(Parameter::Float(arr.value(row) as f64))
        }
        DataType::Float64 => {
            let arr = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("float64 downcast");
            Ok(Parameter::Float(arr.value(row)))
        }
        DataType::Utf8 => {
            let arr = array
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("utf8 downcast");
            Ok(Parameter::String(arr.value(row).to_string()))
        }
        DataType::LargeUtf8 => {
            let arr = array
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .expect("large utf8 downcast");
            Ok(Parameter::String(arr.value(row).to_string()))
        }
        DataType::Binary => {
            let arr = array
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("binary downcast");
            Ok(Parameter::Binary(arr.value(row).to_vec()))
        }
        DataType::LargeBinary => {
            let arr = array
                .as_any()
                .downcast_ref::<LargeBinaryArray>()
                .expect("large binary downcast");
            Ok(Parameter::Binary(arr.value(row).to_vec()))
        }
        DataType::Date32 => {
            // Date32 stores days since Unix epoch. Convert to "YYYY-MM-DD" string.
            let arr = array
                .as_any()
                .downcast_ref::<Date32Array>()
                .expect("date32 downcast");
            let days = arr.value(row);
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            let date = epoch + chrono::Duration::days(days as i64);
            Ok(Parameter::String(date.format("%Y-%m-%d").to_string()))
        }
        DataType::Timestamp(unit, _tz) => {
            // Convert timestamp to ISO 8601 string for Exasol
            let nanos = match unit {
                arrow::datatypes::TimeUnit::Second => {
                    let arr = array
                        .as_any()
                        .downcast_ref::<TimestampSecondArray>()
                        .expect("timestamp_s downcast");
                    arr.value(row) * 1_000_000_000
                }
                arrow::datatypes::TimeUnit::Millisecond => {
                    let arr = array
                        .as_any()
                        .downcast_ref::<TimestampMillisecondArray>()
                        .expect("timestamp_ms downcast");
                    arr.value(row) * 1_000_000
                }
                arrow::datatypes::TimeUnit::Microsecond => {
                    let arr = array
                        .as_any()
                        .downcast_ref::<TimestampMicrosecondArray>()
                        .expect("timestamp_us downcast");
                    arr.value(row) * 1_000
                }
                arrow::datatypes::TimeUnit::Nanosecond => {
                    let arr = array
                        .as_any()
                        .downcast_ref::<TimestampNanosecondArray>()
                        .expect("timestamp_ns downcast");
                    arr.value(row)
                }
            };
            let secs = nanos.div_euclid(1_000_000_000);
            let subsec_nanos = nanos.rem_euclid(1_000_000_000) as u32;
            let dt = chrono::DateTime::from_timestamp(secs, subsec_nanos).ok_or_else(|| {
                AdbcError::with_message_and_status(
                    format!("Invalid timestamp value: {nanos} nanos"),
                    AdbcStatus::InvalidArguments,
                )
            })?;
            Ok(Parameter::String(
                dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string(),
            ))
        }
        DataType::Decimal128(_precision, scale) => {
            let arr = array
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .expect("decimal128 downcast");
            let raw = arr.value(row);
            // Format decimal with proper scale
            if *scale <= 0 {
                Ok(Parameter::String(raw.to_string()))
            } else {
                let scale_u = *scale as u32;
                let divisor = 10_i128.pow(scale_u);
                let whole = raw / divisor;
                let frac = (raw % divisor).unsigned_abs();
                Ok(Parameter::String(format!(
                    "{}{}.{:0>width$}",
                    if raw < 0 && whole == 0 { "-" } else { "" },
                    whole,
                    frac,
                    width = scale_u as usize
                )))
            }
        }
        other => Err(AdbcError::with_message_and_status(
            format!("Unsupported Arrow type for parameter binding: {other}"),
            AdbcStatus::NotImplemented,
        )),
    }
}

// -----------------------------------------------------------------------------
// FFI Statement
// -----------------------------------------------------------------------------

/// FFI-compatible ADBC Statement wrapper.
///
/// Used to execute SQL queries and retrieve results as Arrow RecordBatches.
///
/// # Example
///
pub struct FfiStatement {
    /// Shared connection handle from the parent FfiConnection
    conn: Option<Arc<Mutex<ExaConnection>>>,
    /// Connection URI (fallback for ephemeral connections)
    uri: String,
    /// SQL query
    sql: Option<String>,
    /// Bound parameters as RecordBatch
    bound_data: Option<RecordBatch>,
    /// Statement options
    options: std::collections::HashMap<String, OptionValue>,
    /// Prepared statement with parameter metadata and bindings
    prepared: Option<PreparedStatement>,
    /// Autocommit state inherited from parent connection
    auto_commit: bool,
}

impl FfiStatement {
    #[cfg(test)]
    fn new(uri: String) -> Self {
        Self {
            conn: None,
            uri,
            sql: None,
            bound_data: None,
            options: std::collections::HashMap::new(),
            prepared: None,
            auto_commit: true,
        }
    }

    fn with_connection(conn: Option<Arc<Mutex<ExaConnection>>>, uri: String) -> Self {
        Self {
            conn,
            uri,
            sql: None,
            bound_data: None,
            options: std::collections::HashMap::new(),
            prepared: None,
            auto_commit: true,
        }
    }

    /// Whether this statement was configured as a bulk ingest rather than SQL.
    fn is_bulk_ingest(&self) -> bool {
        self.options.contains_key(INGEST_TARGET_TABLE_OPTION)
    }

    /// The SQL this statement will run.
    fn require_sql(&self) -> AdbcResult<String> {
        self.sql.clone().ok_or_else(|| {
            AdbcError::with_message_and_status("SQL query not set", AdbcStatus::InvalidState)
        })
    }

    /// The parent connection this statement executes against.
    fn require_connection(&self) -> AdbcResult<Arc<Mutex<ExaConnection>>> {
        self.conn.as_ref().map(Arc::clone).ok_or_else(|| {
            AdbcError::with_message_and_status("No connection available", AdbcStatus::InvalidState)
        })
    }

    /// The server-side prepared statement plus the connection to run it on,
    /// preparing on first use.
    fn prepared_with_connection(
        &mut self,
    ) -> AdbcResult<(&mut PreparedStatement, Arc<Mutex<ExaConnection>>)> {
        if self.prepared.is_none() {
            adbc_core::Statement::prepare(self)?;
        }
        let prepared = self.prepared.as_mut().ok_or_else(|| {
            AdbcError::with_message_and_status(
                "Failed to prepare statement",
                AdbcStatus::InvalidState,
            )
        })?;
        let conn_arc = self.conn.as_ref().map(Arc::clone).ok_or_else(|| {
            AdbcError::with_message_and_status("No connection available", AdbcStatus::InvalidState)
        })?;
        Ok((prepared, conn_arc))
    }

    /// Execute the prepared statement once per row of the bound batch, and
    /// collect every result batch the rows produced.
    fn execute_bound_batch(&mut self, batch: RecordBatch) -> AdbcResult<Vec<RecordBatch>> {
        let (prepared, conn_arc) = self.prepared_with_connection()?;

        let mut all_batches: Vec<RecordBatch> = Vec::new();
        for row_idx in 0..batch.num_rows() {
            bind_row_as_parameters(prepared, &batch, row_idx)?;
            let batches = get_runtime()
                .block_on(async {
                    let mut conn = conn_arc.lock().await;
                    let result_set = conn.execute_prepared(prepared).await?;
                    result_set.fetch_all().await
                })
                .map_err(to_adbc_error)?;
            all_batches.extend(batches);
            prepared.clear_parameters();
        }
        Ok(all_batches)
    }

    /// Execute the prepared statement once per row of the bound batch, and total
    /// the affected-row counts.
    fn execute_bound_batch_update(&mut self, batch: RecordBatch) -> AdbcResult<i64> {
        let (prepared, conn_arc) = self.prepared_with_connection()?;

        let mut total_count: i64 = 0;
        for row_idx in 0..batch.num_rows() {
            bind_row_as_parameters(prepared, &batch, row_idx)?;
            total_count += get_runtime()
                .block_on(async {
                    let mut conn = conn_arc.lock().await;
                    conn.execute_prepared_update(prepared).await
                })
                .map_err(to_adbc_error)?;
            prepared.clear_parameters();
        }
        Ok(total_count)
    }

    /// Run `sql` as a query and collect its batches.
    ///
    /// A statement handed out by a connection shares that connection; a
    /// standalone one opens and closes its own for this call. A statement that
    /// produces no result set yields no batches rather than an error.
    fn query_sql(&self, sql: &str) -> AdbcResult<Vec<RecordBatch>> {
        if let Some(conn_arc) = self.conn.as_ref() {
            let conn_arc = Arc::clone(conn_arc);
            return match get_runtime().block_on(async {
                let mut conn = conn_arc.lock().await;
                conn.query(sql).await
            }) {
                Ok(batches) => Ok(batches),
                Err(QueryError::NoResultSet(_)) => Ok(vec![]),
                Err(e) => Err(to_adbc_error(e)),
            };
        }

        let uri = self.uri.clone();
        let sql = sql.to_string();
        get_runtime()
            .block_on(async {
                let params: crate::connection::ConnectionParams = uri.parse()?;
                let mut conn = ExaConnection::from_params(params).await?;
                let result = conn.query(&sql).await;
                conn.close().await?;
                match result {
                    Ok(batches) => Ok(batches),
                    Err(QueryError::NoResultSet(_)) => Ok(vec![]),
                    Err(e) => Err(ExasolError::from(e)),
                }
            })
            .map_err(to_adbc_error)
    }

    /// Run `sql` as a non-query statement and report the affected-row count.
    ///
    /// Shares the parent connection when there is one, otherwise opens and
    /// closes its own for this call.
    fn update_sql(&self, sql: &str) -> AdbcResult<i64> {
        if let Some(conn_arc) = self.conn.as_ref() {
            let conn_arc = Arc::clone(conn_arc);
            return get_runtime()
                .block_on(async {
                    let mut conn = conn_arc.lock().await;
                    conn.execute_update(sql).await
                })
                .map_err(to_adbc_error);
        }

        let uri = self.uri.clone();
        let sql = sql.to_string();
        get_runtime()
            .block_on(async {
                let params: crate::connection::ConnectionParams = uri.parse()?;
                let mut conn = ExaConnection::from_params(params).await?;
                let count = conn.execute_update(&sql).await?;
                conn.close().await?;
                Ok::<_, ExasolError>(count)
            })
            .map_err(to_adbc_error)
    }

    fn execute_bulk_ingest(&mut self) -> AdbcResult<i64> {
        let target_table = self
            .options
            .get("adbc.ingest.target_table")
            .and_then(|v| match v {
                OptionValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                AdbcError::with_message_and_status(
                    "Target table not set for bulk ingestion",
                    AdbcStatus::InvalidState,
                )
            })?;

        if let Some(OptionValue::String(s)) = self.options.get("adbc.ingest.temporary") {
            if s == "true" {
                return Err(AdbcError::with_message_and_status(
                    "Exasol does not support temporary tables",
                    AdbcStatus::NotImplemented,
                ));
            }
        }

        let target_schema =
            self.options
                .get("adbc.ingest.target_db_schema")
                .and_then(|v| match v {
                    OptionValue::String(s) => Some(s.clone()),
                    _ => None,
                });

        let ingest_mode = self
            .options
            .get("adbc.ingest.mode")
            .and_then(|v| match v {
                OptionValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "adbc.ingest.mode.append".to_string());

        match ingest_mode.as_str() {
            "adbc.ingest.mode.create"
            | "adbc.ingest.mode.append"
            | "adbc.ingest.mode.create_append"
            | "adbc.ingest.mode.replace" => {}
            other => {
                return Err(AdbcError::with_message_and_status(
                    format!("Unknown ingest mode: {}", other),
                    AdbcStatus::InvalidArguments,
                ));
            }
        }

        let qualified_name = build_qualified_table_name(target_schema.as_deref(), &target_table);

        let batch = self.bound_data.take().ok_or_else(|| {
            AdbcError::with_message_and_status(
                "No data bound for bulk ingestion",
                AdbcStatus::InvalidState,
            )
        })?;

        let conn_arc = self.conn.as_ref().ok_or_else(|| {
            AdbcError::with_message_and_status("No connection available", AdbcStatus::InvalidState)
        })?;
        let conn_arc = Arc::clone(conn_arc);

        let arrow_schema = batch.schema();

        match ingest_mode.as_str() {
            "adbc.ingest.mode.create" => {
                let ddl = generate_create_table_ddl(&qualified_name, &arrow_schema)?;
                let conn_arc2 = Arc::clone(&conn_arc);
                get_runtime()
                    .block_on(async {
                        let mut conn = conn_arc2.lock().await;
                        conn.execute_update(&ddl).await
                    })
                    .map_err(to_adbc_error)?;
            }
            "adbc.ingest.mode.create_append" => {
                let ddl = generate_create_table_ddl(&qualified_name, &arrow_schema)?;
                let ddl = ddl.replace("CREATE TABLE", "CREATE TABLE IF NOT EXISTS");
                let conn_arc2 = Arc::clone(&conn_arc);
                get_runtime()
                    .block_on(async {
                        let mut conn = conn_arc2.lock().await;
                        conn.execute_update(&ddl).await
                    })
                    .map_err(to_adbc_error)?;
            }
            "adbc.ingest.mode.replace" => {
                let drop_ddl = format!("DROP TABLE IF EXISTS {}", qualified_name);
                let create_ddl = generate_create_table_ddl(&qualified_name, &arrow_schema)?;
                let conn_arc2 = Arc::clone(&conn_arc);
                get_runtime()
                    .block_on(async {
                        let mut conn = conn_arc2.lock().await;
                        conn.execute_update(&drop_ddl).await?;
                        conn.execute_update(&create_ddl).await
                    })
                    .map_err(to_adbc_error)?;
            }
            _ => {
                // Append mode (default): no DDL needed, table must already exist
            }
        }

        let import_options = crate::import::arrow::ArrowImportOptions::default();
        let row_count = get_runtime()
            .block_on(async {
                let mut conn = conn_arc.lock().await;
                conn.import_from_record_batch(&qualified_name, &batch, import_options)
                    .await
            })
            .map_err(to_adbc_error)?;

        Ok(row_count as i64)
    }
}

impl Optionable for FfiStatement {
    type Option = OptionStatement;

    fn set_option(&mut self, key: Self::Option, value: OptionValue) -> AdbcResult<()> {
        match key {
            OptionStatement::Other(key) => {
                self.options.insert(key, value);
            }
            _ => {
                // Store standard options as strings
                self.options.insert(key.as_ref().to_string(), value);
            }
        }
        Ok(())
    }

    fn get_option_string(&self, key: Self::Option) -> AdbcResult<String> {
        let key_str = match key {
            OptionStatement::Other(ref k) => k.as_str(),
            _ => key.as_ref(),
        };
        if let Some(OptionValue::String(s)) = self.options.get(key_str) {
            Ok(s.clone())
        } else {
            Err(AdbcError::with_message_and_status(
                "Option not found or not a string",
                AdbcStatus::NotFound,
            ))
        }
    }

    fn get_option_bytes(&self, key: Self::Option) -> AdbcResult<Vec<u8>> {
        let key_str = match key {
            OptionStatement::Other(ref k) => k.as_str(),
            _ => key.as_ref(),
        };
        if let Some(OptionValue::Bytes(b)) = self.options.get(key_str) {
            Ok(b.clone())
        } else {
            Err(AdbcError::with_message_and_status(
                "Option not found or not bytes",
                AdbcStatus::NotFound,
            ))
        }
    }

    fn get_option_int(&self, key: Self::Option) -> AdbcResult<i64> {
        let key_str = match key {
            OptionStatement::Other(ref k) => k.as_str(),
            _ => key.as_ref(),
        };
        if let Some(OptionValue::Int(i)) = self.options.get(key_str) {
            Ok(*i)
        } else {
            Err(AdbcError::with_message_and_status(
                "Option not found or not an integer",
                AdbcStatus::NotFound,
            ))
        }
    }

    fn get_option_double(&self, key: Self::Option) -> AdbcResult<f64> {
        let key_str = match key {
            OptionStatement::Other(ref k) => k.as_str(),
            _ => key.as_ref(),
        };
        if let Some(OptionValue::Double(d)) = self.options.get(key_str) {
            Ok(*d)
        } else {
            Err(AdbcError::with_message_and_status(
                "Option not found or not a double",
                AdbcStatus::NotFound,
            ))
        }
    }
}

impl adbc_core::Statement for FfiStatement {
    fn bind(&mut self, batch: RecordBatch) -> AdbcResult<()> {
        self.bound_data = Some(batch);
        Ok(())
    }

    fn bind_stream(&mut self, mut reader: Box<dyn RecordBatchReader + Send>) -> AdbcResult<()> {
        // Collect all batches from the stream
        let mut batches = Vec::new();
        for batch_result in reader.by_ref() {
            let batch = batch_result.map_err(|e| {
                AdbcError::with_message_and_status(e.to_string(), AdbcStatus::Internal)
            })?;
            batches.push(batch);
        }

        // Concatenate batches if there are multiple
        if batches.is_empty() {
            self.bound_data = None;
        } else if batches.len() == 1 {
            self.bound_data = Some(batches.remove(0));
        } else {
            // Use arrow's concat_batches
            let schema = batches[0].schema();
            let combined = concat_batches(&schema, &batches).map_err(|e| {
                AdbcError::with_message_and_status(e.to_string(), AdbcStatus::Internal)
            })?;
            self.bound_data = Some(combined);
        }
        Ok(())
    }

    fn execute(&mut self) -> AdbcResult<Box<dyn RecordBatchReader + Send>> {
        if self.is_bulk_ingest() {
            self.execute_bulk_ingest()?;
            return Ok(reader_over_batches(vec![]));
        }

        let sql = self.require_sql()?;

        if let Some(batch) = self.bound_data.take() {
            return Ok(reader_over_batches(self.execute_bound_batch(batch)?));
        }

        Ok(reader_over_batches(self.query_sql(&sql)?))
    }

    fn execute_update(&mut self) -> AdbcResult<Option<i64>> {
        if self.is_bulk_ingest() {
            return Ok(Some(self.execute_bulk_ingest()?));
        }

        let sql = self.require_sql()?;

        if let Some(batch) = self.bound_data.take() {
            return Ok(Some(self.execute_bound_batch_update(batch)?));
        }

        Ok(Some(self.update_sql(&sql)?))
    }

    fn execute_schema(&mut self) -> AdbcResult<Schema> {
        if self.prepared.is_none() {
            self.prepare()?;
        }

        let sql = self.require_sql()?;
        let conn_arc = self.require_connection()?;
        let batches = query_batches(&conn_arc, &sql)?;

        Ok(batches
            .first()
            .map_or_else(Schema::empty, |batch| batch.schema().as_ref().clone()))
    }

    fn execute_partitions(&mut self) -> AdbcResult<PartitionedResult> {
        Err(AdbcError::with_message_and_status(
            "Partitioned execution not supported",
            AdbcStatus::NotImplemented,
        ))
    }

    fn get_parameter_schema(&self) -> AdbcResult<Schema> {
        use arrow::datatypes::Field;

        let prepared = self.prepared.as_ref().ok_or_else(|| {
            AdbcError::with_message_and_status(
                "Statement not prepared. Call prepare() first.",
                AdbcStatus::InvalidState,
            )
        })?;

        let mut fields = Vec::new();
        let handle_ref = prepared.handle_ref();
        for (i, param_type) in handle_ref.parameter_types.iter().enumerate() {
            let exasol_type = transport_datatype_to_exasol(param_type)?;
            let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).map_err(|e| {
                AdbcError::with_message_and_status(e.to_string(), AdbcStatus::Internal)
            })?;
            let name = handle_ref
                .parameter_names
                .get(i)
                .and_then(|n| n.as_deref())
                .filter(|name| !name.is_empty())
                .map(String::from)
                .unwrap_or_else(|| format!("parameter_{}", i + 1));
            fields.push(Field::new(name, arrow_type, true));
        }

        Ok(Schema::new(fields))
    }

    fn prepare(&mut self) -> AdbcResult<()> {
        let sql = self.sql.as_ref().ok_or_else(|| {
            AdbcError::with_message_and_status("SQL query not set", AdbcStatus::InvalidState)
        })?;
        let sql = sql.clone();

        let conn_arc = self.conn.as_ref().ok_or_else(|| {
            AdbcError::with_message_and_status("No connection available", AdbcStatus::InvalidState)
        })?;
        let conn_arc = Arc::clone(conn_arc);

        let prepared_stmt = get_runtime()
            .block_on(async {
                let mut conn = conn_arc.lock().await;
                conn.prepare(&sql).await
            })
            .map_err(to_adbc_error)?;

        self.prepared = Some(prepared_stmt);

        Ok(())
    }

    fn set_sql_query(&mut self, query: impl AsRef<str>) -> AdbcResult<()> {
        self.sql = Some(query.as_ref().to_string());
        Ok(())
    }

    fn set_substrait_plan(&mut self, _plan: impl AsRef<[u8]>) -> AdbcResult<()> {
        Err(AdbcError::with_message_and_status(
            "Substrait plans not supported",
            AdbcStatus::NotImplemented,
        ))
    }

    fn cancel(&mut self) -> AdbcResult<()> {
        Err(AdbcError::with_message_and_status(
            "Cancel not implemented",
            AdbcStatus::NotImplemented,
        ))
    }
}

// -----------------------------------------------------------------------------
// FFI Export
// -----------------------------------------------------------------------------

// Export the driver using the adbc_ffi macro.
// The exported function will be named `ExarrowDriverInit`.
adbc_ffi::export_driver!(ExarrowDriverInit, FfiDriver);

#[cfg(test)]
mod tests {
    use super::*;
    use adbc_core::{Driver, Statement};
    use arrow::datatypes::{DataType, Field};

    #[test]
    fn test_ffi_driver_creation() {
        let mut driver = FfiDriver;
        let db = driver.new_database();
        assert!(db.is_ok());
    }

    #[test]
    fn test_ffi_database_options() {
        let mut db = FfiDatabase::new();

        // Set URI
        db.set_option(
            OptionDatabase::Uri,
            "exasol://user:pass@localhost:8563".into(),
        )
        .unwrap();

        // Get URI
        let uri = db.get_option_string(OptionDatabase::Uri).unwrap();
        assert_eq!(uri, "exasol://user:pass@localhost:8563");

        // Set username override
        db.set_option(OptionDatabase::Username, "admin".into())
            .unwrap();
        let username = db.get_option_string(OptionDatabase::Username).unwrap();
        assert_eq!(username, "admin");
    }

    #[test]
    fn test_ffi_database_build_uri_with_overrides() {
        let mut db = FfiDatabase::new();

        // Set base URI
        db.set_option(
            OptionDatabase::Uri,
            "exasol://user:pass@localhost:8563/schema".into(),
        )
        .unwrap();

        // Override username
        db.set_option(OptionDatabase::Username, "admin".into())
            .unwrap();

        // Override password
        db.set_option(OptionDatabase::Password, "secret".into())
            .unwrap();

        let uri = db.build_connection_uri().unwrap();
        assert!(uri.contains("admin"));
        assert!(uri.contains("secret"));
        assert!(uri.contains("localhost:8563/schema"));
    }

    #[test]
    fn test_ffi_connection_options() {
        let mut conn = FfiConnection::new("exasol://user@localhost:8563".to_string());

        // Set auto-commit
        conn.set_option(OptionConnection::AutoCommit, "false".into())
            .unwrap();
        let auto_commit = conn
            .get_option_string(OptionConnection::AutoCommit)
            .unwrap();
        assert_eq!(auto_commit, "false");
    }

    #[test]
    fn test_ffi_statement_sql() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());

        stmt.set_sql_query("SELECT 1").unwrap();
        assert_eq!(stmt.sql, Some("SELECT 1".to_string()));
    }

    #[test]
    fn test_generate_create_table_ddl_basic() {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("score", DataType::Float64, true),
        ]);

        let ddl = generate_create_table_ddl("test_table", &schema).unwrap();
        assert!(ddl.starts_with("CREATE TABLE test_table ("));
        assert!(ddl.contains("\"id\" DECIMAL(18,0) NOT NULL"));
        assert!(ddl.contains("\"name\" VARCHAR(2000000)"));
        assert!(ddl.contains("\"score\" DOUBLE"));
    }

    #[test]
    fn test_generate_create_table_ddl_all_not_null() {
        let schema = Schema::new(vec![
            Field::new("a", DataType::Boolean, false),
            Field::new("b", DataType::Date32, false),
        ]);

        let ddl = generate_create_table_ddl("my_table", &schema).unwrap();
        assert!(ddl.contains("\"a\" BOOLEAN NOT NULL"));
        assert!(ddl.contains("\"b\" DATE NOT NULL"));
    }

    #[test]
    fn test_generate_create_table_ddl_empty_schema() {
        let schema = Schema::new(Vec::<Field>::new());
        let ddl = generate_create_table_ddl("empty_table", &schema).unwrap();
        assert_eq!(ddl, "CREATE TABLE empty_table ()");
    }

    #[test]
    fn test_generate_create_table_ddl_column_name_escaping() {
        let schema = Schema::new(vec![Field::new(
            "col with \"quotes\"",
            DataType::Utf8,
            true,
        )]);
        let ddl = generate_create_table_ddl("t", &schema).unwrap();
        assert!(ddl.contains("\"col with \"\"quotes\"\"\""));
    }

    #[test]
    fn test_parse_exasol_type_string_varchar() {
        let t = parse_exasol_type_string("VARCHAR(200)").unwrap();
        assert_eq!(t, ExasolType::Varchar { size: 200 });
    }

    #[test]
    fn test_parse_exasol_type_string_decimal() {
        let t = parse_exasol_type_string("DECIMAL(10,3)").unwrap();
        assert_eq!(
            t,
            ExasolType::Decimal {
                precision: 10,
                scale: 3
            }
        );
    }

    #[test]
    fn test_parse_exasol_type_string_boolean() {
        let t = parse_exasol_type_string("BOOLEAN").unwrap();
        assert_eq!(t, ExasolType::Boolean);
    }

    #[test]
    fn test_parse_exasol_type_string_timestamp_with_tz() {
        let t = parse_exasol_type_string("TIMESTAMP WITH LOCAL TIME ZONE").unwrap();
        assert_eq!(
            t,
            ExasolType::Timestamp {
                with_local_time_zone: true
            }
        );
    }

    #[test]
    fn test_parse_exasol_type_timestamp_with_precision() {
        for input in &["TIMESTAMP(3)", "TIMESTAMP(6)", "TIMESTAMP(9)", "TIMESTAMP"] {
            let t = parse_exasol_type_string(input).unwrap();
            assert_eq!(
                t,
                ExasolType::Timestamp {
                    with_local_time_zone: false
                },
                "Failed for input: {}",
                input
            );
        }
    }

    #[test]
    fn test_parse_exasol_type_timestamp_with_local_time_zone_precision() {
        for input in &[
            "TIMESTAMP WITH LOCAL TIME ZONE",
            "TIMESTAMP(3) WITH LOCAL TIME ZONE",
            "TIMESTAMP(6) WITH LOCAL TIME ZONE",
        ] {
            let t = parse_exasol_type_string(input).unwrap();
            assert_eq!(
                t,
                ExasolType::Timestamp {
                    with_local_time_zone: true
                },
                "Failed for input: {}",
                input
            );
        }
    }

    #[test]
    fn test_parse_exasol_type_string_unknown() {
        let result = parse_exasol_type_string("UNKNOWN_TYPE");
        assert!(result.is_err());
    }

    #[test]
    fn test_transport_datatype_to_exasol_decimal() {
        let dt = TransportDataType {
            type_name: "DECIMAL".to_string(),
            precision: Some(18),
            scale: Some(0),
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        };
        let exasol = transport_datatype_to_exasol(&dt).unwrap();
        assert_eq!(
            exasol,
            ExasolType::Decimal {
                precision: 18,
                scale: 0
            }
        );
    }

    #[test]
    fn test_transport_datatype_to_exasol_varchar() {
        let dt = TransportDataType {
            type_name: "VARCHAR".to_string(),
            precision: None,
            scale: None,
            size: Some(100),
            character_set: Some("UTF8".to_string()),
            with_local_time_zone: None,
            fraction: None,
        };
        let exasol = transport_datatype_to_exasol(&dt).unwrap();
        assert_eq!(exasol, ExasolType::Varchar { size: 100 });
    }

    #[test]
    fn test_get_parameter_schema_not_prepared() {
        let stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        let result = stmt.get_parameter_schema();
        assert!(result.is_err());
    }

    #[test]
    fn test_get_objects_invalid_catalog() {
        let conn = FfiConnection {
            uri: "exasol://user@localhost:8563".to_string(),
            inner: None,
            options: std::collections::HashMap::new(),
            auto_commit: true,
            current_schema: None,
        };

        // With a catalog filter that doesn't match "EXA", should return empty
        let result = adbc_core::Connection::get_objects(
            &conn,
            ObjectDepth::Catalogs,
            Some("OTHER"),
            None,
            None,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_objects_no_connection() {
        let conn = FfiConnection {
            uri: "exasol://user@localhost:8563".to_string(),
            inner: None,
            options: std::collections::HashMap::new(),
            auto_commit: true,
            current_schema: None,
        };

        // Catalogs-only depth with matching catalog should fail (no connection)
        let result = adbc_core::Connection::get_objects(
            &conn,
            ObjectDepth::Schemas,
            Some("EXA"),
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_get_table_schema_no_connection() {
        let conn = FfiConnection {
            uri: "exasol://user@localhost:8563".to_string(),
            inner: None,
            options: std::collections::HashMap::new(),
            auto_commit: true,
            current_schema: None,
        };

        let result = adbc_core::Connection::get_table_schema(&conn, None, None, "SOME_TABLE");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_get_objects_schema() {
        let schema = build_get_objects_schema();
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.field(0).name(), "catalog_name");
        assert_eq!(schema.field(1).name(), "catalog_db_schemas");
    }

    #[test]
    fn test_bulk_ingest_temporary_not_supported() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        stmt.set_option(
            OptionStatement::Other("adbc.ingest.target_table".to_string()),
            OptionValue::String("test_table".to_string()),
        )
        .unwrap();
        stmt.set_option(
            OptionStatement::Other("adbc.ingest.temporary".to_string()),
            OptionValue::String("true".to_string()),
        )
        .unwrap();

        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)])),
            vec![Arc::new(arrow::array::Int32Array::from(vec![1]))],
        )
        .unwrap();
        stmt.bind(batch).unwrap();

        let result = stmt.execute_update();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.status, AdbcStatus::NotImplemented);
        assert!(err.message.contains("temporary tables"));
    }

    #[test]
    fn test_bulk_ingest_no_bound_data() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        stmt.set_option(
            OptionStatement::Other("adbc.ingest.target_table".to_string()),
            OptionValue::String("test_table".to_string()),
        )
        .unwrap();

        let result = stmt.execute_update();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("No data bound"));
    }

    #[test]
    fn test_bulk_ingest_no_connection() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        stmt.set_option(
            OptionStatement::Other("adbc.ingest.target_table".to_string()),
            OptionValue::String("test_table".to_string()),
        )
        .unwrap();

        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)])),
            vec![Arc::new(arrow::array::Int32Array::from(vec![1]))],
        )
        .unwrap();
        stmt.bind(batch).unwrap();

        let result = stmt.execute_update();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("No connection available"));
    }

    #[test]
    fn test_bulk_ingest_unknown_mode() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        stmt.set_option(
            OptionStatement::Other("adbc.ingest.target_table".to_string()),
            OptionValue::String("test_table".to_string()),
        )
        .unwrap();
        stmt.set_option(
            OptionStatement::Other("adbc.ingest.mode".to_string()),
            OptionValue::String("adbc.ingest.mode.unknown".to_string()),
        )
        .unwrap();

        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)])),
            vec![Arc::new(arrow::array::Int32Array::from(vec![1]))],
        )
        .unwrap();
        stmt.bind(batch).unwrap();

        let result = stmt.execute_update();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("Unknown ingest mode"));
    }

    #[test]
    fn test_bulk_ingest_detection_in_execute() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        stmt.set_option(
            OptionStatement::Other("adbc.ingest.target_table".to_string()),
            OptionValue::String("test_table".to_string()),
        )
        .unwrap();

        // Without bound data, should return error (not "SQL query not set")
        let result = stmt.execute();
        match result {
            Ok(_) => panic!("Expected error for missing bound data"),
            Err(err) => assert!(
                err.message.contains("No data bound"),
                "Expected 'No data bound' error, got: {}",
                err.message
            ),
        }
    }

    #[test]
    fn test_autocommit_default_true() {
        let conn = FfiConnection::new("exasol://user@localhost:8563".to_string());
        assert!(conn.auto_commit);
    }

    #[test]
    fn test_autocommit_set_false() {
        let mut conn = FfiConnection::new("exasol://user@localhost:8563".to_string());
        conn.set_option(OptionConnection::AutoCommit, "false".into())
            .unwrap();
        assert!(!conn.auto_commit);
    }

    #[test]
    fn test_statement_inherits_autocommit() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        assert!(stmt.auto_commit);
        stmt.auto_commit = false;
        assert!(!stmt.auto_commit);
    }

    #[test]
    fn test_parse_interval_year_to_month_with_precision() {
        let t = parse_exasol_type_string("INTERVAL YEAR(2) TO MONTH").unwrap();
        assert_eq!(t, ExasolType::IntervalYearToMonth);

        let t = parse_exasol_type_string("INTERVAL YEAR(5) TO MONTH").unwrap();
        assert_eq!(t, ExasolType::IntervalYearToMonth);
    }

    #[test]
    fn test_parse_interval_year_to_month_bare() {
        let t = parse_exasol_type_string("INTERVAL YEAR TO MONTH").unwrap();
        assert_eq!(t, ExasolType::IntervalYearToMonth);
    }

    #[test]
    fn test_parse_interval_day_to_second_with_precision() {
        let t = parse_exasol_type_string("INTERVAL DAY(2) TO SECOND(3)").unwrap();
        assert_eq!(t, ExasolType::IntervalDayToSecond { precision: 3 });

        let t = parse_exasol_type_string("INTERVAL DAY(9) TO SECOND(9)").unwrap();
        assert_eq!(t, ExasolType::IntervalDayToSecond { precision: 9 });
    }

    #[test]
    fn test_parse_interval_day_to_second_bare() {
        let t = parse_exasol_type_string("INTERVAL DAY TO SECOND").unwrap();
        assert_eq!(t, ExasolType::IntervalDayToSecond { precision: 3 });

        let t = parse_exasol_type_string("INTERVAL DAY TO SECOND(6)").unwrap();
        assert_eq!(t, ExasolType::IntervalDayToSecond { precision: 6 });
    }

    // ========================================================================
    // parse_exasol_type_string: the full type grammar
    //
    // These pin down every family the system-view type parser accepts,
    // including each family's default when the type carries no parentheses.
    // ========================================================================

    /// Every accepted spelling, paired with the type it must produce.
    fn type_string_cases() -> Vec<(&'static str, ExasolType)> {
        vec![
            // Unparameterized names.
            ("BOOLEAN", ExasolType::Boolean),
            ("DATE", ExasolType::Date),
            ("DOUBLE", ExasolType::Double),
            ("DOUBLE PRECISION", ExasolType::Double),
            ("FLOAT", ExasolType::Double),
            ("REAL", ExasolType::Double),
            (
                "BIGINT",
                ExasolType::Decimal {
                    precision: 18,
                    scale: 0,
                },
            ),
            (
                "INT",
                ExasolType::Decimal {
                    precision: 18,
                    scale: 0,
                },
            ),
            (
                "INTEGER",
                ExasolType::Decimal {
                    precision: 18,
                    scale: 0,
                },
            ),
            (
                "SMALLINT",
                ExasolType::Decimal {
                    precision: 18,
                    scale: 0,
                },
            ),
            (
                "TINYINT",
                ExasolType::Decimal {
                    precision: 18,
                    scale: 0,
                },
            ),
            // Large character types: always the maximum VARCHAR size.
            ("CLOB", ExasolType::Varchar { size: 2_000_000 }),
            ("LONG VARCHAR", ExasolType::Varchar { size: 2_000_000 }),
            ("LONG VARCHAR(100)", ExasolType::Varchar { size: 2_000_000 }),
            // Temporal families.
            (
                "TIMESTAMP",
                ExasolType::Timestamp {
                    with_local_time_zone: false,
                },
            ),
            (
                "TIMESTAMP(6)",
                ExasolType::Timestamp {
                    with_local_time_zone: false,
                },
            ),
            (
                "TIMESTAMP WITH LOCAL TIME ZONE",
                ExasolType::Timestamp {
                    with_local_time_zone: true,
                },
            ),
            ("INTERVAL YEAR TO MONTH", ExasolType::IntervalYearToMonth),
            (
                "INTERVAL DAY TO SECOND",
                ExasolType::IntervalDayToSecond { precision: 3 },
            ),
            // Parameterized families, with and without their parameters.
            ("GEOMETRY", ExasolType::Geometry { srid: None }),
            ("GEOMETRY(4326)", ExasolType::Geometry { srid: Some(4326) }),
            ("HASHTYPE", ExasolType::Hashtype { byte_size: 16 }),
            ("HASHTYPE(32)", ExasolType::Hashtype { byte_size: 32 }),
            ("CHAR", ExasolType::Char { size: 1 }),
            ("CHAR(10)", ExasolType::Char { size: 10 }),
            ("VARCHAR", ExasolType::Varchar { size: 2_000_000 }),
            ("VARCHAR(100)", ExasolType::Varchar { size: 100 }),
            ("CHAR VARYING(50)", ExasolType::Varchar { size: 50 }),
            (
                "DECIMAL",
                ExasolType::Decimal {
                    precision: 18,
                    scale: 0,
                },
            ),
            (
                "DECIMAL(10,2)",
                ExasolType::Decimal {
                    precision: 10,
                    scale: 2,
                },
            ),
            (
                "NUMERIC(5,1)",
                ExasolType::Decimal {
                    precision: 5,
                    scale: 1,
                },
            ),
        ]
    }

    #[test]
    fn parse_exasol_type_string_accepts_every_supported_type_name() {
        for (input, expected) in type_string_cases() {
            assert_eq!(
                parse_exasol_type_string(input).unwrap_or_else(|e| panic!("{}: {:?}", input, e)),
                expected,
                "type string {:?}",
                input
            );
        }
    }

    /// Type names arrive from system views in mixed case and with surrounding
    /// whitespace, so the parser trims and upper-cases before matching.
    #[test]
    fn parse_exasol_type_string_ignores_case_and_surrounding_whitespace() {
        for (input, expected) in type_string_cases() {
            let noisy = format!("  {}  ", input.to_lowercase());
            assert_eq!(
                parse_exasol_type_string(&noisy).unwrap_or_else(|e| panic!("{}: {:?}", noisy, e)),
                expected,
                "type string {:?}",
                noisy
            );
        }
    }

    /// `CHAR VARYING` is Exasol's alias for `VARCHAR`, so the `CHAR` rule must
    /// not claim it as a fixed-width `CHAR`.
    #[test]
    fn parse_exasol_type_string_treats_char_varying_as_varchar_not_char() {
        assert_eq!(
            parse_exasol_type_string("CHAR VARYING(50)").unwrap(),
            ExasolType::Varchar { size: 50 }
        );
    }

    #[test]
    fn parse_exasol_type_string_rejects_an_unrecognized_name() {
        let error = parse_exasol_type_string("QUANTUM").expect_err("unknown types must be refused");
        assert!(
            error.message.contains("Unknown Exasol type: QUANTUM"),
            "the message must name the type it refused, got: {}",
            error.message
        );
        assert_eq!(error.status, AdbcStatus::Internal);
    }

    /// The error message quotes the caller's original spelling, not the
    /// upper-cased form the parser matched against.
    #[test]
    fn parse_exasol_type_string_error_preserves_the_original_spelling() {
        let error = parse_exasol_type_string("quantum").expect_err("unknown types must be refused");
        assert!(error.message.contains("quantum"), "got: {}", error.message);
    }

    // ========================================================================
    // Column-value extraction from system-view result batches
    // ========================================================================

    #[test]
    fn get_int_value_reads_every_numeric_column_type_exasol_may_report() {
        use arrow::array::{Decimal128Array, Float64Array, Int32Array, Int64Array};

        assert_eq!(get_int_value(&Int32Array::from(vec![7]), 0), Some(7));
        assert_eq!(get_int_value(&Int64Array::from(vec![7_i64]), 0), Some(7));
        assert_eq!(
            get_int_value(&Float64Array::from(vec![7.9_f64]), 0),
            Some(7),
            "a fractional ordinal truncates toward zero"
        );
        assert_eq!(
            get_int_value(&Decimal128Array::from(vec![7_i128]), 0),
            Some(7)
        );
    }

    #[test]
    fn get_int_value_reports_null_and_unsupported_columns_as_absent() {
        use arrow::array::{Decimal128Array, Float64Array, Int32Array, Int64Array, StringArray};

        assert_eq!(get_int_value(&Int32Array::from(vec![None::<i32>]), 0), None);
        assert_eq!(get_int_value(&Int64Array::from(vec![None::<i64>]), 0), None);
        assert_eq!(
            get_int_value(&Float64Array::from(vec![None::<f64>]), 0),
            None
        );
        assert_eq!(
            get_int_value(&Decimal128Array::from(vec![None::<i128>]), 0),
            None
        );
        assert_eq!(
            get_int_value(&StringArray::from(vec!["7"]), 0),
            None,
            "a text column is not a numeric ordinal"
        );
    }

    #[test]
    fn get_int_value_reads_the_requested_row_not_the_first() {
        use arrow::array::Int32Array;

        let array = Int32Array::from(vec![Some(1), None, Some(3)]);
        assert_eq!(get_int_value(&array, 0), Some(1));
        assert_eq!(get_int_value(&array, 1), None);
        assert_eq!(get_int_value(&array, 2), Some(3));
    }

    #[test]
    fn get_nullable_value_reads_a_boolean_flag_column() {
        use arrow::array::BooleanArray;

        assert!(get_nullable_value(&BooleanArray::from(vec![true]), 0));
        assert!(!get_nullable_value(&BooleanArray::from(vec![false]), 0));
    }

    #[test]
    fn get_nullable_value_accepts_every_affirmative_spelling_of_a_text_flag() {
        use arrow::array::StringArray;

        for affirmative in ["TRUE", "true", "True", "YES", "yes", "1"] {
            assert!(
                get_nullable_value(&StringArray::from(vec![affirmative]), 0),
                "{:?} must read as nullable",
                affirmative
            );
        }
    }

    #[test]
    fn get_nullable_value_treats_any_other_text_flag_as_not_nullable() {
        use arrow::array::StringArray;

        for negative in ["FALSE", "false", "NO", "no", "0", ""] {
            assert!(
                !get_nullable_value(&StringArray::from(vec![negative]), 0),
                "{:?} must read as not nullable",
                negative
            );
        }
    }

    /// Nullable is the safe default: anything that cannot be read as a definite
    /// "not nullable" answer must not produce a NOT NULL column.
    #[test]
    fn get_nullable_value_defaults_to_nullable_for_null_and_unexpected_columns() {
        use arrow::array::{BooleanArray, Int32Array, StringArray};

        assert!(get_nullable_value(
            &BooleanArray::from(vec![None::<bool>]),
            0
        ));
        assert!(get_nullable_value(
            &StringArray::from(vec![None::<&str>]),
            0
        ));
        assert!(
            get_nullable_value(&Int32Array::from(vec![0]), 0),
            "a numeric column is not a recognized flag column"
        );
    }

    // ========================================================================
    // GetObjects: catalog filter, depth resolution, and SQL generation
    // ========================================================================

    #[test]
    fn catalog_is_exasol_accepts_no_filter_and_any_casing_of_exa() {
        assert!(catalog_is_exasol(None), "no filter must match");
        assert!(catalog_is_exasol(Some("EXA")));
        assert!(catalog_is_exasol(Some("exa")));
        assert!(catalog_is_exasol(Some("Exa")));
        assert!(!catalog_is_exasol(Some("OTHER")));
        assert!(!catalog_is_exasol(Some("")));
    }

    #[test]
    fn object_depth_levels_are_cumulative() {
        let levels = |depth| ObjectDepthLevels::from(depth);

        assert_eq!(
            levels(ObjectDepth::Catalogs),
            ObjectDepthLevels {
                schemas: false,
                tables: false,
                columns: false
            }
        );
        assert_eq!(
            levels(ObjectDepth::Schemas),
            ObjectDepthLevels {
                schemas: true,
                tables: false,
                columns: false
            }
        );
        assert_eq!(
            levels(ObjectDepth::Tables),
            ObjectDepthLevels {
                schemas: true,
                tables: true,
                columns: false
            }
        );
        let everything = ObjectDepthLevels {
            schemas: true,
            tables: true,
            columns: true,
        };
        assert_eq!(levels(ObjectDepth::Columns), everything);
        assert_eq!(
            levels(ObjectDepth::All),
            everything,
            "All is documented as identical to Columns"
        );
    }

    #[test]
    fn like_condition_doubles_single_quotes_in_the_pattern() {
        assert_eq!(
            like_condition("ROOT_NAME", "SALES"),
            "ROOT_NAME LIKE 'SALES'"
        );
        assert_eq!(
            like_condition("ROOT_NAME", "O'BRIEN"),
            "ROOT_NAME LIKE 'O''BRIEN'"
        );
    }

    #[test]
    fn append_where_clause_joins_conditions_and_omits_an_empty_clause() {
        let mut empty = "SELECT 1".to_string();
        append_where_clause(&mut empty, &[]);
        assert_eq!(empty, "SELECT 1");

        let mut single = "SELECT 1".to_string();
        append_where_clause(&mut single, &["A = 1".to_string()]);
        assert_eq!(single, "SELECT 1 WHERE A = 1");

        let mut several = "SELECT 1".to_string();
        append_where_clause(&mut several, &["A = 1".to_string(), "B = 2".to_string()]);
        assert_eq!(several, "SELECT 1 WHERE A = 1 AND B = 2");
    }

    #[test]
    fn get_objects_schemas_query_filters_only_when_a_pattern_is_given() {
        assert_eq!(
            get_objects_schemas_query(None),
            "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS ORDER BY SCHEMA_NAME"
        );
        assert_eq!(
            get_objects_schemas_query(Some("SAL%")),
            "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS WHERE SCHEMA_NAME LIKE 'SAL%' \
             ORDER BY SCHEMA_NAME"
        );
        assert!(get_objects_schemas_query(Some("O'B")).contains("LIKE 'O''B'"));
    }

    #[test]
    fn get_objects_tables_query_always_restricts_to_tables_and_views() {
        assert_eq!(
            get_objects_tables_query(None, None, None),
            "SELECT OBJECT_NAME, OBJECT_TYPE, ROOT_NAME FROM SYS.EXA_ALL_OBJECTS \
             WHERE OBJECT_TYPE IN ('TABLE', 'VIEW') ORDER BY ROOT_NAME, OBJECT_NAME"
        );
    }

    #[test]
    fn get_objects_tables_query_conjoins_every_supplied_filter() {
        let sql = get_objects_tables_query(Some("SAL%"), Some("ORD%"), Some(&["TABLE"]));

        assert_eq!(
            sql,
            "SELECT OBJECT_NAME, OBJECT_TYPE, ROOT_NAME FROM SYS.EXA_ALL_OBJECTS \
             WHERE OBJECT_TYPE IN ('TABLE', 'VIEW') AND ROOT_NAME LIKE 'SAL%' \
             AND OBJECT_NAME LIKE 'ORD%' AND OBJECT_TYPE IN ('TABLE') \
             ORDER BY ROOT_NAME, OBJECT_NAME"
        );
    }

    #[test]
    fn get_objects_tables_query_quotes_every_requested_table_type() {
        let sql = get_objects_tables_query(None, None, Some(&["TABLE", "VIEW"]));
        assert!(
            sql.contains("AND OBJECT_TYPE IN ('TABLE','VIEW')"),
            "got: {}",
            sql
        );

        let escaped = get_objects_tables_query(None, None, Some(&["O'DD"]));
        assert!(escaped.contains("IN ('O''DD')"), "got: {}", escaped);
    }

    #[test]
    fn get_objects_columns_query_filters_only_on_what_is_supplied() {
        assert_eq!(
            get_objects_columns_query(None, None, None),
            "SELECT COLUMN_NAME, COLUMN_ORDINAL_POSITION, COLUMN_TYPE, COLUMN_SCHEMA, \
             COLUMN_TABLE FROM SYS.EXA_ALL_COLUMNS \
             ORDER BY COLUMN_SCHEMA, COLUMN_TABLE, COLUMN_ORDINAL_POSITION"
        );

        let filtered = get_objects_columns_query(Some("S%"), Some("T%"), Some("C%"));
        assert!(
            filtered.contains(
                "WHERE COLUMN_SCHEMA LIKE 'S%' AND COLUMN_TABLE LIKE 'T%' \
                 AND COLUMN_NAME LIKE 'C%'"
            ),
            "got: {}",
            filtered
        );
    }

    // ========================================================================
    // GetObjects: reading system-view batches
    // ========================================================================

    fn utf8_batch(columns: Vec<(&str, Vec<Option<&str>>)>) -> RecordBatch {
        use arrow::array::StringArray;
        use arrow::datatypes::{DataType, Field};

        let fields: Vec<Field> = columns
            .iter()
            .map(|(name, _)| Field::new(*name, DataType::Utf8, true))
            .collect();
        let arrays: Vec<Arc<dyn Array>> = columns
            .iter()
            .map(|(_, values)| Arc::new(StringArray::from(values.clone())) as Arc<dyn Array>)
            .collect();
        RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).expect("test batch")
    }

    #[test]
    fn string_column_names_the_field_when_the_column_is_not_text() {
        use arrow::array::Int32Array;
        use arrow::datatypes::{DataType, Field};

        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("N", DataType::Int32, true)])),
            vec![Arc::new(Int32Array::from(vec![1]))],
        )
        .expect("test batch");

        let error =
            string_column(&batch, 0, "SCHEMA_NAME").expect_err("must refuse a non-text column");
        assert!(
            error
                .message
                .contains("Expected string column for SCHEMA_NAME"),
            "got: {}",
            error.message
        );
    }

    #[test]
    fn schema_names_from_collects_names_across_batches_and_skips_nulls() {
        let batches = vec![
            utf8_batch(vec![("SCHEMA_NAME", vec![Some("A"), None, Some("B")])]),
            utf8_batch(vec![("SCHEMA_NAME", vec![Some("C")])]),
        ];

        assert_eq!(
            schema_names_from(&batches).expect("read schemas"),
            vec!["A".to_string(), "B".to_string(), "C".to_string()]
        );
    }

    #[test]
    fn schema_names_from_returns_nothing_for_no_batches() {
        assert!(schema_names_from(&[]).expect("read schemas").is_empty());
    }

    #[test]
    fn tables_by_schema_from_groups_tables_under_their_owning_schema() {
        let batches = vec![utf8_batch(vec![
            (
                "OBJECT_NAME",
                vec![Some("ORDERS"), Some("ITEMS"), Some("V")],
            ),
            (
                "OBJECT_TYPE",
                vec![Some("TABLE"), Some("TABLE"), Some("VIEW")],
            ),
            (
                "ROOT_NAME",
                vec![Some("SALES"), Some("SALES"), Some("OTHER")],
            ),
        ])];

        let tables = tables_by_schema_from(&batches).expect("read tables");

        assert_eq!(
            tables.get("SALES"),
            Some(&vec![
                ("ORDERS".to_string(), "TABLE".to_string()),
                ("ITEMS".to_string(), "TABLE".to_string())
            ]),
            "order within a schema must follow the query's ORDER BY"
        );
        assert_eq!(
            tables.get("OTHER"),
            Some(&vec![("V".to_string(), "VIEW".to_string())])
        );
    }

    #[test]
    fn columns_by_table_from_groups_columns_under_their_owning_table() {
        use arrow::array::{Int32Array, StringArray};
        use arrow::datatypes::{DataType, Field};

        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("COLUMN_NAME", DataType::Utf8, true),
                Field::new("COLUMN_ORDINAL_POSITION", DataType::Int32, true),
                Field::new("COLUMN_TYPE", DataType::Utf8, true),
                Field::new("COLUMN_SCHEMA", DataType::Utf8, true),
                Field::new("COLUMN_TABLE", DataType::Utf8, true),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["ID", "NAME"])),
                Arc::new(Int32Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec!["DECIMAL(18,0)", "VARCHAR(50)"])),
                Arc::new(StringArray::from(vec!["SALES", "SALES"])),
                Arc::new(StringArray::from(vec!["ORDERS", "ORDERS"])),
            ],
        )
        .expect("test batch");

        let columns = columns_by_table_from(&[batch]).expect("read columns");

        assert_eq!(
            columns.get(&("SALES".to_string(), "ORDERS".to_string())),
            Some(&vec![
                ("ID".to_string(), 1, "DECIMAL(18,0)".to_string()),
                ("NAME".to_string(), 2, "VARCHAR(50)".to_string())
            ])
        );
    }

    /// A NULL ordinal falls back to the row's own 1-based position so column
    /// order is never lost.
    #[test]
    fn columns_by_table_from_falls_back_to_the_row_position_for_a_null_ordinal() {
        use arrow::array::{Int32Array, StringArray};
        use arrow::datatypes::{DataType, Field};

        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("COLUMN_NAME", DataType::Utf8, true),
                Field::new("COLUMN_ORDINAL_POSITION", DataType::Int32, true),
                Field::new("COLUMN_TYPE", DataType::Utf8, true),
                Field::new("COLUMN_SCHEMA", DataType::Utf8, true),
                Field::new("COLUMN_TABLE", DataType::Utf8, true),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["ID", "NAME"])),
                Arc::new(Int32Array::from(vec![None::<i32>, None::<i32>])),
                Arc::new(StringArray::from(vec!["BOOLEAN", "BOOLEAN"])),
                Arc::new(StringArray::from(vec!["S", "S"])),
                Arc::new(StringArray::from(vec!["T", "T"])),
            ],
        )
        .expect("test batch");

        let columns = columns_by_table_from(&[batch]).expect("read columns");
        let ordinals: Vec<i32> = columns[&("S".to_string(), "T".to_string())]
            .iter()
            .map(|(_, ordinal, _)| *ordinal)
            .collect();

        assert_eq!(ordinals, vec![1, 2]);
    }

    // ========================================================================
    // GetObjects: assembling the nested result batch
    // ========================================================================

    fn only_schema_list(batch: &RecordBatch) -> arrow::array::ListArray {
        batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
            .expect("catalog_db_schemas must be a list")
            .clone()
    }

    /// The `db_schema_name` values of the single catalog row.
    fn schema_names_in(batch: &RecordBatch) -> Vec<String> {
        let entries = only_schema_list(batch).value(0);
        let entries = entries
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .expect("schema entries must be structs");
        let names = entries
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("db_schema_name must be text");
        (0..names.len())
            .map(|i| names.value(i).to_string())
            .collect()
    }

    /// The `table_name` values nested under schema entry `schema_index`.
    fn table_names_in(batch: &RecordBatch, schema_index: usize) -> Vec<String> {
        let entries = only_schema_list(batch).value(0);
        let entries = entries
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .expect("schema entries must be structs");
        let tables_list = entries
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
            .expect("db_schema_tables must be a list");
        let tables = tables_list.value(schema_index);
        let tables = tables
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .expect("table entries must be structs");
        let names = tables
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("table_name must be text");
        (0..names.len())
            .map(|i| names.value(i).to_string())
            .collect()
    }

    fn one_schema_with_one_table() -> (Vec<String>, TablesBySchema, ColumnMetadataMap) {
        let schemas = vec!["SALES".to_string()];
        let mut tables = TablesBySchema::new();
        tables.insert(
            "SALES".to_string(),
            vec![("ORDERS".to_string(), "TABLE".to_string())],
        );
        let mut columns = ColumnMetadataMap::new();
        columns.insert(
            ("SALES".to_string(), "ORDERS".to_string()),
            vec![("ID".to_string(), 1, "DECIMAL(18,0)".to_string())],
        );
        (schemas, tables, columns)
    }

    #[test]
    fn build_get_objects_batch_always_reports_the_single_exa_catalog() {
        let (schemas, tables, columns) = one_schema_with_one_table();

        let batch = build_get_objects_batch(
            &schemas,
            &tables,
            &columns,
            ObjectDepthLevels::from(ObjectDepth::All),
        )
        .expect("build batch");

        assert_eq!(batch.num_rows(), 1, "Exasol exposes exactly one catalog");
        let catalog = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("catalog_name must be text");
        assert_eq!(catalog.value(0), "EXA");
        assert_eq!(batch.schema().as_ref(), &build_get_objects_schema());
    }

    /// At catalog depth the schema list is null rather than empty, so a consumer
    /// can tell "not requested" from "none exist".
    #[test]
    fn build_get_objects_batch_leaves_the_schema_list_null_at_catalog_depth() {
        let (schemas, tables, columns) = one_schema_with_one_table();

        let batch = build_get_objects_batch(
            &schemas,
            &tables,
            &columns,
            ObjectDepthLevels::from(ObjectDepth::Catalogs),
        )
        .expect("build batch");

        assert!(only_schema_list(&batch).is_null(0));
    }

    #[test]
    fn build_get_objects_batch_lists_schemas_at_schema_depth() {
        let (schemas, tables, columns) = one_schema_with_one_table();

        let batch = build_get_objects_batch(
            &schemas,
            &tables,
            &columns,
            ObjectDepthLevels::from(ObjectDepth::Schemas),
        )
        .expect("build batch");

        assert_eq!(schema_names_in(&batch), vec!["SALES".to_string()]);
    }

    /// At schema depth the nested table list is null, not an empty list.
    #[test]
    fn build_get_objects_batch_leaves_the_table_list_null_at_schema_depth() {
        let (schemas, tables, columns) = one_schema_with_one_table();

        let batch = build_get_objects_batch(
            &schemas,
            &tables,
            &columns,
            ObjectDepthLevels::from(ObjectDepth::Schemas),
        )
        .expect("build batch");

        let entries = only_schema_list(&batch).value(0);
        let entries = entries
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .expect("schema entries must be structs");
        let tables_list = entries
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
            .expect("db_schema_tables must be a list");
        assert!(tables_list.is_null(0));
    }

    #[test]
    fn build_get_objects_batch_nests_tables_under_their_schema_at_table_depth() {
        let (schemas, tables, columns) = one_schema_with_one_table();

        let batch = build_get_objects_batch(
            &schemas,
            &tables,
            &columns,
            ObjectDepthLevels::from(ObjectDepth::Tables),
        )
        .expect("build batch");

        assert_eq!(table_names_in(&batch, 0), vec!["ORDERS".to_string()]);
    }

    #[test]
    fn build_get_objects_batch_nests_columns_under_their_table_at_column_depth() {
        let (schemas, tables, columns) = one_schema_with_one_table();

        let batch = build_get_objects_batch(
            &schemas,
            &tables,
            &columns,
            ObjectDepthLevels::from(ObjectDepth::Columns),
        )
        .expect("build batch");

        let entries = only_schema_list(&batch).value(0);
        let entries = entries
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .expect("schema entries must be structs");
        let tables_list = entries
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
            .expect("db_schema_tables must be a list");
        let table_entries = tables_list.value(0);
        let table_entries = table_entries
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .expect("table entries must be structs");
        let columns_list = table_entries
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
            .expect("table_columns must be a list");
        let column_entries = columns_list.value(0);
        let column_entries = column_entries
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .expect("column entries must be structs");

        let names = column_entries
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("column_name must be text");
        assert_eq!(names.value(0), "ID");

        let ordinals = column_entries
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .expect("ordinal_position must be int32");
        assert_eq!(ordinals.value(0), 1);

        let types = column_entries
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("xdbc_type_name must be text");
        assert_eq!(types.value(0), "DECIMAL(18,0)");

        let constraints = table_entries
            .column(3)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
            .expect("table_constraints must be a list");
        assert!(
            constraints.is_null(0),
            "Exasol reports no constraints through GetObjects"
        );
    }

    /// A schema with no tables still produces a schema entry carrying an empty —
    /// not null — table list, since tables were requested.
    #[test]
    fn build_get_objects_batch_emits_an_empty_table_list_for_a_schema_without_tables() {
        let batch = build_get_objects_batch(
            &["EMPTY".to_string()],
            &TablesBySchema::new(),
            &ColumnMetadataMap::new(),
            ObjectDepthLevels::from(ObjectDepth::Tables),
        )
        .expect("build batch");

        assert_eq!(schema_names_in(&batch), vec!["EMPTY".to_string()]);
        assert!(table_names_in(&batch, 0).is_empty());
    }

    #[test]
    fn build_get_objects_batch_emits_an_empty_schema_list_when_no_schema_matches() {
        let batch = build_get_objects_batch(
            &[],
            &TablesBySchema::new(),
            &ColumnMetadataMap::new(),
            ObjectDepthLevels::from(ObjectDepth::All),
        )
        .expect("build batch");

        let schema_list = only_schema_list(&batch);
        assert!(
            !schema_list.is_null(0),
            "the list must be present but empty, distinguishing it from 'not requested'"
        );
        assert!(schema_names_in(&batch).is_empty());
    }

    // ========================================================================
    // Connection options
    //
    // Every case here is reachable without a server: an option that would need
    // one (an autocommit change that opens or closes a transaction) is covered
    // by the driver-manager suite instead.
    // ========================================================================

    fn unconnected_connection() -> FfiConnection {
        FfiConnection::new("exasol://user@localhost:8563".to_string())
    }

    #[test]
    fn set_option_stores_the_current_schema() {
        let mut conn = unconnected_connection();

        conn.set_option(OptionConnection::CurrentSchema, "SALES".into())
            .expect("set schema");

        assert_eq!(
            conn.get_option_string(OptionConnection::CurrentSchema)
                .expect("get schema"),
            "SALES"
        );
    }

    #[test]
    fn set_option_rejects_a_non_string_current_schema() {
        let mut conn = unconnected_connection();

        let error = conn
            .set_option(OptionConnection::CurrentSchema, OptionValue::Int(1))
            .expect_err("a non-string schema must be refused");

        assert_eq!(error.status, AdbcStatus::InvalidArguments);
        assert!(
            error.message.contains("CurrentSchema must be a string"),
            "got: {}",
            error.message
        );
    }

    #[test]
    fn set_option_rejects_a_non_string_auto_commit() {
        let mut conn = unconnected_connection();

        let error = conn
            .set_option(OptionConnection::AutoCommit, OptionValue::Int(0))
            .expect_err("a non-string autocommit must be refused");

        assert_eq!(error.status, AdbcStatus::InvalidArguments);
        assert!(
            error
                .message
                .contains("AutoCommit must be a string ('true' or 'false')"),
            "got: {}",
            error.message
        );
    }

    /// Re-asserting the autocommit value already in force changes no
    /// transaction state, so it must not dial the server.
    #[test]
    fn set_option_auto_commit_to_the_value_already_in_force_needs_no_connection() {
        let mut conn = unconnected_connection();

        conn.set_option(OptionConnection::AutoCommit, "true".into())
            .expect("re-asserting the current value must succeed offline");

        assert!(conn.auto_commit);
        assert!(
            conn.inner.is_none(),
            "no connection may be opened for a no-op option change"
        );
    }

    #[test]
    fn set_option_refuses_a_current_catalog_because_exasol_has_none() {
        let mut conn = unconnected_connection();

        let error = conn
            .set_option(OptionConnection::CurrentCatalog, "OTHER".into())
            .expect_err("catalogs must be refused");

        assert_eq!(error.status, AdbcStatus::NotImplemented);
        assert!(
            error.message.contains("Exasol does not support catalogs"),
            "got: {}",
            error.message
        );
    }

    #[test]
    fn set_option_keeps_read_only_and_isolation_level_as_opaque_values() {
        let mut conn = unconnected_connection();

        conn.set_option(OptionConnection::ReadOnly, "true".into())
            .expect("set read only");
        conn.set_option(OptionConnection::IsolationLevel, "serializable".into())
            .expect("set isolation level");

        let stored = |key: OptionConnection| match conn.options.get(key.as_ref()) {
            Some(OptionValue::String(value)) => value.clone(),
            other => panic!("expected a stored string, got {:?}", other),
        };
        assert_eq!(stored(OptionConnection::ReadOnly), "true");
        assert_eq!(stored(OptionConnection::IsolationLevel), "serializable");
    }

    #[test]
    fn set_option_stores_a_vendor_specific_option_under_its_own_key() {
        let mut conn = unconnected_connection();

        conn.set_option(
            OptionConnection::Other("exasol.custom".to_string()),
            "on".into(),
        )
        .expect("set custom option");

        assert_eq!(
            conn.get_option_string(OptionConnection::Other("exasol.custom".to_string()))
                .expect("get custom option"),
            "on"
        );
    }

    #[test]
    fn get_option_string_reports_auto_commit_as_a_boolean_word() {
        let mut conn = unconnected_connection();
        assert_eq!(
            conn.get_option_string(OptionConnection::AutoCommit)
                .expect("get autocommit"),
            "true"
        );

        conn.auto_commit = false;
        assert_eq!(
            conn.get_option_string(OptionConnection::AutoCommit)
                .expect("get autocommit"),
            "false"
        );
    }

    #[test]
    fn get_option_string_reports_the_single_exa_catalog() {
        assert_eq!(
            unconnected_connection()
                .get_option_string(OptionConnection::CurrentCatalog)
                .expect("get catalog"),
            "EXA"
        );
    }

    #[test]
    fn get_option_string_reports_an_unset_schema_as_not_found() {
        let error = unconnected_connection()
            .get_option_string(OptionConnection::CurrentSchema)
            .expect_err("an unset schema must not be reported as empty");

        assert_eq!(error.status, AdbcStatus::NotFound);
    }

    #[test]
    fn get_option_string_reports_an_unknown_vendor_option_as_not_found() {
        let error = unconnected_connection()
            .get_option_string(OptionConnection::Other("absent".to_string()))
            .expect_err("an unset option must be reported missing");

        assert_eq!(error.status, AdbcStatus::NotFound);
        assert!(
            error.message.contains("Option absent not found"),
            "got: {}",
            error.message
        );
    }

    #[test]
    fn typed_option_getters_return_only_values_stored_with_a_matching_type() {
        let mut conn = unconnected_connection();
        conn.set_option(
            OptionConnection::Other("b".to_string()),
            OptionValue::Bytes(vec![1, 2]),
        )
        .expect("set bytes");
        conn.set_option(
            OptionConnection::Other("i".to_string()),
            OptionValue::Int(9),
        )
        .expect("set int");
        conn.set_option(
            OptionConnection::Other("d".to_string()),
            OptionValue::Double(1.5),
        )
        .expect("set double");

        assert_eq!(
            conn.get_option_bytes(OptionConnection::Other("b".to_string()))
                .expect("get bytes"),
            vec![1, 2]
        );
        assert_eq!(
            conn.get_option_int(OptionConnection::Other("i".to_string()))
                .expect("get int"),
            9
        );
        assert_eq!(
            conn.get_option_double(OptionConnection::Other("d".to_string()))
                .expect("get double"),
            1.5
        );

        // A value stored under one type is not readable as another.
        assert!(conn
            .get_option_int(OptionConnection::Other("b".to_string()))
            .is_err());
        assert!(conn
            .get_option_bytes(OptionConnection::Other("i".to_string()))
            .is_err());
        assert!(conn
            .get_option_double(OptionConnection::Other("i".to_string()))
            .is_err());
    }

    #[test]
    fn typed_option_getters_refuse_the_well_known_option_keys() {
        let conn = unconnected_connection();

        assert!(conn.get_option_bytes(OptionConnection::AutoCommit).is_err());
        assert!(conn.get_option_int(OptionConnection::AutoCommit).is_err());
        assert!(conn
            .get_option_double(OptionConnection::AutoCommit)
            .is_err());
    }

    // ========================================================================
    // GetTableSchema
    // ========================================================================

    #[test]
    fn table_schema_query_matches_the_table_name_exactly() {
        assert_eq!(
            table_schema_query(None, "ORDERS"),
            "SELECT COLUMN_NAME, COLUMN_TYPE, COLUMN_MAXSIZE, COLUMN_NUM_PREC, \
             COLUMN_NUM_SCALE, COLUMN_IS_NULLABLE FROM SYS.EXA_ALL_COLUMNS \
             WHERE COLUMN_TABLE = 'ORDERS' ORDER BY COLUMN_ORDINAL_POSITION"
        );
    }

    #[test]
    fn table_schema_query_adds_the_schema_when_one_is_given() {
        let sql = table_schema_query(Some("SALES"), "ORDERS");
        assert!(
            sql.contains("WHERE COLUMN_TABLE = 'ORDERS' AND COLUMN_SCHEMA = 'SALES'"),
            "got: {}",
            sql
        );
    }

    #[test]
    fn table_schema_query_doubles_single_quotes_in_both_names() {
        let sql = table_schema_query(Some("S'CH"), "O'RD");
        assert!(sql.contains("COLUMN_TABLE = 'O''RD'"), "got: {}", sql);
        assert!(sql.contains("COLUMN_SCHEMA = 'S''CH'"), "got: {}", sql);
    }

    /// One row of the `GetTableSchema` metadata query: name, type, and the three
    /// numeric columns it does not read, then the nullability flag.
    fn table_schema_batch(rows: Vec<(&str, &str, bool)>) -> RecordBatch {
        use arrow::array::{BooleanArray, Int32Array, StringArray};
        use arrow::datatypes::{DataType, Field};

        let names: Vec<&str> = rows.iter().map(|(name, _, _)| *name).collect();
        let types: Vec<&str> = rows.iter().map(|(_, type_name, _)| *type_name).collect();
        let nullable: Vec<bool> = rows.iter().map(|(_, _, flag)| *flag).collect();
        let unused: Vec<i32> = rows.iter().map(|_| 0).collect();

        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("COLUMN_NAME", DataType::Utf8, true),
                Field::new("COLUMN_TYPE", DataType::Utf8, true),
                Field::new("COLUMN_MAXSIZE", DataType::Int32, true),
                Field::new("COLUMN_NUM_PREC", DataType::Int32, true),
                Field::new("COLUMN_NUM_SCALE", DataType::Int32, true),
                Field::new("COLUMN_IS_NULLABLE", DataType::Boolean, true),
            ])),
            vec![
                Arc::new(StringArray::from(names)),
                Arc::new(StringArray::from(types)),
                Arc::new(Int32Array::from(unused.clone())),
                Arc::new(Int32Array::from(unused.clone())),
                Arc::new(Int32Array::from(unused)),
                Arc::new(BooleanArray::from(nullable)),
            ],
        )
        .expect("test batch")
    }

    #[test]
    fn table_schema_fields_from_maps_each_column_to_an_arrow_field() {
        use arrow::datatypes::DataType;

        let batch = table_schema_batch(vec![
            ("ID", "DECIMAL(18,0)", false),
            ("NAME", "VARCHAR(50)", true),
            ("ACTIVE", "BOOLEAN", true),
        ]);

        let fields = table_schema_fields_from(&[batch]).expect("map fields");

        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name(), "ID");
        assert!(!fields[0].is_nullable());
        assert_eq!(fields[1].name(), "NAME");
        assert!(fields[1].is_nullable());
        assert_eq!(fields[1].data_type(), &DataType::Utf8);
        assert_eq!(fields[2].data_type(), &DataType::Boolean);
    }

    /// An empty result carries no fields, which is how the caller recognizes a
    /// table that does not exist.
    #[test]
    fn table_schema_fields_from_yields_nothing_for_an_empty_result() {
        assert!(table_schema_fields_from(&[])
            .expect("map fields")
            .is_empty());
    }

    #[test]
    fn table_schema_fields_from_refuses_a_column_of_an_unknown_type() {
        let batch = table_schema_batch(vec![("ID", "QUANTUM", true)]);

        let error = table_schema_fields_from(&[batch]).expect_err("unknown types must be refused");
        assert!(
            error.message.contains("Unknown Exasol type: QUANTUM"),
            "got: {}",
            error.message
        );
    }

    // ========================================================================
    // Statement result assembly
    // ========================================================================

    #[test]
    fn reader_over_batches_reports_an_empty_schema_for_no_batches() {
        let mut reader = reader_over_batches(vec![]);

        assert_eq!(reader.schema().fields().len(), 0);
        assert!(reader.next().is_none());
    }

    #[test]
    fn reader_over_batches_takes_its_schema_from_the_first_batch() {
        let batch = utf8_batch(vec![("NAME", vec![Some("A")])]);
        let expected = batch.schema();

        let mut reader = reader_over_batches(vec![batch]);

        assert_eq!(reader.schema(), expected);
        assert_eq!(
            reader.next().expect("one batch").expect("valid").num_rows(),
            1
        );
        assert!(reader.next().is_none());
    }

    #[test]
    fn reader_over_batches_yields_every_batch_in_order() {
        let batches = vec![
            utf8_batch(vec![("NAME", vec![Some("A")])]),
            utf8_batch(vec![("NAME", vec![Some("B"), Some("C")])]),
        ];

        let reader = reader_over_batches(batches);
        let row_counts: Vec<usize> = reader
            .map(|batch| batch.expect("valid").num_rows())
            .collect();

        assert_eq!(row_counts, vec![1, 2]);
    }

    #[test]
    fn statement_is_bulk_ingest_only_when_a_target_table_is_configured() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        assert!(!stmt.is_bulk_ingest());

        stmt.options
            .insert(INGEST_TARGET_TABLE_OPTION.to_string(), "T".into());
        assert!(stmt.is_bulk_ingest());
    }

    #[test]
    fn statement_require_sql_reports_an_unset_query() {
        let stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());

        let error = stmt
            .require_sql()
            .expect_err("an unset query must be refused");
        assert_eq!(error.status, AdbcStatus::InvalidState);
        assert!(
            error.message.contains("SQL query not set"),
            "got: {}",
            error.message
        );
    }

    #[test]
    fn statement_require_sql_returns_the_configured_query() {
        let mut stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());
        stmt.set_sql_query("SELECT 1").expect("set query");

        assert_eq!(stmt.require_sql().expect("get query"), "SELECT 1");
    }

    #[test]
    fn statement_require_connection_reports_a_standalone_statement() {
        let stmt = FfiStatement::new("exasol://user@localhost:8563".to_string());

        let error = stmt
            .require_connection()
            .expect_err("a statement with no parent connection must say so");
        assert_eq!(error.status, AdbcStatus::InvalidState);
        assert!(
            error.message.contains("No connection available"),
            "got: {}",
            error.message
        );
    }
}
