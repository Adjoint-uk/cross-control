//! TLS configuration for QUIC connections.

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use tracing::debug;

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

/// Build a quinn `ClientConfig` that skips certificate verification (MVP).
///
/// In Phase 2 this will be replaced with fingerprint-pinning verification.
pub fn client_config_skip_verification() -> Result<quinn::ClientConfig, ProtocolError> {
    let mut tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    tls_config.alpn_protocols = vec![b"cross-control/0.1".to_vec()];

    let config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
            .map_err(|e| ProtocolError::Tls(e.to_string()))?,
    ));
    debug!("built client TLS config (skip verification)");
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

/// Certificate verifier that accepts all server certificates (MVP only).
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
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
