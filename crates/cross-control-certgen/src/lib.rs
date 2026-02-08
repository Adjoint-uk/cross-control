//! TLS certificate generation for cross-control.
//!
//! Generates self-signed certificates for QUIC/TLS 1.3 connections.
//! Certificates are identified by their SHA-256 fingerprint for
//! trust-on-first-use pinning.

pub mod error;

pub use error::CertgenError;

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

/// A generated certificate and private key pair.
pub struct GeneratedCert {
    /// PEM-encoded certificate.
    pub cert_pem: String,
    /// PEM-encoded private key.
    pub key_pem: String,
    /// SHA-256 fingerprint of the DER-encoded certificate.
    pub fingerprint: String,
}

/// Generate a new self-signed certificate for cross-control.
///
/// The certificate is valid for the given hostname and includes
/// `localhost` and `127.0.0.1` as subject alternative names.
pub fn generate_certificate(hostname: &str) -> Result<GeneratedCert, CertgenError> {
    let key_pair = KeyPair::generate().map_err(|e| CertgenError::Generation(e.to_string()))?;

    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, hostname);
    dn.push(DnType::OrganizationName, "cross-control");
    params.distinguished_name = dn;

    params.subject_alt_names = vec![
        rcgen::SanType::DnsName(
            hostname
                .try_into()
                .map_err(|e: rcgen::Error| CertgenError::Generation(e.to_string()))?,
        ),
        rcgen::SanType::DnsName(
            "localhost"
                .try_into()
                .map_err(|e: rcgen::Error| CertgenError::Generation(e.to_string()))?,
        ),
        rcgen::SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
    ];

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| CertgenError::Generation(e.to_string()))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let fingerprint = sha256_fingerprint(cert.der());

    Ok(GeneratedCert {
        cert_pem,
        key_pem,
        fingerprint,
    })
}

/// Compute SHA-256 fingerprint of DER-encoded certificate bytes.
fn sha256_fingerprint(der: &[u8]) -> String {
    use std::fmt::Write;
    let digest = ring::digest::digest(&ring::digest::SHA256, der);
    let mut fingerprint = String::from("SHA256:");
    for (i, byte) in digest.as_ref().iter().enumerate() {
        if i > 0 {
            fingerprint.push(':');
        }
        let _ = write!(fingerprint, "{byte:02x}");
    }
    fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_cert_succeeds() {
        let cert = generate_certificate("test-machine").unwrap();
        assert!(cert.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(cert.key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(cert.fingerprint.starts_with("SHA256:"));
    }

    #[test]
    fn generate_cert_different_each_time() {
        let a = generate_certificate("machine-a").unwrap();
        let b = generate_certificate("machine-b").unwrap();
        assert_ne!(a.cert_pem, b.cert_pem);
        assert_ne!(a.key_pem, b.key_pem);
    }
}
