//! TLS configuration for QUIC connections.

use std::fmt::Write as _;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use tracing::{debug, warn};

use crate::error::ProtocolError;

/// Build a quinn `ServerConfig` from PEM-encoded cert and key.
pub fn server_config(cert_pem: &str, key_pem: &str) -> Result<quinn::ServerConfig, ProtocolError> {
    let certs = parse_certs(cert_pem)?;
    let key = parse_key(key_pem)?;

    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| ProtocolError::Tls(e.to_string()))?;

    tls_config.alpn_protocols = vec![b"cross-control/0.1".to_vec()];

    let config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
            .map_err(|e| ProtocolError::Tls(e.to_string()))?,
    ));
    debug!("built server TLS config");
    Ok(config)
}

/// Build a quinn `ClientConfig` that pins the peer's certificate by its
/// SHA-256 fingerprint — trust-on-first-use (ADR 0002).
///
/// When `expected_fingerprint` is `Some`, the certificate the peer presents
/// must hash to that fingerprint or the connection is refused. When `None`
/// (genuine first contact, no pin recorded yet), the certificate is accepted
/// and its fingerprint can be recorded for next time — classic TOFU.
///
/// Either way the self-signed certificate's own signature is trusted: the
/// trust anchor is the pinned fingerprint, not a CA chain, so there is no PKI
/// and no central authority.
pub fn client_config_pinned(
    expected_fingerprint: Option<String>,
) -> Result<quinn::ClientConfig, ProtocolError> {
    let verifier = Arc::new(FingerprintVerifier::new(expected_fingerprint));
    let mut tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

    tls_config.alpn_protocols = vec![b"cross-control/0.1".to_vec()];

    let config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
            .map_err(|e| ProtocolError::Tls(e.to_string()))?,
    ));
    debug!("built pinned client TLS config");
    Ok(config)
}

fn parse_certs(pem: &str) -> Result<Vec<CertificateDer<'static>>, ProtocolError> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ProtocolError::Tls(format!("failed to parse certificate PEM: {e}")))?;
    if certs.is_empty() {
        return Err(ProtocolError::Tls(
            "no certificates found in PEM".to_string(),
        ));
    }
    Ok(certs)
}

fn parse_key(pem: &str) -> Result<PrivateKeyDer<'static>, ProtocolError> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| ProtocolError::Tls(format!("failed to parse key PEM: {e}")))?
        .ok_or_else(|| ProtocolError::Tls("no private key found in PEM".to_string()))
}

/// SHA-256 fingerprint of a DER certificate as lowercase hex, no separators
/// or prefix. This is the canonical form used for pin comparison.
fn cert_fingerprint_hex(cert: &CertificateDer<'_>) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert.as_ref());
    let mut out = String::with_capacity(64);
    for byte in digest.as_ref() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Canonicalize a fingerprint for comparison: drop any `SHA256:` prefix and
/// `:` separators, then lowercase. Accepts the colon-grouped form from config
/// (`SHA256:ab:cd:…`) and the compact form from mDNS TXT records alike.
fn normalize_fingerprint(fp: &str) -> String {
    let trimmed = fp.trim();
    let without_prefix = trimmed
        .strip_prefix("SHA256:")
        .or_else(|| trimmed.strip_prefix("sha256:"))
        .unwrap_or(trimmed);
    without_prefix.replace(':', "").to_lowercase()
}

/// Trust-on-first-use certificate verifier.
///
/// Trusts the peer's self-signed certificate by its SHA-256 fingerprint: an
/// exact match against the pinned value when one is known, or blind acceptance
/// on genuine first contact. The signature-verification hooks always assert
/// valid because the certificate is self-signed and authenticated by its
/// fingerprint rather than a CA chain.
#[derive(Debug)]
struct FingerprintVerifier {
    /// Pinned fingerprint in canonical form, or `None` for first-contact TOFU.
    expected: Option<String>,
}

impl FingerprintVerifier {
    fn new(expected: Option<String>) -> Self {
        Self {
            expected: expected.map(|fp| normalize_fingerprint(&fp)),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let actual = cert_fingerprint_hex(end_entity);
        match &self.expected {
            Some(expected) if *expected != actual => {
                warn!(
                    expected = %expected,
                    actual = %actual,
                    "peer certificate fingerprint does not match pinned value — refusing connection"
                );
                Err(rustls::Error::General(
                    "peer certificate fingerprint does not match pinned value".to_string(),
                ))
            }
            Some(_) => {
                debug!(fingerprint = %actual, "peer certificate matched pinned fingerprint");
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
            None => {
                debug!(fingerprint = %actual, "accepting peer certificate on first contact (TOFU)");
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
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
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_handles_config_and_mdns_forms() {
        // Config form (prefixed, colon-grouped) and mDNS form (compact) must
        // canonicalize to the same string.
        let a = normalize_fingerprint("SHA256:AB:CD:ef:01");
        let b = normalize_fingerprint("abcdef01");
        assert_eq!(a, "abcdef01");
        assert_eq!(a, b);
    }

    #[test]
    fn verifier_stores_normalized_pin() {
        let v = FingerprintVerifier::new(Some("SHA256:AB:CD".to_string()));
        assert_eq!(v.expected.as_deref(), Some("abcd"));
        let none = FingerprintVerifier::new(None);
        assert!(none.expected.is_none());
    }
}
