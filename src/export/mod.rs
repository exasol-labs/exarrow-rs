//! Export functionality for exporting data from Exasol.
//!
//! This module provides high-level export functions for exporting data from Exasol tables
//! or query results to various destinations including files, streams, in-memory lists,
//! and Arrow RecordBatches.
//!
//! # Architecture
//!
//! Export works using Exasol's HTTP transport protocol:
//! 1. Start an HTTP server on a local port
//! 2. Execute EXPORT SQL statement pointing to our server
//! 3. Exasol connects TO our server and sends data
//! 4. Receive and process the data stream (CSV)
//! 5. Optionally convert CSV to Arrow RecordBatches
//!
//! # Example - CSV Export
//!
//! ```ignore
//! use exarrow_rs::export::{export_to_file, CsvExportOptions};
//! use exarrow_rs::query::export::ExportSource;
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let mut ws_transport = todo!();
//! // Export a table to a file
//! let rows_exported = export_to_file(
//!     &mut ws_transport,
//!     ExportSource::Table {
//!         schema: Some("my_schema".to_string()),
//!         name: "users".to_string(),
//!         columns: vec![],
//!     },
//!     Path::new("/tmp/users.csv"),
//!     CsvExportOptions::default(),
//! ).await?;
//!
//! println!("Exported {} rows", rows_exported);
//! # Ok(())
//! # }
//! ```
//!
//! # Example - Arrow Export
//!
//! ```ignore
//! use exarrow_rs::export::arrow::{ArrowExportOptions, CsvToArrowReader};
//! use arrow::datatypes::{Schema, Field, DataType};
//! use std::sync::Arc;
//!
//! // Create a schema
//! let schema = Arc::new(Schema::new(vec![
//!     Field::new("id", DataType::Int64, false),
//!     Field::new("name", DataType::Utf8, true),
//! ]));
//!
//! // Create options
//! let options = ArrowExportOptions::default()
//!     .with_batch_size(2048)
//!     .with_schema(schema);
//! ```

pub mod arrow;
pub mod csv;
pub mod parquet;

// Re-export CSV types for convenience
pub use csv::{
    export_to_callback, export_to_file, export_to_list, export_to_stream, CsvExportOptions,
    DataPipeReceiver, ExportError,
};

// Re-export Parquet types for convenience
pub use parquet::{
    csv_to_record_batches, exasol_types_to_arrow_schema, export_to_parquet,
    export_to_parquet_stream, export_to_parquet_via_transport, ParquetCompression,
    ParquetExportError, ParquetExportOptions,
};

// Re-export Arrow types for convenience
pub use arrow::{
    export_to_arrow_ipc, export_to_record_batches, ArrowExportOptions, CsvToArrowReader,
    ExportError as ArrowExportError,
};
