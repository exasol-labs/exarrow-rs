//! Type mapping between Exasol and Arrow data types.

pub(crate) mod conversion;
mod infer;
mod mapping;
mod schema;

pub use infer::{
    format_column_name, infer_schema_from_parquet, infer_schema_from_parquet_files,
    quote_column_name, quote_identifier, sanitize_column_name, widen_type, InferredColumn,
    InferredTableSchema,
};
pub use mapping::{ColumnNameMode, ExasolType, TypeMapper};
pub use schema::{ColumnMetadata, SchemaBuilder};
