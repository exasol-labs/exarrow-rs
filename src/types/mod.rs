//! Type mapping between Exasol and Arrow data types.

pub(crate) mod conversion;
mod csv_infer;
mod infer;
mod mapping;
#[cfg(feature = "parquet")]
mod parquet_infer;
mod schema;

pub use csv_infer::{infer_schema_from_csv, infer_schema_from_csv_files, CsvInferenceOptions};
pub use infer::{
    format_column_name, quote_column_name, quote_identifier, sanitize_column_name, widen_type,
    InferredColumn, InferredTableSchema,
};
pub use mapping::{ColumnNameMode, ExasolType, TypeMapper};
#[cfg(feature = "parquet")]
pub use parquet_infer::{infer_schema_from_parquet, infer_schema_from_parquet_files};
pub use schema::{ColumnMetadata, SchemaBuilder};
