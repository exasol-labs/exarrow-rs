//! Shared transport doubles and wire-format fakes for unit tests.
//!
//! Every test module that needs to double the transport layer depends on the
//! doubles below instead of restating the trait's method shape or the EXA wire
//! format locally. Two independent copies drifting out of sync with the real
//! thing (and with each other) is a back-door duplication this module exists
//! to eliminate. It is declared out of line from `transport::mod`, which also
//! keeps every line here out of the production coverage denominator.
//!
//! Doubles here: `MockTransport` and `StalledQueryTransport` for the
//! `TransportProtocol` trait with `transport_session_info` as the session a
//! mocked login reports, `FakeExasolServer` for the HTTP-tunnel wire
//! format (answering, or silent before or after its handshake),
//! `SilentServer` for a peer that accepts and then never answers (plain,
//! after TLS, or after the WebSocket upgrade), and `FakeWebSocketServer`
//! (`websocket` feature) for scripted WebSocket API exchanges.

use crate::error::TransportError;
use crate::transport::http_transport::{
    generate_magic_packet, TlsCertificate, EXA_MAGIC_PACKET_SIZE, EXA_RESPONSE_PACKET_SIZE,
};
use crate::transport::messages::{ResultData, ResultSetHandle, SessionInfo};
use crate::transport::protocol::{
    ConnectionParams, Credentials, PreparedStatementHandle, QueryResult,
};
use crate::transport::TransportProtocol;
use async_trait::async_trait;
use mockall::mock;
use std::future::Future;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
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

/// The session a mocked `authenticate` reports: a plain Exasol 8 login.
pub(crate) fn transport_session_info() -> SessionInfo {
    SessionInfo {
        session_id: "1739284756".to_string(),
        protocol_version: 3,
        release_version: "8.32.0".to_string(),
        database_name: "exadb".to_string(),
        product_name: "EXASolution".to_string(),
        max_data_message_size: 64 * 1024,
        time_zone: Some("Europe/Berlin".to_string()),
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
    magic_packet_received: Option<oneshot::Receiver<()>>,
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
        Self {
            host,
            port,
            peer,
            magic_packet_received: None,
        }
    }

    /// Answers the handshake and then sends nothing, leaving the export's
    /// tunnel task blocked for as long as the connection stays open.
    pub(crate) async fn silent_after_handshake() -> Self {
        let (listener, host, port) = bind_loopback().await;
        let peer = tokio::spawn(async move {
            let mut stream = accept_and_handshake(&listener).await;
            discard_until_end_of_stream(&mut stream).await;
        });
        Self {
            host,
            port,
            peer,
            magic_packet_received: None,
        }
    }

    /// Reads the magic packet and then never answers it, so a client stays in
    /// its EXA handshake. Await [`wait_for_magic_packet`](Self::wait_for_magic_packet)
    /// to know that the client reached that step.
    pub(crate) async fn silent_before_handshake() -> Self {
        let (listener, host, port) = bind_loopback().await;
        let (received, magic_packet_received) = oneshot::channel();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept the tunnel connection");
            read_magic_packet(&mut stream).await;
            let _ = received.send(());
            discard_until_end_of_stream(&mut stream).await;
        });
        Self {
            host,
            port,
            peer,
            magic_packet_received: Some(magic_packet_received),
        }
    }

    /// Waits until the peer of [`silent_before_handshake`](Self::silent_before_handshake)
    /// has read the magic packet.
    pub(crate) async fn wait_for_magic_packet(&mut self) {
        self.magic_packet_received
            .take()
            .expect("only silent_before_handshake reports the magic packet, and only once")
            .await
            .expect("the peer must read the magic packet before it ends");
    }

    /// Waits for the peer task, which ends once the driver closes its end of
    /// the tunnel connection.
    pub(crate) async fn wait_for_disconnect(&mut self) {
        (&mut self.peer)
            .await
            .expect("the fake peer must not panic");
    }
}

impl Drop for FakeExasolServer {
    fn drop(&mut self) {
        self.peer.abort();
    }
}

/// Awaits `future` and panics once `limit` passes, so a loopback test whose
/// deadline regressed fails instead of hanging CI.
pub(crate) async fn finish_within<T>(limit: Duration, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(limit, future)
        .await
        .unwrap_or_else(|_| panic!("the loopback test must finish within {limit:?}"))
}

/// Outer bound of a loopback test whose own deadline is under test.
pub(crate) const LOOPBACK_TEST_BOUND: Duration = Duration::from_secs(10);

/// How long a loopback test waits for the peer to see the client disconnect.
pub(crate) const DISCONNECT_BOUND: Duration = Duration::from_secs(5);

/// Connection timeout the tests give a `SilentServer` client.
pub(crate) const SILENT_SERVER_CONNECTION_TIMEOUT: Duration = Duration::from_millis(300);

const TERMINATED_CAUSE: &str =
    "Transport was terminated after an export gave up on an in-flight response";

pub(crate) fn test_credentials() -> Credentials {
    Credentials::new("sys".to_string(), "exasol".to_string())
}

/// TLS on without certificate validation, and a 300 ms connection timeout.
pub(crate) fn silent_server_params(server: &SilentServer) -> ConnectionParams {
    ConnectionParams::new(server.host.clone(), server.port)
        .with_validate_server_certificate(false)
        .with_timeout(SILENT_SERVER_CONNECTION_TIMEOUT.as_millis() as u64)
}

/// Asserts that `error` tells the caller the transport was terminated and must
/// be reconnected, rather than blaming a missing login or a repeated connect.
pub(crate) fn assert_names_the_termination(error: &TransportError) {
    let message = error.to_string();
    assert!(message.contains(TERMINATED_CAUSE), "{message}");
    assert!(message.contains("reconnect"), "{message}");
    assert!(!message.contains("Must"), "{message}");
    assert!(!message.contains("Already"), "{message}");
}

/// A loopback peer that accepts one connection and then never answers, so a
/// test can show which setup step a client-side deadline interrupts.
pub(crate) struct SilentServer {
    pub(crate) host: String,
    pub(crate) port: u16,
    certificate_der: Option<Vec<u8>>,
    peer: JoinHandle<()>,
}

impl SilentServer {
    /// Accepts one connection, never writes, and reads until end of stream.
    pub(crate) async fn accepting() -> Self {
        let (listener, host, port) = bind_loopback().await;
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept the connection");
            discard_until_end_of_stream(&mut stream).await;
        });
        Self {
            host,
            port,
            certificate_der: None,
            peer,
        }
    }

    /// Completes a server-side TLS handshake with a self-signed certificate
    /// for `localhost`, then reads until end of stream without writing.
    pub(crate) async fn after_tls() -> Self {
        let certificate = TlsCertificate::generate().expect("generate a test certificate");
        let server_config = certificate
            .to_server_config()
            .expect("build the test server config");
        let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server_config));
        let (listener, host, port) = bind_loopback().await;
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept the connection");
            // A client that rejects the certificate fails the handshake; that ends the peer.
            if let Ok(mut tls_stream) = acceptor.accept(stream).await {
                discard_until_end_of_stream(&mut tls_stream).await;
            }
        });
        Self {
            host,
            port,
            certificate_der: Some(certificate.certificate_der),
            peer,
        }
    }

    /// Completes the WebSocket upgrade on a plain connection, then reads until
    /// the client closes without sending a frame.
    #[cfg(feature = "websocket")]
    pub(crate) async fn after_websocket_upgrade() -> Self {
        use futures_util::StreamExt;

        let (listener, host, port) = bind_loopback().await;
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept the connection");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete the WebSocket upgrade");
            while let Some(Ok(_)) = socket.next().await {}
        });
        Self {
            host,
            port,
            certificate_der: None,
            peer,
        }
    }

    /// The DER bytes of the certificate that [`after_tls`](Self::after_tls) presents.
    pub(crate) fn certificate_der(&self) -> &[u8] {
        self.certificate_der
            .as_deref()
            .expect("only SilentServer::after_tls presents a certificate")
    }

    /// Waits until the peer saw end of stream, which shows that the client
    /// released its socket.
    pub(crate) async fn wait_for_disconnect(&mut self) {
        (&mut self.peer)
            .await
            .expect("the silent peer must not panic");
    }
}

impl Drop for SilentServer {
    fn drop(&mut self) {
        self.peer.abort();
    }
}

async fn discard_until_end_of_stream(stream: &mut (impl tokio::io::AsyncRead + Unpin)) {
    let mut discarded = Vec::new();
    let _ = stream.read_to_end(&mut discarded).await;
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

async fn read_magic_packet(stream: &mut TcpStream) {
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
}

async fn accept_and_handshake(listener: &TcpListener) -> TcpStream {
    let (mut stream, _) = listener
        .accept()
        .await
        .expect("accept the export connection");
    read_magic_packet(&mut stream).await;
    stream
        .write_all(&exa_response_packet(INTERNAL_IP, INTERNAL_PORT))
        .await
        .expect("write the response packet");
    stream.flush().await.expect("flush the response packet");
    stream
}

/// A loopback WebSocket peer that answers each request with the next scripted
/// JSON response and records every request it receives.
///
/// It lets a unit test run the real `WebSocketTransport` request and response
/// handling, which a mocked `TransportProtocol` would bypass.
#[cfg(feature = "websocket")]
pub(crate) struct FakeWebSocketServer {
    pub(crate) port: u16,
    requests: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    peer: JoinHandle<()>,
}

#[cfg(feature = "websocket")]
impl FakeWebSocketServer {
    /// Accepts one plain `ws://` connection and replies to the n-th text frame
    /// with `responses[n]`.
    pub(crate) async fn scripted(responses: Vec<serde_json::Value>) -> Self {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (listener, _, port) = bind_loopback().await;
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = std::sync::Arc::clone(&requests);
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept the connection");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete the WebSocket handshake");
            for response in responses {
                let Some(Ok(Message::Text(text))) = socket.next().await else {
                    return;
                };
                recorded
                    .lock()
                    .expect("request log")
                    .push(serde_json::from_str(&text).expect("requests are JSON"));
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .expect("send the scripted response");
            }
            let _ = socket.next().await;
        });
        Self {
            port,
            requests,
            peer,
        }
    }

    /// The requests received so far, in arrival order.
    pub(crate) fn requests(&self) -> Vec<serde_json::Value> {
        self.requests.lock().expect("request log").clone()
    }
}

#[cfg(feature = "websocket")]
impl Drop for FakeWebSocketServer {
    fn drop(&mut self) {
        self.peer.abort();
    }
}
