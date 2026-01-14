//! # exarrow-rs
//!
//! ADBC-compatible driver for Exasol with Apache Arrow data format support.
//!
//! This library provides high-performance database connectivity for Exasol using
//! the ADBC (Arrow Database Connectivity) interface. It enables efficient data transfer
//! using the Apache Arrow columnar format, making it ideal for analytical workloads
//! and data science applications.
//!
//! ## Features
//!
//! - **ADBC Interface**: Standard Arrow Database Connectivity driver
//! - **Query Execution**: Execute SQL queries and retrieve results as Arrow RecordBatches
//! - **Bulk Import**: Import data from CSV, Parquet, and Arrow RecordBatches
//! - **Bulk Export**: Export data to CSV, Parquet, and Arrow RecordBatches
//! - **Streaming**: Memory-efficient streaming for large datasets
//! - **Compression**: Support for gzip, bzip2, snappy, lz4, and zstd compression
//!
//! ## Query Example
//!
//! ```no_run
//! use exarrow_rs::*;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create driver and open database
//! let driver = Driver::new();
//! let database = driver.open("exasol://sys:exasol@localhost:8563")?;
//!
//! // Connect to the database
//! let mut connection = database.connect().await?;
//!
//! // Execute a query
//! let results = connection
//!     .query("SELECT * FROM my_table")
//!     .await?;
//!
//! // Process Arrow RecordBatch
//! for batch in results {
//!     println!("Rows: {}", batch.num_rows());
//! }
//!
//! // Close connection
//! connection.close().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## CSV Import Example
//!
//! ```no_run
//! use exarrow_rs::*;
//! use std::path::Path;
//!
//! # async fn example(connection: &mut Connection) -> Result<(), Box<dyn std::error::Error>> {
//! // Import CSV data into a table
//! let options = CsvImportOptions::default()
//!     .exasol_host("localhost")
//!     .exasol_port(8563);
//!
//! let rows_imported = connection.import_csv_from_file(
//!     "my_table",
//!     Path::new("/path/to/data.csv"),
//!     options,
//! ).await?;
//! println!("Imported {} rows", rows_imported);
//! # Ok(())
//! # }
//! ```
//!
//! ## CSV Export Example
//!
//! ```no_run
//! use exarrow_rs::*;
//! use std::path::Path;
//!
//! # async fn example(connection: &mut Connection) -> Result<(), Box<dyn std::error::Error>> {
//! // Export table data to CSV
//! let options = CsvExportOptions::default()
//!     .exasol_host("localhost")
//!     .exasol_port(8563);
//!
//! let rows_exported = connection.export_csv_to_file(
//!     ExportSource::Table {
//!         schema: None,
//!         name: "my_table".to_string(),
//!         columns: vec![],
//!     },
//!     Path::new("/tmp/export.csv"),
//!     options,
//! ).await?;
//! println!("Exported {} rows", rows_exported);
//! # Ok(())
//! # }
//! ```
//!
//! ## Parquet Import/Export Example
//!
//! ```no_run
//! use exarrow_rs::*;
//! use std::path::Path;
//!
//! # async fn example(connection: &mut Connection) -> Result<(), Box<dyn std::error::Error>> {
//! // Import from Parquet
//! let import_opts = ParquetImportOptions::default()
//!     .with_exasol_host("localhost")
//!     .with_exasol_port(8563);
//!
//! let rows = connection.import_from_parquet(
//!     "my_table",
//!     Path::new("/path/to/data.parquet"),
//!     import_opts,
//! ).await?;
//!
//! // Export to Parquet
//! let export_opts = ParquetExportOptions::default()
//!     .exasol_host("localhost")
//!     .exasol_port(8563)
//!     .with_compression(ParquetCompression::Snappy);
//!
//! let rows = connection.export_to_parquet(
//!     ExportSource::Table {
//!         schema: None,
//!         name: "my_table".to_string(),
//!         columns: vec![],
//!     },
//!     Path::new("/tmp/export.parquet"),
//!     export_opts,
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Arrow RecordBatch Import/Export Example
//!
//! ```no_run
//! use exarrow_rs::*;
//! use arrow::array::RecordBatch;
//! use std::path::Path;
//!
//! # async fn example(connection: &mut Connection, batch: RecordBatch) -> Result<(), Box<dyn std::error::Error>> {
//! // Import Arrow RecordBatch
//! let import_opts = ArrowImportOptions::default()
//!     .exasol_host("localhost")
//!     .exasol_port(8563);
//!
//! let rows = connection.import_from_record_batch(
//!     "my_table",
//!     &batch,
//!     import_opts,
//! ).await?;
//!
//! // Export to Arrow IPC file
//! let export_opts = ArrowExportOptions::default()
//!     .exasol_host("localhost")
//!     .exasol_port(8563)
//!     .with_batch_size(2048);
//!
//! let rows = connection.export_to_arrow_ipc(
//!     ExportSource::Table {
//!         schema: None,
//!         name: "my_table".to_string(),
//!         columns: vec![],
//!     },
//!     Path::new("/tmp/export.arrow"),
//!     export_opts,
//! ).await?;
//! # Ok(())
//! # }
//! ```

// Module declarations
pub mod adbc;
pub mod arrow_conversion;
pub mod connection;
pub mod error;
pub mod export;
pub mod import;
pub mod query;
pub mod transport;
pub mod types;

// FFI module for C-compatible ADBC export (conditionally compiled)
#[cfg(feature = "ffi")]
pub mod adbc_ffi;

// =============================================================================
// ADBC Interface Types
// =============================================================================

/// Re-export ADBC driver and connection types.
pub use adbc::{Connection, Database, Driver, Statement};

// =============================================================================
// Arrow Conversion
// =============================================================================

/// Re-export Arrow conversion utilities.
pub use arrow_conversion::ArrowConverter;

// =============================================================================
// Error Types
// =============================================================================

/// Re-export error types for convenient error handling.
pub use error::{ConnectionError, ConversionError, ExasolError, QueryError};

// =============================================================================
// Type System
// =============================================================================

/// Re-export type mapping utilities.
pub use types::{ExasolType, TypeMapper};

// =============================================================================
// Import Types
// =============================================================================

/// CSV import options and functions.
///
/// The CSV import module provides functionality for importing data from CSV files,
/// streams, iterators, and custom callbacks into Exasol tables.
///
/// # Example
///
/// ```no_run
/// use exarrow_rs::import::{CsvImportOptions, import_from_file};
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let options = CsvImportOptions::default()
///     .exasol_host("localhost")
///     .exasol_port(8563)
///     .column_separator(';')
///     .skip_rows(1);
///
/// // Use via Session or Connection for execution
/// # Ok(())
/// # }
/// ```
pub use import::{
    import_from_arrow_ipc,
    import_from_callback,
    import_from_file,
    import_from_iter,
    import_from_parquet,
    import_from_parquet_stream,
    import_from_record_batch,
    import_from_record_batches,
    import_from_stream,
    // Arrow import
    ArrowImportOptions,
    ArrowToCsvWriter,
    // CSV import
    CsvImportOptions,
    CsvWriterOptions,
    DataPipeSender,
    // Error type
    ImportError,
    // Parquet import
    ParquetImportOptions,
};

// =============================================================================
// Export Types
// =============================================================================

/// CSV export options and functions.
///
/// The export module provides functionality for exporting data from Exasol tables
/// or query results to files, streams, in-memory lists, or custom callbacks.
///
/// # Example
///
/// ```no_run
/// use exarrow_rs::export::{CsvExportOptions, ExportError};
/// use exarrow_rs::query::export::ExportSource;
///
/// # async fn example() -> Result<(), ExportError> {
/// let options = CsvExportOptions::default()
///     .exasol_host("localhost")
///     .exasol_port(8563)
///     .with_column_names(true);
///
/// let source = ExportSource::Table {
///     schema: Some("my_schema".to_string()),
///     name: "users".to_string(),
///     columns: vec![],
/// };
///
/// // Use via Session or Connection for execution
/// # Ok(())
/// # }
/// ```
pub use export::{
    csv_to_record_batches,
    exasol_types_to_arrow_schema,
    export_to_arrow_ipc,
    export_to_callback,
    export_to_file,
    export_to_list,
    export_to_parquet,
    export_to_parquet_stream,
    export_to_parquet_via_transport,
    export_to_record_batches,
    export_to_stream,
    // Arrow export
    ArrowExportOptions,
    // CSV export
    CsvExportOptions,
    CsvToArrowReader,
    DataPipeReceiver,
    ExportError,
    // Parquet export
    ParquetCompression,
    ParquetExportOptions,
};

// =============================================================================
// Query Builder Types
// =============================================================================

/// Query builder types for constructing IMPORT and EXPORT SQL statements.
///
/// These types provide a builder pattern for constructing Exasol IMPORT and EXPORT
/// SQL statements with full control over CSV format options, compression, and
/// encoding settings.
///
/// # Export Query Example
///
/// ```
/// use exarrow_rs::query::export::{ExportQuery, ExportSource, Compression, RowSeparator};
///
/// let query = ExportQuery::from_table("users")
///     .schema("my_schema")
///     .columns(vec!["id", "name", "email"])
///     .at_address("192.168.1.100:8080")
///     .with_column_names(true)
///     .compressed(Compression::Gzip)
///     .row_separator(RowSeparator::CRLF)
///     .build();
/// ```
///
/// # Import Query Example
///
/// ```
/// use exarrow_rs::query::import::{ImportQuery, RowSeparator, Compression, TrimMode};
///
/// let query = ImportQuery::new("users")
///     .schema("my_schema")
///     .columns(vec!["id", "name", "email"])
///     .at_address("192.168.1.100:8080")
///     .skip(1)
///     .trim(TrimMode::Trim)
///     .compressed(Compression::Gzip)
///     .reject_limit(100)
///     .build();
/// ```
pub use query::export::{Compression, DelimitMode, ExportQuery, ExportSource, RowSeparator};
pub use query::import::{
    Compression as ImportCompression, ImportQuery, RowSeparator as ImportRowSeparator, TrimMode,
};

// =============================================================================
// Query Execution Types
// =============================================================================

pub use query::statement::{Parameter, StatementType};
/// Query execution and result handling types.
pub use query::{PreparedStatement, QueryMetadata, ResultSet, ResultSetIterator};

// =============================================================================
// FFI Types (when ffi feature is enabled)
// =============================================================================

/// Re-export FFI types when ffi feature is enabled.
#[cfg(feature = "ffi")]
pub use adbc_ffi::{FfiConnection, FfiDatabase, FfiDriver, FfiStatement};
