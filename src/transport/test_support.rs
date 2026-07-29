//! Shared `TransportProtocol` mock for unit tests.
//!
//! Every test module that needs to double the transport layer depends on this
//! single mock instead of restating the trait's method shape in a local
//! `mockall::mock!` block. Two independent mocks drifting out of sync with the
//! trait (and with each other) is a back-door duplication this module exists
//! to eliminate.

use crate::error::TransportError;
use crate::transport::messages::{ResultData, ResultSetHandle, SessionInfo};
use crate::transport::protocol::{
    ConnectionParams, Credentials, PreparedStatementHandle, QueryResult,
};
use crate::transport::TransportProtocol;
use async_trait::async_trait;
use mockall::mock;

mock! {
    pub Transport {}

    #[async_trait]
    impl TransportProtocol for Transport {
        async fn connect(&mut self, params: &ConnectionParams) -> Result<(), TransportError>;
        async fn authenticate(&mut self, credentials: &Credentials) -> Result<SessionInfo, TransportError>;
        async fn execute_query(&mut self, sql: &str) -> Result<QueryResult, TransportError>;
        async fn fetch_results(&mut self, handle: ResultSetHandle) -> Result<ResultData, TransportError>;
        async fn close_result_set(&mut self, handle: ResultSetHandle) -> Result<(), TransportError>;
        async fn create_prepared_statement(&mut self, sql: &str) -> Result<PreparedStatementHandle, TransportError>;
        async fn execute_prepared_statement(&mut self, handle: &PreparedStatementHandle, parameters: Option<Vec<Vec<serde_json::Value>>>) -> Result<QueryResult, TransportError>;
        async fn close_prepared_statement(&mut self, handle: &PreparedStatementHandle) -> Result<(), TransportError>;
        async fn close(&mut self) -> Result<(), TransportError>;
        fn is_connected(&self) -> bool;
        async fn set_autocommit(&mut self, enabled: bool) -> Result<(), TransportError>;
        async fn set_query_timeout(&mut self, timeout_secs: u64) -> Result<(), TransportError>;
    }
}
