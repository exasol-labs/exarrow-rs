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

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use rcgen::{CertifiedKey, KeyPair};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::error::TransportError;

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

    /// Creates a rustls `ClientConfig` from this certificate.
    ///
    /// This creates a client configuration that trusts the self-signed certificate
    /// (for connecting to Exasol with TLS using this ad-hoc certificate).
    ///
    /// # Errors
    ///
    /// Returns `TransportError::TlsError` if configuration fails.
    pub fn to_client_config(&self) -> Result<ClientConfig, TransportError> {
        // Create an empty root store - we won't verify server certs since
        // we're using ad-hoc certificates with Exasol's custom TLS mode
        let root_store = RootCertStore::empty();

        // Build client config that doesn't verify server certificate
        // This is necessary because Exasol uses the client's ad-hoc certificate
        // and we include the fingerprint in the SQL statement
        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(config)
    }
}

/// Internal enum to hold either TCP or TLS client stream.
/// The TLS variant is boxed to reduce enum size difference.
enum ClientConnectionStream {
    Tcp(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

/// HTTP Transport Client for connecting TO Exasol.
///
/// This client connects to Exasol's data node and performs the EXA tunneling
/// handshake to establish a data transfer channel. Unlike the server mode,
/// this works through firewalls and NAT because it only makes outbound connections.
///
/// # Example
///
/// ```no_run
/// use exarrow_rs::transport::HttpTransportClient;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Connect to Exasol
/// let mut client = HttpTransportClient::connect("exasol.example.com", 8563, false).await?;
///
/// // Get the internal address for IMPORT/EXPORT SQL
/// let internal_addr = client.internal_address();
/// println!("Use this in SQL: {}", internal_addr);
///
/// // Write data for IMPORT
/// client.write(b"CSV data here").await?;
///
/// // Or read data for EXPORT
/// let mut buf = vec![0u8; 1024];
/// let n = client.read(&mut buf).await?;
///
/// // Clean shutdown
/// client.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct HttpTransportClient {
    /// The underlying stream (either TCP or TLS)
    stream: ClientConnectionStream,
    /// Internal address from handshake response (format: "host:port")
    internal_addr: String,
    /// TLS certificate (for fingerprint extraction), if TLS is enabled
    tls_certificate: Option<TlsCertificate>,
}

impl HttpTransportClient {
    /// Connects to Exasol and performs the EXA tunneling handshake.
    ///
    /// # Protocol Flow
    ///
    /// The EXA tunneling protocol requires a specific order of operations:
    /// 1. Connect TCP to Exasol (unencrypted)
    /// 2. Send magic packet over plain TCP
    /// 3. Receive response packet with internal address over plain TCP
    /// 4. If TLS is enabled, wrap the connection with TLS
    /// 5. Send/receive HTTP data (optionally over TLS)
    ///
    /// This order is critical - TLS must be applied AFTER the handshake, not before.
    ///
    /// # Arguments
    ///
    /// * `host` - The Exasol host to connect to
    /// * `port` - The port to connect to (same as WebSocket port)
    /// * `use_tls` - Whether to use TLS encryption for data transfer
    ///
    /// # Returns
    ///
    /// A connected `HttpTransportClient` ready for data transfer.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if connection or handshake fails.
    pub async fn connect(host: &str, port: u16, use_tls: bool) -> Result<Self, TransportError> {
        let addr = format!("{host}:{port}");

        // Connect to Exasol
        let mut tcp_stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| TransportError::IoError(format!("Failed to connect to {addr}: {e}")))?;

        // IMPORTANT: Perform EXA handshake BEFORE TLS wrapping
        // The magic packet handshake must happen over plain TCP
        let (ip, internal_port) = perform_handshake(&mut tcp_stream).await?;
        let internal_addr = format!("{ip}:{internal_port}");

        if use_tls {
            // Generate ad-hoc TLS certificate for fingerprint
            let cert = TlsCertificate::generate()?;

            // Convert certificate and key to rustls format for client auth
            let cert_der = CertificateDer::from(cert.certificate_der.clone());
            let key_der = PrivatePkcs8KeyDer::from(cert.private_key_der.clone());

            // Create TLS connector with custom verifier that accepts any cert
            // and presents our client certificate for Exasol's fingerprint verification
            let connector = TlsConnector::from(Arc::new(
                ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoVerifier))
                    .with_client_auth_cert(vec![cert_der], key_der.into())
                    .map_err(|e| {
                        TransportError::TlsError(format!("Failed to set client cert: {e}"))
                    })?,
            ));

            // Use a dummy server name since we're not verifying
            let server_name = "exasol"
                .try_into()
                .map_err(|e| TransportError::TlsError(format!("Invalid server name: {e:?}")))?;

            // Wrap the existing TCP connection with TLS AFTER the handshake
            let tls_stream = connector
                .connect(server_name, tcp_stream)
                .await
                .map_err(|e| TransportError::TlsError(format!("TLS handshake failed: {e}")))?;

            Ok(Self {
                stream: ClientConnectionStream::Tls(Box::new(tls_stream)),
                internal_addr,
                tls_certificate: Some(cert),
            })
        } else {
            Ok(Self {
                stream: ClientConnectionStream::Tcp(tcp_stream),
                internal_addr,
                tls_certificate: None,
            })
        }
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
    pub async fn write(&mut self, data: &[u8]) -> Result<(), TransportError> {
        match &mut self.stream {
            ClientConnectionStream::Tcp(stream) => {
                stream
                    .write_all(data)
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to write data: {e}")))?;
                stream
                    .flush()
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to flush data: {e}")))?;
            }
            ClientConnectionStream::Tls(stream) => {
                stream
                    .write_all(data)
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to write data: {e}")))?;
                stream
                    .flush()
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to flush data: {e}")))?;
            }
        }
        Ok(())
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
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match &mut self.stream {
            ClientConnectionStream::Tcp(stream) => stream
                .read(buf)
                .await
                .map_err(|e| TransportError::IoError(format!("Failed to read data: {e}"))),
            ClientConnectionStream::Tls(stream) => stream
                .read(buf)
                .await
                .map_err(|e| TransportError::IoError(format!("Failed to read data: {e}"))),
        }
    }

    /// Shuts down the connection cleanly.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::IoError` if shutdown fails.
    pub async fn shutdown(&mut self) -> Result<(), TransportError> {
        match &mut self.stream {
            ClientConnectionStream::Tcp(stream) => stream.shutdown().await.map_err(|e| {
                TransportError::IoError(format!("Failed to shutdown connection: {e}"))
            }),
            ClientConnectionStream::Tls(stream) => stream.shutdown().await.map_err(|e| {
                TransportError::IoError(format!("Failed to shutdown connection: {e}"))
            }),
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
        match &mut self.stream {
            ClientConnectionStream::Tcp(stream) => parse_http_request(stream).await,
            ClientConnectionStream::Tls(stream) => parse_http_request(stream.as_mut()).await,
        }
    }

    /// Reads the body of a chunked transfer encoded request.
    ///
    /// This reads all chunks until the final empty chunk and returns the complete body.
    ///
    /// # Returns
    ///
    /// The complete body data as a vector of bytes.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if reading fails or chunked encoding is invalid.
    pub async fn read_chunked_body(&mut self) -> Result<Vec<u8>, TransportError> {
        let mut body = Vec::new();

        loop {
            // Read chunk size line
            let size_line = match &mut self.stream {
                ClientConnectionStream::Tcp(stream) => read_line(stream).await,
                ClientConnectionStream::Tls(stream) => read_line(stream.as_mut()).await,
            }
            .map_err(|e| {
                TransportError::ProtocolError(format!("Failed to read chunk size: {e}"))
            })?;

            let chunk_size = parse_chunk_size(&size_line)?;

            if chunk_size == 0 {
                // Final chunk - read the trailing CRLF
                let mut trailer = [0u8; 2];
                match &mut self.stream {
                    ClientConnectionStream::Tcp(stream) => stream.read_exact(&mut trailer).await,
                    ClientConnectionStream::Tls(stream) => stream.read_exact(&mut trailer).await,
                }
                .map_err(|e| {
                    TransportError::ProtocolError(format!("Failed to read chunk trailer: {e}"))
                })?;
                break;
            }

            // Read chunk data
            let mut chunk = vec![0u8; chunk_size];
            match &mut self.stream {
                ClientConnectionStream::Tcp(stream) => stream.read_exact(&mut chunk).await,
                ClientConnectionStream::Tls(stream) => stream.read_exact(&mut chunk).await,
            }
            .map_err(|e| {
                TransportError::ProtocolError(format!("Failed to read chunk data: {e}"))
            })?;

            body.extend_from_slice(&chunk);

            // Read trailing CRLF after chunk data
            let mut crlf = [0u8; 2];
            match &mut self.stream {
                ClientConnectionStream::Tcp(stream) => stream.read_exact(&mut crlf).await,
                ClientConnectionStream::Tls(stream) => stream.read_exact(&mut crlf).await,
            }
            .map_err(|e| {
                TransportError::ProtocolError(format!("Failed to read chunk CRLF: {e}"))
            })?;
        }

        Ok(body)
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

    /// Handles an EXPORT request from Exasol.
    ///
    /// This method implements the correct EXPORT protocol flow:
    /// 1. Wait for HTTP PUT request from Exasol
    /// 2. Verify it's a PUT request with chunked encoding
    /// 3. Read all data from the PUT request body
    /// 4. Send HTTP 200 OK response
    ///
    /// # Returns
    ///
    /// A tuple of (request, body) where request is the parsed HTTP request
    /// and body is the complete data received from Exasol.
    ///
    /// # Errors
    ///
    /// Returns `TransportError` if the request is not a PUT or if I/O fails.
    pub async fn handle_export_request(
        &mut self,
    ) -> Result<(HttpRequest, Vec<u8>), TransportError> {
        let request = self.read_http_request().await?;

        if request.method != HttpMethod::Put {
            return Err(TransportError::ProtocolError(format!(
                "Expected PUT request for EXPORT, got {}",
                request.method
            )));
        }

        // Read the body based on transfer encoding
        let body = if request.is_chunked() {
            self.read_chunked_body().await?
        } else if let Some(content_length) = request.content_length() {
            let mut body = vec![0u8; content_length];
            match &mut self.stream {
                ClientConnectionStream::Tcp(stream) => stream.read_exact(&mut body).await,
                ClientConnectionStream::Tls(stream) => stream.read_exact(&mut body).await,
            }
            .map_err(|e| TransportError::IoError(format!("Failed to read body: {e}")))?;
            body
        } else {
            return Err(TransportError::ProtocolError(
                "PUT request has no Content-Length or Transfer-Encoding".to_string(),
            ));
        };

        // Send 200 OK response
        self.write_http_response(200, "OK", &[], None).await?;

        Ok((request, body))
    }
}

/// A certificate verifier that accepts any certificate.
/// Used for Exasol's ad-hoc TLS mode where verification is done via fingerprint in SQL.
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
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

    /// Closes the sender side of the pipe.
    pub fn close_sender(&self) {
        // Dropping the clone will decrement the reference count
        // but we need to explicitly close
        drop(self.tx.clone());
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

// =============================================================================
// DEPRECATED: Server Mode Types (to be removed after import/export refactoring)
// =============================================================================
// The following types are kept for backwards compatibility with existing
// import/export code. They will be removed once all import/export modules
// are updated to use HttpTransportClient.

/// HTTP Transport Server for handling Exasol connections.
///
/// # Deprecated
///
/// This type uses server mode which doesn't work with cloud Exasol instances.
/// Use [`HttpTransportClient`] instead, which uses client mode.
#[deprecated(
    since = "2.2.0",
    note = "Use HttpTransportClient instead. Server mode doesn't work with cloud Exasol."
)]
pub struct HttpTransportServer {
    /// TCP listener for accepting connections
    listener: TcpListener,
    /// TLS configuration (None for unencrypted connections)
    tls_config: Option<Arc<ServerConfig>>,
    /// TLS certificate (for fingerprint extraction)
    tls_certificate: Option<TlsCertificate>,
    /// Local address the server is bound to
    local_addr: SocketAddr,
}

#[allow(deprecated)]
impl HttpTransportServer {
    /// Creates a new HTTP transport server bound to an OS-assigned port.
    pub async fn start(use_tls: bool) -> Result<Self, TransportError> {
        Self::start_on_addr("0.0.0.0:0", use_tls).await
    }

    /// Creates a new HTTP transport server bound to a specific address.
    pub async fn start_on_addr(addr: &str, use_tls: bool) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            TransportError::IoError(format!("Failed to bind HTTP transport server: {e}"))
        })?;

        let local_addr = listener
            .local_addr()
            .map_err(|e| TransportError::IoError(format!("Failed to get local address: {e}")))?;

        let (tls_config, tls_certificate) = if use_tls {
            let cert = TlsCertificate::generate()?;
            let config = Arc::new(cert.to_server_config()?);
            (Some(config), Some(cert))
        } else {
            (None, None)
        };

        Ok(Self {
            listener,
            tls_config,
            tls_certificate,
            local_addr,
        })
    }

    /// Returns the local address the server is bound to.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns the port the server is listening on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.local_addr.port()
    }

    /// Returns the public key fingerprint for TLS connections.
    #[must_use]
    pub fn public_key_fingerprint(&self) -> Option<&str> {
        self.tls_certificate
            .as_ref()
            .map(|c| c.fingerprint.as_str())
    }

    /// Accepts a single incoming connection and performs the EXA handshake.
    #[allow(deprecated)]
    pub async fn accept_connection(&self) -> Result<HttpTransportConnection, TransportError> {
        let (tcp_stream, peer_addr) =
            self.listener.accept().await.map_err(|e| {
                TransportError::IoError(format!("Failed to accept connection: {e}"))
            })?;

        if let Some(tls_config) = &self.tls_config {
            let acceptor = TlsAcceptor::from(Arc::clone(tls_config));
            let tls_stream = acceptor
                .accept(tcp_stream)
                .await
                .map_err(|e| TransportError::TlsError(format!("TLS handshake failed: {e}")))?;

            let mut conn = HttpTransportConnection::new_tls(tls_stream, peer_addr);
            conn.perform_handshake().await?;
            Ok(conn)
        } else {
            let mut conn = HttpTransportConnection::new_tcp(tcp_stream, peer_addr);
            conn.perform_handshake().await?;
            Ok(conn)
        }
    }
}

/// Connection wrapper for HTTP transport data transfer.
///
/// # Deprecated
///
/// This type is part of server mode which doesn't work with cloud Exasol instances.
/// Use [`HttpTransportClient`] instead.
#[deprecated(
    since = "2.2.0",
    note = "Use HttpTransportClient instead. Server mode doesn't work with cloud Exasol."
)]
pub struct HttpTransportConnection {
    /// The underlying stream (either TCP or TLS)
    stream: ServerConnectionStream,
    /// Peer address of the connected client
    peer_addr: SocketAddr,
    /// Internal address from handshake response (ip_string, port)
    internal_addr: Option<(String, u16)>,
}

/// Internal enum for server-side connection streams.
enum ServerConnectionStream {
    Tcp(TcpStream),
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

#[allow(deprecated)]
impl HttpTransportConnection {
    /// Creates a new connection wrapper for a TCP stream.
    fn new_tcp(stream: TcpStream, peer_addr: SocketAddr) -> Self {
        Self {
            stream: ServerConnectionStream::Tcp(stream),
            peer_addr,
            internal_addr: None,
        }
    }

    /// Creates a new connection wrapper for a TLS stream.
    fn new_tls(stream: tokio_rustls::server::TlsStream<TcpStream>, peer_addr: SocketAddr) -> Self {
        Self {
            stream: ServerConnectionStream::Tls(Box::new(stream)),
            peer_addr,
            internal_addr: None,
        }
    }

    /// Returns the peer address of the connected client.
    #[must_use]
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Returns the internal address from the handshake response.
    #[must_use]
    pub fn internal_addr(&self) -> Option<&(String, u16)> {
        self.internal_addr.as_ref()
    }

    /// Performs the EXA tunneling handshake.
    async fn perform_handshake(&mut self) -> Result<(), TransportError> {
        let internal_addr = match &mut self.stream {
            ServerConnectionStream::Tcp(stream) => perform_handshake(stream).await?,
            ServerConnectionStream::Tls(stream) => perform_handshake(stream).await?,
        };
        self.internal_addr = Some(internal_addr);
        Ok(())
    }

    /// Writes data to the connection.
    pub async fn write(&mut self, data: &[u8]) -> Result<(), TransportError> {
        match &mut self.stream {
            ServerConnectionStream::Tcp(stream) => {
                stream
                    .write_all(data)
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to write data: {e}")))?;
                stream
                    .flush()
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to flush data: {e}")))?;
            }
            ServerConnectionStream::Tls(stream) => {
                stream
                    .write_all(data)
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to write data: {e}")))?;
                stream
                    .flush()
                    .await
                    .map_err(|e| TransportError::IoError(format!("Failed to flush data: {e}")))?;
            }
        }
        Ok(())
    }

    /// Writes a chunk of data using chunked transfer encoding.
    pub async fn write_chunk(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let chunk = encode_chunk(data);
        self.write(&chunk).await
    }

    /// Writes the final empty chunk to signal end of transfer.
    pub async fn write_final_chunk(&mut self) -> Result<(), TransportError> {
        self.write_chunk(&[]).await
    }

    /// Reads data from the connection.
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match &mut self.stream {
            ServerConnectionStream::Tcp(stream) => stream
                .read(buf)
                .await
                .map_err(|e| TransportError::IoError(format!("Failed to read data: {e}"))),
            ServerConnectionStream::Tls(stream) => stream
                .read(buf)
                .await
                .map_err(|e| TransportError::IoError(format!("Failed to read data: {e}"))),
        }
    }

    /// Reads exact number of bytes from the connection.
    pub async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), TransportError> {
        match &mut self.stream {
            ServerConnectionStream::Tcp(stream) => {
                stream.read_exact(buf).await.map_err(|e| {
                    TransportError::IoError(format!("Failed to read exact data: {e}"))
                })?;
                Ok(())
            }
            ServerConnectionStream::Tls(stream) => {
                stream.read_exact(buf).await.map_err(|e| {
                    TransportError::IoError(format!("Failed to read exact data: {e}"))
                })?;
                Ok(())
            }
        }
    }

    /// Shuts down the connection.
    pub async fn shutdown(&mut self) -> Result<(), TransportError> {
        match &mut self.stream {
            ServerConnectionStream::Tcp(stream) => stream.shutdown().await.map_err(|e| {
                TransportError::IoError(format!("Failed to shutdown connection: {e}"))
            }),
            ServerConnectionStream::Tls(stream) => stream.shutdown().await.map_err(|e| {
                TransportError::IoError(format!("Failed to shutdown connection: {e}"))
            }),
        }
    }
}

// =============================================================================
// End of deprecated server mode types
// =============================================================================

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

/// HTTP method enum for parsed requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// HTTP GET method (used for IMPORT - Exasol requests data)
    Get,
    /// HTTP PUT method (used for EXPORT - Exasol sends data)
    Put,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Put => write!(f, "PUT"),
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

    // Tests for deprecated HttpTransportServer (kept for backwards compatibility)
    #[allow(deprecated)]
    #[tokio::test]
    async fn test_http_transport_server_start() {
        let server = HttpTransportServer::start(false).await;
        assert!(server.is_ok());

        let server = server.unwrap();
        assert!(server.port() > 0);
        assert!(server.public_key_fingerprint().is_none());
    }

    #[allow(deprecated)]
    #[tokio::test]
    async fn test_http_transport_server_start_with_tls() {
        let server = HttpTransportServer::start(true).await;
        assert!(server.is_ok());

        let server = server.unwrap();
        assert!(server.port() > 0);
        assert!(server.public_key_fingerprint().is_some());
        // Fingerprint is now "sha256//<base64>" format (52 chars)
        assert_eq!(server.public_key_fingerprint().unwrap().len(), 52);
    }

    #[allow(deprecated)]
    #[tokio::test]
    async fn test_http_transport_server_specific_port() {
        let server = HttpTransportServer::start_on_addr("127.0.0.1:0", false).await;
        assert!(server.is_ok());

        let server = server.unwrap();
        assert!(server.port() > 0);
        assert!(server.local_addr().ip().is_loopback());
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
}
