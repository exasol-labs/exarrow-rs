//! The TLS client step of the native and WebSocket transports, and the rustls
//! certificate verifiers it selects.
//!
//! [`client_config`] chooses the verifier from the connection parameters, and
//! [`client_handshake`] runs the handshake on an open TCP stream. Both
//! transports call them, so a TLS change is made once. Each caller keeps its own
//! socket options, deadline, and stream wrapping.
//!
//! The two custom [`rustls::client::danger::ServerCertVerifier`] implementations:
//!
//! - [`NoVerifier`] accepts any certificate (used when validation is disabled).
//! - [`FingerprintVerifier`] validates by SHA-256 fingerprint of the DER-encoded
//!   certificate, bypassing hostname and CA-chain validation, and verifies the
//!   handshake signature against that certificate.

#[cfg(any(feature = "websocket", feature = "native"))]
use std::sync::Arc;

use rustls::pki_types::CertificateDer;
#[cfg(any(feature = "websocket", feature = "native"))]
use rustls::pki_types::ServerName;
#[cfg(any(feature = "websocket", feature = "native"))]
use tokio::net::TcpStream;
#[cfg(any(feature = "websocket", feature = "native"))]
use tokio_rustls::client::TlsStream;

#[cfg(any(feature = "websocket", feature = "native"))]
use crate::error::TransportError;

/// Builds the TLS client configuration of a connection: a certificate
/// fingerprint pins the certificate and takes precedence; otherwise
/// validation trusts the native root certificates, and without it any
/// certificate is accepted.
#[cfg(any(feature = "websocket", feature = "native"))]
pub(crate) fn client_config(
    certificate_fingerprint: Option<&str>,
    validate_server_certificate: bool,
) -> rustls::ClientConfig {
    if let Some(fingerprint) = certificate_fingerprint {
        return rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(FingerprintVerifier::new(fingerprint)))
            .with_no_client_auth();
    }
    if validate_server_certificate {
        let mut root_store = rustls::RootCertStore::empty();
        for certificate in rustls_native_certs::load_native_certs().certs {
            // Some systems ship malformed root certificates; skipping them keeps the rest usable.
            let _ = root_store.add(certificate);
        }
        return rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
    }
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth()
}

/// Runs the TLS handshake for `host` on an open TCP stream.
///
/// It starts no timer and sets no socket option, so each transport runs it
/// under its own connection deadline.
///
/// # Errors
///
/// Returns `TransportError::TlsError` for a host that is not a valid server
/// name and for a failed handshake.
#[cfg(any(feature = "websocket", feature = "native"))]
pub(crate) async fn client_handshake(
    tcp: TcpStream,
    host: &str,
    config: rustls::ClientConfig,
) -> Result<TlsStream<TcpStream>, TransportError> {
    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|e| TransportError::TlsError(format!("Invalid server name: {e}")))?;
    tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(server_name, tcp)
        .await
        .map_err(|e| TransportError::TlsError(e.to_string()))
}

/// The signature schemes that [`NoVerifier`] advertises.
pub(crate) fn all_supported_verify_schemes() -> Vec<rustls::SignatureScheme> {
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

/// A certificate verifier that accepts any certificate.
/// Used when certificate validation is disabled.
#[derive(Debug)]
pub(crate) struct NoVerifier;

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

    // Validation is off by the operator's choice, so a signature check would add no trust.
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
        all_supported_verify_schemes()
    }
}

/// A certificate verifier that pins the SHA-256 fingerprint of the DER-encoded
/// certificate and checks that the handshake is signed with that certificate's key.
/// Bypasses hostname and CA chain validation.
///
/// Only the WebSocket and native-TCP transports construct this; gate it on those
/// features so a `--no-default-features` build (HTTP transport only) does not warn.
#[cfg(any(feature = "websocket", feature = "native"))]
#[derive(Debug)]
pub(crate) struct FingerprintVerifier {
    expected_fingerprint: String,
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}

#[cfg(any(feature = "websocket", feature = "native"))]
impl FingerprintVerifier {
    pub(crate) fn new(expected_fingerprint: &str) -> Self {
        Self {
            expected_fingerprint: expected_fingerprint.to_string(),
            algorithms: rustls::crypto::aws_lc_rs::default_provider()
                .signature_verification_algorithms,
        }
    }
}

#[cfg(any(feature = "websocket", feature = "native"))]
impl rustls::client::danger::ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        use aws_lc_rs::digest;
        let fingerprint = digest::digest(&digest::SHA256, end_entity.as_ref());
        let actual: String = fingerprint
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        if actual == self.expected_fingerprint {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "Certificate fingerprint mismatch: expected {}, got {}",
                self.expected_fingerprint, actual
            )))
        }
    }

    // Like CA validation, check that the handshake is signed with the pinned certificate's key.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::ServerCertVerifier;
    use rustls::internal::msgs::codec::Codec;
    use rustls::pki_types::{ServerName, UnixTime};
    use std::time::Duration;

    /// DER bytes stood in for a server certificate. The verifiers never parse
    /// them — `FingerprintVerifier` only hashes them — so arbitrary bytes with a
    /// known SHA-256 are enough.
    const CERTIFICATE_BYTES: &[u8] = b"exasol-test-certificate";

    /// `sha256sum` of `CERTIFICATE_BYTES`, lowercase hex, as an independent
    /// oracle for the fingerprint the verifier computes itself.
    const CERTIFICATE_SHA256_HEX: &str =
        "9a577d3176aa6c27274ac3c4d4b79aa7a199a1c30f4091acc84b290a65996b87";

    /// A fixed instant, so no test reads the real clock. The verifiers ignore
    /// the `now` argument entirely — expiry is not their concern.
    const FIXED_NOW_SECS: u64 = 1_700_000_000;

    fn certificate() -> CertificateDer<'static> {
        CertificateDer::from(CERTIFICATE_BYTES.to_vec())
    }

    fn server_name() -> ServerName<'static> {
        ServerName::try_from("exasol").expect("'exasol' is a valid DNS name")
    }

    fn fixed_now() -> UnixTime {
        UnixTime::since_unix_epoch(Duration::from_secs(FIXED_NOW_SECS))
    }

    /// Builds a `DigitallySignedStruct` from its TLS wire encoding, because its
    /// constructor is crate-private in rustls and there is no public one: a
    /// `SignatureScheme` as a big-endian `u16`, then the signature as a
    /// `u16`-length-prefixed payload.
    ///
    /// `rustls::internal` is documented by rustls itself as "used in
    /// integration tests... DOES NOT form part of the stable interface", so a
    /// rustls upgrade (even a patch release) can break this function. If it
    /// does, delete this function and the `verify_tls1{2,3}_signature`
    /// assertions that use it; keep the `supported_verify_schemes` assertions,
    /// which need no internal API.
    fn digitally_signed_struct() -> rustls::DigitallySignedStruct {
        const RSA_PKCS1_SHA256: [u8; 2] = [0x04, 0x01];
        let signature: &[u8] = b"not-a-real-signature";
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&RSA_PKCS1_SHA256);
        encoded.extend_from_slice(&(signature.len() as u16).to_be_bytes());
        encoded.extend_from_slice(signature);

        let dss = rustls::DigitallySignedStruct::read_bytes(&encoded)
            .expect("hand-built DigitallySignedStruct encoding should decode");
        assert_eq!(dss.scheme, rustls::SignatureScheme::RSA_PKCS1_SHA256);
        assert_eq!(dss.signature(), signature);
        dss
    }

    #[test]
    fn no_verifier_accepts_any_handshake_signature_and_advertises_the_shared_schemes() {
        let verifier = NoVerifier;
        let signature = digitally_signed_struct();

        assert!(
            verifier
                .verify_tls12_signature(b"transcript", &certificate(), &signature)
                .is_ok(),
            "TLS 1.2 handshake signatures must be accepted unchecked"
        );
        assert!(
            verifier
                .verify_tls13_signature(b"transcript", &certificate(), &signature)
                .is_ok(),
            "TLS 1.3 handshake signatures must be accepted unchecked"
        );
        assert_eq!(
            verifier.supported_verify_schemes(),
            all_supported_verify_schemes()
        );
    }

    #[test]
    fn all_supported_verify_schemes_advertises_the_ten_expected_schemes_in_order() {
        let names: Vec<String> = all_supported_verify_schemes()
            .iter()
            .map(|scheme| format!("{scheme:?}"))
            .collect();

        assert_eq!(
            names,
            [
                "RSA_PKCS1_SHA256",
                "RSA_PKCS1_SHA384",
                "RSA_PKCS1_SHA512",
                "ECDSA_NISTP256_SHA256",
                "ECDSA_NISTP384_SHA384",
                "ECDSA_NISTP521_SHA512",
                "RSA_PSS_SHA256",
                "RSA_PSS_SHA384",
                "RSA_PSS_SHA512",
                "ED25519",
            ]
        );
    }

    #[test]
    fn all_supported_verify_schemes_excludes_the_sha1_schemes() {
        let schemes = all_supported_verify_schemes();

        assert!(!schemes.contains(&rustls::SignatureScheme::RSA_PKCS1_SHA1));
        assert!(!schemes.contains(&rustls::SignatureScheme::ECDSA_SHA1_Legacy));
    }

    #[test]
    fn no_verifier_accepts_a_certificate_it_has_no_reason_to_trust() {
        let verifier = NoVerifier;

        let result =
            verifier.verify_server_cert(&certificate(), &[], &server_name(), &[], fixed_now());

        assert!(result.is_ok(), "NoVerifier must accept any certificate");
    }

    #[test]
    fn no_verifier_accepts_a_certificate_presented_with_intermediates_and_ocsp() {
        let verifier = NoVerifier;
        let intermediate = CertificateDer::from(b"intermediate".to_vec());

        let result = verifier.verify_server_cert(
            &certificate(),
            std::slice::from_ref(&intermediate),
            &server_name(),
            b"ocsp-response",
            fixed_now(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn fingerprint_verifier_accepts_a_certificate_whose_sha256_matches() {
        let verifier = FingerprintVerifier::new(CERTIFICATE_SHA256_HEX);

        let result =
            verifier.verify_server_cert(&certificate(), &[], &server_name(), &[], fixed_now());

        assert!(
            result.is_ok(),
            "matching fingerprint should verify: {:?}",
            result.err()
        );
    }

    #[test]
    fn fingerprint_verifier_ignores_hostname_and_intermediates_when_the_hash_matches() {
        let verifier = FingerprintVerifier::new(CERTIFICATE_SHA256_HEX);
        let intermediate = CertificateDer::from(b"untrusted-intermediate".to_vec());

        let result = verifier.verify_server_cert(
            &certificate(),
            std::slice::from_ref(&intermediate),
            &ServerName::try_from("wrong.example.com").unwrap(),
            &[],
            fixed_now(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn fingerprint_verifier_rejects_a_certificate_whose_sha256_differs() {
        let expected = "0".repeat(64);
        let verifier = FingerprintVerifier::new(&expected);

        let error = verifier
            .verify_server_cert(&certificate(), &[], &server_name(), &[], fixed_now())
            .expect_err("mismatched fingerprint must be rejected");

        let rustls::Error::General(message) = error else {
            panic!("expected rustls::Error::General, got {error:?}");
        };
        assert_eq!(
            message,
            format!(
                "Certificate fingerprint mismatch: expected {expected}, got {CERTIFICATE_SHA256_HEX}"
            )
        );
    }

    #[test]
    fn fingerprint_verifier_rejects_the_correct_hash_in_uppercase_hex() {
        let verifier = FingerprintVerifier::new(&CERTIFICATE_SHA256_HEX.to_uppercase());

        let error = verifier
            .verify_server_cert(&certificate(), &[], &server_name(), &[], fixed_now())
            .expect_err("fingerprint comparison is case-sensitive lowercase hex");

        let rustls::Error::General(message) = error else {
            panic!("expected rustls::Error::General, got {error:?}");
        };
        assert!(
            message.contains(CERTIFICATE_SHA256_HEX),
            "error should report the lowercase hex actually computed: {message}"
        );
    }

    #[test]
    fn fingerprint_verifier_rejects_an_empty_expected_fingerprint() {
        let verifier = FingerprintVerifier::new("");

        let result =
            verifier.verify_server_cert(&certificate(), &[], &server_name(), &[], fixed_now());

        assert!(result.is_err());
    }

    /// Scenario: Certificate fingerprint requires a valid handshake signature
    #[test]
    fn fingerprint_verifier_rejects_a_handshake_signature_that_does_not_verify() {
        let verifier = FingerprintVerifier::new(CERTIFICATE_SHA256_HEX);
        let signature = digitally_signed_struct();

        assert!(verifier
            .verify_tls12_signature(b"transcript", &certificate(), &signature)
            .is_err());
        assert!(verifier
            .verify_tls13_signature(b"transcript", &certificate(), &signature)
            .is_err());
    }

    #[test]
    fn fingerprint_verifier_advertises_only_the_schemes_it_can_verify() {
        let verifier = FingerprintVerifier::new(CERTIFICATE_SHA256_HEX);
        let verifiable = rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes();

        let advertised = verifier.supported_verify_schemes();

        assert_eq!(advertised, verifiable);
        assert!(!advertised.contains(&rustls::SignatureScheme::RSA_PKCS1_SHA1));
        assert!(!advertised.contains(&rustls::SignatureScheme::ECDSA_SHA1_Legacy));
    }

    #[cfg(any(feature = "websocket", feature = "native"))]
    mod handshake {
        use super::super::{client_config, client_handshake};
        use crate::error::TransportError;
        use crate::transport::test_support::{finish_within, SilentServer, LOOPBACK_TEST_BOUND};
        use rcgen::{CertificateParams, KeyPair};
        use rustls::crypto::aws_lc_rs::sign::any_supported_type;
        use rustls::pki_types::PrivateKeyDer;
        use rustls::server::{ClientHello, ResolvesServerCert};
        use rustls::sign::CertifiedKey;
        use rustls::version::{TLS12, TLS13};
        use rustls::{ServerConfig, SupportedProtocolVersion};
        use sha2::{Digest, Sha256};
        use std::sync::Arc;
        use tokio::io::AsyncReadExt;
        use tokio::net::{TcpListener, TcpStream};
        use tokio_rustls::TlsAcceptor;

        /// The name the self-signed certificate of `SilentServer::after_tls` is issued for.
        const HOST: &str = "localhost";

        async fn handshake_with(
            server: &SilentServer,
            config: rustls::ClientConfig,
        ) -> Result<(), TransportError> {
            let tcp = TcpStream::connect((server.host.as_str(), server.port))
                .await
                .expect("connect to the loopback server");
            client_handshake(tcp, HOST, config).await.map(drop)
        }

        fn lowercase_sha256_hex(der: &[u8]) -> String {
            hex::encode(Sha256::digest(der))
        }

        #[tokio::test]
        async fn client_handshake_completes_with_no_verifier_or_a_matching_fingerprint() {
            finish_within(LOOPBACK_TEST_BOUND, async {
                let unverified = SilentServer::after_tls().await;
                handshake_with(&unverified, client_config(None, false))
                    .await
                    .expect("NoVerifier accepts the self-signed certificate");

                let pinned = SilentServer::after_tls().await;
                let fingerprint = lowercase_sha256_hex(pinned.certificate_der());
                handshake_with(&pinned, client_config(Some(&fingerprint), true))
                    .await
                    .expect("a matching fingerprint takes precedence over certificate validation");
            })
            .await;
        }

        #[tokio::test]
        async fn client_handshake_fails_with_a_tls_error_for_a_host_that_is_not_a_server_name() {
            finish_within(LOOPBACK_TEST_BOUND, async {
                let server = SilentServer::accepting().await;
                let tcp = TcpStream::connect((server.host.as_str(), server.port))
                    .await
                    .expect("connect to the loopback server");

                let result = client_handshake(tcp, "not a host name", client_config(None, false)).await;

                assert!(
                    matches!(&result, Err(TransportError::TlsError(message)) if message.starts_with("Invalid server name")),
                    "{:?}",
                    result.map(drop)
                );
            })
            .await;
        }

        #[tokio::test]
        async fn client_handshake_fails_with_a_tls_error_for_a_wrong_fingerprint_or_an_untrusted_certificate(
        ) {
            finish_within(LOOPBACK_TEST_BOUND, async {
                let wrong_fingerprint = "0".repeat(64);
                let configs = [
                    client_config(Some(&wrong_fingerprint), false),
                    client_config(None, true),
                ];

                for config in configs {
                    let server = SilentServer::after_tls().await;
                    let error = handshake_with(&server, config)
                        .await
                        .expect_err("the client must reject the certificate");
                    assert!(matches!(error, TransportError::TlsError(_)), "{error:?}");
                }
            })
            .await;
        }

        /// Serves one fixed certificate and signs the handshake with a key
        /// that need not belong to it, which `with_single_cert` would refuse.
        #[derive(Debug)]
        struct FixedCertificate(Arc<CertifiedKey>);

        impl ResolvesServerCert for FixedCertificate {
            fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
                Some(Arc::clone(&self.0))
            }
        }

        /// A loopback TLS server for one connection that presents the
        /// certificate of `certificate_key` and signs with `signing_key`.
        async fn pinned_server(
            certificate_key: &KeyPair,
            signing_key: &KeyPair,
            version: &'static SupportedProtocolVersion,
        ) -> (u16, String) {
            let certificate = CertificateParams::new(vec![HOST.to_string()])
                .expect("certificate params")
                .self_signed(certificate_key)
                .expect("self-sign the certificate");
            let certificate_der = certificate.der().clone();
            let fingerprint = lowercase_sha256_hex(&certificate_der);
            let signer =
                any_supported_type(&PrivateKeyDer::Pkcs8(signing_key.serialize_der().into()))
                    .expect("load the signing key");
            let resolver =
                FixedCertificate(Arc::new(CertifiedKey::new(vec![certificate_der], signer)));
            let server_config = ServerConfig::builder_with_protocol_versions(&[version])
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(resolver));
            let acceptor = TlsAcceptor::from(Arc::new(server_config));

            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind loopback");
            let port = listener.local_addr().expect("local address").port();
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.expect("accept the connection");
                // A client that rejects the handshake ends the peer here.
                if let Ok(mut tls_stream) = acceptor.accept(stream).await {
                    let mut buffer = [0u8; 64];
                    while matches!(tls_stream.read(&mut buffer).await, Ok(n) if n > 0) {}
                }
            });
            (port, fingerprint)
        }

        async fn pinned_handshake(
            certificate_key: &KeyPair,
            signing_key: &KeyPair,
            version: &'static SupportedProtocolVersion,
        ) -> Result<(), TransportError> {
            let (port, fingerprint) = pinned_server(certificate_key, signing_key, version).await;
            let tcp = TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect to the loopback server");
            client_handshake(tcp, HOST, client_config(Some(&fingerprint), false))
                .await
                .map(drop)
        }

        /// Scenario: Certificate fingerprint requires a valid handshake signature
        #[tokio::test]
        async fn pinned_handshake_fails_when_the_server_signs_with_another_key() {
            finish_within(LOOPBACK_TEST_BOUND, async {
                let mut outcomes = Vec::new();
                for version in [&TLS12, &TLS13] {
                    let certificate_key = KeyPair::generate().expect("certificate key");
                    let other_key = KeyPair::generate().expect("other key");
                    let result = pinned_handshake(&certificate_key, &other_key, version).await;
                    outcomes.push((version.version, result));
                }

                for (version, result) in &outcomes {
                    assert!(
                        matches!(result, Err(TransportError::TlsError(message)) if message.contains("BadSignature")),
                        "{version:?} must reject a handshake signed with another key: {outcomes:?}"
                    );
                }
            })
            .await;
        }

        /// Scenario: Certificate fingerprint requires a valid handshake signature
        #[tokio::test]
        async fn pinned_handshake_completes_when_the_server_signs_with_the_certificate_key() {
            finish_within(LOOPBACK_TEST_BOUND, async {
                for version in [&TLS12, &TLS13] {
                    let certificate_key = KeyPair::generate().expect("certificate key");

                    pinned_handshake(&certificate_key, &certificate_key, version)
                        .await
                        .unwrap_or_else(|error| panic!("{:?}: {error:?}", version.version));
                }
            })
            .await;
        }
    }
}
