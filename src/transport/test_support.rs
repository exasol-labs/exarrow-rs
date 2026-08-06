//! Shared transport doubles and wire-format fakes for unit tests.
//!
//! Every test module that needs to double the transport layer depends on the
//! doubles below instead of restating the trait's method shape or the EXA wire
//! format locally. Two independent copies drifting out of sync with the real
//! thing (and with each other) is a back-door duplication this module exists
//! to eliminate. It is declared out of line from `transport::mod`, which also
//! keeps every line here out of the production coverage denominator.

use crate::error::TransportError;
use crate::transport::http_transport::{
    generate_magic_packet, EXA_MAGIC_PACKET_SIZE, EXA_RESPONSE_PACKET_SIZE,
};
use crate::transport::messages::{ResultData, ResultSetHandle, SessionInfo};
use crate::transport::protocol::{
    ConnectionParams, Credentials, PreparedStatementHandle, QueryResult,
};
use crate::transport::TransportProtocol;
use async_trait::async_trait;
use mockall::mock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

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
        fn terminate(&mut self);
        fn is_connected(&self) -> bool;
        async fn set_autocommit(&mut self, enabled: bool) -> Result<(), TransportError>;
        async fn set_query_timeout(&mut self, timeout_secs: u64) -> Result<(), TransportError>;
    }
}

/// Builds a well-formed EXA response packet: reserved i32, port i32, then the
/// IP as a null-padded 16-byte field.
pub(crate) fn exa_response_packet(ip: &str, port: i32) -> [u8; EXA_RESPONSE_PACKET_SIZE] {
    assert!(ip.len() <= 16, "the IP field holds at most 16 bytes");
    let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];
    packet[4..8].copy_from_slice(&port.to_le_bytes());
    packet[8..8 + ip.len()].copy_from_slice(ip.as_bytes());
    packet
}

/// A transport whose `execute_query` never resolves, delegating every other
/// call to the wrapped mock.
///
/// `mockall` hands back an already-ready future, so a mock on its own cannot
/// represent a SQL response that is still outstanding — the one state a
/// client-side deadline exists to interrupt. Delegating the rest keeps
/// expectations and their verification with the inner mock.
pub(crate) struct StalledQueryTransport {
    inner: MockTransport,
}

impl StalledQueryTransport {
    pub(crate) fn new(inner: MockTransport) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl TransportProtocol for StalledQueryTransport {
    async fn connect(&mut self, params: &ConnectionParams) -> Result<(), TransportError> {
        self.inner.connect(params).await
    }

    async fn authenticate(
        &mut self,
        credentials: &Credentials,
    ) -> Result<SessionInfo, TransportError> {
        self.inner.authenticate(credentials).await
    }

    async fn execute_query(&mut self, _sql: &str) -> Result<QueryResult, TransportError> {
        std::future::pending().await
    }

    async fn fetch_results(
        &mut self,
        handle: ResultSetHandle,
    ) -> Result<ResultData, TransportError> {
        self.inner.fetch_results(handle).await
    }

    async fn close_result_set(&mut self, handle: ResultSetHandle) -> Result<(), TransportError> {
        self.inner.close_result_set(handle).await
    }

    async fn create_prepared_statement(
        &mut self,
        sql: &str,
    ) -> Result<PreparedStatementHandle, TransportError> {
        self.inner.create_prepared_statement(sql).await
    }

    async fn execute_prepared_statement(
        &mut self,
        handle: &PreparedStatementHandle,
        parameters: Option<Vec<Vec<serde_json::Value>>>,
    ) -> Result<QueryResult, TransportError> {
        self.inner
            .execute_prepared_statement(handle, parameters)
            .await
    }

    async fn close_prepared_statement(
        &mut self,
        handle: &PreparedStatementHandle,
    ) -> Result<(), TransportError> {
        self.inner.close_prepared_statement(handle).await
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.inner.close().await
    }

    fn terminate(&mut self) {
        self.inner.terminate();
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    async fn set_autocommit(&mut self, enabled: bool) -> Result<(), TransportError> {
        self.inner.set_autocommit(enabled).await
    }

    async fn set_query_timeout(&mut self, timeout_secs: u64) -> Result<(), TransportError> {
        self.inner.set_query_timeout(timeout_secs).await
    }
}

/// The address the fake below reports as Exasol's internal tunnel endpoint. It
/// only ever reaches the generated EXPORT statement, which a doubled transport
/// accepts without dialling it.
const INTERNAL_IP: &str = "10.0.0.5";
const INTERNAL_PORT: i32 = 8563;

/// A loopback peer that plays Exasol's side of an HTTP-tunnel export.
///
/// The export path opens a real TCP connection and completes the binary tunnel
/// handshake before any HTTP traffic, so none of its runtime behaviour is
/// reachable from a unit test without a peer that speaks that wire format.
pub(crate) struct FakeExasolServer {
    pub(crate) host: String,
    pub(crate) port: u16,
    peer: JoinHandle<()>,
}

impl FakeExasolServer {
    /// Answers the handshake, then delivers `csv` as the body of one HTTP
    /// `PUT`, the shape Exasol uses to push EXPORT results at the driver.
    pub(crate) async fn serving_csv(csv: &str) -> Self {
        let body = csv.as_bytes().to_vec();
        let (listener, host, port) = bind_loopback().await;
        let peer = tokio::spawn(async move {
            let mut stream = accept_and_handshake(&listener).await;
            let head = format!(
                "PUT /000.csv HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).await.expect("write head");
            stream.write_all(&body).await.expect("write body");
            stream.flush().await.expect("flush the PUT");
            let mut acknowledgement = Vec::new();
            let _ = stream.read_to_end(&mut acknowledgement).await;
        });
        Self { host, port, peer }
    }

    /// Answers the handshake and then sends nothing, leaving the export's
    /// tunnel task blocked for as long as the connection stays open.
    pub(crate) async fn silent_after_handshake() -> Self {
        let (listener, host, port) = bind_loopback().await;
        let peer = tokio::spawn(async move {
            let mut stream = accept_and_handshake(&listener).await;
            let mut discarded = Vec::new();
            let _ = stream.read_to_end(&mut discarded).await;
        });
        Self { host, port, peer }
    }
}

impl Drop for FakeExasolServer {
    fn drop(&mut self) {
        self.peer.abort();
    }
}

async fn bind_loopback() -> (TcpListener, String, u16) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the loopback interface must accept an ephemeral port");
    let address = listener
        .local_addr()
        .expect("a bound listener has an address");
    (listener, address.ip().to_string(), address.port())
}

async fn accept_and_handshake(listener: &TcpListener) -> TcpStream {
    let (mut stream, _) = listener
        .accept()
        .await
        .expect("accept the export connection");
    let mut magic = [0u8; EXA_MAGIC_PACKET_SIZE];
    stream
        .read_exact(&mut magic)
        .await
        .expect("read the magic packet");
    assert_eq!(
        magic,
        generate_magic_packet(),
        "the driver must open the tunnel with the EXA magic packet"
    );
    stream
        .write_all(&exa_response_packet(INTERNAL_IP, INTERNAL_PORT))
        .await
        .expect("write the response packet");
    stream.flush().await.expect("flush the response packet");
    stream
}
