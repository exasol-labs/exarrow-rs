//! Import functionality for Exasol data.
//!
//! This module provides utilities for importing data into Exasol from various sources
//! including CSV files, Parquet files, streams, iterators, and custom callbacks.
//!
//! # Overview
//!
//! The import module supports:
//! - CSV import from file paths
//! - CSV import from async readers/streams
//! - CSV import from iterators
//! - CSV import with custom callbacks for data generation
//! - Parquet import (converted to CSV on-the-fly)
//! - Compression support (gzip, bzip2)
//!
//! # Architecture
//!
//! Exasol's IMPORT command only accepts CSV format over HTTP. For non-CSV sources
//! (like Parquet), data is converted to CSV on-the-fly during streaming.
//!
//! The import process works as follows:
//! 1. Start an HTTP transport server on a local port
//! 2. Execute an IMPORT SQL statement via WebSocket that points to our server
//! 3. Stream data through the HTTP transport to Exasol
//!
//! # Example
//!
//! ```no_run
//! use exarrow_rs::import::{ParquetImportOptions, import_from_parquet};
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Import from a Parquet file
//! // let rows = import_from_parquet(&mut session, "my_table", Path::new("data.parquet"), ParquetImportOptions::default()).await?;
//! // println!("Imported {} rows", rows);
//! # Ok(())
//! # }
//! ```

pub mod arrow;
pub mod csv;
pub mod parallel;
pub mod parquet;
pub mod source;

pub use arrow::{
    import_from_arrow_ipc, import_from_record_batch, import_from_record_batches,
    ArrowImportOptions, ArrowToCsvWriter, CsvWriterOptions,
};

pub use csv::{
    import_from_callback, import_from_file, import_from_files, import_from_iter,
    import_from_stream, CsvImportOptions, DataPipeSender,
};

pub use parquet::{
    import_from_parquet, import_from_parquet_files, import_from_parquet_stream,
    ParquetImportOptions,
};

// Re-export ColumnNameMode for convenient access
pub use crate::types::ColumnNameMode;

pub use parallel::{ImportFileEntry, ParallelTransportPool};
pub use source::IntoFileSources;

use thiserror::Error;

/// Errors that can occur during import operations.
#[derive(Error, Debug)]
pub enum ImportError {
    /// IO error during file operations
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Parquet file reading error
    #[error("Parquet error: {0}")]
    ParquetError(String),

    /// Arrow conversion error
    #[error("Arrow error: {0}")]
    ArrowError(String),

    /// Transport error during HTTP streaming
    #[error("Transport error: {0}")]
    TransportError(#[from] crate::error::TransportError),

    /// Query execution error
    #[error("Query error: {0}")]
    QueryError(String),

    /// Data conversion error
    #[error("Conversion error: {0}")]
    ConversionError(String),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// CSV writing error
    #[error("CSV write error: {0}")]
    CsvWriteError(String),

    /// Arrow IPC reading error
    #[error("Arrow IPC error: {0}")]
    ArrowIpcError(String),

    /// SQL execution error
    #[error("SQL execution failed: {0}")]
    SqlError(String),

    /// HTTP transport server failed
    #[error("HTTP transport failed: {0}")]
    HttpTransportError(String),

    /// Data streaming error
    #[error("Data streaming error: {0}")]
    StreamError(String),

    /// Compression error
    #[error("Compression error: {0}")]
    CompressionError(String),

    /// Invalid session state
    #[error("Invalid session state: {0}")]
    InvalidSessionState(String),

    /// Channel communication error
    #[error("Channel error: {0}")]
    ChannelError(String),

    /// Parallel import error (connection, streaming, or conversion failure)
    #[error("Parallel import error: {0}")]
    ParallelImportError(String),

    /// Schema inference failed (could not read metadata or convert types)
    #[error("Schema inference failed: {0}")]
    SchemaInferenceError(String),

    /// Schema mismatch between multiple files
    #[error("Schema mismatch between files: {0}")]
    SchemaMismatchError(String),
}

impl From<::arrow::error::ArrowError> for ImportError {
    fn from(err: ::arrow::error::ArrowError) -> Self {
        ImportError::ArrowError(err.to_string())
    }
}

impl From<::parquet::errors::ParquetError> for ImportError {
    fn from(err: ::parquet::errors::ParquetError) -> Self {
        ImportError::ParquetError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::ImportError;

    #[test]
    fn test_import_error_from_arrow_error_keeps_the_arrow_message() {
        let arrow_error =
            ::arrow::error::ArrowError::SchemaError("column NAME missing".to_string());
        let arrow_message = arrow_error.to_string();

        let err: ImportError = arrow_error.into();

        assert!(matches!(err, ImportError::ArrowError(_)));
        assert_eq!(err.to_string(), format!("Arrow error: {}", arrow_message));
    }

    #[test]
    fn test_import_error_from_parquet_error_keeps_the_parquet_message() {
        let parquet_error = ::parquet::errors::ParquetError::General("bad footer".to_string());
        let parquet_message = parquet_error.to_string();

        let err: ImportError = parquet_error.into();

        assert!(matches!(err, ImportError::ParquetError(_)));
        assert_eq!(
            err.to_string(),
            format!("Parquet error: {}", parquet_message)
        );
    }

    #[test]
    fn test_import_error_from_io_error_keeps_the_io_message() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "data.csv");

        let err: ImportError = io_error.into();

        assert!(matches!(err, ImportError::IoError(_)));
        assert_eq!(err.to_string(), "IO error: data.csv");
    }

    #[test]
    fn test_import_error_from_transport_error_keeps_the_transport_message() {
        let transport_error = crate::error::TransportError::IoError("socket closed".to_string());

        let err: ImportError = transport_error.into();

        assert!(matches!(err, ImportError::TransportError(_)));
        assert_eq!(
            err.to_string(),
            "Transport error: Network I/O error: socket closed"
        );
    }

    #[test]
    fn test_every_import_error_variant_renders_its_context() {
        let cases: Vec<(ImportError, &str)> = vec![
            (
                ImportError::ParquetError("bad footer".to_string()),
                "Parquet error: bad footer",
            ),
            (
                ImportError::ArrowError("schema".to_string()),
                "Arrow error: schema",
            ),
            (
                ImportError::QueryError("IMPORT rejected".to_string()),
                "Query error: IMPORT rejected",
            ),
            (
                ImportError::ConversionError("i128 overflow".to_string()),
                "Conversion error: i128 overflow",
            ),
            (
                ImportError::InvalidConfig("no columns".to_string()),
                "Invalid configuration: no columns",
            ),
            (
                ImportError::CsvWriteError("disk full".to_string()),
                "CSV write error: disk full",
            ),
            (
                ImportError::ArrowIpcError("truncated".to_string()),
                "Arrow IPC error: truncated",
            ),
            (
                ImportError::SqlError("syntax".to_string()),
                "SQL execution failed: syntax",
            ),
            (
                ImportError::HttpTransportError("bind failed".to_string()),
                "HTTP transport failed: bind failed",
            ),
            (
                ImportError::StreamError("reset".to_string()),
                "Data streaming error: reset",
            ),
            (
                ImportError::CompressionError("gzip".to_string()),
                "Compression error: gzip",
            ),
            (
                ImportError::InvalidSessionState("not connected".to_string()),
                "Invalid session state: not connected",
            ),
            (
                ImportError::ChannelError("receiver dropped".to_string()),
                "Channel error: receiver dropped",
            ),
            (
                ImportError::ParallelImportError("worker 2 failed".to_string()),
                "Parallel import error: worker 2 failed",
            ),
            (
                ImportError::SchemaInferenceError("no metadata".to_string()),
                "Schema inference failed: no metadata",
            ),
            (
                ImportError::SchemaMismatchError("file 2 differs".to_string()),
                "Schema mismatch between files: file 2 differs",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }
}
