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

/// Emits the [`rustls::client::danger::ServerCertVerifier`] members that every
/// verifier in this module shares.
///
/// Both verifiers deliberately skip handshake-signature checking — they
/// establish trust from the certificate itself (or from the operator's explicit
/// decision to skip validation), never from the signature — and both advertise
/// the same scheme list. Only `verify_server_cert` differs, so it stays
/// hand-written per verifier while these three members are generated once.
macro_rules! accept_any_handshake_signature {
    () => {
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
    };
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

    accept_any_handshake_signature!();
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

    accept_any_handshake_signature!();
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
    /// does, delete this function and the two `verify_tls1{2,3}_signature`
    /// assertions in `assert_shared_verifier_members_are_permissive` below —
    /// keep its `supported_verify_schemes` assertion, which needs no internal
    /// API and does not depend on this helper.
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

    /// Asserts the members the `accept_any_handshake_signature!` macro generates
    /// for every verifier: neither handshake-signature check inspects anything,
    /// and the advertised scheme list is the shared one.
    fn assert_shared_verifier_members_are_permissive(verifier: &dyn ServerCertVerifier) {
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
    fn no_verifier_accepts_any_handshake_signature_and_advertises_the_shared_schemes() {
        assert_shared_verifier_members_are_permissive(&NoVerifier);
    }

    #[test]
    fn fingerprint_verifier_accepts_a_certificate_whose_sha256_matches() {
        let verifier = FingerprintVerifier {
            expected_fingerprint: CERTIFICATE_SHA256_HEX.to_string(),
        };

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
        let verifier = FingerprintVerifier {
            expected_fingerprint: CERTIFICATE_SHA256_HEX.to_string(),
        };
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
        let verifier = FingerprintVerifier {
            expected_fingerprint: expected.clone(),
        };

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
        let verifier = FingerprintVerifier {
            expected_fingerprint: CERTIFICATE_SHA256_HEX.to_uppercase(),
        };

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
        let verifier = FingerprintVerifier {
            expected_fingerprint: String::new(),
        };

        let result =
            verifier.verify_server_cert(&certificate(), &[], &server_name(), &[], fixed_now());

        assert!(result.is_err());
    }

    #[test]
    fn fingerprint_verifier_accepts_any_handshake_signature_and_advertises_the_shared_schemes() {
        assert_shared_verifier_members_are_permissive(&FingerprintVerifier {
            expected_fingerprint: CERTIFICATE_SHA256_HEX.to_string(),
        });
    }
}
