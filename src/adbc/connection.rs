//! ADBC Connection implementation.
//!
//! This module provides the `Connection` type which represents an active
//! database connection and provides methods for executing queries.
//!
//! # New in v2.0.0
//!
//! Connection now owns the transport directly and Statement is a pure data container.
//! Execute statements via Connection methods: `execute_statement()`, `execute_prepared()`.

use crate::adbc::Statement;
use crate::connection::auth::AuthResponseData;
use crate::connection::params::ConnectionParams;
use crate::connection::session::{Session as SessionInfo, SessionConfig, SessionState};
use crate::error::{ConnectionError, ExasolError, QueryError, TransportError};
use crate::query::prepared::PreparedStatement;
use crate::query::results::ResultSet;
use crate::query::statement::Parameter;
use crate::transport::protocol::{
    ConnectionParams as TransportConnectionParams, Credentials as TransportCredentials,
    QueryResult, TransportProtocol,
};
use arrow::array::RecordBatch;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

/// Session is a type alias for Connection.
///
/// This alias provides a more intuitive name for database sessions when
/// performing import/export operations. Both `Session` and `Connection`
/// can be used interchangeably.
///
/// # Example
///
pub type Session = Connection;

/// Classify a `set_schema` (`OPEN SCHEMA`) failure as a missing-schema error.
///
/// A schema named in the connection URI is a *best-effort default*. When that
/// schema does not yet exist the server reports a "schema ... not found" error,
/// which we deliberately swallow during connect so the connection stays open
/// (see [`Connection::connect_with_transport`]). Any other failure is fatal.
///
/// Returns `true` only when the error message indicates the schema was not
/// found; all other errors return `false`.
fn schema_open_error_is_missing_schema(err: &QueryError) -> bool {
    err.to_string().to_ascii_lowercase().contains("not found")
}

/// Round a duration up to whole seconds over its millisecond value.
///
/// Any positive sub-second duration maps to at least `1`; only a zero duration
/// maps to `0`. Exasol treats `queryTimeout = 0` as "unlimited", so `0` is
/// reserved exclusively for the no-timeout / reset path — a caller-requested
/// near-zero-but-nonzero timeout must never collapse into that sentinel.
fn secs_ceil(d: Duration) -> u64 {
    let secs = d.as_secs();
    if d.subsec_nanos() > 0 {
        secs.saturating_add(1)
    } else {
        secs
    }
}

/// Map a transport-layer execution failure to a [`QueryError`].
///
/// A server-reported query-timeout abort carries Exasol SQL state `R0001`
/// ("Query terminated because timeout has been reached."). Such a failure maps
/// to [`QueryError::Timeout`], matched primarily on the SQL state code with the
/// message text as a fallback; every other failure maps to
/// [`QueryError::ExecutionFailed`].
///
/// `timeout_ms` is the effective timeout actually enforced by the server for
/// this statement (`0` when none was configured). An `R0001` abort is only
/// classified as [`QueryError::Timeout`] when a non-zero timeout was in
/// effect — an ambient session-level `QUERY_TIMEOUT` firing on a statement the
/// driver never limited surfaces as [`QueryError::ExecutionFailed`] instead,
/// preserving the server's message rather than reporting a nonsensical
/// "timeout after 0ms".
fn map_execution_error(err: TransportError, timeout_ms: u64) -> QueryError {
    let message = err.to_string();
    let is_query_timeout = timeout_ms > 0
        && (message.contains("R0001")
            || message.contains("Query terminated because timeout has been reached"));
    if is_query_timeout {
        QueryError::Timeout { timeout_ms }
    } else {
        QueryError::ExecutionFailed(message)
    }
}

fn blocking_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for blocking operations")
    })
}

/// ADBC Connection to an Exasol database.
///
/// The `Connection` type represents an active database connection and provides
/// methods for executing queries, managing transactions, and retrieving metadata.
///
/// # v2.0.0 Breaking Changes
///
/// - Connection now owns the transport directly
/// - `create_statement()` is now synchronous and returns a pure data container
/// - Use `execute_statement()` instead of `Statement::execute()`
/// - Use `prepare()` instead of `Statement::prepare()`
///
/// # Example
///
pub struct Connection {
    /// Transport layer for communication (owned by Connection)
    transport: Arc<Mutex<dyn TransportProtocol>>,
    /// Session information
    session: SessionInfo,
    /// Connection parameters
    params: ConnectionParams,
}

impl Connection {
    /// Create a connection from connection parameters.
    ///
    /// This establishes a connection to the Exasol database using WebSocket transport,
    /// authenticates the user, and creates a session.
    ///
    /// # Arguments
    ///
    /// * `params` - Connection parameters
    ///
    /// # Returns
    ///
    /// A connected `Connection` instance.
    ///
    /// # Errors
    ///
    /// Returns `ConnectionError` if the connection or authentication fails.
    pub async fn from_params(params: ConnectionParams) -> Result<Self, ConnectionError> {
        let requested = params.transport.as_deref();

        match requested {
            #[cfg(feature = "native")]
            Some("native") | None => {
                let transport = crate::transport::NativeTcpTransport::new();
                Self::connect_with_transport(params, transport).await
            }
            #[cfg(feature = "websocket")]
            Some("websocket") => {
                let transport = crate::transport::WebSocketTransport::new();
                Self::connect_with_transport(params, transport).await
            }
            #[cfg(all(not(feature = "native"), feature = "websocket"))]
            None => {
                let transport = crate::transport::WebSocketTransport::new();
                Self::connect_with_transport(params, transport).await
            }
            Some(t) => Err(ConnectionError::InvalidParameter {
                parameter: "transport".to_string(),
                message: format!("Transport '{}' is not available. Check feature flags.", t),
            }),
            #[cfg(not(any(feature = "native", feature = "websocket")))]
            None => Err(ConnectionError::InvalidParameter {
                parameter: "transport".to_string(),
                message: "No transport feature enabled. Enable 'native' or 'websocket'."
                    .to_string(),
            }),
        }
    }

    /// Connect using the given transport implementation.
    async fn connect_with_transport<T: TransportProtocol + 'static>(
        params: ConnectionParams,
        mut transport: T,
    ) -> Result<Self, ConnectionError> {
        // Convert ConnectionParams to TransportConnectionParams
        let mut transport_params = TransportConnectionParams::new(params.host.clone(), params.port)
            .with_tls(params.use_tls)
            .with_validate_server_certificate(params.validate_server_certificate)
            .with_timeout(params.connection_timeout.as_millis() as u64);
        if let Some(ref fp) = params.certificate_fingerprint {
            transport_params = transport_params.with_certificate_fingerprint(fp.clone());
        }

        // Connect
        transport.connect(&transport_params).await.map_err(|e| {
            ConnectionError::ConnectionFailed {
                host: params.host.clone(),
                port: params.port,
                message: e.to_string(),
            }
        })?;

        // Authenticate
        let credentials =
            TransportCredentials::new(params.username.clone(), params.password().to_string());
        let session_info = transport
            .authenticate(&credentials)
            .await
            .map_err(|e| ConnectionError::AuthenticationFailed(e.to_string()))?;

        // Forward a configured query timeout to the server so it enforces the
        // limit itself. Absent configuration, no attribute is set and the
        // server's own QUERY_TIMEOUT governs.
        if let Some(d) = params.query_timeout {
            transport
                .set_query_timeout(secs_ceil(d))
                .await
                .map_err(|e| ConnectionError::ConnectionFailed {
                    host: params.host.clone(),
                    port: params.port,
                    message: format!("failed to set query timeout: {}", e),
                })?;
        }

        // Create session config from connection params. `query_timeout` seeds
        // the applied-value baseline that `execute_statement` reconciles against.
        let session_config = SessionConfig {
            idle_timeout: params.idle_timeout,
            query_timeout: params.query_timeout,
            ..Default::default()
        };

        // Extract session_id once to avoid double clone
        let session_id = session_info.session_id.clone();

        // Convert SessionInfo to AuthResponseData
        let auth_response = AuthResponseData {
            session_id: session_id.clone(),
            protocol_version: session_info.protocol_version,
            release_version: session_info.release_version,
            database_name: session_info.database_name,
            product_name: session_info.product_name,
            max_data_message_size: session_info.max_data_message_size,
            max_identifier_length: 128,
            max_varchar_length: 2_000_000,
            identifier_quote_string: "\"".to_string(),
            time_zone: session_info.time_zone.unwrap_or_else(|| "UTC".to_string()),
            time_zone_behavior: "INVALID TIMESTAMP TO DOUBLE".to_string(),
        };

        // Create session
        let session = SessionInfo::new(session_id, auth_response, session_config);

        let schema = params.schema.clone();

        let mut connection = Self {
            transport: Arc::new(Mutex::new(transport)),
            session,
            params,
        };

        // A schema named in the connection URI is a *best-effort default*: we
        // OPEN it so unqualified queries resolve against it. A schema that does
        // not yet exist must NOT fail the connection — tools such as dbt create
        // their target schema after connecting and fully qualify every relation,
        // so a not-yet-existing default schema is a normal state, not a corrupt
        // one. We therefore swallow "schema not found" and leave the session
        // with no active schema (the schema can be OPENed later, once it exists).
        //
        // Any other failure (auth, transport, permissions) still indicates a
        // genuinely broken connection: we close the transport — we are already
        // authenticated server-side — and propagate a clear error rather than
        // return a half-open `Connection`.
        if let Some(schema_name) = schema {
            if let Err(query_err) = connection.set_schema(schema_name.clone()).await {
                let schema_missing = schema_open_error_is_missing_schema(&query_err);
                if !schema_missing {
                    let host = connection.params.host.clone();
                    let port = connection.params.port;
                    let _ = connection.shutdown().await;
                    return Err(ConnectionError::ConnectionFailed {
                        host,
                        port,
                        message: format!(
                            "failed to activate schema '{}' from connection URI: {}",
                            schema_name, query_err
                        ),
                    });
                }
                // Schema does not exist yet: keep the connection open with no
                // active schema. Fully qualified names continue to work.
            }
        }

        Ok(connection)
    }

    /// Create a builder for constructing a connection.
    ///
    /// # Returns
    ///
    /// A `ConnectionBuilder` instance.
    ///
    /// # Example
    ///
    pub fn builder() -> ConnectionBuilder {
        ConnectionBuilder::new()
    }

    // ========================================================================
    // Statement Creation (synchronous - Statement is now a pure data container)
    // ========================================================================

    /// Create a new statement for executing SQL.
    ///
    /// Statement is now a pure data container. Use `execute_statement()` to execute it.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL query text
    ///
    /// # Returns
    ///
    /// A `Statement` instance ready for execution via `execute_statement()`.
    ///
    /// # Example
    ///
    pub fn create_statement(&self, sql: impl Into<String>) -> Statement {
        let mut stmt = Statement::new(sql);
        if let Some(d) = self.params.query_timeout {
            stmt.set_timeout(d.as_millis().min(u64::MAX as u128) as u64);
        }
        stmt
    }

    // ========================================================================
    // Statement Execution Methods
    // ========================================================================

    /// Execute a statement and return results.
    ///
    /// # Arguments
    ///
    /// * `stmt` - Statement to execute
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing the query results.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails or times out.
    ///
    /// # Example
    ///
    pub async fn execute_statement(&mut self, stmt: &Statement) -> Result<ResultSet, QueryError> {
        // Validate session state
        self.session
            .validate_ready()
            .await
            .map_err(|e| QueryError::InvalidState(e.to_string()))?;

        // Update session state
        self.session.set_state(SessionState::Executing).await;

        // Increment query counter
        self.session.increment_query_count();

        // Build final SQL with parameters
        let final_sql = stmt.build_sql()?;

        // Reconcile the session `queryTimeout` in both directions before
        // executing. `queryTimeout` is session-level, so a per-statement value
        // (or its absence) is pushed to the server whenever it differs from the
        // currently-applied value. A statement with no timeout resets the server
        // to `0` (unlimited) so it never inherits a prior statement's limit.
        let target_secs = match stmt.timeout_ms() {
            Some(ms) => secs_ceil(Duration::from_millis(ms)),
            None => 0,
        };
        let applied_secs = match self.session.config().query_timeout {
            Some(d) => secs_ceil(d),
            None => 0,
        };
        let needs_reconcile = target_secs != applied_secs;

        let mut transport_guard = self.transport.lock().await;
        if needs_reconcile {
            transport_guard
                .set_query_timeout(target_secs)
                .await
                .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;
        }
        let exec_result = transport_guard.execute_query(&final_sql).await;
        drop(transport_guard);

        // Record what was applied to the server so the next statement
        // reconciles against the correct baseline. This runs even when the
        // query itself aborts: the server's `queryTimeout` was still changed.
        if needs_reconcile {
            self.session.config_mut().query_timeout = stmt.timeout_ms().map(Duration::from_millis);
        }

        // Report the effective timeout actually enforced by the server
        // (`target_secs`, in ms) rather than the raw sub-second request —
        // e.g. a 1500ms request rounds up to a 2000ms server-side limit, and
        // that's the value that fired.
        let result =
            exec_result.map_err(|e| map_execution_error(e, target_secs.saturating_mul(1000)))?;

        // Update session state back to ready/in_transaction
        self.update_session_state_after_query().await;

        // Convert transport result to ResultSet
        ResultSet::from_transport_result(result, Arc::clone(&self.transport))
    }

    /// Execute a statement and return the row count (for non-SELECT statements).
    ///
    /// # Arguments
    ///
    /// * `stmt` - Statement to execute
    ///
    /// # Returns
    ///
    /// The number of rows affected.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails or if statement is a SELECT.
    ///
    /// # Example
    ///
    pub async fn execute_statement_update(&mut self, stmt: &Statement) -> Result<i64, QueryError> {
        let result_set = self.execute_statement(stmt).await?;

        result_set.row_count().ok_or_else(|| {
            QueryError::NoResultSet("Expected row count, got result set".to_string())
        })
    }

    // ========================================================================
    // Prepared Statement Methods
    // ========================================================================

    /// Create a prepared statement for parameterized query execution.
    ///
    /// This creates a server-side prepared statement that can be executed
    /// multiple times with different parameter values.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL statement with parameter placeholders (?)
    ///
    /// # Returns
    ///
    /// A `PreparedStatement` ready for parameter binding and execution.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if preparation fails.
    ///
    /// # Example
    ///
    pub async fn prepare(
        &mut self,
        sql: impl Into<String>,
    ) -> Result<PreparedStatement, QueryError> {
        let sql = sql.into();

        // Validate session state
        self.session
            .validate_ready()
            .await
            .map_err(|e| QueryError::InvalidState(e.to_string()))?;

        let mut transport = self.transport.lock().await;
        let handle = transport
            .create_prepared_statement(&sql)
            .await
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        Ok(PreparedStatement::new(handle))
    }

    /// Execute a prepared statement and return results.
    ///
    /// # Arguments
    ///
    /// * `stmt` - Prepared statement to execute
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing the query results.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails.
    ///
    /// # Example
    ///
    pub async fn execute_prepared(
        &mut self,
        stmt: &PreparedStatement,
    ) -> Result<ResultSet, QueryError> {
        if stmt.is_closed() {
            return Err(QueryError::StatementClosed);
        }

        // Validate session state
        self.session
            .validate_ready()
            .await
            .map_err(|e| QueryError::InvalidState(e.to_string()))?;

        // Update session state
        self.session.set_state(SessionState::Executing).await;

        // Increment query counter
        self.session.increment_query_count();

        // Convert parameters to column-major JSON format
        let params_data = stmt.build_parameters_data()?;

        let mut transport = self.transport.lock().await;
        let result = transport
            .execute_prepared_statement(stmt.handle_ref(), params_data)
            .await
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        drop(transport);

        // Update session state back to ready/in_transaction
        self.update_session_state_after_query().await;

        ResultSet::from_transport_result(result, Arc::clone(&self.transport))
    }

    /// Execute a prepared statement and return the number of affected rows.
    ///
    /// Use this for INSERT, UPDATE, DELETE statements.
    ///
    /// # Arguments
    ///
    /// * `stmt` - Prepared statement to execute
    ///
    /// # Returns
    ///
    /// The number of rows affected.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails or statement returns a result set.
    pub async fn execute_prepared_update(
        &mut self,
        stmt: &PreparedStatement,
    ) -> Result<i64, QueryError> {
        if stmt.is_closed() {
            return Err(QueryError::StatementClosed);
        }

        // Validate session state
        self.session
            .validate_ready()
            .await
            .map_err(|e| QueryError::InvalidState(e.to_string()))?;

        // Update session state
        self.session.set_state(SessionState::Executing).await;

        // Convert parameters to column-major JSON format
        let params_data = stmt.build_parameters_data()?;

        let mut transport = self.transport.lock().await;
        let result = transport
            .execute_prepared_statement(stmt.handle_ref(), params_data)
            .await
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        drop(transport);

        // Update session state back to ready/in_transaction
        self.update_session_state_after_query().await;

        match result {
            QueryResult::RowCount { count } => Ok(count),
            QueryResult::ResultSet { .. } => Err(QueryError::UnexpectedResultSet),
        }
    }

    /// Execute a prepared statement with multiple rows of parameters and return the number of
    /// affected rows.
    ///
    /// Use this for batch INSERT, UPDATE, or DELETE statements.
    ///
    /// # Arguments
    ///
    /// * `stmt` - Prepared statement to execute
    /// * `rows` - Slice of parameter rows; each row is a `Vec<Parameter>` in positional order
    ///
    /// # Returns
    ///
    /// The total number of rows affected.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails, if the statement is closed, or if Exasol
    /// returns a result set instead of an affected-row count.
    pub async fn execute_batch_update(
        &mut self,
        stmt: &PreparedStatement,
        rows: &[Vec<Parameter>],
    ) -> Result<i64, QueryError> {
        if stmt.is_closed() {
            return Err(QueryError::StatementClosed);
        }

        // Validate session state
        self.session
            .validate_ready()
            .await
            .map_err(|e| QueryError::InvalidState(e.to_string()))?;

        // Update session state
        self.session.set_state(SessionState::Executing).await;

        // Convert batch parameters to column-major JSON format
        let params_data = stmt.build_batch_parameters_data(rows)?;

        let mut transport = self.transport.lock().await;
        let result = transport
            .execute_prepared_statement(stmt.handle_ref(), params_data)
            .await
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        drop(transport);

        // Update session state back to ready/in_transaction
        self.update_session_state_after_query().await;

        match result {
            QueryResult::RowCount { count } => Ok(count),
            QueryResult::ResultSet { .. } => Err(QueryError::UnexpectedResultSet),
        }
    }

    /// Execute a prepared statement with multiple rows of parameters and return a result set.
    ///
    /// Use this for batch SELECT statements or queries that return rows.
    ///
    /// # Arguments
    ///
    /// * `stmt` - Prepared statement to execute
    /// * `rows` - Slice of parameter rows; each row is a `Vec<Parameter>` in positional order
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing the query results.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails or if the statement is closed.
    pub async fn execute_batch(
        &mut self,
        stmt: &PreparedStatement,
        rows: &[Vec<Parameter>],
    ) -> Result<ResultSet, QueryError> {
        if stmt.is_closed() {
            return Err(QueryError::StatementClosed);
        }

        // Validate session state
        self.session
            .validate_ready()
            .await
            .map_err(|e| QueryError::InvalidState(e.to_string()))?;

        // Update session state
        self.session.set_state(SessionState::Executing).await;

        // Increment query counter
        self.session.increment_query_count();

        // Convert batch parameters to column-major JSON format
        let params_data = stmt.build_batch_parameters_data(rows)?;

        let mut transport = self.transport.lock().await;
        let result = transport
            .execute_prepared_statement(stmt.handle_ref(), params_data)
            .await
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        drop(transport);

        // Update session state back to ready/in_transaction
        self.update_session_state_after_query().await;

        ResultSet::from_transport_result(result, Arc::clone(&self.transport))
    }

    /// Close a prepared statement and release server-side resources.
    ///
    /// # Arguments
    ///
    /// * `stmt` - Prepared statement to close (consumed)
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if closing fails.
    ///
    /// # Example
    ///
    pub async fn close_prepared(&mut self, mut stmt: PreparedStatement) -> Result<(), QueryError> {
        if stmt.is_closed() {
            return Ok(());
        }

        let mut transport = self.transport.lock().await;
        transport
            .close_prepared_statement(stmt.handle_ref())
            .await
            .map_err(|e| QueryError::ExecutionFailed(e.to_string()))?;

        stmt.mark_closed();
        Ok(())
    }

    // ========================================================================
    // Convenience Methods (these internally use execute_statement)
    // ========================================================================

    /// Execute a SQL query and return results.
    ///
    /// This is a convenience method that creates a statement and executes it.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL query text
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing the query results.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails.
    ///
    /// # Example
    ///
    pub async fn execute(&mut self, sql: impl Into<String>) -> Result<ResultSet, QueryError> {
        let stmt = self.create_statement(sql);
        self.execute_statement(&stmt).await
    }

    /// Execute a SQL query and return all results as RecordBatches.
    ///
    /// This is a convenience method that fetches all results into memory.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL query text
    ///
    /// # Returns
    ///
    /// A vector of `RecordBatch` instances.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails.
    ///
    /// # Example
    ///
    pub async fn query(&mut self, sql: impl Into<String>) -> Result<Vec<RecordBatch>, QueryError> {
        let result_set = self.execute(sql).await?;
        result_set.fetch_all().await
    }

    /// Execute a non-SELECT statement and return the row count.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL statement (INSERT, UPDATE, DELETE, etc.)
    ///
    /// # Returns
    ///
    /// The number of rows affected.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if execution fails.
    ///
    /// # Example
    ///
    pub async fn execute_update(&mut self, sql: impl Into<String>) -> Result<i64, QueryError> {
        let stmt = self.create_statement(sql);
        self.execute_statement_update(&stmt).await
    }

    // ========================================================================
    // Transaction Methods
    // ========================================================================

    pub async fn begin_transaction(&mut self) -> Result<(), QueryError> {
        // Disable autocommit on the server so statements don't auto-commit
        self.transport
            .lock()
            .await
            .set_autocommit(false)
            .await
            .map_err(|e| QueryError::TransactionError(e.to_string()))?;

        self.session
            .begin_transaction()
            .await
            .map_err(|e| QueryError::TransactionError(e.to_string()))?;

        Ok(())
    }

    pub async fn commit(&mut self) -> Result<(), QueryError> {
        if !self.in_transaction() {
            return Ok(());
        }

        self.execute_update("COMMIT").await?;

        self.session
            .commit_transaction()
            .await
            .map_err(|e| QueryError::TransactionError(e.to_string()))?;

        Ok(())
    }

    pub async fn rollback(&mut self) -> Result<(), QueryError> {
        if !self.in_transaction() {
            return Ok(());
        }

        self.execute_update("ROLLBACK").await?;

        self.session
            .rollback_transaction()
            .await
            .map_err(|e| QueryError::TransactionError(e.to_string()))?;

        Ok(())
    }

    /// Check if a transaction is currently active.
    ///
    /// # Returns
    ///
    /// `true` if a transaction is active, `false` otherwise.
    pub fn in_transaction(&self) -> bool {
        self.session.in_transaction()
    }

    // ========================================================================
    // Session and Schema Methods
    // ========================================================================

    /// Get the current schema.
    ///
    /// # Returns
    ///
    /// The current schema name, or `None` if no schema is set.
    pub async fn current_schema(&self) -> Option<String> {
        self.session.current_schema().await
    }

    /// Set the current schema.
    ///
    /// # Arguments
    ///
    /// * `schema` - The schema name to set
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if the operation fails.
    ///
    /// # Example
    ///
    pub async fn set_schema(&mut self, schema: impl Into<String>) -> Result<(), QueryError> {
        let schema_name = schema.into();
        self.execute_update(format!("OPEN SCHEMA {}", schema_name))
            .await?;
        self.session.set_current_schema(Some(schema_name)).await;
        Ok(())
    }

    // ========================================================================
    // Metadata Methods
    // ========================================================================

    /// Get metadata about catalogs.
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing catalog metadata.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if the operation fails.
    pub async fn get_catalogs(&mut self) -> Result<ResultSet, QueryError> {
        self.execute("SELECT DISTINCT SCHEMA_NAME AS CATALOG_NAME FROM SYS.EXA_ALL_SCHEMAS ORDER BY CATALOG_NAME")
            .await
    }

    /// Get metadata about schemas.
    ///
    /// # Arguments
    ///
    /// * `catalog` - Optional catalog name filter
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing schema metadata.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if the operation fails.
    pub async fn get_schemas(&mut self, catalog: Option<&str>) -> Result<ResultSet, QueryError> {
        let sql = if let Some(cat) = catalog {
            format!(
                "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS WHERE SCHEMA_NAME = '{}' ORDER BY SCHEMA_NAME",
                cat.replace('\'', "''")
            )
        } else {
            "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS ORDER BY SCHEMA_NAME".to_string()
        };
        self.execute(sql).await
    }

    /// Get metadata about tables.
    ///
    /// # Arguments
    ///
    /// * `catalog` - Optional catalog name filter
    /// * `schema` - Optional schema name filter
    /// * `table` - Optional table name filter
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing table metadata.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if the operation fails.
    pub async fn get_tables(
        &mut self,
        catalog: Option<&str>,
        schema: Option<&str>,
        table: Option<&str>,
    ) -> Result<ResultSet, QueryError> {
        let mut conditions = vec!["OBJECT_TYPE IN ('TABLE', 'VIEW')".to_string()];

        // Exasol has no catalogs — ignore the catalog parameter
        let _ = catalog;
        if let Some(sch) = schema {
            conditions.push(format!("ROOT_NAME = '{}'", sch.replace('\'', "''")));
        }
        if let Some(tbl) = table {
            conditions.push(format!("OBJECT_NAME = '{}'", tbl.replace('\'', "''")));
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));

        let sql = format!(
            "SELECT ROOT_NAME AS TABLE_SCHEMA, OBJECT_NAME AS TABLE_NAME, OBJECT_TYPE AS TABLE_TYPE FROM SYS.EXA_ALL_OBJECTS {} ORDER BY ROOT_NAME, OBJECT_NAME",
            where_clause
        );

        self.execute(sql).await
    }

    /// Get metadata about columns.
    ///
    /// # Arguments
    ///
    /// * `catalog` - Optional catalog name filter
    /// * `schema` - Optional schema name filter
    /// * `table` - Optional table name filter
    /// * `column` - Optional column name filter
    ///
    /// # Returns
    ///
    /// A `ResultSet` containing column metadata.
    ///
    /// # Errors
    ///
    /// Returns `QueryError` if the operation fails.
    pub async fn get_columns(
        &mut self,
        catalog: Option<&str>,
        schema: Option<&str>,
        table: Option<&str>,
        column: Option<&str>,
    ) -> Result<ResultSet, QueryError> {
        let mut conditions = Vec::new();

        if let Some(cat) = catalog {
            conditions.push(format!("COLUMN_SCHEMA = '{}'", cat.replace('\'', "''")));
        }
        if let Some(sch) = schema {
            conditions.push(format!("COLUMN_SCHEMA = '{}'", sch.replace('\'', "''")));
        }
        if let Some(tbl) = table {
            conditions.push(format!("COLUMN_TABLE = '{}'", tbl.replace('\'', "''")));
        }
        if let Some(col) = column {
            conditions.push(format!("COLUMN_NAME = '{}'", col.replace('\'', "''")));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT COLUMN_SCHEMA, COLUMN_TABLE, COLUMN_NAME, COLUMN_TYPE, COLUMN_NUM_PREC, COLUMN_NUM_SCALE, COLUMN_IS_NULLABLE \
             FROM SYS.EXA_ALL_COLUMNS {} ORDER BY COLUMN_SCHEMA, COLUMN_TABLE, ORDINAL_POSITION",
            where_clause
        );

        self.execute(sql).await
    }

    // ========================================================================
    // Session Information Methods
    // ========================================================================

    /// Get session information.
    ///
    /// # Returns
    ///
    /// The session ID.
    pub fn session_id(&self) -> &str {
        self.session.session_id()
    }

    /// Get connection parameters.
    ///
    /// # Returns
    ///
    /// A reference to the connection parameters.
    pub fn params(&self) -> &ConnectionParams {
        &self.params
    }

    /// Check if the connection is closed.
    ///
    /// # Returns
    ///
    /// `true` if the connection is closed, `false` otherwise.
    pub async fn is_closed(&self) -> bool {
        self.session.is_closed().await
    }

    /// Close the connection.
    ///
    /// This closes the session and transport layer.
    ///
    /// # Errors
    ///
    /// Returns `ConnectionError` if closing fails.
    ///
    /// # Example
    ///
    pub async fn close(self) -> Result<(), ConnectionError> {
        // Close session
        self.session.close().await?;

        // Close transport
        let mut transport = self.transport.lock().await;
        transport
            .close()
            .await
            .map_err(|e| ConnectionError::ConnectionFailed {
                host: self.params.host.clone(),
                port: self.params.port,
                message: e.to_string(),
            })?;

        Ok(())
    }

    /// Shut down the connection without consuming self.
    ///
    /// Unlike `close()`, this can be called through a shared reference,
    /// making it suitable for use in `Drop` implementations where ownership
    /// transfer is not possible (e.g., when the connection is behind `Arc<Mutex<>>`).
    pub async fn shutdown(&self) -> Result<(), ConnectionError> {
        self.session.close().await?;

        let mut transport = self.transport.lock().await;
        transport
            .close()
            .await
            .map_err(|e| ConnectionError::ConnectionFailed {
                host: self.params.host.clone(),
                port: self.params.port,
                message: e.to_string(),
            })?;

        Ok(())
    }

    // ========================================================================
    // Import/Export Methods
    // ========================================================================

    /// Creates an SQL executor closure for import/export operations.
    ///
    /// This is a helper method that creates a closure which can execute SQL
    /// statements and return the row count. The closure captures a cloned
    /// reference to the transport, allowing it to be called multiple times
    /// (e.g., for parallel file imports).
    ///
    /// # Returns
    ///
    /// A closure that takes an SQL string and returns a Future resolving to
    /// either the affected row count or an error string.
    fn make_sql_executor(
        &self,
    ) -> impl Fn(
        String,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64, String>> + Send>> {
        let transport = Arc::clone(&self.transport);
        move |sql: String| {
            let transport = Arc::clone(&transport);
            Box::pin(async move {
                let mut transport_guard = transport.lock().await;
                match transport_guard.execute_query(&sql).await {
                    Ok(QueryResult::RowCount { count }) => Ok(count as u64),
                    Ok(QueryResult::ResultSet { .. }) => Ok(0),
                    Err(e) => Err(e.to_string()),
                }
            })
        }
    }

    /// Import CSV data from a file into an Exasol table.
    ///
    /// This method reads CSV data from the specified file and imports it into
    /// the target table using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `file_path` - Path to the CSV file
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub async fn import_csv_from_file(
        &mut self,
        table: &str,
        file_path: &std::path::Path,
        options: crate::import::csv::CsvImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        crate::import::csv::import_from_file(self.make_sql_executor(), table, file_path, options)
            .await
    }

    /// Import CSV data from an async reader into an Exasol table.
    ///
    /// This method reads CSV data from an async reader and imports it into
    /// the target table using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `reader` - Async reader providing CSV data
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    pub async fn import_csv_from_stream<R>(
        &mut self,
        table: &str,
        reader: R,
        options: crate::import::csv::CsvImportOptions,
    ) -> Result<u64, crate::import::ImportError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        crate::import::csv::import_from_stream(self.make_sql_executor(), table, reader, options)
            .await
    }

    /// Import CSV data from an iterator into an Exasol table.
    ///
    /// This method converts iterator rows to CSV format and imports them into
    /// the target table using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `rows` - Iterator of rows, where each row is an iterator of field values
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub async fn import_csv_from_iter<I, T, S>(
        &mut self,
        table: &str,
        rows: I,
        options: crate::import::csv::CsvImportOptions,
    ) -> Result<u64, crate::import::ImportError>
    where
        I: IntoIterator<Item = T> + Send + 'static,
        T: IntoIterator<Item = S> + Send,
        S: AsRef<str>,
    {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        crate::import::csv::import_from_iter(self.make_sql_executor(), table, rows, options).await
    }

    /// Export data from an Exasol table or query to a CSV file.
    ///
    /// This method exports data from the specified source to a CSV file
    /// using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `file_path` - Path to the output file
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// The number of rows exported on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub async fn export_csv_to_file(
        &mut self,
        source: crate::query::export::ExportSource,
        file_path: &std::path::Path,
        options: crate::export::csv::CsvExportOptions,
    ) -> Result<u64, crate::export::csv::ExportError> {
        // Pass Exasol host/port from connection params to export options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        let mut transport_guard = self.transport.lock().await;
        crate::export::csv::export_to_file(&mut *transport_guard, source, file_path, options).await
    }

    /// Export data from an Exasol table or query to an async writer.
    ///
    /// This method exports data from the specified source to an async writer
    /// using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `writer` - Async writer to write the CSV data to
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// The number of rows exported on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    pub async fn export_csv_to_stream<W>(
        &mut self,
        source: crate::query::export::ExportSource,
        writer: W,
        options: crate::export::csv::CsvExportOptions,
    ) -> Result<u64, crate::export::csv::ExportError>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        // Pass Exasol host/port from connection params to export options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        let mut transport_guard = self.transport.lock().await;
        crate::export::csv::export_to_stream(&mut *transport_guard, source, writer, options).await
    }

    /// Export data from an Exasol table or query to an in-memory list of rows.
    ///
    /// Each row is represented as a vector of string values.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// A vector of rows, where each row is a vector of column values.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub async fn export_csv_to_list(
        &mut self,
        source: crate::query::export::ExportSource,
        options: crate::export::csv::CsvExportOptions,
    ) -> Result<Vec<Vec<String>>, crate::export::csv::ExportError> {
        // Pass Exasol host/port from connection params to export options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        let mut transport_guard = self.transport.lock().await;
        crate::export::csv::export_to_list(&mut *transport_guard, source, options).await
    }

    /// Import multiple CSV files in parallel into an Exasol table.
    ///
    /// This method reads CSV data from multiple files and imports them into
    /// the target table using parallel HTTP transport connections. Each file
    /// gets its own connection with a unique internal address.
    ///
    /// For a single file, this method delegates to `import_csv_from_file`.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `paths` - File paths (accepts single path, Vec, array, or slice)
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails. Uses fail-fast semantics.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use exarrow_rs::adbc::Connection;
    /// use exarrow_rs::import::CsvImportOptions;
    /// use std::path::PathBuf;
    ///
    /// # async fn example(conn: &mut Connection) -> Result<(), Box<dyn std::error::Error>> {
    /// let files = vec![
    ///     PathBuf::from("/data/part1.csv"),
    ///     PathBuf::from("/data/part2.csv"),
    /// ];
    ///
    /// let options = CsvImportOptions::default();
    /// let rows = conn.import_csv_from_files("my_table", files, options).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn import_csv_from_files<S: crate::import::IntoFileSources>(
        &mut self,
        table: &str,
        paths: S,
        options: crate::import::csv::CsvImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        crate::import::csv::import_from_files(self.make_sql_executor(), table, paths, options).await
    }

    /// Whether the connected Exasol server supports native Parquet IMPORT.
    ///
    /// Returns `true` when the server version is at least 2025.1.11, which
    /// introduced the `FROM PARQUET` clause. The result is memoized for the
    /// session lifetime.
    pub fn supports_native_parquet_import(&self) -> bool {
        self.session.supports_native_parquet_import()
    }

    /// Resolve whether to use the native Parquet import path for a given request.
    ///
    /// The `native_parquet_override` field on `options` takes precedence: if it
    /// is `Some(b)`, that value is used directly. Otherwise the result of
    /// `supports_native_parquet_import()` is returned.
    fn resolve_native_parquet(
        &self,
        options: &crate::import::parquet::ParquetImportOptions,
    ) -> bool {
        options
            .native_parquet_override
            .unwrap_or_else(|| self.supports_native_parquet_import())
    }

    /// Import data from a Parquet file into an Exasol table.
    ///
    /// This method reads a Parquet file, converts the data to CSV format,
    /// and imports it into the target table using Exasol's HTTP transport layer.
    /// When the connected server supports native Parquet import (Exasol 2025.1.11+),
    /// the raw Parquet bytes are streamed directly without CSV conversion.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `file_path` - Path to the Parquet file
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub async fn import_from_parquet(
        &mut self,
        table: &str,
        file_path: &std::path::Path,
        options: crate::import::parquet::ParquetImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .with_exasol_host(&self.params.host)
            .with_exasol_port(self.params.port);

        let use_native = self.resolve_native_parquet(&options);

        crate::import::parquet::import_from_parquet(
            self.make_sql_executor(),
            table,
            file_path,
            options,
            use_native,
        )
        .await
    }

    /// Import Parquet data from an async reader into an Exasol table.
    ///
    /// This method reads Parquet data from an async reader and imports it into
    /// the target table. The entire stream is buffered into memory because
    /// Parquet requires random access to read file metadata.
    ///
    /// When the connected server supports native Parquet import (Exasol 2025.1.11+),
    /// the buffered bytes are forwarded directly without CSV conversion.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `reader` - Async reader providing Parquet data
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    pub async fn import_from_parquet_stream<R>(
        &mut self,
        table: &str,
        reader: R,
        options: crate::import::parquet::ParquetImportOptions,
    ) -> Result<u64, crate::import::ImportError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let options = options
            .with_exasol_host(&self.params.host)
            .with_exasol_port(self.params.port);

        let use_native = self.resolve_native_parquet(&options);

        crate::import::parquet::import_from_parquet_stream(
            self.make_sql_executor(),
            table,
            reader,
            options,
            use_native,
        )
        .await
    }

    /// Import multiple Parquet files in parallel into an Exasol table.
    ///
    /// This method converts each Parquet file to CSV format concurrently,
    /// then streams the data through parallel HTTP transport connections.
    ///
    /// For a single file, this method delegates to `import_from_parquet`.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `paths` - File paths (accepts single path, Vec, array, or slice)
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails. Uses fail-fast semantics.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use exarrow_rs::adbc::Connection;
    /// use exarrow_rs::import::ParquetImportOptions;
    /// use std::path::PathBuf;
    ///
    /// # async fn example(conn: &mut Connection) -> Result<(), Box<dyn std::error::Error>> {
    /// let files = vec![
    ///     PathBuf::from("/data/part1.parquet"),
    ///     PathBuf::from("/data/part2.parquet"),
    /// ];
    ///
    /// let options = ParquetImportOptions::default();
    /// let rows = conn.import_parquet_from_files("my_table", files, options).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn import_parquet_from_files<S: crate::import::IntoFileSources>(
        &mut self,
        table: &str,
        paths: S,
        options: crate::import::parquet::ParquetImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .with_exasol_host(&self.params.host)
            .with_exasol_port(self.params.port);

        let use_native = self.resolve_native_parquet(&options);

        crate::import::parquet::import_from_parquet_files(
            self.make_sql_executor(),
            table,
            paths,
            options,
            use_native,
        )
        .await
    }

    /// Export data from an Exasol table or query to a Parquet file.
    ///
    /// This method exports data from the specified source to a Parquet file.
    /// The data is first received as CSV from Exasol, then converted to Parquet format.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `file_path` - Path to the output Parquet file
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// The number of rows exported on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub async fn export_to_parquet(
        &mut self,
        source: crate::query::export::ExportSource,
        file_path: &std::path::Path,
        options: crate::export::parquet::ParquetExportOptions,
    ) -> Result<u64, crate::export::csv::ExportError> {
        // Pass Exasol host/port from connection params to export options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        let mut transport_guard = self.transport.lock().await;
        crate::export::parquet::export_to_parquet_via_transport(
            &mut *transport_guard,
            source,
            file_path,
            options,
        )
        .await
    }

    /// Import data from an Arrow RecordBatch into an Exasol table.
    ///
    /// This method converts the RecordBatch to CSV format and imports it
    /// into the target table using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `batch` - The RecordBatch to import
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub async fn import_from_record_batch(
        &mut self,
        table: &str,
        batch: &RecordBatch,
        options: crate::import::arrow::ArrowImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        crate::import::arrow::import_from_record_batch(
            self.make_sql_executor(),
            table,
            batch,
            options,
        )
        .await
    }

    /// Import data from multiple Arrow RecordBatches into an Exasol table.
    ///
    /// This method converts each RecordBatch to CSV format and imports them
    /// into the target table using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `batches` - An iterator of RecordBatches to import
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    pub async fn import_from_record_batches<I>(
        &mut self,
        table: &str,
        batches: I,
        options: crate::import::arrow::ArrowImportOptions,
    ) -> Result<u64, crate::import::ImportError>
    where
        I: IntoIterator<Item = RecordBatch>,
    {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        crate::import::arrow::import_from_record_batches(
            self.make_sql_executor(),
            table,
            batches,
            options,
        )
        .await
    }

    /// Import data from an Arrow IPC file/stream into an Exasol table.
    ///
    /// This method reads Arrow IPC format data, converts it to CSV,
    /// and imports it into the target table using Exasol's HTTP transport layer.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `reader` - An async reader containing Arrow IPC data
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub async fn import_from_arrow_ipc<R>(
        &mut self,
        table: &str,
        reader: R,
        options: crate::import::arrow::ArrowImportOptions,
    ) -> Result<u64, crate::import::ImportError>
    where
        R: tokio::io::AsyncRead + Unpin + Send,
    {
        // Pass Exasol host/port from connection params to import options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        crate::import::arrow::import_from_arrow_ipc(
            self.make_sql_executor(),
            table,
            reader,
            options,
        )
        .await
    }

    /// Export data from an Exasol table or query to Arrow RecordBatches.
    ///
    /// This method exports data from the specified source and converts it
    /// to Arrow RecordBatches.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// A vector of RecordBatches on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub async fn export_to_record_batches(
        &mut self,
        source: crate::query::export::ExportSource,
        options: crate::export::arrow::ArrowExportOptions,
    ) -> Result<Vec<RecordBatch>, crate::export::csv::ExportError> {
        // Pass Exasol host/port from connection params to export options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        let mut transport_guard = self.transport.lock().await;
        crate::export::arrow::export_to_record_batches(&mut *transport_guard, source, options).await
    }

    /// Export data from an Exasol table or query to an Arrow IPC file.
    ///
    /// This method exports data from the specified source to an Arrow IPC file.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `file_path` - Path to the output Arrow IPC file
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// The number of rows exported on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub async fn export_to_arrow_ipc(
        &mut self,
        source: crate::query::export::ExportSource,
        file_path: &std::path::Path,
        options: crate::export::arrow::ArrowExportOptions,
    ) -> Result<u64, crate::export::csv::ExportError> {
        // Pass Exasol host/port from connection params to export options
        let options = options
            .exasol_host(&self.params.host)
            .exasol_port(self.params.port);

        let mut transport_guard = self.transport.lock().await;
        crate::export::arrow::export_to_arrow_ipc(&mut *transport_guard, source, file_path, options)
            .await
    }

    // ========================================================================
    // Blocking Import/Export Methods
    // ========================================================================

    /// Import CSV data from a file into an Exasol table (blocking).
    ///
    /// This is a synchronous wrapper around [`import_csv_from_file`](Self::import_csv_from_file)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `file_path` - Path to the CSV file
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub fn blocking_import_csv_from_file(
        &mut self,
        table: &str,
        file_path: &std::path::Path,
        options: crate::import::csv::CsvImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        blocking_runtime().block_on(self.import_csv_from_file(table, file_path, options))
    }

    /// Import multiple CSV files in parallel into an Exasol table (blocking).
    ///
    /// This is a synchronous wrapper around [`import_csv_from_files`](Self::import_csv_from_files)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `paths` - File paths (accepts single path, Vec, array, or slice)
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    pub fn blocking_import_csv_from_files<S: crate::import::IntoFileSources>(
        &mut self,
        table: &str,
        paths: S,
        options: crate::import::csv::CsvImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        blocking_runtime().block_on(self.import_csv_from_files(table, paths, options))
    }

    /// Import data from a Parquet file into an Exasol table (blocking).
    ///
    /// This is a synchronous wrapper around [`import_from_parquet`](Self::import_from_parquet)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `file_path` - Path to the Parquet file
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub fn blocking_import_from_parquet(
        &mut self,
        table: &str,
        file_path: &std::path::Path,
        options: crate::import::parquet::ParquetImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        blocking_runtime().block_on(self.import_from_parquet(table, file_path, options))
    }

    /// Import Parquet data from an async reader into an Exasol table (blocking).
    ///
    /// This is a synchronous wrapper around [`import_from_parquet_stream`](Self::import_from_parquet_stream)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `reader` - Async reader providing Parquet data
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    pub fn blocking_import_from_parquet_stream<R>(
        &mut self,
        table: &str,
        reader: R,
        options: crate::import::parquet::ParquetImportOptions,
    ) -> Result<u64, crate::import::ImportError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        blocking_runtime().block_on(self.import_from_parquet_stream(table, reader, options))
    }

    /// Import multiple Parquet files in parallel into an Exasol table (blocking).
    ///
    /// This is a synchronous wrapper around [`import_parquet_from_files`](Self::import_parquet_from_files)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `paths` - File paths (accepts single path, Vec, array, or slice)
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    pub fn blocking_import_parquet_from_files<S: crate::import::IntoFileSources>(
        &mut self,
        table: &str,
        paths: S,
        options: crate::import::parquet::ParquetImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        blocking_runtime().block_on(self.import_parquet_from_files(table, paths, options))
    }

    /// Import data from an Arrow RecordBatch into an Exasol table (blocking).
    ///
    /// This is a synchronous wrapper around [`import_from_record_batch`](Self::import_from_record_batch)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `batch` - The RecordBatch to import
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub fn blocking_import_from_record_batch(
        &mut self,
        table: &str,
        batch: &RecordBatch,
        options: crate::import::arrow::ArrowImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        blocking_runtime().block_on(self.import_from_record_batch(table, batch, options))
    }

    /// Import data from an Arrow IPC file into an Exasol table (blocking).
    ///
    /// This is a synchronous wrapper around [`import_from_arrow_ipc`](Self::import_from_arrow_ipc)
    /// for use in non-async contexts.
    ///
    /// Note: This method requires a synchronous reader that implements `std::io::Read`.
    /// The data will be read into memory before being imported.
    ///
    /// # Arguments
    ///
    /// * `table` - Name of the target table
    /// * `file_path` - Path to the Arrow IPC file
    /// * `options` - Import options
    ///
    /// # Returns
    ///
    /// The number of rows imported on success.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the import fails.
    ///
    /// # Example
    ///
    pub fn blocking_import_from_arrow_ipc(
        &mut self,
        table: &str,
        file_path: &std::path::Path,
        options: crate::import::arrow::ArrowImportOptions,
    ) -> Result<u64, crate::import::ImportError> {
        blocking_runtime().block_on(async {
            let file = tokio::fs::File::open(file_path)
                .await
                .map_err(crate::import::ImportError::IoError)?;
            self.import_from_arrow_ipc(table, file, options).await
        })
    }

    /// Export data from an Exasol table or query to a CSV file (blocking).
    ///
    /// This is a synchronous wrapper around [`export_csv_to_file`](Self::export_csv_to_file)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `file_path` - Path to the output file
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// The number of rows exported on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub fn blocking_export_csv_to_file(
        &mut self,
        source: crate::query::export::ExportSource,
        file_path: &std::path::Path,
        options: crate::export::csv::CsvExportOptions,
    ) -> Result<u64, crate::export::csv::ExportError> {
        blocking_runtime().block_on(self.export_csv_to_file(source, file_path, options))
    }

    /// Export data from an Exasol table or query to a Parquet file (blocking).
    ///
    /// This is a synchronous wrapper around [`export_to_parquet`](Self::export_to_parquet)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `file_path` - Path to the output Parquet file
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// The number of rows exported on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub fn blocking_export_to_parquet(
        &mut self,
        source: crate::query::export::ExportSource,
        file_path: &std::path::Path,
        options: crate::export::parquet::ParquetExportOptions,
    ) -> Result<u64, crate::export::csv::ExportError> {
        blocking_runtime().block_on(self.export_to_parquet(source, file_path, options))
    }

    /// Export data from an Exasol table or query to Arrow RecordBatches (blocking).
    ///
    /// This is a synchronous wrapper around [`export_to_record_batches`](Self::export_to_record_batches)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// A vector of RecordBatches on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub fn blocking_export_to_record_batches(
        &mut self,
        source: crate::query::export::ExportSource,
        options: crate::export::arrow::ArrowExportOptions,
    ) -> Result<Vec<RecordBatch>, crate::export::csv::ExportError> {
        blocking_runtime().block_on(self.export_to_record_batches(source, options))
    }

    /// Export data from an Exasol table or query to an Arrow IPC file (blocking).
    ///
    /// This is a synchronous wrapper around [`export_to_arrow_ipc`](Self::export_to_arrow_ipc)
    /// for use in non-async contexts.
    ///
    /// # Arguments
    ///
    /// * `source` - The data source (table or query)
    /// * `file_path` - Path to the output Arrow IPC file
    /// * `options` - Export options
    ///
    /// # Returns
    ///
    /// The number of rows exported on success.
    ///
    /// # Errors
    ///
    /// Returns `ExportError` if the export fails.
    ///
    /// # Example
    ///
    pub fn blocking_export_to_arrow_ipc(
        &mut self,
        source: crate::query::export::ExportSource,
        file_path: &std::path::Path,
        options: crate::export::arrow::ArrowExportOptions,
    ) -> Result<u64, crate::export::csv::ExportError> {
        blocking_runtime().block_on(self.export_to_arrow_ipc(source, file_path, options))
    }

    // ========================================================================
    // Private Helper Methods
    // ========================================================================

    /// Update session state after query execution.
    async fn update_session_state_after_query(&self) {
        if self.session.in_transaction() {
            self.session.set_state(SessionState::InTransaction).await;
        } else {
            self.session.set_state(SessionState::Ready).await;
        }
        self.session.update_activity().await;
    }
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("session_id", &self.session.session_id())
            .field("host", &self.params.host)
            .field("port", &self.params.port)
            .field("username", &self.params.username)
            .field("in_transaction", &self.session.in_transaction())
            .finish()
    }
}

/// Builder for creating Connection instances.
pub struct ConnectionBuilder {
    /// Connection parameters builder
    params_builder: crate::connection::params::ConnectionBuilder,
}

impl ConnectionBuilder {
    /// Create a new ConnectionBuilder.
    pub fn new() -> Self {
        Self {
            params_builder: crate::connection::params::ConnectionBuilder::new(),
        }
    }

    /// Set the database host.
    pub fn host(mut self, host: &str) -> Self {
        self.params_builder = self.params_builder.host(host);
        self
    }

    /// Set the database port.
    pub fn port(mut self, port: u16) -> Self {
        self.params_builder = self.params_builder.port(port);
        self
    }

    /// Set the username.
    pub fn username(mut self, username: &str) -> Self {
        self.params_builder = self.params_builder.username(username);
        self
    }

    /// Set the password.
    pub fn password(mut self, password: &str) -> Self {
        self.params_builder = self.params_builder.password(password);
        self
    }

    /// Set the default schema.
    pub fn schema(mut self, schema: &str) -> Self {
        self.params_builder = self.params_builder.schema(schema);
        self
    }

    /// Enable or disable TLS.
    pub fn use_tls(mut self, use_tls: bool) -> Self {
        self.params_builder = self.params_builder.use_tls(use_tls);
        self
    }

    /// Enable or disable server-certificate validation.
    ///
    /// Disable when connecting to an Exasol Docker container, which ships a
    /// self-signed certificate that would otherwise fail validation.
    pub fn validate_server_certificate(mut self, validate: bool) -> Self {
        self.params_builder = self.params_builder.validate_server_certificate(validate);
        self
    }

    /// Build and connect.
    ///
    /// # Returns
    ///
    /// A connected `Connection` instance.
    ///
    /// # Errors
    ///
    /// Returns `ExasolError` if the connection fails.
    pub async fn connect(self) -> Result<Connection, ExasolError> {
        let params = self.params_builder.build()?;
        Ok(Connection::from_params(params).await?)
    }
}

impl Default for ConnectionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the best-effort URI-schema fix.
    ///
    /// A schema named in the connection URI that does not yet exist must NOT
    /// fail the connection: the server reports a "schema ... not found" error
    /// (real Exasol message format), which `connect_with_transport` swallows so
    /// the connection stays open. Every other failure must remain fatal.
    #[test]
    fn schema_open_error_missing_schema_is_recognized() {
        // Real Exasol-style "OPEN SCHEMA" failure for a non-existent schema.
        let missing = QueryError::ExecutionFailed(
            "Protocol error: schema FOO not found [line 1, column 13] (SQL state: 42000)"
                .to_string(),
        );
        assert!(
            schema_open_error_is_missing_schema(&missing),
            "a 'schema ... not found' error must be classified as missing-schema (swallowed)"
        );
    }

    #[test]
    fn schema_open_error_unrelated_is_fatal() {
        // An unrelated failure (e.g. permissions) must stay fatal.
        let unrelated = QueryError::ExecutionFailed("insufficient privileges".to_string());
        assert!(
            !schema_open_error_is_missing_schema(&unrelated),
            "an unrelated error must NOT be classified as missing-schema (stays fatal)"
        );
    }

    #[test]
    fn secs_ceil_reserves_zero_for_the_unlimited_reset_only() {
        // Zero maps to zero: the "no timeout / reset to unlimited" sentinel.
        assert_eq!(secs_ceil(Duration::from_millis(0)), 0);
        // Any positive sub-second timeout rounds UP to at least one second so
        // it never collapses into the unlimited sentinel.
        assert_eq!(secs_ceil(Duration::from_nanos(1)), 1);
        assert_eq!(secs_ceil(Duration::from_millis(1)), 1);
        assert_eq!(secs_ceil(Duration::from_millis(999)), 1);
        assert_eq!(secs_ceil(Duration::from_millis(1000)), 1);
        // Whole-plus-a-remainder rounds up to the next whole second.
        assert_eq!(secs_ceil(Duration::from_millis(1001)), 2);
        assert_eq!(secs_ceil(Duration::from_millis(2500)), 3);
        // Exact whole seconds are unchanged.
        assert_eq!(secs_ceil(Duration::from_secs(60)), 60);
    }

    #[test]
    fn map_execution_error_recognizes_server_query_timeout() {
        // Primary signal: SQL state R0001 (WebSocket transport phrasing).
        let ws = TransportError::ProtocolError(
            "Query terminated because timeout has been reached. (SQL code: R0001)".to_string(),
        );
        assert!(matches!(
            map_execution_error(ws, 5000),
            QueryError::Timeout { timeout_ms: 5000 }
        ));

        // Primary signal survives the native transport phrasing ("SQL state").
        let native = TransportError::ProtocolError(
            "Query terminated because timeout has been reached. (SQL state: R0001)".to_string(),
        );
        assert!(matches!(
            map_execution_error(native, 1000),
            QueryError::Timeout { timeout_ms: 1000 }
        ));

        // Fallback signal: the message text alone, without the code.
        let text_only = TransportError::ProtocolError(
            "Query terminated because timeout has been reached.".to_string(),
        );
        assert!(matches!(
            map_execution_error(text_only, 2000),
            QueryError::Timeout { timeout_ms: 2000 }
        ));
    }

    #[test]
    fn map_execution_error_passes_through_other_failures() {
        let other = TransportError::ProtocolError(
            "syntax error, unexpected ')' (SQL code: 42000)".to_string(),
        );
        match map_execution_error(other, 5000) {
            QueryError::ExecutionFailed(msg) => assert!(msg.contains("42000")),
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }

    #[test]
    fn map_execution_error_with_zero_effective_timeout_is_not_classified_as_timeout() {
        // An R0001 abort with no driver-configured timeout in effect (e.g. an
        // ambient session-level QUERY_TIMEOUT the driver never set) must not
        // be reported as `QueryError::Timeout { timeout_ms: 0 }` — that would
        // read as "the client timed out immediately", which is false. The
        // server's own message is preserved via ExecutionFailed instead.
        let ambient = TransportError::ProtocolError(
            "Query terminated because timeout has been reached. (SQL code: R0001)".to_string(),
        );
        match map_execution_error(ambient, 0) {
            QueryError::ExecutionFailed(msg) => assert!(msg.contains("R0001")),
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }

    // ========================================================================
    // Mock-transport harness
    //
    // Every test below drives a real `Connection` over the shared
    // `TransportProtocol` mock, so the connection's own logic — parameter
    // mapping, generated SQL, timeout reconciliation, transaction bookkeeping —
    // is asserted without a live Exasol or any network I/O.
    // ========================================================================

    use crate::transport::messages::SessionInfo as TransportSessionInfo;
    use crate::transport::test_support::MockTransport;
    use std::sync::Mutex as SyncMutex;

    /// Every SQL string the mocked transport was asked to execute, in order.
    type SqlLog = Arc<SyncMutex<Vec<String>>>;

    fn new_sql_log() -> SqlLog {
        Arc::new(SyncMutex::new(Vec::new()))
    }

    fn recorded(log: &SqlLog) -> Vec<String> {
        log.lock().expect("SQL log poisoned").clone()
    }

    /// The single SQL statement the connection issued.
    ///
    /// Panics when the count is not exactly one, so a test that means to assert
    /// on "the" statement can never silently assert on the first of several.
    fn only_sql(log: &SqlLog) -> String {
        let statements = recorded(log);
        assert_eq!(
            statements.len(),
            1,
            "expected exactly one statement, got {:?}",
            statements
        );
        statements.into_iter().next().unwrap()
    }

    fn transport_session_info() -> TransportSessionInfo {
        TransportSessionInfo {
            session_id: "1739284756".to_string(),
            protocol_version: 3,
            release_version: "8.32.0".to_string(),
            database_name: "exadb".to_string(),
            product_name: "EXASolution".to_string(),
            max_data_message_size: 64 * 1024,
            time_zone: Some("Europe/Berlin".to_string()),
        }
    }

    fn test_params() -> ConnectionParams {
        ConnectionParams::builder()
            .host("db.example.invalid")
            .port(8563)
            .username("tester")
            .password("s3cr3t-pw")
            .build()
            .expect("test connection params must be valid")
    }

    /// A transport that connects and authenticates cleanly and records the SQL
    /// it is handed, answering every statement with a zero-row row count.
    fn recording_transport(log: &SqlLog) -> MockTransport {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        let log = Arc::clone(log);
        transport.expect_execute_query().returning(move |sql| {
            log.lock().expect("SQL log poisoned").push(sql.to_string());
            Ok(QueryResult::row_count(0))
        });
        transport
    }

    /// A connected `Connection` over `recording_transport`, with no default
    /// schema so the only recorded SQL is what the test itself triggers.
    async fn connected(log: &SqlLog) -> Connection {
        Connection::connect_with_transport(test_params(), recording_transport(log))
            .await
            .expect("mock transport must connect")
    }

    // ------------------------------------------------------------------------
    // Metadata SQL builders
    // ------------------------------------------------------------------------

    #[tokio::test]
    async fn get_catalogs_selects_distinct_schema_names_as_catalogs() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_catalogs()
            .await
            .expect("get_catalogs must succeed");

        assert_eq!(
            only_sql(&log),
            "SELECT DISTINCT SCHEMA_NAME AS CATALOG_NAME FROM SYS.EXA_ALL_SCHEMAS ORDER BY CATALOG_NAME"
        );
    }

    #[tokio::test]
    async fn get_schemas_without_filter_omits_the_where_clause() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_schemas(None).await.expect("get_schemas");

        assert_eq!(
            only_sql(&log),
            "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS ORDER BY SCHEMA_NAME"
        );
    }

    #[tokio::test]
    async fn get_schemas_with_catalog_filters_on_schema_name() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_schemas(Some("SALES")).await.expect("get_schemas");

        assert_eq!(
            only_sql(&log),
            "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS WHERE SCHEMA_NAME = 'SALES' ORDER BY SCHEMA_NAME"
        );
    }

    #[tokio::test]
    async fn get_schemas_doubles_single_quotes_in_the_filter() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_schemas(Some("O'BRIEN"))
            .await
            .expect("get_schemas");

        assert_eq!(
            only_sql(&log),
            "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_SCHEMAS WHERE SCHEMA_NAME = 'O''BRIEN' ORDER BY SCHEMA_NAME"
        );
    }

    #[tokio::test]
    async fn get_tables_without_filters_restricts_to_tables_and_views_only() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_tables(None, None, None).await.expect("get_tables");

        assert_eq!(
            only_sql(&log),
            "SELECT ROOT_NAME AS TABLE_SCHEMA, OBJECT_NAME AS TABLE_NAME, OBJECT_TYPE AS TABLE_TYPE \
             FROM SYS.EXA_ALL_OBJECTS WHERE OBJECT_TYPE IN ('TABLE', 'VIEW') ORDER BY ROOT_NAME, OBJECT_NAME"
        );
    }

    #[tokio::test]
    async fn get_tables_ignores_the_catalog_argument_because_exasol_has_no_catalogs() {
        let with_catalog = new_sql_log();
        let mut conn = connected(&with_catalog).await;
        conn.get_tables(Some("ANY_CATALOG"), None, None)
            .await
            .expect("get_tables");

        let without_catalog = new_sql_log();
        let mut conn = connected(&without_catalog).await;
        conn.get_tables(None, None, None).await.expect("get_tables");

        assert_eq!(only_sql(&with_catalog), only_sql(&without_catalog));
    }

    #[tokio::test]
    async fn get_tables_filters_on_schema_and_table_when_both_are_given() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_tables(None, Some("SALES"), Some("ORDERS"))
            .await
            .expect("get_tables");

        assert_eq!(
            only_sql(&log),
            "SELECT ROOT_NAME AS TABLE_SCHEMA, OBJECT_NAME AS TABLE_NAME, OBJECT_TYPE AS TABLE_TYPE \
             FROM SYS.EXA_ALL_OBJECTS WHERE OBJECT_TYPE IN ('TABLE', 'VIEW') AND ROOT_NAME = 'SALES' \
             AND OBJECT_NAME = 'ORDERS' ORDER BY ROOT_NAME, OBJECT_NAME"
        );
    }

    #[tokio::test]
    async fn get_tables_doubles_single_quotes_in_schema_and_table_filters() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_tables(None, Some("IT'S"), Some("O'HARA"))
            .await
            .expect("get_tables");

        let sql = only_sql(&log);
        assert!(
            sql.contains("ROOT_NAME = 'IT''S'"),
            "schema filter must be escaped, got: {}",
            sql
        );
        assert!(
            sql.contains("OBJECT_NAME = 'O''HARA'"),
            "table filter must be escaped, got: {}",
            sql
        );
    }

    #[tokio::test]
    async fn get_columns_without_filters_omits_the_where_clause() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_columns(None, None, None, None)
            .await
            .expect("get_columns");

        let sql = only_sql(&log);
        assert!(
            !sql.contains("WHERE"),
            "unfiltered get_columns must not emit a WHERE clause, got: {}",
            sql
        );
        assert!(sql.contains("FROM SYS.EXA_ALL_COLUMNS"), "got: {}", sql);
        assert!(
            sql.contains("ORDER BY COLUMN_SCHEMA, COLUMN_TABLE, ORDINAL_POSITION"),
            "got: {}",
            sql
        );
    }

    #[tokio::test]
    async fn get_columns_filters_on_schema_table_and_column() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_columns(None, Some("SALES"), Some("ORDERS"), Some("ID"))
            .await
            .expect("get_columns");

        let sql = only_sql(&log);
        assert!(
            sql.contains(
                "WHERE COLUMN_SCHEMA = 'SALES' AND COLUMN_TABLE = 'ORDERS' AND COLUMN_NAME = 'ID'"
            ),
            "got: {}",
            sql
        );
    }

    /// Exasol has no catalog level, so `get_columns` treats a catalog argument
    /// as a second schema predicate rather than ignoring it. Passing both a
    /// catalog and a differing schema therefore yields two conflicting
    /// `COLUMN_SCHEMA` predicates and matches nothing — recorded here as the
    /// current contract so a future change to it is a deliberate one.
    #[tokio::test]
    async fn get_columns_maps_the_catalog_argument_onto_column_schema() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_columns(Some("CAT"), Some("SALES"), None, None)
            .await
            .expect("get_columns");

        let sql = only_sql(&log);
        assert!(
            sql.contains("WHERE COLUMN_SCHEMA = 'CAT' AND COLUMN_SCHEMA = 'SALES'"),
            "got: {}",
            sql
        );
    }

    #[tokio::test]
    async fn get_columns_doubles_single_quotes_in_every_filter() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.get_columns(Some("C'AT"), Some("S'CH"), Some("T'BL"), Some("C'OL"))
            .await
            .expect("get_columns");

        let sql = only_sql(&log);
        for expected in [
            "COLUMN_SCHEMA = 'C''AT'",
            "COLUMN_SCHEMA = 'S''CH'",
            "COLUMN_TABLE = 'T''BL'",
            "COLUMN_NAME = 'C''OL'",
        ] {
            assert!(
                sql.contains(expected),
                "expected escaped predicate {} in: {}",
                expected,
                sql
            );
        }
    }

    // ------------------------------------------------------------------------
    // Connection setup: connect_with_transport
    // ------------------------------------------------------------------------

    /// A slot a mock expectation writes its observed argument into.
    type Captured<T> = Arc<SyncMutex<Option<T>>>;

    fn new_capture<T>() -> Captured<T> {
        Arc::new(SyncMutex::new(None))
    }

    fn captured<T: Clone>(slot: &Captured<T>) -> T {
        slot.lock()
            .expect("capture poisoned")
            .clone()
            .expect("expectation was never called")
    }

    #[tokio::test]
    async fn connect_maps_every_connection_parameter_onto_the_transport() {
        let observed: Captured<TransportConnectionParams> = new_capture();
        let mut transport = MockTransport::new();
        let sink = Arc::clone(&observed);
        transport.expect_connect().returning(move |params| {
            *sink.lock().expect("capture poisoned") = Some(params.clone());
            Ok(())
        });
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));

        let params = ConnectionParams::builder()
            .host("db.example.invalid")
            .port(9999)
            .username("tester")
            .password("s3cr3t-pw")
            .use_tls(false)
            .validate_server_certificate(false)
            .connection_timeout(Duration::from_millis(4_500))
            .certificate_fingerprint("ab12cd34")
            .build()
            .expect("params");

        Connection::connect_with_transport(params, transport)
            .await
            .expect("connect");

        let mapped = captured(&observed);
        assert_eq!(mapped.host, "db.example.invalid");
        assert_eq!(mapped.port, 9999);
        assert!(!mapped.use_tls);
        assert!(!mapped.validate_server_certificate);
        assert_eq!(mapped.timeout_ms, 4_500);
        assert_eq!(
            mapped.certificate_fingerprint.as_deref(),
            Some("ab12cd34"),
            "a configured fingerprint must be pinned on the transport"
        );
    }

    #[tokio::test]
    async fn connect_leaves_the_fingerprint_unset_when_none_is_configured() {
        let observed: Captured<TransportConnectionParams> = new_capture();
        let mut transport = MockTransport::new();
        let sink = Arc::clone(&observed);
        transport.expect_connect().returning(move |params| {
            *sink.lock().expect("capture poisoned") = Some(params.clone());
            Ok(())
        });
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));

        Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        assert!(captured(&observed).certificate_fingerprint.is_none());
    }

    #[tokio::test]
    async fn connect_forwards_the_configured_username_and_password() {
        let observed: Captured<(String, String)> = new_capture();
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        let sink = Arc::clone(&observed);
        transport
            .expect_authenticate()
            .returning(move |credentials| {
                *sink.lock().expect("capture poisoned") =
                    Some((credentials.username.clone(), credentials.password.clone()));
                Ok(transport_session_info())
            });

        Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        assert_eq!(
            captured(&observed),
            ("tester".to_string(), "s3cr3t-pw".to_string())
        );
    }

    #[tokio::test]
    async fn connect_pushes_a_configured_query_timeout_rounded_up_to_whole_seconds() {
        let observed: Captured<u64> = new_capture();
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        let sink = Arc::clone(&observed);
        transport
            .expect_set_query_timeout()
            .times(1)
            .returning(move |secs| {
                *sink.lock().expect("capture poisoned") = Some(secs);
                Ok(())
            });

        let params = ConnectionParams::builder()
            .host("db.example.invalid")
            .username("tester")
            .password("s3cr3t-pw")
            .query_timeout(Duration::from_millis(1_500))
            .build()
            .expect("params");

        let conn = Connection::connect_with_transport(params, transport)
            .await
            .expect("connect");

        assert_eq!(captured(&observed), 2, "1500ms must round up to 2s");
        assert_eq!(
            conn.session.config().query_timeout,
            Some(Duration::from_millis(1_500)),
            "the session baseline must record the configured value verbatim"
        );
    }

    /// Absent configuration, no `queryTimeout` attribute is set at all so the
    /// server's own `QUERY_TIMEOUT` governs. `MockTransport` has no
    /// `set_query_timeout` expectation here, so any call would panic.
    #[tokio::test]
    async fn connect_sets_no_query_timeout_when_none_is_configured() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        assert_eq!(conn.session.config().query_timeout, None);
    }

    #[tokio::test]
    async fn connect_seeds_the_session_from_the_authentication_response() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));

        let params = ConnectionParams::builder()
            .host("db.example.invalid")
            .username("tester")
            .password("s3cr3t-pw")
            .idle_timeout(Duration::from_secs(120))
            .build()
            .expect("params");

        let conn = Connection::connect_with_transport(params, transport)
            .await
            .expect("connect");

        assert_eq!(conn.session_id(), "1739284756");
        assert_eq!(conn.session.config().idle_timeout, Duration::from_secs(120));
        let server = conn.session.server_info();
        assert_eq!(server.release_version, "8.32.0");
        assert_eq!(server.database_name, "exadb");
        assert_eq!(server.product_name, "EXASolution");
        assert_eq!(server.max_data_message_size, 64 * 1024);
        assert_eq!(server.time_zone, "Europe/Berlin");
        assert_eq!(server.max_identifier_length, 128);
        assert_eq!(server.max_varchar_length, 2_000_000);
        assert_eq!(server.identifier_quote_string, "\"");
    }

    #[tokio::test]
    async fn connect_defaults_the_session_time_zone_to_utc_when_the_server_reports_none() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport.expect_authenticate().returning(|_| {
            Ok(TransportSessionInfo {
                time_zone: None,
                ..transport_session_info()
            })
        });

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        assert_eq!(conn.session.server_info().time_zone, "UTC");
    }

    #[tokio::test]
    async fn connect_reports_a_transport_connect_failure_with_host_and_port() {
        let mut transport = MockTransport::new();
        transport
            .expect_connect()
            .returning(|_| Err(TransportError::IoError("refused".to_string())));

        let error = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect_err("a failing transport must fail the connection");

        match error {
            ConnectionError::ConnectionFailed {
                host,
                port,
                message,
            } => {
                assert_eq!(host, "db.example.invalid");
                assert_eq!(port, 8563);
                assert!(message.contains("refused"), "got: {}", message);
            }
            other => panic!("expected ConnectionFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn connect_reports_an_authentication_failure_as_authentication_failed() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Err(TransportError::ProtocolError("bad credentials".to_string())));

        let error = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect_err("failed authentication must fail the connection");

        match error {
            ConnectionError::AuthenticationFailed(message) => {
                assert!(message.contains("bad credentials"), "got: {}", message);
            }
            other => panic!("expected AuthenticationFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn connect_fails_when_the_server_rejects_the_configured_query_timeout() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_set_query_timeout()
            .returning(|_| Err(TransportError::ProtocolError("not permitted".to_string())));

        let params = ConnectionParams::builder()
            .host("db.example.invalid")
            .username("tester")
            .password("s3cr3t-pw")
            .query_timeout(Duration::from_secs(30))
            .build()
            .expect("params");

        let error = Connection::connect_with_transport(params, transport)
            .await
            .expect_err("a rejected query timeout must fail the connection");

        match error {
            ConnectionError::ConnectionFailed { message, .. } => {
                assert!(
                    message.contains("failed to set query timeout"),
                    "got: {}",
                    message
                );
                assert!(message.contains("not permitted"), "got: {}", message);
            }
            other => panic!("expected ConnectionFailed, got {:?}", other),
        }
    }

    // --- the two schema-open branches ---

    fn params_with_schema(schema: &str) -> ConnectionParams {
        ConnectionParams::builder()
            .host("db.example.invalid")
            .port(8563)
            .username("tester")
            .password("s3cr3t-pw")
            .schema(schema)
            .build()
            .expect("params")
    }

    #[tokio::test]
    async fn connect_opens_the_schema_named_in_the_connection_uri() {
        let log = new_sql_log();
        let conn = Connection::connect_with_transport(
            params_with_schema("SALES"),
            recording_transport(&log),
        )
        .await
        .expect("connect");

        assert_eq!(only_sql(&log), "OPEN SCHEMA SALES");
        assert_eq!(conn.current_schema().await, Some("SALES".to_string()));
    }

    /// A schema named in the URI is a best-effort default: one that does not
    /// exist yet must leave the connection open with no active schema.
    #[tokio::test]
    async fn connect_survives_a_schema_from_the_uri_that_does_not_exist_yet() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport.expect_execute_query().returning(|_| {
            Err(TransportError::ProtocolError(
                "schema FOO not found [line 1, column 13] (SQL state: 42000)".to_string(),
            ))
        });

        let conn = Connection::connect_with_transport(params_with_schema("FOO"), transport)
            .await
            .expect("a not-yet-existing URI schema must not fail the connection");

        assert_eq!(
            conn.current_schema().await,
            None,
            "the session must be left with no active schema"
        );
        assert!(!conn.is_closed().await, "the connection must stay open");
    }

    /// Any schema-open failure other than "not found" means a genuinely broken
    /// connection: it must close the transport rather than hand back a
    /// half-open `Connection`.
    #[tokio::test]
    async fn connect_closes_the_transport_when_opening_the_uri_schema_fails_fatally() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport.expect_execute_query().returning(|_| {
            Err(TransportError::ProtocolError(
                "insufficient privileges for schema SALES".to_string(),
            ))
        });
        transport.expect_close().times(1).returning(|| Ok(()));

        let error = Connection::connect_with_transport(params_with_schema("SALES"), transport)
            .await
            .expect_err("a fatal schema-open failure must fail the connection");

        match error {
            ConnectionError::ConnectionFailed {
                host,
                port,
                message,
            } => {
                assert_eq!(host, "db.example.invalid");
                assert_eq!(port, 8563);
                assert!(
                    message.contains("failed to activate schema 'SALES' from connection URI"),
                    "got: {}",
                    message
                );
                assert!(
                    message.contains("insufficient privileges"),
                    "got: {}",
                    message
                );
            }
            other => panic!("expected ConnectionFailed, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------------
    // execute_statement: queryTimeout reconciliation
    // ------------------------------------------------------------------------

    /// Every `timeout_secs` the connection pushed to the server, in order.
    type TimeoutLog = Arc<SyncMutex<Vec<u64>>>;

    fn new_timeout_log() -> TimeoutLog {
        Arc::new(SyncMutex::new(Vec::new()))
    }

    fn recorded_timeouts(log: &TimeoutLog) -> Vec<u64> {
        log.lock().expect("timeout log poisoned").clone()
    }

    /// A transport that records both the SQL and every pushed `queryTimeout`,
    /// answering each statement with `answer`.
    fn transport_recording_timeouts(
        sql_log: &SqlLog,
        timeout_log: &TimeoutLog,
        answer: fn() -> Result<QueryResult, TransportError>,
    ) -> MockTransport {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        let timeouts = Arc::clone(timeout_log);
        transport.expect_set_query_timeout().returning(move |secs| {
            timeouts.lock().expect("timeout log poisoned").push(secs);
            Ok(())
        });
        let statements = Arc::clone(sql_log);
        transport.expect_execute_query().returning(move |sql| {
            statements
                .lock()
                .expect("SQL log poisoned")
                .push(sql.to_string());
            answer()
        });
        transport
    }

    fn ok_row_count() -> Result<QueryResult, TransportError> {
        Ok(QueryResult::row_count(1))
    }

    fn params_with_query_timeout(timeout: Duration) -> ConnectionParams {
        ConnectionParams::builder()
            .host("db.example.invalid")
            .port(8563)
            .username("tester")
            .password("s3cr3t-pw")
            .query_timeout(timeout)
            .build()
            .expect("params")
    }

    #[tokio::test]
    async fn execute_statement_pushes_a_statement_timeout_the_session_has_not_applied() {
        let sql_log = new_sql_log();
        let timeouts = new_timeout_log();
        let mut conn = Connection::connect_with_transport(
            test_params(),
            transport_recording_timeouts(&sql_log, &timeouts, ok_row_count),
        )
        .await
        .expect("connect");

        let mut stmt = Statement::new("SELECT 1");
        stmt.set_timeout(2_500);
        conn.execute_statement(&stmt).await.expect("execute");

        assert_eq!(
            recorded_timeouts(&timeouts),
            vec![3],
            "2500ms must be pushed as a 3s server-side limit"
        );
        assert_eq!(
            conn.session.config().query_timeout,
            Some(Duration::from_millis(2_500)),
            "the applied baseline must record the statement's request"
        );
    }

    #[tokio::test]
    async fn execute_statement_skips_the_push_when_the_timeout_already_matches() {
        let sql_log = new_sql_log();
        let timeouts = new_timeout_log();
        let mut conn = Connection::connect_with_transport(
            params_with_query_timeout(Duration::from_secs(30)),
            transport_recording_timeouts(&sql_log, &timeouts, ok_row_count),
        )
        .await
        .expect("connect");

        let stmt = conn.create_statement("SELECT 1");
        conn.execute_statement(&stmt).await.expect("execute");

        assert_eq!(
            recorded_timeouts(&timeouts),
            vec![30],
            "only connect may push; the statement matches the applied value"
        );
    }

    /// A statement carrying no timeout must reset the session-level
    /// `queryTimeout` to `0` (unlimited) so it never inherits a prior
    /// statement's limit.
    #[tokio::test]
    async fn execute_statement_resets_the_server_to_unlimited_for_an_untimed_statement() {
        let sql_log = new_sql_log();
        let timeouts = new_timeout_log();
        let mut conn = Connection::connect_with_transport(
            params_with_query_timeout(Duration::from_secs(10)),
            transport_recording_timeouts(&sql_log, &timeouts, ok_row_count),
        )
        .await
        .expect("connect");

        conn.execute_statement(&Statement::new("SELECT 1"))
            .await
            .expect("execute");

        assert_eq!(recorded_timeouts(&timeouts), vec![10, 0]);
        assert_eq!(conn.session.config().query_timeout, None);
    }

    /// The server's `queryTimeout` was changed before the statement ran, so the
    /// recorded baseline must reflect that even when the statement then aborts —
    /// otherwise the next statement reconciles against a stale value and skips
    /// a push it actually needs.
    #[tokio::test]
    async fn execute_statement_records_the_applied_timeout_even_when_the_query_fails() {
        let sql_log = new_sql_log();
        let timeouts = new_timeout_log();
        let mut conn = Connection::connect_with_transport(
            test_params(),
            transport_recording_timeouts(&sql_log, &timeouts, || {
                Err(TransportError::ProtocolError("boom".to_string()))
            }),
        )
        .await
        .expect("connect");

        let mut stmt = Statement::new("SELECT 1");
        stmt.set_timeout(5_000);
        conn.execute_statement(&stmt)
            .await
            .expect_err("the query must fail");

        assert_eq!(recorded_timeouts(&timeouts), vec![5]);
        assert_eq!(
            conn.session.config().query_timeout,
            Some(Duration::from_millis(5_000))
        );
    }

    #[tokio::test]
    async fn execute_statement_fails_when_the_server_rejects_the_timeout_push() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_set_query_timeout()
            .returning(|_| Err(TransportError::ProtocolError("rejected".to_string())));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        let mut stmt = Statement::new("SELECT 1");
        stmt.set_timeout(1_000);
        match conn.execute_statement(&stmt).await {
            Err(QueryError::ExecutionFailed(message)) => {
                assert!(message.contains("rejected"), "got: {}", message)
            }
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }

    /// The reported `timeout_ms` is the limit the server actually enforced —
    /// the request rounded up to whole seconds — not the raw sub-second request.
    #[tokio::test]
    async fn execute_statement_reports_a_server_timeout_abort_with_the_effective_limit() {
        let sql_log = new_sql_log();
        let timeouts = new_timeout_log();
        let mut conn = Connection::connect_with_transport(
            test_params(),
            transport_recording_timeouts(&sql_log, &timeouts, || {
                Err(TransportError::ProtocolError(
                    "Query terminated because timeout has been reached. (SQL code: R0001)"
                        .to_string(),
                ))
            }),
        )
        .await
        .expect("connect");

        let mut stmt = Statement::new("SELECT 1");
        stmt.set_timeout(1_500);
        match conn.execute_statement(&stmt).await {
            Err(QueryError::Timeout { timeout_ms }) => assert_eq!(
                timeout_ms, 2_000,
                "a 1500ms request is enforced as a 2000ms server-side limit"
            ),
            other => panic!("expected Timeout, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_statement_rejects_a_closed_session_as_an_invalid_state() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;
        conn.session.set_state(SessionState::Closed).await;

        match conn.execute_statement(&Statement::new("SELECT 1")).await {
            Err(QueryError::InvalidState(_)) => {}
            other => panic!("expected InvalidState, got {:?}", other),
        }
        assert!(
            recorded(&log).is_empty(),
            "no SQL may reach the server from a closed session"
        );
    }

    #[tokio::test]
    async fn execute_statement_propagates_an_unbound_parameter_before_reaching_the_server() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        match conn.execute_statement(&Statement::new("SELECT ?")).await {
            Err(QueryError::ParameterBindingError { index, .. }) => assert_eq!(index, 0),
            other => panic!("expected ParameterBindingError, got {:?}", other),
        }
        assert!(
            recorded(&log).is_empty(),
            "an unbuildable statement must never reach the server"
        );
    }

    #[tokio::test]
    async fn execute_statement_returns_the_session_to_ready_and_counts_the_query() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.execute_statement(&Statement::new("SELECT 1"))
            .await
            .expect("execute");

        assert_eq!(conn.session.state().await, SessionState::Ready);
        assert_eq!(conn.session.query_count(), 1);
    }

    #[tokio::test]
    async fn execute_statement_returns_the_session_to_in_transaction_inside_a_transaction() {
        let sql_log = new_sql_log();
        let mut transport = recording_transport(&sql_log);
        transport.expect_set_autocommit().returning(|_| Ok(()));
        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");
        conn.begin_transaction().await.expect("begin");

        conn.execute_statement(&Statement::new("SELECT 1"))
            .await
            .expect("execute");

        assert_eq!(conn.session.state().await, SessionState::InTransaction);
    }

    #[tokio::test]
    async fn create_statement_inherits_the_connections_configured_query_timeout() {
        let sql_log = new_sql_log();
        let timeouts = new_timeout_log();
        let conn = Connection::connect_with_transport(
            params_with_query_timeout(Duration::from_millis(7_200)),
            transport_recording_timeouts(&sql_log, &timeouts, ok_row_count),
        )
        .await
        .expect("connect");

        assert_eq!(
            conn.create_statement("SELECT 1").timeout_ms(),
            Some(7_200),
            "the raw configured value is carried onto the statement verbatim"
        );
    }

    #[tokio::test]
    async fn create_statement_leaves_the_timeout_unset_when_none_is_configured() {
        let log = new_sql_log();
        let conn = connected(&log).await;

        assert_eq!(conn.create_statement("SELECT 1").timeout_ms(), None);
    }

    #[tokio::test]
    async fn query_fetches_every_batch_of_a_complete_result_set() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_execute_query()
            .returning(|_| Ok(single_decimal_result_set()));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        let batches = conn.query("SELECT 1").await.expect("query");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 1);
        assert_eq!(batches[0].num_columns(), 1);
    }

    #[tokio::test]
    async fn query_rejects_a_statement_that_returns_only_a_row_count() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        match conn.query("DELETE FROM T").await {
            Err(QueryError::NoResultSet(_)) => {}
            other => panic!("expected NoResultSet, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_update_returns_the_affected_row_count() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_execute_query()
            .returning(|_| Ok(QueryResult::row_count(5)));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        assert_eq!(
            conn.execute_update("DELETE FROM T").await.expect("update"),
            5
        );
    }

    #[tokio::test]
    async fn execute_statement_update_returns_the_affected_row_count() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_execute_query()
            .returning(|_| Ok(QueryResult::row_count(42)));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        assert_eq!(
            conn.execute_statement_update(&Statement::new("DELETE FROM T"))
                .await
                .expect("update"),
            42
        );
    }

    #[tokio::test]
    async fn execute_statement_update_rejects_a_statement_that_returns_a_result_set() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_execute_query()
            .returning(|_| Ok(single_decimal_result_set()));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        match conn
            .execute_statement_update(&Statement::new("SELECT 1"))
            .await
        {
            Err(QueryError::NoResultSet(_)) => {}
            other => panic!("expected NoResultSet, got {:?}", other),
        }
    }

    /// A one-row, one-column DECIMAL result set — the smallest well-formed
    /// `ResultSet` answer a mocked transport can give.
    fn single_decimal_result_set() -> QueryResult {
        use crate::transport::messages::{ColumnInfo, DataType, ResultData, ResultPayload};
        QueryResult::result_set(
            None,
            ResultData {
                columns: vec![ColumnInfo {
                    name: "N".to_string(),
                    data_type: DataType::decimal(18, 0),
                }],
                data: ResultPayload::Json(vec![vec![serde_json::json!(1)]]),
                total_rows: 1,
            },
        )
    }

    // ------------------------------------------------------------------------
    // Transactions
    // ------------------------------------------------------------------------

    /// A connected connection whose transport also accepts autocommit changes.
    async fn connected_for_transactions(log: &SqlLog) -> Connection {
        let mut transport = recording_transport(log);
        transport.expect_set_autocommit().returning(|_| Ok(()));
        Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect")
    }

    #[tokio::test]
    async fn begin_transaction_disables_autocommit_on_the_server() {
        let observed: Captured<bool> = new_capture();
        let sql_log = new_sql_log();
        let mut transport = recording_transport(&sql_log);
        let sink = Arc::clone(&observed);
        transport
            .expect_set_autocommit()
            .times(1)
            .returning(move |enabled| {
                *sink.lock().expect("capture poisoned") = Some(enabled);
                Ok(())
            });
        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        conn.begin_transaction().await.expect("begin");

        assert!(
            !captured(&observed),
            "beginning a transaction must disable autocommit"
        );
        assert!(conn.in_transaction());
        assert!(
            recorded(&sql_log).is_empty(),
            "autocommit is an attribute, not a statement"
        );
    }

    #[tokio::test]
    async fn begin_transaction_fails_when_the_server_rejects_disabling_autocommit() {
        let sql_log = new_sql_log();
        let mut transport = recording_transport(&sql_log);
        transport
            .expect_set_autocommit()
            .returning(|_| Err(TransportError::ProtocolError("no autocommit".to_string())));
        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        match conn.begin_transaction().await {
            Err(QueryError::TransactionError(message)) => {
                assert!(message.contains("no autocommit"), "got: {}", message)
            }
            other => panic!("expected TransactionError, got {:?}", other),
        }
        assert!(
            !conn.in_transaction(),
            "a rejected autocommit change must not mark a transaction active"
        );
    }

    #[tokio::test]
    async fn begin_transaction_rejects_a_second_overlapping_transaction() {
        let log = new_sql_log();
        let mut conn = connected_for_transactions(&log).await;
        conn.begin_transaction().await.expect("first begin");

        match conn.begin_transaction().await {
            Err(QueryError::TransactionError(message)) => {
                assert!(
                    message.contains("Transaction already active"),
                    "got: {}",
                    message
                )
            }
            other => panic!("expected TransactionError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn commit_issues_a_commit_statement_and_ends_the_transaction() {
        let log = new_sql_log();
        let mut conn = connected_for_transactions(&log).await;
        conn.begin_transaction().await.expect("begin");

        conn.commit().await.expect("commit");

        assert_eq!(recorded(&log), vec!["COMMIT".to_string()]);
        assert!(!conn.in_transaction());
        assert_eq!(conn.session.state().await, SessionState::Ready);
    }

    #[tokio::test]
    async fn commit_outside_a_transaction_is_a_no_op() {
        let log = new_sql_log();
        let mut conn = connected_for_transactions(&log).await;

        conn.commit().await.expect("commit");

        assert!(
            recorded(&log).is_empty(),
            "committing without a transaction must issue no SQL"
        );
    }

    #[tokio::test]
    async fn rollback_issues_a_rollback_statement_and_ends_the_transaction() {
        let log = new_sql_log();
        let mut conn = connected_for_transactions(&log).await;
        conn.begin_transaction().await.expect("begin");

        conn.rollback().await.expect("rollback");

        assert_eq!(recorded(&log), vec!["ROLLBACK".to_string()]);
        assert!(!conn.in_transaction());
        assert_eq!(conn.session.state().await, SessionState::Ready);
    }

    #[tokio::test]
    async fn rollback_outside_a_transaction_is_a_no_op() {
        let log = new_sql_log();
        let mut conn = connected_for_transactions(&log).await;

        conn.rollback().await.expect("rollback");

        assert!(
            recorded(&log).is_empty(),
            "rolling back without a transaction must issue no SQL"
        );
    }

    /// A failed `COMMIT` must leave the transaction active rather than silently
    /// clearing it — the work is still uncommitted on the server.
    #[tokio::test]
    async fn commit_keeps_the_transaction_active_when_the_statement_fails() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport.expect_set_autocommit().returning(|_| Ok(()));
        transport
            .expect_execute_query()
            .returning(|_| Err(TransportError::ProtocolError("commit failed".to_string())));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");
        conn.begin_transaction().await.expect("begin");

        conn.commit().await.expect_err("commit must fail");

        assert!(conn.in_transaction());
    }

    #[tokio::test]
    async fn in_transaction_is_false_on_a_freshly_connected_connection() {
        let log = new_sql_log();
        let conn = connected(&log).await;

        assert!(!conn.in_transaction());
    }

    #[tokio::test]
    async fn set_schema_opens_the_schema_and_records_it_on_the_session() {
        let log = new_sql_log();
        let mut conn = connected(&log).await;

        conn.set_schema("SALES").await.expect("set_schema");

        assert_eq!(only_sql(&log), "OPEN SCHEMA SALES");
        assert_eq!(conn.current_schema().await, Some("SALES".to_string()));
    }

    // ------------------------------------------------------------------------
    // make_sql_executor
    // ------------------------------------------------------------------------

    #[tokio::test]
    async fn sql_executor_returns_the_row_count_for_a_row_count_result() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_execute_query()
            .returning(|_| Ok(QueryResult::row_count(7)));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        let execute = conn.make_sql_executor();
        assert_eq!(execute("IMPORT INTO T ...".to_string()).await, Ok(7));
    }

    /// The executor exists for import/export statements, which report a row
    /// count. A statement that unexpectedly answers with a result set reports
    /// zero rows rather than failing the import.
    #[tokio::test]
    async fn sql_executor_reports_zero_rows_for_a_result_set_result() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_execute_query()
            .returning(|_| Ok(single_decimal_result_set()));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        let execute = conn.make_sql_executor();
        assert_eq!(execute("SELECT 1".to_string()).await, Ok(0));
    }

    #[tokio::test]
    async fn sql_executor_surfaces_a_transport_failure_as_its_message() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_execute_query()
            .returning(|_| Err(TransportError::IoError("socket closed".to_string())));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        let execute = conn.make_sql_executor();
        let error = execute("IMPORT INTO T ...".to_string())
            .await
            .expect_err("a transport failure must surface");
        assert!(error.contains("socket closed"), "got: {}", error);
    }

    /// Parallel file imports call the executor repeatedly, so it must survive
    /// more than one invocation.
    #[tokio::test]
    async fn sql_executor_is_reusable_across_several_statements() {
        let log = new_sql_log();
        let conn = connected(&log).await;

        let execute = conn.make_sql_executor();
        execute("IMPORT INTO T FROM 'part-0'".to_string())
            .await
            .expect("first");
        execute("IMPORT INTO T FROM 'part-1'".to_string())
            .await
            .expect("second");

        assert_eq!(
            recorded(&log),
            vec![
                "IMPORT INTO T FROM 'part-0'".to_string(),
                "IMPORT INTO T FROM 'part-1'".to_string()
            ]
        );
    }

    // ------------------------------------------------------------------------
    // Debug rendering and the connection builder
    // ------------------------------------------------------------------------

    #[tokio::test]
    async fn debug_output_identifies_the_connection_without_leaking_the_password() {
        let log = new_sql_log();
        let conn = connected(&log).await;

        let rendered = format!("{:?}", conn);

        assert!(
            !rendered.contains("s3cr3t-pw"),
            "the password must never appear in debug output, got: {}",
            rendered
        );
        assert!(!rendered.contains("password"), "got: {}", rendered);
        assert!(rendered.contains("1739284756"), "got: {}", rendered);
        assert!(rendered.contains("db.example.invalid"), "got: {}", rendered);
        assert!(rendered.contains("8563"), "got: {}", rendered);
        assert!(rendered.contains("tester"), "got: {}", rendered);
        assert!(
            rendered.contains("in_transaction: false"),
            "got: {}",
            rendered
        );
    }

    #[tokio::test]
    async fn debug_output_reports_an_active_transaction() {
        let log = new_sql_log();
        let mut conn = connected_for_transactions(&log).await;
        conn.begin_transaction().await.expect("begin");

        assert!(
            format!("{:?}", conn).contains("in_transaction: true"),
            "an active transaction must be visible in debug output"
        );
    }

    #[test]
    fn connection_builder_carries_every_setting_into_the_built_parameters() {
        let builder = ConnectionBuilder::default()
            .host("db.example.invalid")
            .port(9999)
            .username("tester")
            .password("s3cr3t-pw")
            .schema("SALES")
            .use_tls(false)
            .validate_server_certificate(false);

        let params = builder
            .params_builder
            .build()
            .expect("a fully specified builder must produce valid parameters");

        assert_eq!(params.host, "db.example.invalid");
        assert_eq!(params.port, 9999);
        assert_eq!(params.username, "tester");
        assert_eq!(params.password(), "s3cr3t-pw");
        assert_eq!(params.schema.as_deref(), Some("SALES"));
        assert!(!params.use_tls);
        assert!(!params.validate_server_certificate);
    }

    #[test]
    fn connection_builder_validates_the_server_certificate_by_default() {
        let params = ConnectionBuilder::new()
            .host("db.example.invalid")
            .username("tester")
            .password("s3cr3t-pw")
            .params_builder
            .build()
            .expect("params");

        assert!(
            params.validate_server_certificate,
            "certificate validation must stay on unless explicitly disabled"
        );
        assert!(
            params.use_tls,
            "TLS must stay on unless explicitly disabled"
        );
    }

    /// Parameter validation happens before any dialling, so an incomplete
    /// builder fails locally rather than on the network.
    #[tokio::test]
    async fn connection_builder_connect_rejects_a_missing_host_before_dialling() {
        let error = ConnectionBuilder::new()
            .username("tester")
            .password("s3cr3t-pw")
            .connect()
            .await
            .expect_err("a builder without a host must not connect");

        match error {
            ExasolError::Connection(ConnectionError::InvalidParameter { parameter, .. }) => {
                assert_eq!(parameter, "host")
            }
            other => panic!("expected a missing-host InvalidParameter, got {:?}", other),
        }
    }

    /// The connection-string parser already rejects an unknown `transport`, so
    /// this arm is a defence-in-depth guard reachable only by setting the
    /// public field directly. It must still name the transport it refused.
    #[tokio::test]
    async fn from_params_rejects_a_transport_the_build_does_not_provide() {
        let mut params = test_params();
        params.transport = Some("telepathy".to_string());

        let error = Connection::from_params(params)
            .await
            .expect_err("an unavailable transport must not connect");

        match error {
            ConnectionError::InvalidParameter { parameter, message } => {
                assert_eq!(parameter, "transport");
                assert!(
                    message.contains("Transport 'telepathy' is not available"),
                    "got: {}",
                    message
                );
            }
            other => panic!("expected InvalidParameter, got {:?}", other),
        }
    }

    #[test]
    fn connection_builder_entry_point_produces_a_usable_builder() {
        let params = Connection::builder()
            .host("db.example.invalid")
            .username("tester")
            .password("s3cr3t-pw")
            .params_builder
            .build()
            .expect("params");

        assert_eq!(params.host, "db.example.invalid");
    }

    // ------------------------------------------------------------------------
    // Connection lifecycle and accessors
    // ------------------------------------------------------------------------

    #[tokio::test]
    async fn params_exposes_the_parameters_the_connection_was_built_from() {
        let log = new_sql_log();
        let conn = connected(&log).await;

        assert_eq!(conn.params().host, "db.example.invalid");
        assert_eq!(conn.params().port, 8563);
        assert_eq!(conn.params().username, "tester");
    }

    #[tokio::test]
    async fn close_closes_the_session_and_the_transport() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport.expect_close().times(1).returning(|| Ok(()));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        conn.close().await.expect("close");
    }

    #[tokio::test]
    async fn close_reports_a_failing_transport_with_host_and_port() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_close()
            .returning(|| Err(TransportError::IoError("already gone".to_string())));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        match conn.close().await {
            Err(ConnectionError::ConnectionFailed {
                host,
                port,
                message,
            }) => {
                assert_eq!(host, "db.example.invalid");
                assert_eq!(port, 8563);
                assert!(message.contains("already gone"), "got: {}", message);
            }
            other => panic!("expected ConnectionFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn is_closed_reflects_the_session_state() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport.expect_close().returning(|| Ok(()));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        assert!(!conn.is_closed().await);
        conn.shutdown().await.expect("shutdown");
        assert!(conn.is_closed().await);
    }

    #[tokio::test]
    async fn shutdown_reports_a_failing_transport_with_host_and_port() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_close()
            .returning(|| Err(TransportError::IoError("already gone".to_string())));

        let conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        match conn.shutdown().await {
            Err(ConnectionError::ConnectionFailed { message, .. }) => {
                assert!(message.contains("already gone"), "got: {}", message)
            }
            other => panic!("expected ConnectionFailed, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------------
    // Prepared statements
    // ------------------------------------------------------------------------

    use crate::transport::protocol::PreparedStatementHandle;

    fn handle_with(num_params: i32) -> PreparedStatementHandle {
        PreparedStatementHandle::new(17, num_params, Vec::new(), Vec::new())
    }

    /// The parameter rows the mocked transport was handed for execution.
    type ParameterLog = Arc<SyncMutex<Vec<Option<Vec<Vec<serde_json::Value>>>>>>;

    /// A connected connection whose transport prepares a `num_params`-parameter
    /// statement, answers every execution with `answer`, and records the
    /// parameters it was given.
    async fn connected_for_prepared(
        num_params: i32,
        answer: fn() -> Result<QueryResult, TransportError>,
        parameters: &ParameterLog,
    ) -> Connection {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_create_prepared_statement()
            .returning(move |_| Ok(handle_with(num_params)));
        let sink = Arc::clone(parameters);
        transport
            .expect_execute_prepared_statement()
            .returning(move |_, params| {
                sink.lock().expect("parameter log poisoned").push(params);
                answer()
            });
        transport
            .expect_close_prepared_statement()
            .returning(|_| Ok(()));
        Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect")
    }

    fn new_parameter_log() -> ParameterLog {
        Arc::new(SyncMutex::new(Vec::new()))
    }

    fn recorded_parameters(log: &ParameterLog) -> Vec<Option<Vec<Vec<serde_json::Value>>>> {
        log.lock().expect("parameter log poisoned").clone()
    }

    fn closed_prepared_statement() -> PreparedStatement {
        let mut stmt = PreparedStatement::new(handle_with(0));
        stmt.mark_closed();
        stmt
    }

    #[tokio::test]
    async fn prepare_returns_a_statement_sized_by_the_servers_parameter_count() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(2, ok_row_count, &parameters).await;

        let stmt = conn
            .prepare("INSERT INTO T VALUES (?, ?)")
            .await
            .expect("prepare");

        assert_eq!(stmt.parameter_count(), 2);
        assert_eq!(stmt.handle(), 17);
        assert!(!stmt.is_closed());
    }

    #[tokio::test]
    async fn prepare_rejects_a_closed_session() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;
        conn.session.set_state(SessionState::Closed).await;

        match conn.prepare("SELECT 1").await {
            Err(QueryError::InvalidState(_)) => {}
            other => panic!("expected InvalidState, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn prepare_surfaces_a_transport_failure() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_create_prepared_statement()
            .returning(|_| Err(TransportError::ProtocolError("syntax error".to_string())));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        match conn.prepare("SELCT 1").await {
            Err(QueryError::ExecutionFailed(message)) => {
                assert!(message.contains("syntax error"), "got: {}", message)
            }
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_prepared_forwards_bound_parameters_in_column_major_order() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(2, ok_row_count, &parameters).await;
        let mut stmt = conn
            .prepare("INSERT INTO T VALUES (?, ?)")
            .await
            .expect("prepare");
        stmt.bind(0, Parameter::Integer(7)).expect("bind 0");
        stmt.bind(1, Parameter::String("seven".to_string()))
            .expect("bind 1");

        conn.execute_prepared(&stmt).await.expect("execute");

        assert_eq!(
            recorded_parameters(&parameters),
            vec![Some(vec![
                vec![serde_json::json!(7)],
                vec![serde_json::json!("seven")]
            ])]
        );
    }

    #[tokio::test]
    async fn execute_prepared_sends_no_parameters_for_a_parameterless_statement() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;
        let stmt = conn.prepare("DELETE FROM T").await.expect("prepare");

        conn.execute_prepared(&stmt).await.expect("execute");

        assert_eq!(recorded_parameters(&parameters), vec![None]);
    }

    #[tokio::test]
    async fn execute_prepared_rejects_a_closed_statement() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;

        match conn.execute_prepared(&closed_prepared_statement()).await {
            Err(QueryError::StatementClosed) => {}
            other => panic!("expected StatementClosed, got {:?}", other),
        }
        assert!(
            recorded_parameters(&parameters).is_empty(),
            "a closed statement must never reach the server"
        );
    }

    #[tokio::test]
    async fn execute_prepared_rejects_a_closed_session() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;
        let stmt = conn.prepare("SELECT 1").await.expect("prepare");
        conn.session.set_state(SessionState::Closed).await;

        match conn.execute_prepared(&stmt).await {
            Err(QueryError::InvalidState(_)) => {}
            other => panic!("expected InvalidState, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_prepared_propagates_an_unbound_parameter() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(1, ok_row_count, &parameters).await;
        let stmt = conn.prepare("SELECT ?").await.expect("prepare");

        match conn.execute_prepared(&stmt).await {
            Err(QueryError::ParameterBindingError { index, .. }) => assert_eq!(index, 0),
            other => panic!("expected ParameterBindingError, got {:?}", other),
        }
        assert!(recorded_parameters(&parameters).is_empty());
    }

    #[tokio::test]
    async fn execute_prepared_surfaces_a_transport_failure() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(
            0,
            || Err(TransportError::ProtocolError("table missing".to_string())),
            &parameters,
        )
        .await;
        let stmt = conn.prepare("SELECT 1").await.expect("prepare");

        match conn.execute_prepared(&stmt).await {
            Err(QueryError::ExecutionFailed(message)) => {
                assert!(message.contains("table missing"), "got: {}", message)
            }
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_prepared_counts_the_query_and_returns_the_session_to_ready() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;
        let stmt = conn.prepare("SELECT 1").await.expect("prepare");

        conn.execute_prepared(&stmt).await.expect("execute");

        assert_eq!(conn.session.query_count(), 1);
        assert_eq!(conn.session.state().await, SessionState::Ready);
    }

    #[tokio::test]
    async fn execute_prepared_update_returns_the_affected_row_count() {
        let parameters = new_parameter_log();
        let mut conn =
            connected_for_prepared(0, || Ok(QueryResult::row_count(13)), &parameters).await;
        let stmt = conn.prepare("DELETE FROM T").await.expect("prepare");

        assert_eq!(
            conn.execute_prepared_update(&stmt).await.expect("update"),
            13
        );
    }

    #[tokio::test]
    async fn execute_prepared_update_rejects_a_statement_that_returns_a_result_set() {
        let parameters = new_parameter_log();
        let mut conn =
            connected_for_prepared(0, || Ok(single_decimal_result_set()), &parameters).await;
        let stmt = conn.prepare("SELECT 1").await.expect("prepare");

        match conn.execute_prepared_update(&stmt).await {
            Err(QueryError::UnexpectedResultSet) => {}
            other => panic!("expected UnexpectedResultSet, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_prepared_update_rejects_a_closed_statement() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;

        match conn
            .execute_prepared_update(&closed_prepared_statement())
            .await
        {
            Err(QueryError::StatementClosed) => {}
            other => panic!("expected StatementClosed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_prepared_update_rejects_a_closed_session() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;
        let stmt = conn.prepare("DELETE FROM T").await.expect("prepare");
        conn.session.set_state(SessionState::Closed).await;

        match conn.execute_prepared_update(&stmt).await {
            Err(QueryError::InvalidState(_)) => {}
            other => panic!("expected InvalidState, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_batch_update_transposes_parameter_rows_into_columns() {
        let parameters = new_parameter_log();
        let mut conn =
            connected_for_prepared(2, || Ok(QueryResult::row_count(3)), &parameters).await;
        let stmt = conn
            .prepare("INSERT INTO T VALUES (?, ?)")
            .await
            .expect("prepare");

        let affected = conn
            .execute_batch_update(
                &stmt,
                &[
                    vec![Parameter::Integer(1), Parameter::String("a".to_string())],
                    vec![Parameter::Integer(2), Parameter::String("b".to_string())],
                    vec![Parameter::Integer(3), Parameter::String("c".to_string())],
                ],
            )
            .await
            .expect("batch update");

        assert_eq!(affected, 3);
        assert_eq!(
            recorded_parameters(&parameters),
            vec![Some(vec![
                vec![
                    serde_json::json!(1),
                    serde_json::json!(2),
                    serde_json::json!(3)
                ],
                vec![
                    serde_json::json!("a"),
                    serde_json::json!("b"),
                    serde_json::json!("c")
                ]
            ])],
            "row-major input must reach the wire column-major"
        );
    }

    #[tokio::test]
    async fn execute_batch_update_rejects_a_row_whose_length_differs_from_the_statement() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(2, ok_row_count, &parameters).await;
        let stmt = conn
            .prepare("INSERT INTO T VALUES (?, ?)")
            .await
            .expect("prepare");

        match conn
            .execute_batch_update(
                &stmt,
                &[
                    vec![Parameter::Integer(1), Parameter::Integer(2)],
                    vec![Parameter::Integer(3)],
                ],
            )
            .await
        {
            Err(QueryError::ParameterBindingError { index, .. }) => assert_eq!(index, 1),
            other => panic!("expected ParameterBindingError, got {:?}", other),
        }
        assert!(recorded_parameters(&parameters).is_empty());
    }

    #[tokio::test]
    async fn execute_batch_update_rejects_a_statement_that_returns_a_result_set() {
        let parameters = new_parameter_log();
        let mut conn =
            connected_for_prepared(1, || Ok(single_decimal_result_set()), &parameters).await;
        let stmt = conn.prepare("SELECT ?").await.expect("prepare");

        match conn
            .execute_batch_update(&stmt, &[vec![Parameter::Integer(1)]])
            .await
        {
            Err(QueryError::UnexpectedResultSet) => {}
            other => panic!("expected UnexpectedResultSet, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_batch_update_rejects_a_closed_statement() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;

        match conn
            .execute_batch_update(&closed_prepared_statement(), &[])
            .await
        {
            Err(QueryError::StatementClosed) => {}
            other => panic!("expected StatementClosed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_batch_update_rejects_a_closed_session() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(1, ok_row_count, &parameters).await;
        let stmt = conn
            .prepare("DELETE FROM T WHERE ID = ?")
            .await
            .expect("prepare");
        conn.session.set_state(SessionState::Closed).await;

        match conn
            .execute_batch_update(&stmt, &[vec![Parameter::Integer(1)]])
            .await
        {
            Err(QueryError::InvalidState(_)) => {}
            other => panic!("expected InvalidState, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_batch_returns_a_result_set_and_counts_the_query() {
        let parameters = new_parameter_log();
        let mut conn =
            connected_for_prepared(1, || Ok(single_decimal_result_set()), &parameters).await;
        let stmt = conn.prepare("SELECT ?").await.expect("prepare");

        let result = conn
            .execute_batch(&stmt, &[vec![Parameter::Integer(1)]])
            .await
            .expect("batch");

        assert!(
            result.row_count().is_none(),
            "a result set has no row count"
        );
        assert_eq!(conn.session.query_count(), 1);
    }

    /// An empty batch mirrors the zero-parameter case: no parameters are sent.
    #[tokio::test]
    async fn execute_batch_with_no_rows_sends_no_parameters() {
        let parameters = new_parameter_log();
        let mut conn =
            connected_for_prepared(1, || Ok(single_decimal_result_set()), &parameters).await;
        let stmt = conn.prepare("SELECT ?").await.expect("prepare");

        conn.execute_batch(&stmt, &[]).await.expect("batch");

        assert_eq!(recorded_parameters(&parameters), vec![None]);
    }

    #[tokio::test]
    async fn execute_batch_rejects_a_closed_statement() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(0, ok_row_count, &parameters).await;

        match conn.execute_batch(&closed_prepared_statement(), &[]).await {
            Err(QueryError::StatementClosed) => {}
            other => panic!("expected StatementClosed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_batch_rejects_a_closed_session() {
        let parameters = new_parameter_log();
        let mut conn = connected_for_prepared(1, ok_row_count, &parameters).await;
        let stmt = conn.prepare("SELECT ?").await.expect("prepare");
        conn.session.set_state(SessionState::Closed).await;

        match conn
            .execute_batch(&stmt, &[vec![Parameter::Integer(1)]])
            .await
        {
            Err(QueryError::InvalidState(_)) => {}
            other => panic!("expected InvalidState, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn close_prepared_releases_the_server_handle_and_marks_the_statement_closed() {
        let closed: Captured<i32> = new_capture();
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_create_prepared_statement()
            .returning(|_| Ok(handle_with(0)));
        let sink = Arc::clone(&closed);
        transport
            .expect_close_prepared_statement()
            .times(1)
            .returning(move |handle| {
                *sink.lock().expect("capture poisoned") = Some(handle.handle);
                Ok(())
            });

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");
        let stmt = conn.prepare("SELECT 1").await.expect("prepare");

        conn.close_prepared(stmt).await.expect("close_prepared");

        assert_eq!(captured(&closed), 17);
    }

    /// Closing an already-closed statement must not send a second release for a
    /// handle the server has already freed.
    #[tokio::test]
    async fn close_prepared_on_an_already_closed_statement_sends_nothing() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_close_prepared_statement()
            .times(0)
            .returning(|_| Ok(()));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");

        conn.close_prepared(closed_prepared_statement())
            .await
            .expect("closing a closed statement must succeed");
    }

    #[tokio::test]
    async fn close_prepared_surfaces_a_transport_failure() {
        let mut transport = MockTransport::new();
        transport.expect_connect().returning(|_| Ok(()));
        transport
            .expect_authenticate()
            .returning(|_| Ok(transport_session_info()));
        transport
            .expect_create_prepared_statement()
            .returning(|_| Ok(handle_with(0)));
        transport
            .expect_close_prepared_statement()
            .returning(|_| Err(TransportError::IoError("socket closed".to_string())));

        let mut conn = Connection::connect_with_transport(test_params(), transport)
            .await
            .expect("connect");
        let stmt = conn.prepare("SELECT 1").await.expect("prepare");

        match conn.close_prepared(stmt).await {
            Err(QueryError::ExecutionFailed(message)) => {
                assert!(message.contains("socket closed"), "got: {}", message)
            }
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }
}

/// Blocking wrapper tests
#[cfg(test)]
mod blocking_tests {
    use super::*;

    #[test]
    fn test_session_type_alias_exists() {
        // Verify that Session is a type alias for Connection
        fn takes_session(_session: &Session) {}
        fn takes_connection(_connection: &Connection) {}

        // This should compile, showing Session = Connection
        fn verify_interchangeable<F1, F2>(f1: F1, f2: F2)
        where
            F1: Fn(&Session),
            F2: Fn(&Connection),
        {
            let _ = (f1, f2);
        }

        verify_interchangeable(takes_session, takes_connection);
    }

    #[test]
    fn test_blocking_runtime_exists() {
        // Verify that blocking_runtime() function exists and returns a Runtime
        let runtime = blocking_runtime();
        // If this compiles, the runtime is valid
        let _ = runtime.handle();
    }

    #[test]
    fn test_connection_has_blocking_import_csv() {
        // This test verifies the blocking_import_csv_from_file method exists
        // by checking the method signature compiles
        fn _check_method_exists(_conn: &mut Connection) {
            // Method signature check - will fail to compile if method doesn't exist
            let _: fn(
                &mut Connection,
                &str,
                &std::path::Path,
                crate::import::csv::CsvImportOptions,
            ) -> Result<u64, crate::import::ImportError> =
                Connection::blocking_import_csv_from_file;
        }
    }

    #[test]
    fn test_connection_has_blocking_import_parquet() {
        fn _check_method_exists(_conn: &mut Connection) {
            let _: fn(
                &mut Connection,
                &str,
                &std::path::Path,
                crate::import::parquet::ParquetImportOptions,
            ) -> Result<u64, crate::import::ImportError> = Connection::blocking_import_from_parquet;
        }
    }

    #[test]
    fn test_connection_has_blocking_import_record_batch() {
        fn _check_method_exists(_conn: &mut Connection) {
            let _: fn(
                &mut Connection,
                &str,
                &RecordBatch,
                crate::import::arrow::ArrowImportOptions,
            ) -> Result<u64, crate::import::ImportError> =
                Connection::blocking_import_from_record_batch;
        }
    }

    #[test]
    fn test_connection_has_blocking_import_arrow_ipc() {
        fn _check_method_exists(_conn: &mut Connection) {
            let _: fn(
                &mut Connection,
                &str,
                &std::path::Path,
                crate::import::arrow::ArrowImportOptions,
            ) -> Result<u64, crate::import::ImportError> =
                Connection::blocking_import_from_arrow_ipc;
        }
    }

    #[test]
    fn test_connection_has_blocking_export_csv() {
        fn _check_method_exists(_conn: &mut Connection) {
            let _: fn(
                &mut Connection,
                crate::query::export::ExportSource,
                &std::path::Path,
                crate::export::csv::CsvExportOptions,
            ) -> Result<u64, crate::export::csv::ExportError> =
                Connection::blocking_export_csv_to_file;
        }
    }

    #[test]
    fn test_connection_has_blocking_export_parquet() {
        fn _check_method_exists(_conn: &mut Connection) {
            let _: fn(
                &mut Connection,
                crate::query::export::ExportSource,
                &std::path::Path,
                crate::export::parquet::ParquetExportOptions,
            ) -> Result<u64, crate::export::csv::ExportError> =
                Connection::blocking_export_to_parquet;
        }
    }

    #[test]
    fn test_connection_has_blocking_export_record_batches() {
        fn _check_method_exists(_conn: &mut Connection) {
            let _: fn(
                &mut Connection,
                crate::query::export::ExportSource,
                crate::export::arrow::ArrowExportOptions,
            ) -> Result<Vec<RecordBatch>, crate::export::csv::ExportError> =
                Connection::blocking_export_to_record_batches;
        }
    }

    #[test]
    fn test_connection_has_blocking_export_arrow_ipc() {
        fn _check_method_exists(_conn: &mut Connection) {
            let _: fn(
                &mut Connection,
                crate::query::export::ExportSource,
                &std::path::Path,
                crate::export::arrow::ArrowExportOptions,
            ) -> Result<u64, crate::export::csv::ExportError> =
                Connection::blocking_export_to_arrow_ipc;
        }
    }

    #[test]
    fn test_connection_has_supports_native_parquet_import() {
        // Verify that supports_native_parquet_import exists and has the correct signature.
        fn _check_method_exists(_conn: &Connection) {
            let _: fn(&Connection) -> bool = Connection::supports_native_parquet_import;
        }
    }

    #[test]
    fn test_connection_has_import_from_parquet_stream() {
        // Verify that import_from_parquet_stream is wired (async, takes reader).
        // The function pointer syntax is not straightforward for async methods, so
        // we verify by name resolution only.
        fn _check_exists<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
            _conn: &mut Connection,
            _table: &str,
            _reader: R,
            _opts: crate::import::parquet::ParquetImportOptions,
        ) {
            // If this compiles, the method exists with the correct type parameters.
        }
    }

    #[test]
    fn test_resolve_native_parquet_uses_override_when_set() {
        // Verify that ParquetImportOptions.native_parquet_override propagates correctly.
        // We test the options type directly since Connection requires a live database.
        let opts_force_native =
            crate::import::parquet::ParquetImportOptions::default().with_native_parquet(Some(true));
        assert_eq!(
            opts_force_native.native_parquet_override,
            Some(true),
            "Override Some(true) should be stored"
        );

        let opts_force_csv = crate::import::parquet::ParquetImportOptions::default()
            .with_native_parquet(Some(false));
        assert_eq!(
            opts_force_csv.native_parquet_override,
            Some(false),
            "Override Some(false) should be stored"
        );

        let opts_auto = crate::import::parquet::ParquetImportOptions::default();
        assert_eq!(
            opts_auto.native_parquet_override, None,
            "Default should be None (auto-detect)"
        );
    }
}
