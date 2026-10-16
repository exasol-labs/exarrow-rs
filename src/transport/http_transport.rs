//! HTTP Transport for Exasol bulk data import/export.
//!
//! This module implements the EXA tunneling protocol for high-performance bulk data transfer.
//! The protocol works as follows:
//!
//! 1. Client starts an HTTP server on a local port
//! 2. Exasol connects TO the client's HTTP server (reverse connection)
//! 3. Data is streamed via HTTP with chunked transfer encoding
//! 4. A special EXA tunneling protocol handshake is required
//!
//! # EXA Tunneling Protocol
//!
//! The handshake uses a magic packet `0x02212102` followed by version info.
//! The server responds with 24 bytes containing internal IP and port.
//!
//! # TLS Support
//!
//! For encrypted connections, ad-hoc self-signed certificates are generated using `rcgen`.
//! SHA-256 fingerprints are computed for Exasol 8.32.0+ compatibility.
//! The driver is the TLS server of the tunnel: Exasol starts the TLS handshake
//! as the client after it receives the IMPORT or EXPORT statement.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use rcgen::{CertifiedKey, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::{server, TlsAcceptor};

use crate::error::TransportError;

use super::deadline::{SetupDeadline, SetupStep};

/// EXA tunneling protocol magic number.
/// This is sent as the first 4 bytes of the handshake.
pub const EXA_MAGIC_NUMBER: u32 = 0x02212102;

/// EXA protocol version (major).
pub const EXA_PROTOCOL_VERSION_MAJOR: u32 = 1;

/// EXA protocol version (minor).
pub const EXA_PROTOCOL_VERSION_MINOR: u32 = 1;

/// Size of the EXA magic packet sent by client (12 bytes: magic + major + minor).
pub const EXA_MAGIC_PACKET_SIZE: usize = 12;

/// Size of the EXA response packet from server (24 bytes: internal addr info).
pub const EXA_RESPONSE_PACKET_SIZE: usize = 24;

/// HTTP chunk size for data transfer (64KB).
pub const HTTP_CHUNK_SIZE: usize = 64 * 1024;

/// The largest piece of an EXPORT body that the driver forwards in one channel
/// message, whatever chunk size or Content-Length Exasol declares.
pub(crate) const EXPORT_PIECE_MAX_BYTES: usize = 64 * 1024;

/// The bound that [`HttpTransportClient::connect`] puts on the TCP connect and
/// the EXA handshake together, and separately on the TLS handshake.
const TUNNEL_SETUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Generates the EXA tunneling magic packet.
///
/// The packet consists of:
/// - 4 bytes: magic number `0x02212102` (little-endian)
/// - 4 bytes: major version (little-endian)
/// - 4 bytes: minor version (little-endian)
///
/// # Returns
///
/// A 12-byte array containing the magic packet.
#[must_use]
pub fn generate_magic_packet() -> [u8; EXA_MAGIC_PACKET_SIZE] {
    let mut packet = [0u8; EXA_MAGIC_PACKET_SIZE];
    packet[0..4].copy_from_slice(&EXA_MAGIC_NUMBER.to_le_bytes());
    packet[4..8].copy_from_slice(&EXA_PROTOCOL_VERSION_MAJOR.to_le_bytes());
    packet[8..12].copy_from_slice(&EXA_PROTOCOL_VERSION_MINOR.to_le_bytes());
    packet
}

/// Parses the EXA tunneling magic packet.
///
/// # Arguments
///
/// * `packet` - A 12-byte slice containing the magic packet
///
/// # Returns
///
/// A tuple of (magic_number, major_version, minor_version) on success.
///
/// # Errors
///
/// Returns `TransportError::ProtocolError` if the packet is invalid.
pub fn parse_magic_packet(packet: &[u8]) -> Result<(u32, u32, u32), TransportError> {
    if packet.len() < EXA_MAGIC_PACKET_SIZE {
        return Err(TransportError::ProtocolError(format!(
            "Magic packet too short: expected {} bytes, got {}",
            EXA_MAGIC_PACKET_SIZE,
            packet.len()
        )));
    }

    let magic = u32::from_le_bytes([packet[0], packet[1], packet[2], packet[3]]);
    let major = u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]);
    let minor = u32::from_le_bytes([packet[8], packet[9], packet[10], packet[11]]);

    if magic != EXA_MAGIC_NUMBER {
        return Err(TransportError::ProtocolError(format!(
            "Invalid magic number: expected 0x{:08X}, got 0x{:08X}",
            EXA_MAGIC_NUMBER, magic
        )));
    }

    Ok((magic, major, minor))
}

/// Parses the EXA server response packet.
///
/// The response packet format matches PyExasol's `struct.unpack("ii16s", data)`:
/// - Bytes 0-3: Reserved/unused i32 (little-endian)
/// - Bytes 4-7: Internal port as i32 (little-endian)
/// - Bytes 8-23: Internal IP address as 16-byte null-terminated string
///
/// # Arguments
///
/// * `packet` - A 24-byte slice containing the response packet
///
/// # Returns
///
/// A tuple of (ip_string, port) on success.
///
/// # Errors
///
/// Returns `TransportError::ProtocolError` if the packet is invalid.
pub fn parse_response_packet(packet: &[u8]) -> Result<(String, u16), TransportError> {
    if packet.len() < EXA_RESPONSE_PACKET_SIZE {
        return Err(TransportError::ProtocolError(format!(
            "Response packet too short: expected {} bytes, got {}",
            EXA_RESPONSE_PACKET_SIZE,
            packet.len()
        )));
    }

    // Bytes 0-3: Reserved/unused (first i32)
    // (we ignore this value)

    // Bytes 4-7: Port as i32 little-endian
    let port = i32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]);
    if port < 0 || port > u16::MAX as i32 {
        return Err(TransportError::ProtocolError(format!(
            "Invalid port in response packet: {}",
            port
        )));
    }

    // Bytes 8-23: IP address as 16-byte null-terminated string
    let ip_bytes = &packet[8..24];
    let ip_string = String::from_utf8_lossy(ip_bytes)
        .trim_end_matches('\0')
        .to_string();

    if ip_string.is_empty() {
        return Err(TransportError::ProtocolError(
            "Empty IP address in response packet".to_string(),
        ));
    }

    Ok((ip_string, port as u16))
}

/// Performs the EXA tunneling handshake over a TCP connection.
///
/// # Arguments
///
/// * `stream` - A mutable reference to an async stream (TcpStream or TLS stream)
///
/// # Returns
///
/// A tuple of (ip_string, port) from the server response.
///
/// # Errors
///
/// Returns `TransportError` if the handshake fails.
pub async fn perform_handshake<S>(stream: &mut S) -> Result<(String, u16), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Send magic packet
    let magic_packet = generate_magic_packet();
    stream
        .write_all(&magic_packet)
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to send magic packet: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to flush magic packet: {e}")))?;

    // Read response
    let mut response = [0u8; EXA_RESPONSE_PACKET_SIZE];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to read response packet: {e}")))?;

    parse_response_packet(&response)
}

/// TLS certificate and key pair for secure connections.
#[derive(Clone)]
pub struct TlsCertificate {
    /// DER-encoded certificate
    pub certificate_der: Vec<u8>,
    /// DER-encoded private key
    pub private_key_der: Vec<u8>,
    /// DER-encoded public key (SubjectPublicKeyInfo format)
    pub public_key_der: Vec<u8>,
    /// SHA-256 fingerprint in Exasol format: sha256//<base64>
    pub fingerprint: String,
}

impl TlsCertificate {
    /// Generates a new self-signed TLS certificate.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::TlsError` if certificate generation fails.
    pub fn generate() -> Result<Self, TransportError> {
        // Generate a new key pair
        let key_pair = KeyPair::generate()
            .map_err(|e| TransportError::TlsError(format!("Failed to generate key pair: {e}")))?;

        // Create self-signed certificate with default parameters
        // rcgen uses sensible defaults for validity period
        let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).map_err(|e| {
            TransportError::TlsError(format!("Failed to create certificate params: {e}"))
        })?;

        // Generate the certificate
        let cert = params.self_signed(&key_pair).map_err(|e| {
            TransportError::TlsError(format!("Failed to generate certificate: {e}"))
        })?;

        let certified_key = CertifiedKey { cert, key_pair };
        let certificate_der = certified_key.cert.der().to_vec();
        let private_key_der = certified_key.key_pair.serialize_der();
        let public_key_der = certified_key.key_pair.public_key_der().to_vec();

        // Compute SHA-256 fingerprint from public key in Exasol format
        let fingerprint = compute_public_key_fingerprint(&public_key_der);

        Ok(Self {
            certificate_der,
            private_key_der,
            public_key_der,
            fingerprint,
        })
    }

    /// Creates a rustls `ServerConfig` from this certificate.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::TlsError` if configuration fails.
    pub fn to_server_config(&self) -> Result<ServerConfig, TransportError> {
        let cert = CertificateDer::from(self.certificate_der.clone());
        let key = PrivateKeyDer::try_from(self.private_key_der.clone())
            .map_err(|e| TransportError::TlsError(format!("Failed to parse private key: {e}")))?;

        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .map_err(|e| TransportError::TlsError(format!("Failed to create TLS config: {e}")))
    }

    /// Returns a rustls `ClientConfig` with an empty root store and no client certificate.
    ///
    /// The HTTP tunnel does not use it, because the driver is the TLS server of the tunnel.
    /// It stays for API compatibility.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::TlsError` if configuration fails.
    pub fn to_client_config(&self) -> Result<ClientConfig, TransportError> {
        let root_store = RootCertStore::empty();
        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(config)
    }
}

/// A tunnel byte stream that can carry HTTP: plain TCP, or TLS after the handshake.
trait TunnelIo: AsyncRead + AsyncWrite + Unpin + Send + Sync {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> TunnelIo for T {}

/// The TLS server side of a tunnel whose handshake has not started yet.
struct PendingTlsHandshake {
    tcp_stream: TcpStream,
    acceptor: TlsAcceptor,
    bound: Duration,
}

impl PendingTlsHandshake {
    /// Accepts the handshake that Exasol starts as the TLS client, within a
    /// bound that starts now and includes the wait for Exasol's first byte.
    async fn complete(self) -> Result<server::TlsStream<TcpStream>, TransportError> {
        let Self {
            tcp_stream,
            acceptor,
            bound,
        } = self;
        SetupDeadline::start("HTTP tunnel setup", bound)
            .run(SetupStep::TlsHandshake, async {
                acceptor
                    .accept(tcp_stream)
                    .await
                    .map_err(|e| TransportError::TlsError(format!("TLS handshake failed: {e}")))
            })
            .await
    }
}

enum TunnelStream {
    Open(Box<dyn TunnelIo>),
    TlsHandshakePending(PendingTlsHandshake),
    /// The TLS handshake failed, was abandoned, or was skipped by `shutdown`,
    /// and the socket is closed.
    Closed,
}

/// HTTP Transport Client for connecting TO Exasol.
///
/// This client connects to Exasol's data node and performs the EXA tunneling
/// handshake to establish a data transfer channel. Unlike the server mode,
/// this works through firewalls and NAT because it only makes outbound connections.
///
/// With TLS, the client is the TLS server of the tunnel. Exasol starts the TLS
/// handshake as the client once it receives the IMPORT or EXPORT statement that
/// names the tunnel, so the client completes the handshake inside its first
/// tunnel read or write. That read or write may begin before the statement is
/// dispatched: as the TLS server, the client sends no TLS byte until Exasol's
/// first handshake message arrives. A tunnel whose TLS handshake failed stays
/// failed, and every later read or write returns an error at once.
pub struct HttpTransportClient {
    stream: TunnelStream,
    /// Internal address from handshake response (format: "host:port")
    internal_addr: String,
    /// TLS certificate (for fingerprint extraction), if TLS is enabled
    tls_certificate: Option<TlsCertificate>,
}

impl HttpTransportClient {
    /// Connects to Exasol and performs the EXA tunneling handshake, with
    /// 30-second setup bounds.
    ///
    /// One 30-second deadline bounds the TCP connect and the EXA handshake
    /// together. With TLS, a separate 30-second bound covers the TLS handshake,
    /// which runs inside the first tunnel read or write. Neither bound follows
    /// the connection timeout of any `Connection`. A stalled step fails with
    /// `HTTP tunnel setup timeout after 30000ms (<step>)`. Use
    /// [`connect_with_timeout`](Self::connect_with_timeout) to choose the bounds.
    ///
    /// # Arguments
    ///
    /// * `host` - The Exasol host to connect to
    /// * `port` - The port to connect to (same as WebSocket port)
    /// * `use_tls` - Whether to use TLS encryption for data transfer
    ///
    /// # Returns
    ///
    /// A connected `HttpTransportClient` ready for data transfer. With TLS, its
    /// TLS handshake is still pending.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if the TCP connect or the EXA handshake fails or
    /// does not finish within 30 seconds, or if the TLS certificate cannot be
    /// generated. TLS handshake errors come from the first tunnel read or write.
    pub async fn connect(host: &str, port: u16, use_tls: bool) -> Result<Self, TransportError> {
        Self::connect_with_timeout(host, port, use_tls, TUNNEL_SETUP_TIMEOUT).await
    }

    /// Connects to Exasol and performs the EXA tunneling handshake, with
    /// setup bounds of `setup_timeout`.
    ///
    /// # Protocol Flow
    ///
    /// 1. Connect TCP to Exasol (unencrypted).
    /// 2. Send the magic packet over plain TCP.
    /// 3. Receive the response packet with the internal address over plain TCP.
    ///    Without TLS, the tunnel is ready and this method returns.
    /// 4. With TLS, generate an ad-hoc certificate for the `PUBLIC KEY` clause
    ///    and return with the TLS handshake pending. The client is the TLS
    ///    server of the tunnel and sends no TLS byte on its own.
    /// 5. The caller sends the IMPORT or EXPORT statement and serves the tunnel
    ///    concurrently. Exasol starts the TLS handshake as the client after it
    ///    receives the statement, and the first tunnel read or write completes
    ///    it. That read or write can begin before the statement is dispatched.
    /// 6. HTTP data flows, over TLS when it is enabled.
    ///
    /// One deadline of `setup_timeout` bounds steps 1 to 3 together: time spent
    /// in one step is not granted again to the next. `CsvExportOptions::timeout_ms`
    /// does not cover them. When that deadline passes, this method fails with
    /// `HTTP tunnel setup timeout after <ms>ms (<step>)`, where the step is
    /// `TCP connect` or `EXA handshake`, and the socket is released.
    ///
    /// With TLS, the handshake of step 5 gets a separate bound of
    /// `setup_timeout`, however long steps 1 to 3 took. That bound starts when
    /// the first tunnel read or write begins, so it can include statement
    /// dispatch, and it includes the wait for Exasol's first TLS byte. When it
    /// passes, that read or write fails with `HTTP tunnel setup timeout after
    /// <ms>ms (TLS handshake)`, and the socket is released.
    ///
    /// # Arguments
    ///
    /// * `host` - The Exasol host to connect to
    /// * `port` - The port to connect to (same as WebSocket port)
    /// * `use_tls` - Whether to use TLS encryption for data transfer
    /// * `setup_timeout` - The bound on steps 1 to 3, and the separate bound on
    ///   the TLS handshake
    ///
    /// # Returns
    ///
    /// A connected `HttpTransportClient` ready for data transfer. With TLS, its
    /// TLS handshake is still pending.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if the TCP connect or the EXA handshake fails or
    /// does not finish within `setup_timeout`, or if the TLS certificate cannot
    /// be generated. TLS handshake errors come from the first tunnel read or write.
    pub async fn connect_with_timeout(
        host: &str,
        port: u16,
        use_tls: bool,
        setup_timeout: Duration,
    ) -> Result<Self, TransportError> {
        let addr = format!("{host}:{port}");
        let deadline = SetupDeadline::start("HTTP tunnel setup", setup_timeout);

        let mut tcp_stream = deadline
            .run(SetupStep::TcpConnect, async {
                TcpStream::connect(&addr).await.map_err(|e| {
                    TransportError::IoError(format!("Failed to connect to {addr}: {e}"))
                })
            })
            .await?;

        // The magic packet handshake must happen over plain TCP, before TLS wrapping.
        let (ip, internal_port) = deadline
            .run(SetupStep::ExaHandshake, perform_handshake(&mut tcp_stream))
            .await?;
        let internal_addr = format!("{ip}:{internal_port}");

        if !use_tls {
            return Ok(Self {
                stream: TunnelStream::Open(Box::new(tcp_stream)),
                internal_addr,
                tls_certificate: None,
            });
        }

        let cert = TlsCertificate::generate()?;
        let acceptor = TlsAcceptor::from(Arc::new(cert.to_server_config()?));

        Ok(Self {
            stream: TunnelStream::TlsHandshakePending(PendingTlsHandshake {
                tcp_stream,
                acceptor,
                bound: setup_timeout,
            }),
            internal_addr,
            tls_certificate: Some(cert),
        })
    }

    /// Returns the internal address to use in IMPORT/EXPORT SQL statements.
    ///
    /// The format is "host:port" (e.g., "10.0.0.5:8563").
    #[must_use]
    pub fn internal_address(&self) -> &str {
        &self.internal_addr
    }

    /// Returns the public key fingerprint for TLS connections.
    ///
    /// This fingerprint should be included in the IMPORT/EXPORT SQL statement
    /// when using TLS (PUBLIC KEY clause).
    ///
    /// # Returns
    ///
    /// The SHA-256 fingerprint in base64 format, or `None` if TLS is not enabled.
    #[must_use]
    pub fn public_key_fingerprint(&self) -> Option<&str> {
        self.tls_certificate
            .as_ref()
            .map(|c| c.fingerprint.as_str())
    }

    /// Writes data to Exasol (for IMPORT operations).
    ///
    /// # Arguments
    ///
    /// * `data` - The data to write
    ///
    /// # Errors
    ///
    /// Returns `TransportError::IoError` if writing fails.
    ///
    /// On a TLS tunnel, the first read or write completes the TLS handshake and returns
    /// `TransportError::TlsError` if it fails, or `TransportError::IoError` with
    /// `HTTP tunnel setup timeout after <ms>ms (TLS handshake)` if the bound elapses.
    /// After a failed handshake, every call returns `TransportError::IoError` at once.
    pub async fn write(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let stream = self.open_stream().await?;
        stream
            .write_all(data)
            .await
            .map_err(|e| TransportError::IoError(format!("Failed to write data: {e}")))?;
        stream
            .flush()
            .await
            .map_err(|e| TransportError::IoError(format!("Failed to flush data: {e}")))
    }

    /// Reads data from Exasol (for EXPORT operations).
    ///
    /// # Arguments
    ///
    /// * `buf` - The buffer to read into
    ///
    /// # Returns
    ///
    /// The number of bytes read.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::IoError` if reading fails.
    ///
    /// On a TLS tunnel, the first read or write completes the TLS handshake and returns
    /// `TransportError::TlsError` if it fails, or `TransportError::IoError` with
    /// `HTTP tunnel setup timeout after <ms>ms (TLS handshake)` if the bound elapses.
    /// After a failed handshake, every call returns `TransportError::IoError` at once.
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        self.open_stream()
            .await?
            .read(buf)
            .await
            .map_err(|e| TransportError::IoError(format!("Failed to read data: {e}")))
    }

    /// Shuts down the connection cleanly.
    ///
    /// A tunnel whose TLS handshake has not started is shut down over plain
    /// TCP without a handshake, and it stays closed.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::IoError` if shutdown fails.
    pub async fn shutdown(&mut self) -> Result<(), TransportError> {
        let result = match &mut self.stream {
            TunnelStream::Open(stream) => stream.shutdown().await,
            TunnelStream::TlsHandshakePending(pending) => {
                let result = pending.tcp_stream.shutdown().await;
                self.stream = TunnelStream::Closed;
                result
            }
            TunnelStream::Closed => Ok(()),
        };
        result.map_err(|e| TransportError::IoError(format!("Failed to shutdown connection: {e}")))
    }

    /// Returns the stream that carries HTTP, completing a pending TLS handshake
    /// first. The tunnel is marked closed while the handshake runs, so a failed
    /// or abandoned handshake leaves it closed and a later call fails at once
    /// instead of reusing the consumed socket.
    async fn open_stream(&mut self) -> Result<&mut Box<dyn TunnelIo>, TransportError> {
        self.stream = match std::mem::replace(&mut self.stream, TunnelStream::Closed) {
            TunnelStream::TlsHandshakePending(pending) => {
                TunnelStream::Open(Box::new(pending.complete().await?))
            }
            state => state,
        };
        match &mut self.stream {
            TunnelStream::Open(stream) => Ok(stream),
            TunnelStream::TlsHandshakePending(_) | TunnelStream::Closed => {
                Err(TransportError::IoError(
                    "HTTP tunnel is closed: its TLS handshake failed or never ran".to_string(),
                ))
            }
        }
    }

    /// Reads and parses an HTTP request from the stream.
    ///
    /// This is used when Exasol initiates data transfer:
    /// - For IMPORT: Exasol sends GET request to fetch data from client
    /// - For EXPORT: Exasol sends PUT request to send data to client
    ///
    /// # Returns
    ///
    /// The parsed HTTP request with method, path, and headers.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if reading or parsing fails.
    pub async fn read_http_request(&mut self) -> Result<HttpRequest, TransportError> {
        parse_http_request(self.open_stream().await?).await
    }

    /// Reads the body of a chunked transfer encoded request up to its
    /// zero-length terminator and forwards it to `sink` in pieces of at most
    /// 64 KiB, in tunnel order, as it reads them.
    ///
    /// The body never sits in memory as a whole: the next piece is read only
    /// after `sink` accepted the previous one, so a full channel stops the
    /// tunnel read and the server waits. A closed `sink` does not stop the read:
    /// the rest of the body is read and discarded, so the request can still be
    /// answered and the EXPORT statement can finish.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if reading fails or chunked encoding is invalid.
    pub async fn read_chunked_body(
        &mut self,
        sink: &mpsc::Sender<Vec<u8>>,
    ) -> Result<(), TransportError> {
        forward_chunked_body(self.open_stream().await?, sink).await
    }

    /// Writes an HTTP response with headers and optional body.
    ///
    /// # Arguments
    ///
    /// * `status_code` - The HTTP status code (e.g., 200)
    /// * `status_text` - The status text (e.g., "OK")
    /// * `headers` - Additional headers as (name, value) pairs
    /// * `body` - Optional body content
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if writing fails.
    pub async fn write_http_response(
        &mut self,
        status_code: u16,
        status_text: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<(), TransportError> {
        let response = build_http_response(status_code, status_text, headers, body);
        self.write(&response).await
    }

    /// Writes the HTTP response headers for a chunked transfer encoded response.
    ///
    /// This should be called before writing chunked body data.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if writing fails.
    pub async fn write_chunked_response_headers(&mut self) -> Result<(), TransportError> {
        let headers = build_chunked_response_headers();
        self.write(&headers).await
    }

    /// Writes a chunk of data using chunked transfer encoding.
    ///
    /// The data is automatically encoded with the chunk size prefix and CRLF suffix.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to write as a chunk
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if writing fails.
    pub async fn write_chunked_body(&mut self, data: &[u8]) -> Result<(), TransportError> {
        if data.is_empty() {
            return Ok(());
        }
        let chunk = encode_chunk(data);
        self.write(&chunk).await
    }

    /// Writes the final empty chunk to signal end of chunked transfer.
    ///
    /// This must be called after writing all data chunks to properly
    /// terminate the chunked transfer encoding.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if writing fails.
    pub async fn write_final_chunk(&mut self) -> Result<(), TransportError> {
        // encode_chunk with empty data produces "0\r\n\r\n"
        let final_chunk = encode_chunk(&[]);
        self.write(&final_chunk).await
    }

    /// Handles an IMPORT request from Exasol.
    ///
    /// This method implements the correct IMPORT protocol flow:
    /// 1. Wait for HTTP GET request from Exasol
    /// 2. Verify it's a GET request
    /// 3. Send HTTP response headers with chunked encoding
    /// 4. Return, allowing caller to write data chunks
    ///
    /// After calling this method, use `write_chunked_body()` to send data,
    /// then `write_final_chunk()` to complete the transfer.
    ///
    /// # Returns
    ///
    /// The parsed HTTP GET request from Exasol.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if the request is not a GET or if I/O fails.
    pub async fn handle_import_request(&mut self) -> Result<HttpRequest, TransportError> {
        let request = self.read_http_request().await?;

        if request.method != HttpMethod::Get {
            return Err(TransportError::ProtocolError(format!(
                "Expected GET request for IMPORT, got {}",
                request.method
            )));
        }

        // Send response headers - data will be written by caller
        self.write_chunked_response_headers().await?;

        Ok(request)
    }

    /// Serves a Parquet file to Exasol via HTTP range requests.
    ///
    /// On Exasol 2025.1.11+ servers, native `IMPORT ... FROM PARQUET` pulls
    /// the file from the driver using HTTP range requests instead of the
    /// chunked-body push used for CSV. After the EXA tunneling handshake,
    /// Exasol typically issues:
    ///
    /// 1. A `HEAD /<file>.parquet` to learn the file size; the driver
    ///    responds with `200 OK` + `Content-Length` and no body.
    /// 2. One or more `GET /<file>.parquet` requests carrying a
    ///    `Range: bytes=<start>-<end>` header (footer first, then row
    ///    groups); the driver responds with `206 Partial Content` and the
    ///    requested byte slice.
    /// 3. Optionally a `GET` without a `Range` header, in which case the
    ///    driver responds with `200 OK` + `Content-Length` and the full
    ///    body (rare but legal).
    ///
    /// With TLS, the TLS handshake completes before the first request is read.
    /// The method then loops until the peer closes the connection (signalled
    /// by an `IoError` or `ProtocolError` from `read_http_request`), at which
    /// point it returns `Ok(())`. Any unexpected method (e.g. `PUT`) is
    /// rejected with `405 Method Not Allowed` and yields a
    /// `TransportError::ProtocolError`. A malformed `Range` header yields
    /// a `400 Bad Request` and the loop continues.
    ///
    /// # Arguments
    ///
    /// * `file_bytes` - The complete Parquet file bytes to serve.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` only on unrecoverable conditions: a failed TLS
    /// handshake, an unexpected method, or a write failure. Connection close
    /// after the handshake is treated as successful completion.
    pub async fn handle_parquet_import_requests(
        &mut self,
        file_bytes: &[u8],
    ) -> Result<(), TransportError> {
        serve_parquet_range_requests(self.open_stream().await?, file_bytes).await
    }

    /// Handles the EXPORT request from Exasol: reads the HTTP PUT, forwards
    /// its body to `sink` in pieces of at most 64 KiB while Exasol sends it,
    /// and answers HTTP 200 OK after the last piece.
    ///
    /// The body may be chunked or carry a Content-Length. Exasol returns the
    /// EXPORT statement only after it received the 200 OK, so the caller must
    /// drain `sink` while this runs: a full channel stops the tunnel read. A
    /// closed `sink` makes this read and discard the rest of the body and still
    /// answer 200 OK, so the EXPORT statement finishes.
    ///
    /// # Returns
    ///
    /// The parsed HTTP PUT request.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if the request is not a PUT, if it has neither
    /// a Content-Length nor chunked encoding, or if I/O fails.
    pub async fn handle_export_request(
        &mut self,
        sink: &mpsc::Sender<Vec<u8>>,
    ) -> Result<HttpRequest, TransportError> {
        let request = self.read_http_request().await?;

        if request.method != HttpMethod::Put {
            return Err(TransportError::ProtocolError(format!(
                "Expected PUT request for EXPORT, got {}",
                request.method
            )));
        }

        if request.is_chunked() {
            self.read_chunked_body(sink).await?;
        } else if let Some(content_length) = request.content_length() {
            forward_sized_body(self.open_stream().await?, content_length, sink).await?;
        } else {
            return Err(TransportError::ProtocolError(
                "PUT request has no Content-Length or Transfer-Encoding".to_string(),
            ));
        }

        self.write_http_response(200, "OK", &[], None).await?;

        Ok(request)
    }
}

/// Computes the SHA-256 fingerprint of data in base64 format.
///
/// This is a low-level function. For Exasol's PUBLIC KEY clause,
/// use `compute_public_key_fingerprint` instead which returns the
/// correct `sha256//<base64>` format.
#[must_use]
pub fn compute_sha256_fingerprint(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();
    BASE64_STANDARD.encode(hash)
}

/// Computes the public key fingerprint in Exasol format.
///
/// The fingerprint is computed from the DER-encoded public key (SubjectPublicKeyInfo)
/// and returned in the format `sha256//<base64>` as required by Exasol's PUBLIC KEY clause.
///
/// # Arguments
///
/// * `public_key_der` - The DER-encoded public key (SubjectPublicKeyInfo format)
///
/// # Returns
///
/// The fingerprint in format `sha256//<base64>`
#[must_use]
pub fn compute_public_key_fingerprint(public_key_der: &[u8]) -> String {
    let base64_hash = compute_sha256_fingerprint(public_key_der);
    format!("sha256//{base64_hash}")
}

/// HTTP transport data pipe for streaming data.
///
/// This provides a channel-based abstraction for reading and writing data
/// through the HTTP transport connection.
pub struct DataPipe {
    /// Sender for writing data to the transport
    tx: mpsc::Sender<Vec<u8>>,
    /// Receiver for reading data from the transport
    rx: mpsc::Receiver<Vec<u8>>,
}

impl DataPipe {
    /// Creates a new data pipe pair for bidirectional communication.
    ///
    /// # Arguments
    ///
    /// * `buffer_size` - The size of the channel buffer
    ///
    /// # Returns
    ///
    /// A tuple of (writer_pipe, reader_pipe) for the two ends of the pipe.
    #[must_use]
    pub fn create_pair(buffer_size: usize) -> (Self, Self) {
        let (tx1, rx1) = mpsc::channel(buffer_size);
        let (tx2, rx2) = mpsc::channel(buffer_size);

        let writer = DataPipe { tx: tx1, rx: rx2 };
        let reader = DataPipe { tx: tx2, rx: rx1 };

        (writer, reader)
    }

    /// Sends data through the pipe.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to send
    ///
    /// # Errors
    ///
    /// Returns `TransportError::SendError` if the channel is closed.
    pub async fn send(&self, data: Vec<u8>) -> Result<(), TransportError> {
        self.tx.send(data).await.map_err(|e| {
            TransportError::SendError(format!("Failed to send data through pipe: {e}"))
        })
    }

    /// Receives data from the pipe.
    ///
    /// # Returns
    ///
    /// `Some(data)` if data is available, `None` if the channel is closed.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.rx.recv().await
    }
}

/// Encodes data using HTTP chunked transfer encoding.
///
/// # Arguments
///
/// * `data` - The data to encode
///
/// # Returns
///
/// The chunked-encoded data as bytes.
#[must_use]
pub fn encode_chunk(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        // Final chunk
        b"0\r\n\r\n".to_vec()
    } else {
        let size_hex = format!("{:X}\r\n", data.len());
        let mut result = Vec::with_capacity(size_hex.len() + data.len() + 2);
        result.extend_from_slice(size_hex.as_bytes());
        result.extend_from_slice(data);
        result.extend_from_slice(b"\r\n");
        result
    }
}

/// Decodes a chunk from HTTP chunked transfer encoding.
///
/// This function expects the chunk size line (without CRLF) as input
/// and returns the expected chunk size.
///
/// # Arguments
///
/// * `size_line` - The chunk size line (hex string)
///
/// # Returns
///
/// The chunk size as usize.
///
/// # Errors
///
/// Returns `TransportError::ProtocolError` if the size line is invalid.
pub fn parse_chunk_size(size_line: &str) -> Result<usize, TransportError> {
    // Remove any chunk extension (after semicolon)
    let size_str = size_line.split(';').next().unwrap_or(size_line).trim();

    usize::from_str_radix(size_str, 16)
        .map_err(|e| TransportError::ProtocolError(format!("Invalid chunk size '{size_str}': {e}")))
}

/// Reads a line (terminated by CRLF) from the stream.
///
/// This is used for parsing HTTP headers and chunk size lines.
///
/// # Arguments
///
/// * `stream` - The stream to read from
///
/// # Returns
///
/// The line as a string (without CRLF).
///
/// # Errors
///
/// Returns `io::Error` if reading fails.
pub async fn read_line<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<String> {
    let mut line = Vec::new();
    let mut buf = [0u8; 1];

    loop {
        stream.read_exact(&mut buf).await?;
        if buf[0] == b'\r' {
            // Read the expected \n
            stream.read_exact(&mut buf).await?;
            if buf[0] == b'\n' {
                break;
            }
            // If not \n, include both bytes
            line.push(b'\r');
            line.push(buf[0]);
        } else {
            line.push(buf[0]);
        }
    }

    String::from_utf8(line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Forwards a chunked transfer encoded body from `stream` to `sink` up to its
/// zero-length terminator, as [`HttpTransportClient::read_chunked_body`]
/// describes. Each chunk goes through [`forward_sized_body`], so a declared
/// chunk size never sets the size of an allocation.
async fn forward_chunked_body<S: AsyncRead + Unpin>(
    stream: &mut S,
    sink: &mpsc::Sender<Vec<u8>>,
) -> Result<(), TransportError> {
    loop {
        let size_line = read_line(stream).await.map_err(|e| {
            TransportError::ProtocolError(format!("Failed to read chunk size: {e}"))
        })?;
        let chunk_size = parse_chunk_size(&size_line)?;
        forward_sized_body(stream, chunk_size, sink).await?;

        let mut crlf = [0u8; 2];
        stream.read_exact(&mut crlf).await.map_err(|e| {
            TransportError::ProtocolError(format!(
                "Failed to read the CRLF after a {chunk_size}-byte chunk: {e}"
            ))
        })?;
        if chunk_size == 0 {
            return Ok(());
        }
    }
}

/// Forwards the next `len` bytes of `stream` to `sink` in pieces of at most
/// [`EXPORT_PIECE_MAX_BYTES`], reading each piece only after `sink` accepted
/// the previous one. Once `sink` is closed, the remaining pieces are read and
/// discarded, so the caller can still answer the request.
async fn forward_sized_body<S: AsyncRead + Unpin>(
    stream: &mut S,
    len: usize,
    sink: &mpsc::Sender<Vec<u8>>,
) -> Result<(), TransportError> {
    let mut remaining = len;
    while remaining > 0 {
        let mut piece = vec![0u8; remaining.min(EXPORT_PIECE_MAX_BYTES)];
        stream.read_exact(&mut piece).await.map_err(|e| {
            TransportError::IoError(format!(
                "Failed to read body: {remaining} of {len} bytes still expected: {e}"
            ))
        })?;
        remaining -= piece.len();
        // A closed channel means the consumer stopped early; the body is still read to its end.
        let _ = sink.send(piece).await;
    }
    Ok(())
}

/// Serves a Parquet file over the given async stream using HTTP range
/// requests.
///
/// This is the underlying loop driving
/// [`HttpTransportClient::handle_parquet_import_requests`]. It is generic
/// over any `AsyncRead + AsyncWrite + Unpin` stream so that it can be
/// exercised in unit tests via `tokio::io::duplex` without spinning up a
/// real TCP/TLS connection.
///
/// See [`HttpTransportClient::handle_parquet_import_requests`] for the
/// full protocol description.
async fn serve_parquet_range_requests<S>(
    stream: &mut S,
    file_bytes: &[u8],
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let request = match parse_http_request(stream).await {
            Ok(req) => req,
            // A stream-level I/O error means the peer closed the connection
            // cleanly; a malformed/empty request line at this stage usually
            // signals the same. Either way, end the conversation.
            Err(TransportError::IoError(_) | TransportError::ProtocolError(_)) => return Ok(()),
            Err(e) => return Err(e),
        };

        match request.method {
            HttpMethod::Head => write_file_size_response(stream, file_bytes.len()).await?,
            HttpMethod::Get => match request.headers.get("range") {
                Some(range_header) => serve_byte_range(stream, file_bytes, range_header).await?,
                // Full GET without Range header — rare but legal.
                None => write_whole_file_response(stream, file_bytes).await?,
            },
            HttpMethod::Put => {
                let _ = stream
                    .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                    .await;
                let _ = stream.flush().await;
                return Err(TransportError::ProtocolError(
                    "Unexpected PUT in Parquet import handler".to_string(),
                ));
            }
        }
    }
}

/// Answers one `Range` request: `206 Partial Content` with the requested slice,
/// or `400 Bad Request` for any range this file cannot satisfy.
///
/// A rejected range is not an error for the caller — Exasol may retry with a
/// different range or just close the connection — so both outcomes return `Ok`.
async fn serve_byte_range<S>(
    stream: &mut S,
    file_bytes: &[u8],
    range_header: &str,
) -> Result<(), TransportError>
where
    S: AsyncWrite + Unpin,
{
    let Some((start, end)) = parse_byte_range(range_header) else {
        return write_bad_request_response(stream).await;
    };

    if file_bytes.is_empty() || start >= file_bytes.len() {
        return write_bad_request_response(stream).await;
    }

    let clamped_end = end.min(file_bytes.len().saturating_sub(1));
    if clamped_end < start {
        return write_bad_request_response(stream).await;
    }

    write_partial_content_response(stream, file_bytes, start, clamped_end).await
}

/// Parses a `Range: bytes=<start>-<end>` header value into inclusive bounds.
///
/// Returns `None` for anything malformed: a missing `bytes=` prefix, a missing
/// dash, or a non-numeric bound.
fn parse_byte_range(range_header: &str) -> Option<(usize, usize)> {
    let (start_str, end_str) = range_header.strip_prefix("bytes=")?.split_once('-')?;
    let start = start_str.trim().parse::<usize>().ok()?;
    let end = end_str.trim().parse::<usize>().ok()?;
    Some((start, end))
}

/// Writes `200 OK` with a `Content-Length` and no body, answering a `HEAD`
/// probe for the file size.
async fn write_file_size_response<S>(stream: &mut S, file_len: usize) -> Result<(), TransportError>
where
    S: AsyncWrite + Unpin,
{
    let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {file_len}\r\n\r\n");
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to write HEAD response: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to flush HEAD response: {e}")))
}

/// Writes `200 OK` followed by the complete file body.
async fn write_whole_file_response<S>(
    stream: &mut S,
    file_bytes: &[u8],
) -> Result<(), TransportError>
where
    S: AsyncWrite + Unpin,
{
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        file_bytes.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to write GET headers: {e}")))?;
    stream
        .write_all(file_bytes)
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to write GET body: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to flush GET response: {e}")))
}

/// Writes `206 Partial Content` followed by `file_bytes[start..=end]`.
async fn write_partial_content_response<S>(
    stream: &mut S,
    file_bytes: &[u8],
    start: usize,
    end: usize,
) -> Result<(), TransportError>
where
    S: AsyncWrite + Unpin,
{
    let slice = &file_bytes[start..=end];
    let response = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n",
        slice.len(),
        start,
        end,
        file_bytes.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to write 206 headers: {e}")))?;
    stream
        .write_all(slice)
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to write 206 body: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to flush 206 response: {e}")))
}

/// Writes a bare `400 Bad Request` with no body.
async fn write_bad_request_response<S>(stream: &mut S) -> Result<(), TransportError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to write 400: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| TransportError::IoError(format!("Failed to flush 400: {e}")))
}

/// HTTP method enum for parsed requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// HTTP GET method (used for IMPORT - Exasol requests data)
    Get,
    /// HTTP PUT method (used for EXPORT - Exasol sends data)
    Put,
    /// HTTP HEAD method (used for native Parquet IMPORT - Exasol probes file size)
    Head,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Put => write!(f, "PUT"),
            HttpMethod::Head => write!(f, "HEAD"),
        }
    }
}

/// Parsed HTTP request with method, path, and headers.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// The HTTP method (GET or PUT)
    pub method: HttpMethod,
    /// The request path (e.g., "/001.csv")
    pub path: String,
    /// HTTP headers as key-value pairs (keys are lowercase)
    pub headers: std::collections::HashMap<String, String>,
}

impl HttpRequest {
    /// Returns the Content-Length header value if present.
    #[must_use]
    pub fn content_length(&self) -> Option<usize> {
        self.headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
    }

    /// Returns true if the request uses chunked transfer encoding.
    #[must_use]
    pub fn is_chunked(&self) -> bool {
        self.headers
            .get("transfer-encoding")
            .map(|v| v.to_lowercase().contains("chunked"))
            .unwrap_or(false)
    }

    /// Returns the Host header value if present.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        self.headers.get("host").map(|s| s.as_str())
    }
}

/// Parses an HTTP request from a stream.
///
/// Reads the request line and headers. Does not read the body.
///
/// # Arguments
///
/// * `stream` - The stream to read from
///
/// # Returns
///
/// A parsed `HttpRequest` with method, path, and headers.
///
/// # Errors
///
/// Returns `TransportError::ProtocolError` if the request is malformed.
pub async fn parse_http_request<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<HttpRequest, TransportError> {
    // Read the request line (e.g., "GET /001.csv HTTP/1.1")
    let request_line = read_line(stream)
        .await
        .map_err(|e| TransportError::ProtocolError(format!("Failed to read request line: {e}")))?;

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(TransportError::ProtocolError(format!(
            "Invalid HTTP request line: '{request_line}'"
        )));
    }

    let method = match parts[0] {
        "GET" => HttpMethod::Get,
        "PUT" => HttpMethod::Put,
        "HEAD" => HttpMethod::Head,
        other => {
            return Err(TransportError::ProtocolError(format!(
                "Unsupported HTTP method: '{other}'"
            )))
        }
    };

    let path = parts[1].to_string();

    // Parse headers until empty line
    let mut headers = std::collections::HashMap::new();
    loop {
        let line = read_line(stream).await.map_err(|e| {
            TransportError::ProtocolError(format!("Failed to read header line: {e}"))
        })?;

        if line.is_empty() {
            break;
        }

        // Parse "Header-Name: value"
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }

    Ok(HttpRequest {
        method,
        path,
        headers,
    })
}

/// Builds an HTTP response with the given status, headers, and optional body.
///
/// # Arguments
///
/// * `status_code` - The HTTP status code (e.g., 200)
/// * `status_text` - The status text (e.g., "OK")
/// * `headers` - Additional headers as (name, value) pairs
/// * `body` - Optional body content
///
/// # Returns
///
/// The complete HTTP response as bytes.
#[must_use]
pub fn build_http_response(
    status_code: u16,
    status_text: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status_code} {status_text}\r\n");

    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }

    // If body is provided and no Content-Length is specified, add it
    if let Some(body_data) = body {
        if !headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        {
            response.push_str(&format!("Content-Length: {}\r\n", body_data.len()));
        }
    }

    response.push_str("\r\n");

    let mut result = response.into_bytes();
    if let Some(body_data) = body {
        result.extend_from_slice(body_data);
    }

    result
}

/// Builds an HTTP 200 OK response with chunked transfer encoding headers.
///
/// This is used for IMPORT operations where we stream data to Exasol.
///
/// # Returns
///
/// The HTTP response headers as bytes (no body included).
#[must_use]
pub fn build_chunked_response_headers() -> Vec<u8> {
    build_http_response(
        200,
        "OK",
        &[
            ("Content-Type", "application/octet-stream"),
            ("Transfer-Encoding", "chunked"),
        ],
        None,
    )
}

/// Builds a simple HTTP 200 OK response with no body.
///
/// This is used for EXPORT operations to acknowledge receipt of data.
///
/// # Returns
///
/// The complete HTTP response as bytes.
#[must_use]
pub fn build_ok_response() -> Vec<u8> {
    build_http_response(200, "OK", &[], None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::test_support::{
        exa_response_packet, finish_within, FakeExasolServer, SilentServer, DISCONNECT_BOUND,
        LOOPBACK_TEST_BOUND,
    };
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    const SETUP_BOUND: Duration = Duration::from_millis(300);

    #[test]
    fn test_generate_magic_packet() {
        let packet = generate_magic_packet();

        // Verify packet size
        assert_eq!(packet.len(), EXA_MAGIC_PACKET_SIZE);

        // Verify magic number (little-endian)
        let magic = u32::from_le_bytes([packet[0], packet[1], packet[2], packet[3]]);
        assert_eq!(magic, EXA_MAGIC_NUMBER);

        // Verify version
        let major = u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]);
        let minor = u32::from_le_bytes([packet[8], packet[9], packet[10], packet[11]]);
        assert_eq!(major, EXA_PROTOCOL_VERSION_MAJOR);
        assert_eq!(minor, EXA_PROTOCOL_VERSION_MINOR);
    }

    #[test]
    fn test_parse_magic_packet_valid() {
        let packet = generate_magic_packet();
        let result = parse_magic_packet(&packet);

        assert!(result.is_ok());
        let (magic, major, minor) = result.unwrap();
        assert_eq!(magic, EXA_MAGIC_NUMBER);
        assert_eq!(major, EXA_PROTOCOL_VERSION_MAJOR);
        assert_eq!(minor, EXA_PROTOCOL_VERSION_MINOR);
    }

    #[test]
    fn test_parse_magic_packet_too_short() {
        let packet = [0u8; 8]; // Too short
        let result = parse_magic_packet(&packet);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TransportError::ProtocolError(_)));
    }

    #[test]
    fn test_parse_magic_packet_invalid_magic() {
        let mut packet = generate_magic_packet();
        // Corrupt the magic number
        packet[0] = 0xFF;

        let result = parse_magic_packet(&packet);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TransportError::ProtocolError(_)));
    }

    #[test]
    fn test_parse_response_packet_valid() {
        // PyExasol format: struct.unpack("ii16s", data)
        // - Bytes 0-3: unused/reserved i32
        // - Bytes 4-7: port as i32 little-endian
        // - Bytes 8-23: IP address as 16-byte null-terminated string
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];

        // First i32 (unused) - set to 0
        packet[0..4].copy_from_slice(&0i32.to_le_bytes());

        // Port as i32 at bytes 4-7 (8563)
        packet[4..8].copy_from_slice(&8563i32.to_le_bytes());

        // IP address as null-terminated string (192.168.1.100)
        let ip_addr = b"192.168.1.100\0\0\0"; // 16 bytes total
        packet[8..24].copy_from_slice(ip_addr);

        let result = parse_response_packet(&packet);
        assert!(result.is_ok());
        let (addr, p) = result.unwrap();
        assert_eq!(addr, "192.168.1.100");
        assert_eq!(p, 8563);
    }

    #[test]
    fn test_parse_response_packet_ip_format() {
        // Test with IP 10.0.0.5 (common internal address)
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];

        // First i32 (unused)
        packet[0..4].copy_from_slice(&0i32.to_le_bytes());

        // Port as i32 at bytes 4-7 (8563)
        packet[4..8].copy_from_slice(&8563i32.to_le_bytes());

        // IP address as null-terminated string
        let ip_addr = b"10.0.0.5\0\0\0\0\0\0\0\0"; // 16 bytes total
        packet[8..24].copy_from_slice(ip_addr);

        let result = parse_response_packet(&packet);
        assert!(result.is_ok());
        let (addr, p) = result.unwrap();
        assert_eq!(addr, "10.0.0.5");
        assert_eq!(p, 8563);
    }

    #[test]
    fn test_parse_response_packet_pyexasol_format() {
        // PyExasol format: struct.unpack("ii16s", data)
        // - Bytes 0-3: unused/reserved i32
        // - Bytes 4-7: port as i32 little-endian
        // - Bytes 8-23: IP address as 16-byte null-terminated string
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];

        // First i32 (unused) - set to 0
        packet[0..4].copy_from_slice(&0i32.to_le_bytes());

        // Port as i32 at bytes 4-7 (8563)
        packet[4..8].copy_from_slice(&8563i32.to_le_bytes());

        // IP address as null-terminated string at bytes 8-23
        let ip_addr = b"10.0.0.5\0\0\0\0\0\0\0\0"; // 16 bytes total
        packet[8..24].copy_from_slice(ip_addr);

        let result = parse_response_packet(&packet);
        assert!(result.is_ok());
        let (addr, port) = result.unwrap();
        assert_eq!(addr, "10.0.0.5");
        assert_eq!(port, 8563);
    }

    #[test]
    fn test_parse_response_packet_with_longer_ip() {
        // Test with longer IP address like 192.168.100.123
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];

        // First i32 (unused)
        packet[0..4].copy_from_slice(&0i32.to_le_bytes());

        // Port as i32 at bytes 4-7
        packet[4..8].copy_from_slice(&12345i32.to_le_bytes());

        // IP address as null-terminated string
        let ip_addr = b"192.168.100.123\0"; // 16 bytes total
        packet[8..24].copy_from_slice(ip_addr);

        let result = parse_response_packet(&packet);
        assert!(result.is_ok());
        let (addr, port) = result.unwrap();
        assert_eq!(addr, "192.168.100.123");
        assert_eq!(port, 12345);
    }

    #[test]
    fn test_parse_response_packet_too_short() {
        let packet = [0u8; 16]; // Too short
        let result = parse_response_packet(&packet);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TransportError::ProtocolError(_)));
    }

    #[test]
    fn test_encode_chunk_with_data() {
        let data = b"Hello, World!";
        let chunk = encode_chunk(data);

        // Expected format: "D\r\nHello, World!\r\n" (D is hex for 13)
        let expected = b"D\r\nHello, World!\r\n";
        assert_eq!(chunk, expected);
    }

    #[test]
    fn test_encode_chunk_empty() {
        let data = b"";
        let chunk = encode_chunk(data);

        // Final chunk: "0\r\n\r\n"
        let expected = b"0\r\n\r\n";
        assert_eq!(chunk, expected);
    }

    #[test]
    fn test_encode_chunk_large() {
        let data = vec![0xAB; 256];
        let chunk = encode_chunk(&data);

        // 256 in hex is "100"
        assert!(chunk.starts_with(b"100\r\n"));
        assert!(chunk.ends_with(b"\r\n"));
        assert_eq!(chunk.len(), 5 + 256 + 2); // "100\r\n" + data + "\r\n"
    }

    #[test]
    fn test_parse_chunk_size_valid() {
        assert_eq!(parse_chunk_size("D").unwrap(), 13);
        assert_eq!(parse_chunk_size("100").unwrap(), 256);
        assert_eq!(parse_chunk_size("0").unwrap(), 0);
        assert_eq!(parse_chunk_size("FF").unwrap(), 255);
        assert_eq!(parse_chunk_size("ff").unwrap(), 255); // lowercase
    }

    #[test]
    fn test_parse_chunk_size_with_extension() {
        // Chunk extensions are separated by semicolon
        assert_eq!(parse_chunk_size("D;extension=value").unwrap(), 13);
        assert_eq!(parse_chunk_size("100;ext").unwrap(), 256);
    }

    #[test]
    fn test_parse_chunk_size_with_whitespace() {
        assert_eq!(parse_chunk_size("  D  ").unwrap(), 13);
        assert_eq!(parse_chunk_size("\t100\t").unwrap(), 256);
    }

    #[test]
    fn test_parse_chunk_size_invalid() {
        let result = parse_chunk_size("not_hex");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TransportError::ProtocolError(_)
        ));
    }

    /// A paused clock reaches it only once the forwarder is idle, so a
    /// forwarder still running after it is waiting on its channel.
    const STALL_WINDOW: Duration = Duration::from_millis(50);

    /// Counts the bytes read through it, so a test sees how far a forwarder
    /// has read its tunnel while the forwarder still borrows the stream.
    struct CountingReader<R> {
        inner: R,
        bytes_read: Arc<AtomicUsize>,
    }

    impl<R: AsyncRead + Unpin> AsyncRead for CountingReader<R> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let before = buf.filled().len();
            let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
            self.bytes_read
                .fetch_add(buf.filled().len() - before, Ordering::SeqCst);
            polled
        }
    }

    async fn received_pieces(mut receiver: mpsc::Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut pieces = Vec::new();
        while let Some(piece) = receiver.recv().await {
            pieces.push(piece);
        }
        pieces
    }

    fn numbered_bytes(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// Scenario: Export data flow (Exasol sends data to client)
    /// Scenario: Export body is forwarded in bounded pieces
    #[tokio::test]
    async fn test_forward_chunked_body_delivers_the_first_chunk_before_the_terminator_arrives() {
        let (mut peer, mut stream) = tokio::io::duplex(1024);
        let (sink, mut receiver) = mpsc::channel(16);
        peer.write_all(&encode_chunk(b"1,alice\n")).await.unwrap();

        let forwarding = forward_chunked_body(&mut stream, &sink);
        tokio::pin!(forwarding);
        let first = tokio::select! {
            piece = receiver.recv() => piece,
            result = &mut forwarding => panic!("the body ended before its terminator arrived: {result:?}"),
        };
        assert_eq!(first.as_deref(), Some(&b"1,alice\n"[..]));

        peer.write_all(&encode_chunk(b"2,bob\n")).await.unwrap();
        peer.write_all(&encode_chunk(&[])).await.unwrap();
        forwarding
            .await
            .expect("a complete chunked body must forward without error");
        assert_eq!(receiver.recv().await.as_deref(), Some(&b"2,bob\n"[..]));
        assert!(receiver.try_recv().is_err(), "the body holds two pieces");
    }

    /// Scenario: Export body is forwarded in bounded pieces
    #[tokio::test]
    async fn test_forward_chunked_body_splits_a_chunk_larger_than_64_kib_into_pieces() {
        let data = numbered_bytes(2 * 65_536 + 100);
        let body = [encode_chunk(&data), encode_chunk(&[])].concat();
        let (sink, receiver) = mpsc::channel(16);

        forward_chunked_body(&mut body.as_slice(), &sink)
            .await
            .expect("a complete chunked body must forward without error");
        drop(sink);

        let pieces = received_pieces(receiver).await;
        let sizes: Vec<usize> = pieces.iter().map(Vec::len).collect();
        assert_eq!(sizes, [65_536, 65_536, 100]);
        assert_eq!(pieces.concat(), data);
    }

    /// Scenario: Export data flow (Exasol sends data to client)
    /// Scenario: Export body is forwarded in bounded pieces
    #[tokio::test]
    async fn test_forward_sized_body_forwards_a_content_length_body_in_pieces() {
        let data = numbered_bytes(65_536 + 10);
        let tunnel_bytes = [data.as_slice(), b"NEXT"].concat();
        let mut stream = tunnel_bytes.as_slice();
        let (sink, receiver) = mpsc::channel(16);

        forward_sized_body(&mut stream, data.len(), &sink)
            .await
            .expect("a complete Content-Length body must forward without error");
        drop(sink);

        let pieces = received_pieces(receiver).await;
        let sizes: Vec<usize> = pieces.iter().map(Vec::len).collect();
        assert_eq!(sizes, [65_536, 10]);
        assert_eq!(pieces.concat(), data);
        assert_eq!(stream, b"NEXT", "the body ends at its Content-Length");
    }

    /// Scenario: Export applies backpressure to a slow consumer
    /// Scenario: Export body is forwarded in bounded pieces
    #[tokio::test(start_paused = true)]
    async fn test_forward_chunked_body_stops_reading_while_the_channel_is_full() {
        let chunk = encode_chunk(b"row\n");
        let body = [&chunk[..], &chunk, &chunk, &encode_chunk(&[])].concat();
        let bytes_read = Arc::new(AtomicUsize::new(0));
        let mut stream = CountingReader {
            inner: body.as_slice(),
            bytes_read: Arc::clone(&bytes_read),
        };
        let (sink, mut receiver) = mpsc::channel(1);
        let forwarding = forward_chunked_body(&mut stream, &sink);
        tokio::pin!(forwarding);
        let chunk_crlf = 2;

        assert!(tokio::time::timeout(STALL_WINDOW, &mut forwarding)
            .await
            .is_err());
        // One piece waits in the channel and one is read but not queued.
        assert_eq!(
            bytes_read.load(Ordering::SeqCst),
            2 * chunk.len() - chunk_crlf
        );

        assert_eq!(receiver.recv().await.as_deref(), Some(&b"row\n"[..]));
        assert!(tokio::time::timeout(STALL_WINDOW, &mut forwarding)
            .await
            .is_err());
        assert_eq!(
            bytes_read.load(Ordering::SeqCst),
            3 * chunk.len() - chunk_crlf
        );

        let (forwarded, rest) = tokio::join!(forwarding, async {
            [receiver.recv().await, receiver.recv().await]
        });
        forwarded.expect("a drained channel lets the forwarder finish the body");
        assert_eq!(rest, [Some(b"row\n".to_vec()), Some(b"row\n".to_vec())]);
        assert_eq!(bytes_read.load(Ordering::SeqCst), body.len());
    }

    /// Scenario: Export body is forwarded in bounded pieces
    #[tokio::test]
    async fn test_forward_chunked_body_discards_the_rest_of_the_body_when_the_receiver_is_dropped()
    {
        let rows: Vec<Vec<u8>> = (0..4).map(|i| format!("{i},row\n").into_bytes()).collect();
        let chunks: Vec<Vec<u8>> = rows.iter().map(|row| encode_chunk(row)).collect();
        let body = [chunks.concat(), encode_chunk(&[])].concat();
        let mut stream = body.as_slice();
        let (sink, mut receiver) = mpsc::channel(1);

        let (forwarded, first) = tokio::join!(forward_chunked_body(&mut stream, &sink), async {
            let first = receiver.recv().await;
            drop(receiver);
            first
        });

        forwarded.expect("a closed channel must not fail the body read");
        assert_eq!(first, Some(rows[0].clone()));
        assert!(stream.is_empty(), "the body must be read to its terminator");
    }

    #[tokio::test]
    async fn test_forward_chunked_body_fails_when_the_body_ends_inside_a_declared_chunk() {
        let body = b"FFFFFFFFFFFF\r\nonly a few bytes";
        let (sink, _receiver) = mpsc::channel(16);

        let error = forward_chunked_body(&mut body.as_slice(), &sink)
            .await
            .expect_err("a body that ends inside a chunk must fail");

        assert!(
            matches!(error, TransportError::IoError(_)),
            "expected an IoError, got {error:?}"
        );
    }

    #[test]
    fn test_tls_certificate_generation() {
        let result = TlsCertificate::generate();
        assert!(result.is_ok());

        let cert = result.unwrap();

        // Certificate should not be empty
        assert!(!cert.certificate_der.is_empty());
        assert!(!cert.private_key_der.is_empty());
        assert!(!cert.public_key_der.is_empty());

        // Fingerprint should be in format "sha256//<base64>"
        assert!(!cert.fingerprint.is_empty());
        assert!(
            cert.fingerprint.starts_with("sha256//"),
            "Fingerprint should start with 'sha256//': {}",
            cert.fingerprint
        );
        // sha256// prefix (8 chars) + base64 of SHA-256 (44 chars) = 52 chars
        assert_eq!(cert.fingerprint.len(), 52);
    }

    #[test]
    fn test_compute_sha256_fingerprint() {
        let data = b"test certificate data";
        let fingerprint = compute_sha256_fingerprint(data);

        // Fingerprint should be base64-encoded SHA-256
        assert!(!fingerprint.is_empty());
        assert_eq!(fingerprint.len(), 44); // 32 bytes -> 44 base64 chars

        // Verify it's valid base64
        assert!(BASE64_STANDARD.decode(&fingerprint).is_ok());
    }

    #[test]
    fn test_fingerprint_format() {
        // Fingerprint should be in format: sha256//<base64>
        let cert = TlsCertificate::generate().unwrap();
        let fingerprint = cert.fingerprint.clone();

        assert!(
            fingerprint.starts_with("sha256//"),
            "Fingerprint should start with 'sha256//': {}",
            fingerprint
        );

        // Extract the base64 part and verify it's valid
        let base64_part = fingerprint.strip_prefix("sha256//").unwrap();
        assert_eq!(base64_part.len(), 44); // 32 bytes -> 44 base64 chars
        assert!(BASE64_STANDARD.decode(base64_part).is_ok());
    }

    #[test]
    fn test_compute_sha256_fingerprint_consistency() {
        let data = b"consistent test data";

        let fp1 = compute_sha256_fingerprint(data);
        let fp2 = compute_sha256_fingerprint(data);

        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_compute_public_key_fingerprint() {
        let data = b"test public key data";
        let fingerprint = compute_public_key_fingerprint(data);

        // Should start with sha256// prefix
        assert!(fingerprint.starts_with("sha256//"));

        // Extract base64 part and verify
        let base64_part = fingerprint.strip_prefix("sha256//").unwrap();
        assert_eq!(base64_part.len(), 44); // 32 bytes -> 44 base64 chars
        assert!(BASE64_STANDARD.decode(base64_part).is_ok());
    }

    #[test]
    fn test_tls_certificate_public_key_stored() {
        let cert = TlsCertificate::generate().unwrap();

        // Public key DER should not be empty
        assert!(!cert.public_key_der.is_empty());

        // Fingerprint should be computed from public key
        let expected_fingerprint = compute_public_key_fingerprint(&cert.public_key_der);
        assert_eq!(cert.fingerprint, expected_fingerprint);
    }

    #[test]
    fn test_tls_certificate_to_server_config() {
        let cert = TlsCertificate::generate().unwrap();
        let config = cert.to_server_config();

        assert!(config.is_ok());
    }

    #[tokio::test]
    async fn test_data_pipe_send_recv() {
        let (writer, mut reader) = DataPipe::create_pair(10);

        let data = vec![1, 2, 3, 4, 5];
        writer.send(data.clone()).await.unwrap();

        let received = reader.recv().await;
        assert!(received.is_some());
        assert_eq!(received.unwrap(), data);
    }

    #[tokio::test]
    async fn test_data_pipe_multiple_messages() {
        let (writer, mut reader) = DataPipe::create_pair(10);

        for i in 0..5 {
            writer.send(vec![i]).await.unwrap();
        }

        for i in 0..5 {
            let received = reader.recv().await.unwrap();
            assert_eq!(received, vec![i]);
        }
    }

    #[tokio::test]
    async fn test_read_line() {
        use tokio::io::AsyncWriteExt;

        let data = b"Hello\r\nWorld\r\n";

        // We need a tokio-compatible reader
        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            server.write_all(data).await.unwrap();
        });

        let line1 = read_line(&mut client).await.unwrap();
        assert_eq!(line1, "Hello");

        let line2 = read_line(&mut client).await.unwrap();
        assert_eq!(line2, "World");
    }

    #[tokio::test]
    async fn test_parse_http_request_get() {
        use tokio::io::AsyncWriteExt;

        let request = b"GET /001.csv HTTP/1.1\r\nHost: 10.0.0.5:8563\r\n\r\n";

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server.write_all(request).await.unwrap();
        });

        let parsed = parse_http_request(&mut client).await.unwrap();
        assert_eq!(parsed.method, HttpMethod::Get);
        assert_eq!(parsed.path, "/001.csv");
        assert_eq!(parsed.host(), Some("10.0.0.5:8563"));
        assert!(!parsed.is_chunked());
        assert!(parsed.content_length().is_none());
    }

    #[tokio::test]
    async fn test_parse_http_request_put_chunked() {
        use tokio::io::AsyncWriteExt;

        let request = b"PUT /001.csv HTTP/1.1\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n";

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server.write_all(request).await.unwrap();
        });

        let parsed = parse_http_request(&mut client).await.unwrap();
        assert_eq!(parsed.method, HttpMethod::Put);
        assert_eq!(parsed.path, "/001.csv");
        assert!(parsed.is_chunked());
    }

    #[tokio::test]
    async fn test_parse_http_request_put_content_length() {
        use tokio::io::AsyncWriteExt;

        let request = b"PUT /data.csv HTTP/1.1\r\nContent-Length: 1024\r\n\r\n";

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server.write_all(request).await.unwrap();
        });

        let parsed = parse_http_request(&mut client).await.unwrap();
        assert_eq!(parsed.method, HttpMethod::Put);
        assert_eq!(parsed.content_length(), Some(1024));
        assert!(!parsed.is_chunked());
    }

    #[tokio::test]
    async fn test_parse_http_request_invalid_method() {
        use tokio::io::AsyncWriteExt;

        let request = b"POST /data HTTP/1.1\r\n\r\n";

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server.write_all(request).await.unwrap();
        });

        let result = parse_http_request(&mut client).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TransportError::ProtocolError(_)
        ));
    }

    #[test]
    fn test_build_http_response_simple() {
        let response = build_http_response(200, "OK", &[], None);
        assert_eq!(response, b"HTTP/1.1 200 OK\r\n\r\n");
    }

    #[test]
    fn test_build_http_response_with_headers() {
        let response = build_http_response(200, "OK", &[("Content-Type", "text/plain")], None);
        assert_eq!(
            response,
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n"
        );
    }

    #[test]
    fn test_build_http_response_with_body() {
        let response = build_http_response(200, "OK", &[], Some(b"Hello"));
        assert_eq!(
            response,
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello"
        );
    }

    #[test]
    fn test_build_chunked_response_headers() {
        let response = build_chunked_response_headers();
        let expected = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(response, expected);
    }

    #[test]
    fn test_build_ok_response() {
        let response = build_ok_response();
        assert_eq!(response, b"HTTP/1.1 200 OK\r\n\r\n");
    }

    #[test]
    fn test_http_method_display() {
        assert_eq!(HttpMethod::Get.to_string(), "GET");
        assert_eq!(HttpMethod::Put.to_string(), "PUT");
    }

    #[test]
    fn test_http_request_content_length() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("content-length".to_string(), "1024".to_string());

        let request = HttpRequest {
            method: HttpMethod::Put,
            path: "/data.csv".to_string(),
            headers,
        };

        assert_eq!(request.content_length(), Some(1024));
    }

    #[test]
    fn test_http_request_is_chunked() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("transfer-encoding".to_string(), "chunked".to_string());

        let request = HttpRequest {
            method: HttpMethod::Put,
            path: "/data.csv".to_string(),
            headers,
        };

        assert!(request.is_chunked());
    }

    #[test]
    fn test_http_request_host() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("host".to_string(), "10.0.0.5:8563".to_string());

        let request = HttpRequest {
            method: HttpMethod::Get,
            path: "/001.csv".to_string(),
            headers,
        };

        assert_eq!(request.host(), Some("10.0.0.5:8563"));
    }

    #[test]
    fn test_encode_chunk_roundtrip() {
        let data = b"Hello, World!";
        let encoded = encode_chunk(data);

        // Parse the chunk back
        let hex_end = encoded.iter().position(|&b| b == b'\r').unwrap();
        let size_str = std::str::from_utf8(&encoded[..hex_end]).unwrap();
        let size = usize::from_str_radix(size_str, 16).unwrap();

        assert_eq!(size, data.len());

        // Verify data is present after "size\r\n"
        let data_start = hex_end + 2;
        let data_end = data_start + size;
        assert_eq!(&encoded[data_start..data_end], data);
    }

    #[test]
    fn test_http_transport_client_internal_address_format() {
        // Test that internal address is formatted as "host:port"
        // This test verifies the format_internal_address helper
        let ip = "10.0.0.5";
        let port: u16 = 8563;
        let formatted = format!("{}:{}", ip, port);
        assert_eq!(formatted, "10.0.0.5:8563");
    }

    #[test]
    fn test_tls_certificate_to_client_config() {
        let cert = TlsCertificate::generate().unwrap();
        let config = cert.to_client_config();

        assert!(config.is_ok());
    }

    #[test]
    fn test_http_method_display_head() {
        assert_eq!(HttpMethod::Head.to_string(), "HEAD");
    }

    #[tokio::test]
    async fn test_parse_http_request_head() {
        use tokio::io::AsyncWriteExt;

        let request = b"HEAD /001.parquet HTTP/1.1\r\nHost: 10.0.0.5:8563\r\n\r\n";

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server.write_all(request).await.unwrap();
        });

        let parsed = parse_http_request(&mut client).await.unwrap();
        assert_eq!(parsed.method, HttpMethod::Head);
        assert_eq!(parsed.path, "/001.parquet");
    }

    #[tokio::test]
    async fn test_handle_parquet_import_requests_serves_head_and_range() {
        // Script: HEAD probe → GET Range bytes=0-3 → GET Range bytes=4-7
        // → GET (no Range, full body) → close. Assert each response shape
        // and body slice match the expected bytes from a known file payload.
        use tokio::io::AsyncWriteExt;

        // 16 bytes of distinguishable content; large enough for two
        // non-overlapping range slices and one full-file response.
        let file_bytes: Vec<u8> = (0u8..16u8).collect();
        let file_len = file_bytes.len();

        // `server_side` is what the handler sees; `client_side` is the
        // scripted Exasol mock that sends requests and reads responses.
        let (mut server_side, mut client_side) = tokio::io::duplex(4096);

        let file_bytes_for_handler = file_bytes.clone();
        let handler = tokio::spawn(async move {
            serve_parquet_range_requests(&mut server_side, &file_bytes_for_handler).await
        });

        // 1. HEAD request — expect "HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\n"
        client_side
            .write_all(b"HEAD /001.parquet HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        client_side.flush().await.unwrap();

        let head_response = read_exactly(&mut client_side, 39).await;
        let head_str = std::str::from_utf8(&head_response).unwrap();
        assert!(
            head_str.starts_with("HTTP/1.1 200 OK\r\n"),
            "HEAD response should start with 200 OK: {:?}",
            head_str
        );
        assert!(
            head_str.contains(&format!("Content-Length: {file_len}\r\n")),
            "HEAD response should advertise Content-Length: {file_len}"
        );
        assert!(head_str.ends_with("\r\n\r\n"));

        // 2. GET with Range bytes=0-3 — expect 206 Partial Content + 4 bytes
        client_side
            .write_all(b"GET /001.parquet HTTP/1.1\r\nHost: x\r\nRange: bytes=0-3\r\n\r\n")
            .await
            .unwrap();
        client_side.flush().await.unwrap();

        let (status_line_1, body_1) = read_response_with_body(&mut client_side, 4).await;
        assert!(
            status_line_1.starts_with("HTTP/1.1 206 Partial Content\r\n"),
            "expected 206 Partial Content, got {:?}",
            status_line_1
        );
        assert!(
            status_line_1.contains("Content-Length: 4\r\n"),
            "expected Content-Length: 4: {:?}",
            status_line_1
        );
        assert!(
            status_line_1.contains(&format!("Content-Range: bytes 0-3/{file_len}\r\n")),
            "expected Content-Range: bytes 0-3/{file_len}: {:?}",
            status_line_1
        );
        assert_eq!(body_1, file_bytes[0..=3], "body slice for 0-3 mismatch");

        // 3. GET with Range bytes=4-7 — expect 206 Partial Content + 4 bytes
        client_side
            .write_all(b"GET /001.parquet HTTP/1.1\r\nHost: x\r\nRange: bytes=4-7\r\n\r\n")
            .await
            .unwrap();
        client_side.flush().await.unwrap();

        let (status_line_2, body_2) = read_response_with_body(&mut client_side, 4).await;
        assert!(
            status_line_2.contains("Content-Range: bytes 4-7/16\r\n"),
            "expected Content-Range: bytes 4-7/16: {:?}",
            status_line_2
        );
        assert_eq!(body_2, file_bytes[4..=7], "body slice for 4-7 mismatch");

        // 4. GET without Range header — expect full 200 OK + 16 bytes
        client_side
            .write_all(b"GET /001.parquet HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        client_side.flush().await.unwrap();

        let (status_line_3, body_3) = read_response_with_body(&mut client_side, file_len).await;
        assert!(
            status_line_3.starts_with("HTTP/1.1 200 OK\r\n"),
            "expected full 200 OK, got {:?}",
            status_line_3
        );
        assert!(
            status_line_3.contains(&format!("Content-Length: {file_len}\r\n")),
            "expected Content-Length: {file_len}: {:?}",
            status_line_3
        );
        assert_eq!(body_3, file_bytes, "full-body slice mismatch");

        // 5. Drop the client side to close the connection — handler returns Ok(()).
        drop(client_side);

        let result = handler.await.expect("handler task panicked");
        assert!(
            result.is_ok(),
            "handler returned error on connection close: {:?}",
            result
        );
    }

    /// Reads exactly `n` bytes from the scripted peer.
    async fn read_exactly(stream: &mut tokio::io::DuplexStream, n: usize) -> Vec<u8> {
        use tokio::io::AsyncReadExt;

        let mut buf = vec![0u8; n];
        stream.read_exact(&mut buf).await.unwrap();
        buf
    }

    /// Reads until end of headers (`\r\n\r\n`), then reads `body_len` body
    /// bytes. Returns (header_section_as_string, body).
    async fn read_response_with_body(
        stream: &mut tokio::io::DuplexStream,
        body_len: usize,
    ) -> (String, Vec<u8>) {
        use tokio::io::AsyncReadExt;

        let mut headers = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).await.unwrap();
            headers.push(byte[0]);
            if headers.ends_with(b"\r\n\r\n") {
                break;
            }
            if headers.len() > 4096 {
                panic!("headers exceeded sanity limit");
            }
        }
        let header_str = String::from_utf8(headers).unwrap();
        let mut body = vec![0u8; body_len];
        if body_len > 0 {
            stream.read_exact(&mut body).await.unwrap();
        }
        (header_str, body)
    }

    /// Runs [`serve_parquet_range_requests`] against one end of a
    /// `tokio::io::duplex` pair, returning the scripted-peer end and the
    /// handler task. No real socket and no Exasol instance are involved.
    fn spawn_parquet_handler(
        file_bytes: Vec<u8>,
    ) -> (
        tokio::io::DuplexStream,
        tokio::task::JoinHandle<Result<(), TransportError>>,
    ) {
        let (mut server_side, client_side) = tokio::io::duplex(4096);
        let handler = tokio::spawn(async move {
            serve_parquet_range_requests(&mut server_side, &file_bytes).await
        });
        (client_side, handler)
    }

    /// Sends one `GET` carrying `range_header` and asserts the handler answers
    /// `400 Bad Request`, stays in its loop, and completes cleanly on close.
    async fn assert_range_header_is_rejected(file_bytes: Vec<u8>, range_header: &str) {
        let (mut client, handler) = spawn_parquet_handler(file_bytes);

        let request =
            format!("GET /001.parquet HTTP/1.1\r\nHost: x\r\nRange: {range_header}\r\n\r\n");
        client.write_all(request.as_bytes()).await.unwrap();
        client.flush().await.unwrap();

        let (headers, _) = read_response_with_body(&mut client, 0).await;
        assert_eq!(
            headers, "HTTP/1.1 400 Bad Request\r\n\r\n",
            "range header {range_header:?} should yield a bare 400"
        );

        drop(client);
        assert!(
            handler.await.expect("handler task panicked").is_ok(),
            "a rejected range must not abort the request loop"
        );
    }

    #[tokio::test]
    async fn test_perform_handshake_sends_the_magic_packet_and_returns_the_internal_address() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut client, mut server) = tokio::io::duplex(64);
        let peer = tokio::spawn(async move {
            let mut magic = [0u8; EXA_MAGIC_PACKET_SIZE];
            server.read_exact(&mut magic).await.unwrap();
            server
                .write_all(&exa_response_packet("10.0.0.5", 8563))
                .await
                .unwrap();
            server.flush().await.unwrap();
            magic
        });

        let (ip, port) = perform_handshake(&mut client)
            .await
            .expect("a well-formed response completes the handshake");

        assert_eq!(ip, "10.0.0.5");
        assert_eq!(port, 8563);
        assert_eq!(
            peer.await.unwrap(),
            generate_magic_packet(),
            "the peer must receive exactly the magic packet"
        );
    }

    #[tokio::test]
    async fn test_perform_handshake_fails_when_the_peer_closes_without_responding() {
        use tokio::io::AsyncReadExt;

        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            let mut magic = [0u8; EXA_MAGIC_PACKET_SIZE];
            server.read_exact(&mut magic).await.unwrap();
        });

        let error = perform_handshake(&mut client)
            .await
            .expect_err("no response packet means no handshake");

        let TransportError::IoError(message) = error else {
            panic!("expected IoError, got {error:?}");
        };
        assert!(
            message.starts_with("Failed to read response packet: "),
            "unexpected message: {message}"
        );
    }

    #[tokio::test]
    async fn test_perform_handshake_propagates_an_invalid_response_packet() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            let mut magic = [0u8; EXA_MAGIC_PACKET_SIZE];
            server.read_exact(&mut magic).await.unwrap();
            // Valid length, but the IP field is all nulls.
            server
                .write_all(&[0u8; EXA_RESPONSE_PACKET_SIZE])
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let error = perform_handshake(&mut client)
            .await
            .expect_err("an empty IP field is not a usable internal address");

        let TransportError::ProtocolError(message) = error else {
            panic!("expected ProtocolError, got {error:?}");
        };
        assert_eq!(message, "Empty IP address in response packet");
    }

    #[tokio::test]
    async fn test_data_pipe_send_reports_a_closed_receiver() {
        let (writer, reader) = DataPipe::create_pair(1);

        drop(reader);

        let error = writer
            .send(vec![1, 2, 3])
            .await
            .expect_err("sending into a dropped pipe end must fail");

        let TransportError::SendError(message) = error else {
            panic!("expected SendError, got {error:?}");
        };
        assert!(
            message.starts_with("Failed to send data through pipe: "),
            "unexpected message: {message}"
        );
    }

    #[tokio::test]
    async fn test_data_pipe_recv_returns_none_once_the_sender_is_gone() {
        let (writer, mut reader) = DataPipe::create_pair(1);

        drop(writer);

        assert!(reader.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_parse_http_request_fails_when_the_headers_are_truncated() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            // A request line but no blank line closing the header block.
            server
                .write_all(b"GET /001.csv HTTP/1.1\r\nHost: x\r\n")
                .await
                .unwrap();
        });

        let error = parse_http_request(&mut client)
            .await
            .expect_err("an unterminated header block is malformed");

        let TransportError::ProtocolError(message) = error else {
            panic!("expected ProtocolError, got {error:?}");
        };
        assert!(
            message.starts_with("Failed to read header line: "),
            "unexpected message: {message}"
        );
    }

    #[tokio::test]
    async fn test_parse_http_request_ignores_a_header_line_without_a_colon() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server
                .write_all(b"GET /001.csv HTTP/1.1\r\ngarbage-without-colon\r\nHost: x\r\n\r\n")
                .await
                .unwrap();
        });

        let parsed = parse_http_request(&mut client).await.unwrap();

        assert_eq!(parsed.host(), Some("x"));
        assert_eq!(parsed.headers.len(), 1);
    }

    #[tokio::test]
    async fn test_parse_http_request_rejects_a_request_line_with_too_few_parts() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(256);
        tokio::spawn(async move {
            server.write_all(b"GET /001.csv\r\n\r\n").await.unwrap();
        });

        let error = parse_http_request(&mut client)
            .await
            .expect_err("a request line needs method, path, and version");

        let TransportError::ProtocolError(message) = error else {
            panic!("expected ProtocolError, got {error:?}");
        };
        assert_eq!(message, "Invalid HTTP request line: 'GET /001.csv'");
    }

    #[test]
    fn test_build_http_response_keeps_a_caller_supplied_content_length() {
        let response = build_http_response(200, "OK", &[("content-length", "99")], Some(b"Hello"));

        let text = String::from_utf8_lossy(&response);
        assert_eq!(text.matches("ontent-length").count(), 1);
        assert!(!text.contains("Content-Length: 5"));
        assert!(text.ends_with("Hello"));
    }

    #[test]
    fn test_parse_byte_range_reads_inclusive_bounds() {
        assert_eq!(parse_byte_range("bytes=0-3"), Some((0, 3)));
        assert_eq!(parse_byte_range("bytes=7-7"), Some((7, 7)));
        assert_eq!(parse_byte_range("bytes=1024-2047"), Some((1024, 2047)));
    }

    #[test]
    fn test_parse_byte_range_tolerates_whitespace_around_the_bounds() {
        assert_eq!(parse_byte_range("bytes= 4 - 8 "), Some((4, 8)));
    }

    #[test]
    fn test_parse_byte_range_accepts_bounds_out_of_order() {
        // Ordering is the caller's concern: it decides whether to answer 400.
        assert_eq!(parse_byte_range("bytes=8-3"), Some((8, 3)));
    }

    #[test]
    fn test_parse_byte_range_rejects_malformed_headers() {
        assert_eq!(parse_byte_range("0-3"), None, "missing bytes= prefix");
        assert_eq!(parse_byte_range("items=0-3"), None, "wrong unit");
        assert_eq!(parse_byte_range("bytes=0"), None, "missing dash");
        assert_eq!(
            parse_byte_range("bytes=abc-def"),
            None,
            "non-numeric bounds"
        );
        assert_eq!(parse_byte_range("bytes=-3"), None, "missing start");
        assert_eq!(parse_byte_range("bytes=0-"), None, "missing end");
        assert_eq!(parse_byte_range("bytes=-1--2"), None, "negative bounds");
        assert_eq!(parse_byte_range(""), None, "empty header");
    }

    #[test]
    fn test_parse_response_packet_rejects_a_negative_port() {
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];
        packet[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        packet[8..24].copy_from_slice(b"10.0.0.5\0\0\0\0\0\0\0\0");

        let error = parse_response_packet(&packet).expect_err("negative port is invalid");

        let TransportError::ProtocolError(message) = error else {
            panic!("expected ProtocolError, got {error:?}");
        };
        assert_eq!(message, "Invalid port in response packet: -1");
    }

    #[test]
    fn test_parse_response_packet_rejects_a_port_above_u16_max() {
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];
        packet[4..8].copy_from_slice(&65_536i32.to_le_bytes());
        packet[8..24].copy_from_slice(b"10.0.0.5\0\0\0\0\0\0\0\0");

        let error = parse_response_packet(&packet).expect_err("65536 does not fit in a u16");

        let TransportError::ProtocolError(message) = error else {
            panic!("expected ProtocolError, got {error:?}");
        };
        assert_eq!(message, "Invalid port in response packet: 65536");
    }

    #[test]
    fn test_parse_response_packet_accepts_the_maximum_port() {
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];
        packet[4..8].copy_from_slice(&i32::from(u16::MAX).to_le_bytes());
        packet[8..24].copy_from_slice(b"10.0.0.5\0\0\0\0\0\0\0\0");

        let (ip, port) = parse_response_packet(&packet).expect("65535 is a valid port");

        assert_eq!(ip, "10.0.0.5");
        assert_eq!(port, u16::MAX);
    }

    #[test]
    fn test_parse_response_packet_rejects_an_all_null_ip_field() {
        let mut packet = [0u8; EXA_RESPONSE_PACKET_SIZE];
        packet[4..8].copy_from_slice(&8563i32.to_le_bytes());

        let error = parse_response_packet(&packet).expect_err("an empty IP is unusable");

        let TransportError::ProtocolError(message) = error else {
            panic!("expected ProtocolError, got {error:?}");
        };
        assert_eq!(message, "Empty IP address in response packet");
    }

    #[tokio::test]
    async fn test_read_line_keeps_a_carriage_return_not_followed_by_a_newline() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            server.write_all(b"a\rb\r\n").await.unwrap();
        });

        let line = read_line(&mut client).await.unwrap();

        assert_eq!(
            line, "a\rb",
            "a lone CR belongs to the line, together with the byte after it"
        );
    }

    #[tokio::test]
    async fn test_read_line_keeps_consecutive_carriage_returns() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            server.write_all(b"\r\r\r\n").await.unwrap();
        });

        let line = read_line(&mut client).await.unwrap();

        assert_eq!(line, "\r\r");
    }

    #[tokio::test]
    async fn test_read_line_returns_an_empty_string_for_a_bare_crlf() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            server.write_all(b"\r\n").await.unwrap();
        });

        let line = read_line(&mut client).await.unwrap();

        assert!(line.is_empty());
    }

    #[tokio::test]
    async fn test_read_line_rejects_a_line_that_is_not_valid_utf8() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            server.write_all(b"\xff\xfe\r\n").await.unwrap();
        });

        let error = read_line(&mut client)
            .await
            .expect_err("0xff 0xfe is not valid UTF-8");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn test_read_line_fails_when_the_stream_ends_before_a_newline() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            server.write_all(b"no terminator").await.unwrap();
        });

        let error = read_line(&mut client)
            .await
            .expect_err("an unterminated line cannot be read");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_rejects_a_range_without_the_bytes_prefix() {
        assert_range_header_is_rejected((0u8..16u8).collect(), "0-3").await;
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_rejects_a_range_without_a_dash() {
        assert_range_header_is_rejected((0u8..16u8).collect(), "bytes=0").await;
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_rejects_a_non_numeric_range() {
        assert_range_header_is_rejected((0u8..16u8).collect(), "bytes=abc-def").await;
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_rejects_a_start_beyond_the_last_byte() {
        assert_range_header_is_rejected((0u8..16u8).collect(), "bytes=16-20").await;
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_rejects_any_range_on_an_empty_file() {
        assert_range_header_is_rejected(vec![], "bytes=0-3").await;
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_rejects_an_end_before_the_start() {
        assert_range_header_is_rejected((0u8..16u8).collect(), "bytes=8-3").await;
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_clamps_a_range_end_past_the_last_byte() {
        use tokio::io::AsyncWriteExt;

        let file_bytes: Vec<u8> = (0u8..16u8).collect();
        let (mut client, handler) = spawn_parquet_handler(file_bytes.clone());

        client
            .write_all(b"GET /001.parquet HTTP/1.1\r\nHost: x\r\nRange: bytes=12-99\r\n\r\n")
            .await
            .unwrap();
        client.flush().await.unwrap();

        let (headers, body) = read_response_with_body(&mut client, 4).await;

        assert!(headers.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(
            headers.contains("Content-Range: bytes 12-15/16\r\n"),
            "end should be clamped to the last byte: {headers:?}"
        );
        assert_eq!(body, file_bytes[12..=15]);

        drop(client);
        assert!(handler.await.expect("handler task panicked").is_ok());
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_serves_a_single_byte_range() {
        use tokio::io::AsyncWriteExt;

        let file_bytes: Vec<u8> = (0u8..16u8).collect();
        let (mut client, handler) = spawn_parquet_handler(file_bytes.clone());

        client
            .write_all(b"GET /001.parquet HTTP/1.1\r\nHost: x\r\nRange: bytes=7-7\r\n\r\n")
            .await
            .unwrap();
        client.flush().await.unwrap();

        let (headers, body) = read_response_with_body(&mut client, 1).await;

        assert!(headers.contains("Content-Length: 1\r\n"), "{headers:?}");
        assert!(
            headers.contains("Content-Range: bytes 7-7/16\r\n"),
            "{headers:?}"
        );
        assert_eq!(body, vec![7u8]);

        drop(client);
        assert!(handler.await.expect("handler task panicked").is_ok());
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_answers_a_put_with_405_and_an_error() {
        use tokio::io::AsyncWriteExt;

        let (mut client, handler) = spawn_parquet_handler((0u8..16u8).collect());

        client
            .write_all(b"PUT /001.parquet HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        client.flush().await.unwrap();

        let (headers, _) = read_response_with_body(&mut client, 0).await;
        assert_eq!(headers, "HTTP/1.1 405 Method Not Allowed\r\n\r\n");

        let error = handler
            .await
            .expect("handler task panicked")
            .expect_err("a PUT must abort the Parquet import handler");

        let TransportError::ProtocolError(message) = error else {
            panic!("expected ProtocolError, got {error:?}");
        };
        assert_eq!(message, "Unexpected PUT in Parquet import handler");
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_completes_when_the_peer_closes_first() {
        let (client, handler) = spawn_parquet_handler((0u8..16u8).collect());

        drop(client);

        assert!(
            handler.await.expect("handler task panicked").is_ok(),
            "an immediate close is a normal end of conversation"
        );
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_completes_on_an_unparseable_request() {
        use tokio::io::AsyncWriteExt;

        let (mut client, handler) = spawn_parquet_handler((0u8..16u8).collect());

        client.write_all(b"garbage\r\n").await.unwrap();
        client.flush().await.unwrap();
        drop(client);

        assert!(
            handler.await.expect("handler task panicked").is_ok(),
            "a malformed request line is treated as peer close"
        );
    }

    #[tokio::test]
    async fn test_serve_parquet_range_requests_serves_an_empty_file_for_head_and_full_get() {
        use tokio::io::AsyncWriteExt;

        let (mut client, handler) = spawn_parquet_handler(vec![]);

        client
            .write_all(b"HEAD /001.parquet HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        client.flush().await.unwrap();
        let (head_headers, _) = read_response_with_body(&mut client, 0).await;
        assert_eq!(head_headers, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");

        client
            .write_all(b"GET /001.parquet HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        client.flush().await.unwrap();
        let (get_headers, body) = read_response_with_body(&mut client, 0).await;
        assert_eq!(get_headers, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        assert!(body.is_empty());

        drop(client);
        assert!(handler.await.expect("handler task panicked").is_ok());
    }

    /// Scenario: Tunnel setup fails with the step named when the peer stops answering
    #[tokio::test]
    async fn connect_with_timeout_fails_at_the_exa_handshake_when_the_peer_never_answers() {
        let mut server = SilentServer::accepting().await;

        let error = finish_within(
            LOOPBACK_TEST_BOUND,
            HttpTransportClient::connect_with_timeout(
                &server.host,
                server.port,
                false,
                SETUP_BOUND,
            ),
        )
        .await
        .err()
        .expect("a peer that never answers the magic packet must fail the setup");

        assert!(
            error
                .to_string()
                .contains("HTTP tunnel setup timeout after 300ms (EXA handshake)"),
            "{error}"
        );
        finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
    }

    /// How fast an operation on a tunnel whose TLS handshake already failed
    /// must fail: well below `SETUP_BOUND`, so it cannot have waited for one.
    const FAILED_TUNNEL_BOUND: Duration = Duration::from_millis(50);

    async fn connect_with_tls(server: &FakeExasolServer) -> HttpTransportClient {
        finish_within(
            LOOPBACK_TEST_BOUND,
            HttpTransportClient::connect_with_timeout(&server.host, server.port, true, SETUP_BOUND),
        )
        .await
        .expect("a TLS tunnel connects once the EXA handshake completes")
    }

    fn assert_tls_handshake_timed_out(error: &TransportError) {
        assert!(
            error
                .to_string()
                .contains("HTTP tunnel setup timeout after 300ms (TLS handshake)"),
            "{error}"
        );
    }

    /// Runs a second import request on a tunnel whose TLS handshake failed and
    /// checks that it fails at once, sends nothing, and opens no connection.
    async fn assert_tunnel_stays_failed(
        client: &mut HttpTransportClient,
        server: &mut FakeExasolServer,
    ) {
        let received = finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;

        finish_within(FAILED_TUNNEL_BOUND, client.handle_import_request())
            .await
            .expect_err("a tunnel whose TLS handshake failed must not serve a request");

        assert!(
            received.is_empty(),
            "the driver sent {} bytes into a failed TLS handshake",
            received.len()
        );
        assert!(
            !server.accepts_another_connection().await,
            "a failed TLS tunnel must not open a new connection"
        );
    }

    /// Scenario: TLS handshake starts after the IMPORT or EXPORT statement is sent
    #[tokio::test]
    async fn connect_with_tls_returns_before_any_tls_data_is_exchanged() {
        let mut server = FakeExasolServer::silent_after_handshake().await;

        let client = connect_with_tls(&server).await;
        assert!(client.public_key_fingerprint().is_some());
        drop(client);

        let received = finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        assert!(
            received.is_empty(),
            "connect sent {} bytes after the magic packet",
            received.len()
        );
    }

    #[tokio::test]
    async fn shutdown_of_a_pending_tls_tunnel_closes_it_without_a_handshake() {
        let mut server = FakeExasolServer::silent_after_handshake().await;
        let mut client = connect_with_tls(&server).await;

        client
            .shutdown()
            .await
            .expect("a pending TLS tunnel shuts down over plain TCP");

        let received = finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        let error = finish_within(FAILED_TUNNEL_BOUND, client.handle_import_request())
            .await
            .expect_err("a shut down tunnel must not serve a request");
        assert!(
            received.is_empty(),
            "shutdown sent {} bytes",
            received.len()
        );
        assert!(
            error.to_string().contains("HTTP tunnel is closed"),
            "{error}"
        );
        assert!(!server.accepts_another_connection().await);
    }

    /// Scenario: TLS encryption with ad-hoc certificates
    /// Scenario: HTTP transport TLS against Exasol SaaS / production
    /// Scenario: TLS tunnel fails with the step named when Exasol does not start the TLS handshake
    #[tokio::test]
    async fn tls_tunnel_accepts_the_handshake_that_the_peer_starts_as_tls_client() {
        let mut server = FakeExasolServer::requesting_over_tls_after(Duration::ZERO).await;
        let mut client = connect_with_tls(&server).await;

        let request = finish_within(LOOPBACK_TEST_BOUND, client.handle_import_request())
            .await
            .expect("the driver must serve the GET over TLS");
        client.write_chunked_body(b"1,a\n").await.expect("write");
        client.write_final_chunk().await.expect("final chunk");
        drop(client);

        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.path, "/001.csv");
        let response = finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        let response = String::from_utf8(response).expect("the response is text");
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response.ends_with("\r\n\r\n4\r\n1,a\n\r\n0\r\n\r\n"),
            "{response}"
        );
    }

    /// Scenario: Tunnel setup fails with the step named when the peer stops answering
    #[tokio::test]
    async fn tls_handshake_bound_starts_at_the_first_tunnel_read_or_write() {
        let server = FakeExasolServer::requesting_over_tls_after(SETUP_BOUND * 2).await;
        let mut client = connect_with_tls(&server).await;

        tokio::time::sleep(SETUP_BOUND * 2).await;
        let request = finish_within(LOOPBACK_TEST_BOUND, client.handle_import_request())
            .await
            .expect("the TLS bound starts at the first read, so the late handshake fits in it");

        assert_eq!(request.path, "/001.csv");
    }

    /// Scenario: Tunnel setup fails with the step named when the peer stops answering
    /// Scenario: TLS tunnel fails with the step named when Exasol does not start the TLS handshake
    #[tokio::test]
    async fn tls_tunnel_fails_with_the_step_named_when_the_peer_sends_no_tls_byte() {
        let mut server = FakeExasolServer::silent_after_handshake().await;
        let mut client = connect_with_tls(&server).await;

        let error = finish_within(LOOPBACK_TEST_BOUND, client.handle_import_request())
            .await
            .expect_err("a peer that never starts the TLS handshake must fail the tunnel");

        assert_tls_handshake_timed_out(&error);
        finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        drop(client);
    }

    /// Scenario: Tunnel setup fails with the step named when the peer stops answering
    /// Scenario: A TLS tunnel stays failed after a failed TLS handshake
    #[tokio::test]
    async fn tls_handshake_fails_with_the_step_named_when_the_peer_stalls_mid_handshake() {
        let mut server = FakeExasolServer::stalling_mid_tls_handshake().await;
        let mut client = connect_with_tls(&server).await;

        let error = finish_within(LOOPBACK_TEST_BOUND, client.handle_import_request())
            .await
            .expect_err("a peer that stalls mid-handshake must fail the tunnel");

        assert_tls_handshake_timed_out(&error);
        assert_tunnel_stays_failed(&mut client, &mut server).await;
    }

    /// Scenario: Tunnel setup fails with the step named when the peer stops answering
    #[tokio::test]
    async fn parquet_tunnel_fails_with_the_step_named_when_the_peer_stalls_mid_handshake() {
        let server = FakeExasolServer::stalling_mid_tls_handshake().await;
        let mut client = connect_with_tls(&server).await;

        let error = finish_within(
            LOOPBACK_TEST_BOUND,
            client.handle_parquet_import_requests(b"PAR1"),
        )
        .await
        .expect_err("a stalled TLS handshake must fail the Parquet tunnel, not end it as a close");

        assert!(error.to_string().contains("(TLS handshake)"), "{error}");
    }

    /// Scenario: Tunnel setup fails with the step named when the peer stops answering
    /// Scenario: TLS tunnel fails with the step named when Exasol does not start the TLS handshake
    /// Scenario: A TLS tunnel stays failed after a failed TLS handshake
    #[tokio::test]
    async fn tls_tunnel_fails_when_the_peer_closes_before_the_handshake() {
        let mut server = FakeExasolServer::closing_after_handshake().await;
        let mut client = connect_with_tls(&server).await;

        let started = tokio::time::Instant::now();
        let error = finish_within(LOOPBACK_TEST_BOUND, client.handle_import_request())
            .await
            .expect_err("a peer that closes before the TLS handshake must fail the tunnel");

        assert!(started.elapsed() < SETUP_BOUND, "{error}");
        assert!(!error.to_string().contains("timeout"), "{error}");
        assert_tunnel_stays_failed(&mut client, &mut server).await;
    }

    /// Scenario: EXA tunneling handshake in client mode
    #[tokio::test]
    async fn connect_with_timeout_returns_the_internal_address_within_the_bound() {
        let server = FakeExasolServer::silent_after_handshake().await;

        let client = finish_within(
            LOOPBACK_TEST_BOUND,
            HttpTransportClient::connect_with_timeout(
                &server.host,
                server.port,
                false,
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("a peer that answers the handshake in time completes the setup");

        assert_eq!(client.internal_address(), "10.0.0.5:8563");
    }
}
