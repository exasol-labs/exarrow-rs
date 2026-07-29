//! Error types for exarrow-rs.
//!
//! This module defines domain-specific error types organized by functional area.

use thiserror::Error;

/// Top-level error type encompassing all possible errors.
#[derive(Error, Debug)]
pub enum ExasolError {
    /// Connection-related errors
    #[error(transparent)]
    Connection(#[from] ConnectionError),

    /// Query execution errors
    #[error(transparent)]
    Query(#[from] QueryError),

    /// Data conversion errors
    #[error(transparent)]
    Conversion(#[from] ConversionError),

    /// Transport protocol errors
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Errors related to database connections.
#[derive(Error, Debug)]
pub enum ConnectionError {
    /// Failed to establish connection to the database
    #[error("Failed to connect to {host}:{port}: {message}")]
    ConnectionFailed {
        host: String,
        port: u16,
        message: String,
    },

    /// Authentication failure
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Invalid connection parameters
    #[error("Invalid connection parameter '{parameter}': {message}")]
    InvalidParameter { parameter: String, message: String },

    /// Connection string parsing error
    #[error("Failed to parse connection string: {0}")]
    ParseError(String),

    /// Connection timeout
    #[error("Connection timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// Connection is closed
    #[error("Connection is closed")]
    ConnectionClosed,

    /// TLS/SSL error
    #[error("TLS error: {0}")]
    TlsError(String),
}

/// Errors related to query execution.
#[derive(Error, Debug)]
pub enum QueryError {
    /// SQL syntax error
    #[error("SQL syntax error at position {position}: {message}")]
    SyntaxError { position: usize, message: String },

    /// Query execution failed
    #[error("Query execution failed: {0}")]
    ExecutionFailed(String),

    /// Query timeout
    #[error("Query timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// Invalid query state
    #[error("Invalid query state: {0}")]
    InvalidState(String),

    /// Parameter binding error
    #[error("Parameter binding error for parameter {index}: {message}")]
    ParameterBindingError { index: usize, message: String },

    /// Result set not available
    #[error("Result set not available: {0}")]
    NoResultSet(String),

    /// Transaction error
    #[error("Transaction error: {0}")]
    TransactionError(String),

    /// SQL injection attempt detected
    #[error("Potential SQL injection detected")]
    SqlInjectionDetected,

    /// Prepared statement has been closed
    #[error("Prepared statement has been closed")]
    StatementClosed,

    /// Unexpected result set when row count was expected
    #[error("Expected row count but received result set")]
    UnexpectedResultSet,
}

/// Errors related to data type conversion.
#[derive(Error, Debug)]
pub enum ConversionError {
    /// Unsupported Exasol type
    #[error("Unsupported Exasol type: {exasol_type}")]
    UnsupportedType { exasol_type: String },

    /// Failed to convert value
    #[error("Failed to convert value at row {row}, column {column}: {message}")]
    ValueConversionFailed {
        row: usize,
        column: usize,
        message: String,
    },

    /// Schema mismatch
    #[error("Schema mismatch: {0}")]
    SchemaMismatch(String),

    /// Invalid data format
    #[error("Invalid data format: {0}")]
    InvalidFormat(String),

    /// Overflow during conversion
    #[error("Numeric overflow at row {row}, column {column}")]
    NumericOverflow { row: usize, column: usize },

    /// Invalid UTF-8 string
    #[error("Invalid UTF-8 string at row {row}, column {column}")]
    InvalidUtf8 { row: usize, column: usize },

    /// Arrow error
    #[error("Arrow error: {0}")]
    ArrowError(String),
}

/// Errors related to transport protocol.
#[derive(Error, Debug)]
pub enum TransportError {
    /// WebSocket connection error
    #[error("WebSocket error: {0}")]
    WebSocketError(String),

    /// Message serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Message deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// Protocol error
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// Invalid response from server
    #[error("Invalid server response: {0}")]
    InvalidResponse(String),

    /// Network I/O error
    #[error("Network I/O error: {0}")]
    IoError(String),

    /// Message send error
    #[error("Failed to send message: {0}")]
    SendError(String),

    /// Message receive error
    #[error("Failed to receive message: {0}")]
    ReceiveError(String),

    /// TLS/SSL error
    #[error("TLS error: {0}")]
    TlsError(String),
}

// Conversions from external error types
impl From<arrow::error::ArrowError> for ConversionError {
    fn from(err: arrow::error::ArrowError) -> Self {
        ConversionError::ArrowError(err.to_string())
    }
}

impl From<serde_json::Error> for TransportError {
    fn from(err: serde_json::Error) -> Self {
        TransportError::SerializationError(err.to_string())
    }
}

#[cfg(feature = "websocket")]
impl From<tokio_tungstenite::tungstenite::Error> for TransportError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        TransportError::WebSocketError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_error_display() {
        let err = ConnectionError::ConnectionFailed {
            host: "localhost".to_string(),
            port: 8563,
            message: "Connection refused".to_string(),
        };
        assert!(err.to_string().contains("localhost"));
        assert!(err.to_string().contains("8563"));
    }

    #[test]
    fn test_query_error_display() {
        let err = QueryError::SyntaxError {
            position: 10,
            message: "Unexpected token".to_string(),
        };
        assert!(err.to_string().contains("position 10"));
    }

    #[test]
    fn test_conversion_error_display() {
        let err = ConversionError::ValueConversionFailed {
            row: 5,
            column: 2,
            message: "Invalid number format".to_string(),
        };
        assert!(err.to_string().contains("row 5"));
        assert!(err.to_string().contains("column 2"));
    }

    #[test]
    fn test_transport_tls_error() {
        let err = TransportError::TlsError("Certificate validation failed".to_string());
        assert!(err.to_string().contains("TLS error"));
        assert!(err.to_string().contains("Certificate validation failed"));
    }

    #[test]
    fn test_statement_closed_error() {
        let err = QueryError::StatementClosed;
        assert!(err.to_string().contains("closed"));
    }

    #[test]
    fn test_unexpected_result_set_error() {
        let err = QueryError::UnexpectedResultSet;
        assert!(err.to_string().contains("result set"));
    }

    // =========================================================================
    // Conversions from external error types
    // =========================================================================

    #[test]
    fn test_conversion_error_from_arrow_error_keeps_the_arrow_message() {
        let arrow_error = arrow::error::ArrowError::InvalidArgumentError("bad column".to_string());
        let arrow_message = arrow_error.to_string();

        let err: ConversionError = arrow_error.into();

        assert!(matches!(err, ConversionError::ArrowError(_)));
        assert_eq!(err.to_string(), format!("Arrow error: {}", arrow_message));
    }

    #[test]
    fn test_transport_error_from_serde_json_error_keeps_the_parser_message() {
        let json_error = serde_json::from_str::<serde_json::Value>("{not json").unwrap_err();
        let json_message = json_error.to_string();

        let err: TransportError = json_error.into();

        assert!(matches!(err, TransportError::SerializationError(_)));
        assert_eq!(
            err.to_string(),
            format!("Serialization error: {}", json_message)
        );
    }

    // =========================================================================
    // ExasolError wraps each domain error transparently
    // =========================================================================

    #[test]
    fn test_exasol_error_from_connection_error_is_transparent() {
        let err: ExasolError = ConnectionError::ConnectionClosed.into();

        assert!(matches!(err, ExasolError::Connection(_)));
        assert_eq!(err.to_string(), "Connection is closed");
    }

    #[test]
    fn test_exasol_error_from_query_error_is_transparent() {
        let err: ExasolError = QueryError::StatementClosed.into();

        assert!(matches!(err, ExasolError::Query(_)));
        assert_eq!(err.to_string(), "Prepared statement has been closed");
    }

    #[test]
    fn test_exasol_error_from_conversion_error_is_transparent() {
        let err: ExasolError = ConversionError::SchemaMismatch("two columns".to_string()).into();

        assert!(matches!(err, ExasolError::Conversion(_)));
        assert_eq!(err.to_string(), "Schema mismatch: two columns");
    }

    #[test]
    fn test_exasol_error_from_transport_error_is_transparent() {
        let err: ExasolError = TransportError::ProtocolError("bad frame".to_string()).into();

        assert!(matches!(err, ExasolError::Transport(_)));
        assert_eq!(err.to_string(), "Protocol error: bad frame");
    }

    // =========================================================================
    // Every variant renders the context a caller needs to act on
    // =========================================================================

    #[test]
    fn test_every_connection_error_variant_renders_its_context() {
        let cases: Vec<(ConnectionError, &str)> = vec![
            (
                ConnectionError::ConnectionFailed {
                    host: "db.example".to_string(),
                    port: 8563,
                    message: "refused".to_string(),
                },
                "Failed to connect to db.example:8563: refused",
            ),
            (
                ConnectionError::AuthenticationFailed("wrong user".to_string()),
                "Authentication failed: wrong user",
            ),
            (
                ConnectionError::InvalidParameter {
                    parameter: "port".to_string(),
                    message: "not a number".to_string(),
                },
                "Invalid connection parameter 'port': not a number",
            ),
            (
                ConnectionError::ParseError("missing host".to_string()),
                "Failed to parse connection string: missing host",
            ),
            (
                ConnectionError::Timeout { timeout_ms: 2500 },
                "Connection timeout after 2500ms",
            ),
            (ConnectionError::ConnectionClosed, "Connection is closed"),
            (
                ConnectionError::TlsError("untrusted certificate".to_string()),
                "TLS error: untrusted certificate",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn test_every_query_error_variant_renders_its_context() {
        let cases: Vec<(QueryError, &str)> = vec![
            (
                QueryError::SyntaxError {
                    position: 7,
                    message: "unexpected FROM".to_string(),
                },
                "SQL syntax error at position 7: unexpected FROM",
            ),
            (
                QueryError::ExecutionFailed("table missing".to_string()),
                "Query execution failed: table missing",
            ),
            (
                QueryError::Timeout { timeout_ms: 30_000 },
                "Query timeout after 30000ms",
            ),
            (
                QueryError::InvalidState("not prepared".to_string()),
                "Invalid query state: not prepared",
            ),
            (
                QueryError::ParameterBindingError {
                    index: 2,
                    message: "NaN".to_string(),
                },
                "Parameter binding error for parameter 2: NaN",
            ),
            (
                QueryError::NoResultSet("row count only".to_string()),
                "Result set not available: row count only",
            ),
            (
                QueryError::TransactionError("rollback failed".to_string()),
                "Transaction error: rollback failed",
            ),
            (
                QueryError::SqlInjectionDetected,
                "Potential SQL injection detected",
            ),
            (
                QueryError::StatementClosed,
                "Prepared statement has been closed",
            ),
            (
                QueryError::UnexpectedResultSet,
                "Expected row count but received result set",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn test_every_conversion_error_variant_renders_its_context() {
        let cases: Vec<(ConversionError, &str)> = vec![
            (
                ConversionError::UnsupportedType {
                    exasol_type: "XML".to_string(),
                },
                "Unsupported Exasol type: XML",
            ),
            (
                ConversionError::ValueConversionFailed {
                    row: 3,
                    column: 4,
                    message: "not a number".to_string(),
                },
                "Failed to convert value at row 3, column 4: not a number",
            ),
            (
                ConversionError::SchemaMismatch("column count".to_string()),
                "Schema mismatch: column count",
            ),
            (
                ConversionError::InvalidFormat("Empty decimal string".to_string()),
                "Invalid data format: Empty decimal string",
            ),
            (
                ConversionError::NumericOverflow { row: 1, column: 2 },
                "Numeric overflow at row 1, column 2",
            ),
            (
                ConversionError::InvalidUtf8 { row: 5, column: 6 },
                "Invalid UTF-8 string at row 5, column 6",
            ),
            (
                ConversionError::ArrowError("length mismatch".to_string()),
                "Arrow error: length mismatch",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn test_every_transport_error_variant_renders_its_context() {
        let cases: Vec<(TransportError, &str)> = vec![
            (
                TransportError::WebSocketError("handshake".to_string()),
                "WebSocket error: handshake",
            ),
            (
                TransportError::SerializationError("bad value".to_string()),
                "Serialization error: bad value",
            ),
            (
                TransportError::DeserializationError("bad frame".to_string()),
                "Deserialization error: bad frame",
            ),
            (
                TransportError::ProtocolError("unexpected state".to_string()),
                "Protocol error: unexpected state",
            ),
            (
                TransportError::InvalidResponse("no status".to_string()),
                "Invalid server response: no status",
            ),
            (
                TransportError::IoError("broken pipe".to_string()),
                "Network I/O error: broken pipe",
            ),
            (
                TransportError::SendError("closed".to_string()),
                "Failed to send message: closed",
            ),
            (
                TransportError::ReceiveError("timeout".to_string()),
                "Failed to receive message: timeout",
            ),
            (
                TransportError::TlsError("unknown ca".to_string()),
                "TLS error: unknown ca",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }
}
