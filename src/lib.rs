//! # exarrow-rs
//!
//! ADBC-compatible driver for Exasol with Apache Arrow data format support.
//!
//! This library provides high-performance database connectivity for Exasol using
//! the ADBC (Arrow Database Connectivity) interface. Phase 1 implements the
//! WebSocket-based transport protocol, with future plans for Arrow-native gRPC.
//!

// Module declarations
pub mod adbc;
pub mod arrow_conversion;
pub mod connection;
pub mod error;
pub mod query;
pub mod transport;
pub mod types;

// FFI module for C-compatible ADBC export (conditionally compiled)
#[cfg(feature = "ffi")]
pub mod adbc_ffi;

// Re-export public API
pub use adbc::{Connection, Database, Driver, Statement};
pub use arrow_conversion::ArrowConverter;
pub use error::{ConnectionError, ConversionError, ExasolError, QueryError};
pub use types::{ExasolType, TypeMapper};

// Re-export FFI types when ffi feature is enabled
#[cfg(feature = "ffi")]
pub use adbc_ffi::{FfiConnection, FfiDatabase, FfiDriver, FfiStatement};
