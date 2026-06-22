//! Shared rustls certificate verifiers used across the transport implementations.
//!
//! The WebSocket, HTTP-transport, and native-TCP code paths all need the same two
//! custom [`rustls::client::danger::ServerCertVerifier`] implementations:
//!
//! - [`NoVerifier`] accepts any certificate (used when validation is disabled).
//! - [`FingerprintVerifier`] validates by SHA-256 fingerprint of the DER-encoded
//!   certificate, bypassing hostname and CA-chain validation.
//!
//! Both report the same set of supported signature schemes via
//! [`all_supported_verify_schemes`].

use rustls::pki_types::CertificateDer;

/// The signature schemes advertised by the custom verifiers.
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

/// A certificate verifier that validates by SHA-256 fingerprint of the DER-encoded certificate.
/// Bypasses hostname and CA chain validation.
///
/// Only the WebSocket and native-TCP transports construct this; gate it on those
/// features so a `--no-default-features` build (HTTP transport only) does not warn.
#[cfg(any(feature = "websocket", feature = "native"))]
#[derive(Debug)]
pub(crate) struct FingerprintVerifier {
    pub(crate) expected_fingerprint: String,
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
