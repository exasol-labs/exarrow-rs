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
    import_from_callback, import_from_file, import_from_files, import_from_iter, import_from_stream,
    CsvImportOptions, DataPipeSender,
};

pub use parquet::{
    import_from_parquet, import_from_parquet_files, import_from_parquet_stream, ParquetImportOptions,
};

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
