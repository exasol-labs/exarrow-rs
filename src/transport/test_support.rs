//! Shared transport doubles and wire-format fakes for unit tests.
//!
//! Every test module that needs to double the transport layer depends on the
//! doubles below instead of restating the trait's method shape or the EXA wire
//! format locally. Two independent copies drifting out of sync with the real
//! thing (and with each other) is a back-door duplication this module exists
//! to eliminate. It is declared out of line from `transport::mod`, which also
//! keeps every line here out of the production coverage denominator.
//!
//! Doubles here: `MockTransport`, `StalledQueryTransport`, and
//! `GatedQueryTransport` for the `TransportProtocol` trait with
//! `transport_session_info` as the session a mocked login reports,
//! `FakeExasolServer` for the HTTP-tunnel wire format (answering, requesting an
//! import, streaming a chunked EXPORT body behind a gate or breaking off after
//! its first chunk, silent before or after its handshake, or starting,
//! stalling, or refusing the TLS handshake that Exasol runs as the client)
//! with `ChunkedBody` to split what it received,
//! `SilentServer` for a peer that accepts and then never answers (plain,
//! after TLS, or after the WebSocket upgrade), and `FakeWebSocketServer`
//! (`websocket` feature) for scripted WebSocket API exchanges.

use crate::error::TransportError;
use crate::transport::http_transport::{
    encode_chunk, generate_magic_packet, TlsCertificate, EXA_MAGIC_PACKET_SIZE,
    EXA_RESPONSE_PACKET_SIZE,
};
use crate::transport::messages::{ResultData, ResultSetHandle, SessionInfo};
use crate::transport::protocol::{
    ConnectionParams, Credentials, PreparedStatementHandle, QueryResult,
};
use crate::transport::tls::{client_config, client_handshake};
use crate::transport::{HttpTransportClient, TransportProtocol};
use async_trait::async_trait;
use mockall::mock;
use std::future::Future;
use std::sync::{mpsc, Arc};
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
        async fn set_current_schema(&mut self, schema: &str) -> Result<(), TransportError>;
        async fn refresh_current_schema(&mut self) -> Result<Option<String>, TransportError>;
        fn current_schema(&self) -> Option<String>;
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

/// A transport whose `execute_query` is held back, delegating every other call
/// to the wrapped mock. It either never resolves ([`Self::stalled`]) or
/// resolves with a preset result once a signal fires ([`Self::new`]).
///
/// `mockall` hands back an already-ready future, so a mock on its own cannot
/// represent a SQL response that is still outstanding, the one state a
/// client-side deadline exists to interrupt. Delegating the rest keeps
/// expectations and their verification with the inner mock.
///
/// Exasol answers an EXPORT statement only after the tunnel transfer ended, so
/// a loopback test of the export order has to decide when the response
/// arrives, which a mock's ready future cannot. A dropped signal never fires.
pub(crate) struct GatedQueryTransport {
    inner: MockTransport,
    answer: Option<Answer>,
}

struct Answer {
    release: oneshot::Receiver<()>,
    result: Result<QueryResult, TransportError>,
}

impl GatedQueryTransport {
    pub(crate) fn new(
        inner: MockTransport,
        release: oneshot::Receiver<()>,
        result: Result<QueryResult, TransportError>,
    ) -> Self {
        Self {
            inner,
            answer: Some(Answer { release, result }),
        }
    }

    pub(crate) fn stalled(inner: MockTransport) -> Self {
        Self {
            inner,
            answer: None,
        }
    }
}

#[async_trait]
impl TransportProtocol for GatedQueryTransport {
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
        let Some(answer) = self.answer.take() else {
            return std::future::pending().await;
        };
        if answer.release.await.is_err() {
            std::future::pending::<()>().await;
        }
        answer.result
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

    async fn set_current_schema(&mut self, schema: &str) -> Result<(), TransportError> {
        self.inner.set_current_schema(schema).await
    }

    async fn refresh_current_schema(&mut self) -> Result<Option<String>, TransportError> {
        self.inner.refresh_current_schema().await
    }

    fn current_schema(&self) -> Option<String> {
        self.inner.current_schema()
    }
}

/// The address the fake below reports as Exasol's internal tunnel endpoint. It
/// only ever reaches the generated EXPORT statement, which a doubled transport
/// accepts without dialling it.
const INTERNAL_IP: &str = "10.0.0.5";
const INTERNAL_PORT: i32 = 8563;

/// The header of a TLS handshake record that announces a 512-byte body, as a
/// ClientHello from Exasol starts.
const TLS_HANDSHAKE_RECORD_HEADER: [u8; 5] = [0x16, 0x03, 0x01, 0x02, 0x00];

/// How long [`FakeExasolServer::accepts_another_connection`] waits. A connect
/// the driver already made sits in the listener backlog, so this only has to
/// cover the accept itself.
const RECONNECT_WINDOW: Duration = Duration::from_millis(100);

/// A loopback peer that plays Exasol's side of an HTTP tunnel.
///
/// The import and export paths open a real TCP connection and complete the
/// binary tunnel handshake before any HTTP traffic, so none of their runtime
/// behaviour is reachable from a unit test without a peer that speaks that
/// wire format.
pub(crate) struct FakeExasolServer {
    pub(crate) host: String,
    pub(crate) port: u16,
    listener: Arc<TcpListener>,
    peer: JoinHandle<Vec<u8>>,
    magic_packet_received: Option<oneshot::Receiver<()>>,
    first_body_chunk_received: Option<mpsc::Receiver<()>>,
}

impl FakeExasolServer {
    /// Answers the handshake, then delivers `csv` as the body of one HTTP
    /// `PUT`, the shape Exasol uses to push EXPORT results at the driver.
    pub(crate) async fn serving_csv(csv: &str) -> Self {
        let body = csv.as_bytes().to_vec();
        Self::spawn(|listener| async move {
            let mut stream = accept_and_handshake(&listener).await;
            let head = format!(
                "PUT /000.csv HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).await.expect("write head");
            stream.write_all(&body).await.expect("write body");
            stream.flush().await.expect("flush the PUT");
            read_until_end_of_stream(&mut stream).await
        })
        .await
    }

    /// Answers the handshake and sends a chunked `PUT` whose first chunk is
    /// `chunks[0]`, then waits for `gate` before it sends the remaining chunks
    /// and the terminator, as Exasol streams an EXPORT body while the statement
    /// runs. The returned receiver fires once the peer has read the driver's
    /// 200 OK, which a test can make the EXPORT response wait for.
    ///
    /// A dropped gate never opens. A driver that closes the tunnel while the
    /// peer waits at the gate ends the peer, as `wait_for_disconnect` observes,
    /// and the receiver never fires. `wait_for_disconnect` returns the bytes
    /// the peer read after its last chunk.
    pub(crate) async fn serving_chunked_put(
        chunks: Vec<Vec<u8>>,
        gate: oneshot::Receiver<()>,
    ) -> (Self, oneshot::Receiver<()>) {
        let (report_response, response_received) = oneshot::channel();
        let server = Self::spawn(|listener| async move {
            let mut stream = accept_and_handshake(&listener).await;
            let mut chunks = chunks.into_iter();
            write_chunked_put_head(&mut stream).await;
            if let Some(first) = chunks.next() {
                write_and_flush(&mut stream, &encode_chunk(&first)).await;
            }
            if !gate_opens_before_disconnect(gate, &mut stream).await {
                return Vec::new();
            }
            for chunk in chunks {
                write_and_flush(&mut stream, &encode_chunk(&chunk)).await;
            }
            write_and_flush(&mut stream, &encode_chunk(&[])).await;

            let mut received = read_response_head(&mut stream).await;
            if received.ends_with(b"\r\n\r\n") {
                let _ = report_response.send(());
            }
            received.extend(read_until_end_of_stream(&mut stream).await);
            received
        })
        .await;
        (server, response_received)
    }

    /// Answers the handshake, sends a chunked `PUT` with `first_chunk`, and
    /// then closes its sending side without a terminator, as a tunnel that
    /// breaks in the middle of the body. The returned receiver fires once the
    /// peer has closed.
    pub(crate) async fn closing_after_first_chunk(
        first_chunk: Vec<u8>,
    ) -> (Self, oneshot::Receiver<()>) {
        let (report_close, closed) = oneshot::channel();
        let server = Self::spawn(|listener| async move {
            let mut stream = accept_and_handshake(&listener).await;
            write_chunked_put_head(&mut stream).await;
            write_and_flush(&mut stream, &encode_chunk(&first_chunk)).await;
            stream.shutdown().await.expect("close the sending side");
            let _ = report_close.send(());
            read_until_end_of_stream(&mut stream).await
        })
        .await;
        (server, closed)
    }

    /// Answers the handshake and then sends nothing, leaving the export's
    /// tunnel task blocked for as long as the connection stays open.
    pub(crate) async fn silent_after_handshake() -> Self {
        Self::spawn(|listener| async move {
            let mut stream = accept_and_handshake(&listener).await;
            read_until_end_of_stream(&mut stream).await
        })
        .await
    }

    /// Reads the magic packet and then never answers it, so a client stays in
    /// its EXA handshake. Await [`wait_for_magic_packet`](Self::wait_for_magic_packet)
    /// to know that the client reached that step.
    pub(crate) async fn silent_before_handshake() -> Self {
        let (received, magic_packet_received) = oneshot::channel();
        let mut server = Self::spawn(|listener| async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept the tunnel connection");
            read_magic_packet(&mut stream).await;
            let _ = received.send(());
            read_until_end_of_stream(&mut stream).await
        })
        .await;
        server.magic_packet_received = Some(magic_packet_received);
        server
    }

    /// Answers the handshake, sends a plain `GET /001.csv` as Exasol does for
    /// an IMPORT, and reads the response until the driver closes the tunnel.
    /// [`first_body_chunk_signal`](Self::first_body_chunk_signal) fires once
    /// the first whole body chunk has arrived.
    pub(crate) async fn requesting_import() -> Self {
        let (received, first_body_chunk_received) = mpsc::channel();
        let mut server = Self::spawn(|listener| async move {
            let mut stream = accept_and_handshake(&listener).await;
            let request =
                format!("GET /001.csv HTTP/1.1\r\nHost: {INTERNAL_IP}:{INTERNAL_PORT}\r\n\r\n");
            stream
                .write_all(request.as_bytes())
                .await
                .expect("send the GET");
            stream.flush().await.expect("flush the GET");
            read_response_signalling_first_body_chunk(&mut stream, received).await
        })
        .await;
        server.first_body_chunk_received = Some(first_body_chunk_received);
        server
    }

    /// Opens a plain tunnel to this peer and completes the EXA handshake.
    pub(crate) async fn connect_tunnel(&self) -> HttpTransportClient {
        HttpTransportClient::connect(&self.host, self.port, false)
            .await
            .expect("the tunnel connects once the EXA handshake completes")
    }

    /// Answers the handshake, waits `delay`, then starts TLS as the client
    /// that accepts any certificate, as Exasol does once it receives the
    /// IMPORT statement, and sends `GET /001.csv` over it.
    pub(crate) async fn requesting_over_tls_after(delay: Duration) -> Self {
        Self::spawn(move |listener| async move {
            let stream = accept_and_handshake(&listener).await;
            tokio::time::sleep(delay).await;
            let mut tls = client_handshake(stream, "localhost", client_config(None, false))
                .await
                .expect("the driver must accept the TLS handshake that the peer starts");
            let request =
                format!("GET /001.csv HTTP/1.1\r\nHost: {INTERNAL_IP}:{INTERNAL_PORT}\r\n\r\n");
            tls.write_all(request.as_bytes())
                .await
                .expect("send the GET over TLS");
            tls.flush().await.expect("flush the GET");
            read_until_end_of_stream(&mut tls).await
        })
        .await
    }

    /// Answers the handshake, then sends only the 5-byte header of a TLS
    /// handshake record and stops, so the driver stalls mid-handshake.
    pub(crate) async fn stalling_mid_tls_handshake() -> Self {
        Self::spawn(|listener| async move {
            let mut stream = accept_and_handshake(&listener).await;
            stream
                .write_all(&TLS_HANDSHAKE_RECORD_HEADER)
                .await
                .expect("write the TLS record header");
            stream.flush().await.expect("flush the TLS record header");
            read_until_end_of_stream(&mut stream).await
        })
        .await
    }

    /// Answers the handshake and then closes its sending side before any TLS
    /// byte, reading on until the driver closes the tunnel.
    pub(crate) async fn closing_after_handshake() -> Self {
        Self::spawn(|listener| async move {
            let mut stream = accept_and_handshake(&listener).await;
            stream.shutdown().await.expect("close the sending side");
            read_until_end_of_stream(&mut stream).await
        })
        .await
    }

    /// Reports whether the driver opened another tunnel connection after the
    /// first one.
    pub(crate) async fn accepts_another_connection(&self) -> bool {
        tokio::time::timeout(RECONNECT_WINDOW, self.listener.accept())
            .await
            .is_ok()
    }

    async fn spawn<F, Fut>(script: F) -> Self
    where
        F: FnOnce(Arc<TcpListener>) -> Fut,
        Fut: Future<Output = Vec<u8>> + Send + 'static,
    {
        let (listener, host, port) = bind_loopback().await;
        let listener = Arc::new(listener);
        let peer = tokio::spawn(script(Arc::clone(&listener)));
        Self {
            host,
            port,
            listener,
            peer,
            magic_packet_received: None,
            first_body_chunk_received: None,
        }
    }

    /// The signal of [`requesting_import`](Self::requesting_import) that fires
    /// once the first whole body chunk has arrived. It is a std channel, so a
    /// blocking thread such as a Parquet read can wait on it with a timeout.
    pub(crate) fn first_body_chunk_signal(&mut self) -> mpsc::Receiver<()> {
        self.first_body_chunk_received
            .take()
            .expect("only requesting_import signals the first body chunk, and only once")
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
    /// the tunnel connection, and returns the bytes the peer read after its
    /// scripted exchange.
    pub(crate) async fn wait_for_disconnect(&mut self) -> Vec<u8> {
        (&mut self.peer)
            .await
            .expect("the fake peer must not panic")
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
            read_until_end_of_stream(&mut stream).await;
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
                read_until_end_of_stream(&mut tls_stream).await;
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

/// A TLS read fails when the driver closes without `close_notify`, so any
/// read error ends the stream here like end of stream does.
async fn read_until_end_of_stream(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut received = Vec::new();
    let _ = stream.read_to_end(&mut received).await;
    received
}

async fn read_response_signalling_first_body_chunk(
    stream: &mut TcpStream,
    first_body_chunk: mpsc::Sender<()>,
) -> Vec<u8> {
    let mut received = Vec::new();
    let mut pending_signal = Some(first_body_chunk);
    let mut buffer = [0u8; 16 * 1024];
    while let Ok(read @ 1..) = stream.read(&mut buffer).await {
        received.extend_from_slice(&buffer[..read]);
        if let Some(signal) =
            pending_signal.take_if(|_| !ChunkedBody::parse(&received).chunks.is_empty())
        {
            let _ = signal.send(());
        }
    }
    received
}

/// The body of a chunked HTTP response as a fake peer received it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ChunkedBody {
    pub(crate) chunks: Vec<Vec<u8>>,
    /// Whether the zero-length chunk that ends a chunked body arrived.
    pub(crate) terminated: bool,
}

impl ChunkedBody {
    /// Splits `response` into its body chunks. An incomplete trailing chunk is
    /// left out, as a peer that saw the connection close mid-chunk would.
    pub(crate) fn parse(response: &[u8]) -> Self {
        let mut body = Self {
            chunks: Vec::new(),
            terminated: false,
        };
        let Some((_, mut rest)) = split_at_first(response, b"\r\n\r\n") else {
            return body;
        };
        while let Some((size_line, after_size)) = split_at_first(rest, b"\r\n") {
            let size_line = std::str::from_utf8(size_line).expect("a chunk size line is ASCII");
            let size = usize::from_str_radix(size_line.trim(), 16).expect("a chunk size is hex");
            if size == 0 {
                body.terminated = after_size.starts_with(b"\r\n");
                break;
            }
            if after_size.len() < size + 2 {
                break;
            }
            body.chunks.push(after_size[..size].to_vec());
            rest = &after_size[size + 2..];
        }
        body
    }

    /// The body bytes, every chunk joined in order.
    pub(crate) fn data(&self) -> Vec<u8> {
        self.chunks.concat()
    }
}

fn split_at_first<'a>(bytes: &'a [u8], separator: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let position = bytes
        .windows(separator.len())
        .position(|window| window == separator)?;
    Some((&bytes[..position], &bytes[position + separator.len()..]))
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

async fn write_and_flush(stream: &mut TcpStream, bytes: &[u8]) {
    stream.write_all(bytes).await.expect("write to the driver");
    stream.flush().await.expect("flush to the driver");
}

async fn write_chunked_put_head(stream: &mut TcpStream) {
    write_and_flush(
        stream,
        b"PUT /000.csv HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
    )
    .await;
}

/// Reports whether `gate` opened before the driver closed the tunnel. The
/// driver sends nothing before its 200 OK, so a read that completes means the
/// tunnel closed.
async fn gate_opens_before_disconnect(gate: oneshot::Receiver<()>, stream: &mut TcpStream) -> bool {
    let opened = async {
        if gate.await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let mut probe = [0u8; 1];
    tokio::select! {
        () = opened => true,
        _ = stream.read(&mut probe) => false,
    }
}

/// Reads up to and including the blank line that ends an HTTP response head,
/// or until the driver closes the tunnel.
async fn read_response_head(stream: &mut TcpStream) -> Vec<u8> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte).await {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    head
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

mod tests {
    use super::{
        finish_within, ChunkedBody, FakeExasolServer, DISCONNECT_BOUND, LOOPBACK_TEST_BOUND,
    };

    const HEADERS: &[u8] = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";

    fn response(body: &[u8]) -> Vec<u8> {
        [HEADERS, body].concat()
    }

    #[test]
    fn chunked_body_splits_the_chunks_and_reports_the_terminating_chunk() {
        let body = ChunkedBody::parse(&response(b"4\r\n1,a\n\r\n3\r\nb\nc\r\n0\r\n\r\n"));

        assert_eq!(body.chunks, vec![b"1,a\n".to_vec(), b"b\nc".to_vec()]);
        assert!(body.terminated);
        assert_eq!(body.data(), b"1,a\nb\nc");
    }

    #[test]
    fn chunked_body_of_a_truncated_response_keeps_the_whole_chunks_only() {
        let body = ChunkedBody::parse(&response(b"4\r\n1,a\n\r\n5\r\nab"));

        assert_eq!(body.chunks, vec![b"1,a\n".to_vec()]);
        assert!(!body.terminated);
    }

    #[test]
    fn chunked_body_without_complete_headers_is_empty() {
        let body = ChunkedBody::parse(b"HTTP/1.1 200 OK\r\n");

        assert!(body.chunks.is_empty());
        assert!(!body.terminated);
    }

    #[tokio::test]
    async fn requesting_import_sends_a_get_and_signals_the_first_body_chunk() {
        let mut server = FakeExasolServer::requesting_import().await;
        let first_body_chunk = server.first_body_chunk_signal();
        let mut client = server.connect_tunnel().await;

        let request = finish_within(LOOPBACK_TEST_BOUND, client.handle_import_request())
            .await
            .expect("the peer sends a GET");
        assert!(first_body_chunk.try_recv().is_err());
        client.write_chunked_body(b"1,a\n").await.expect("write");
        tokio::task::spawn_blocking(move || first_body_chunk.recv_timeout(DISCONNECT_BOUND))
            .await
            .expect("the waiting thread must not panic")
            .expect("the peer signals the first body chunk");
        client.write_final_chunk().await.expect("final chunk");
        drop(client);

        assert_eq!(request.path, "/001.csv");
        let received = finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        let body = ChunkedBody::parse(&received);
        assert_eq!(body.chunks, vec![b"1,a\n".to_vec()]);
        assert!(body.terminated);
    }
}
