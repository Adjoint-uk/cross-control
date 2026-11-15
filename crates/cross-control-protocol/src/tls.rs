//! TLS configuration for QUIC connections.
//!
//! cross-control has no certificate authority. Every machine has a
//! self-signed certificate, and a peer is identified by the SHA-256
//! fingerprint of that certificate (ADR 0002). Both directions are
//! authenticated (mutual TLS):
//!
//! - the **client** checks the server's certificate against a pinned
//!   fingerprint when it has one;
//! - the **server** requires a client certificate, and the daemon decides
//!   after the handshake whether that fingerprint is trusted.
//!
//! In both directions the TLS handshake signature is verified against the
//! presented certificate, so a peer cannot claim a trusted fingerprint by
//! replaying someone else's (public) certificate: it must hold the private
//! key behind it.

use std::fmt::Write as _;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use tracing::{debug, warn};

use crate::error::ProtocolError;

/// ALPN identifier for the cross-control protocol.
const ALPN: &[u8] = b"cross-control/0.1";

/// Build a quinn `ServerConfig` from PEM-encoded cert and key.
///
/// Clients must present a certificate and prove they hold its key. Which
/// certificates are *trusted* is not decided here: the daemon checks the
/// fingerprint (see `PeerConnection::peer_fingerprint`) once the handshake
/// completes, against its own trust store.
pub fn server_config(cert_pem: &str, key_pem: &str) -> Result<quinn::ServerConfig, ProtocolError> {
    let certs = parse_certs(cert_pem)?;
    let key = parse_key(key_pem)?;

    let mut tls_config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(Arc::new(PeerCertVerifier::new()))
        .with_single_cert(certs, key)
        .map_err(|e| ProtocolError::Tls(e.to_string()))?;

    tls_config.alpn_protocols = vec![ALPN.to_vec()];
    quic_server_config(tls_config)
}

fn quic_server_config(
    tls_config: rustls::ServerConfig,
) -> Result<quinn::ServerConfig, ProtocolError> {
    let config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
            .map_err(|e| ProtocolError::Tls(e.to_string()))?,
    ));
    debug!("built server TLS config");
    Ok(config)
}

/// Build a quinn `ClientConfig` that presents our certificate (mutual TLS)
/// and pins the server's certificate by its SHA-256 fingerprint.
///
/// When `expected_fingerprint` is `Some`, the certificate the server presents
/// must hash to that fingerprint or the connection is refused. When `None`,
/// any certificate is accepted at the TLS layer and the caller must decide
/// afterwards whether `peer_fingerprint()` is trusted (first contact).
///
/// Either way the server must prove possession of the certificate's private
/// key: the handshake signature is always verified.
pub fn client_config_pinned(
    cert_pem: &str,
    key_pem: &str,
    expected_fingerprint: Option<String>,
) -> Result<quinn::ClientConfig, ProtocolError> {
    let certs = parse_certs(cert_pem)?;
    let key = parse_key(key_pem)?;
    let verifier = Arc::new(FingerprintVerifier::new(expected_fingerprint));
    let mut tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(certs, key)
        .map_err(|e| ProtocolError::Tls(e.to_string()))?;

    tls_config.alpn_protocols = vec![ALPN.to_vec()];
    quic_client_config(tls_config)
}

fn quic_client_config(
    tls_config: rustls::ClientConfig,
) -> Result<quinn::ClientConfig, ProtocolError> {
    let config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
            .map_err(|e| ProtocolError::Tls(e.to_string()))?,
    ));
    debug!("built pinned client TLS config");
    Ok(config)
}

fn parse_certs(pem: &str) -> Result<Vec<CertificateDer<'static>>, ProtocolError> {
    let certs = CertificateDer::pem_slice_iter(pem.as_bytes())
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
    PrivateKeyDer::from_pem_slice(pem.as_bytes())
        .map_err(|e| ProtocolError::Tls(format!("failed to parse key PEM: {e}")))
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

/// SHA-256 fingerprint of a DER certificate in the display form
/// `SHA256:aa:bb:cc:…` — the same shape used in `config.toml` and `status`,
/// so it can be copy-pasted into a `[[screens]] fingerprint` entry.
pub fn fingerprint_display(cert: &CertificateDer<'_>) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert.as_ref());
    let mut out = String::from("SHA256:");
    for (i, byte) in digest.as_ref().iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Canonicalize a fingerprint for comparison: drop any `SHA256:` prefix and
/// `:` separators, then lowercase. Accepts the colon-grouped form from config
/// (`SHA256:ab:cd:…`) and the compact form from mDNS TXT records alike.
#[must_use]
pub fn normalize_fingerprint(fp: &str) -> String {
    let trimmed = fp.trim();
    let without_prefix = trimmed
        .strip_prefix("SHA256:")
        .or_else(|| trimmed.strip_prefix("sha256:"))
        .unwrap_or(trimmed);
    without_prefix.replace(':', "").to_lowercase()
}

/// Signature algorithms of the process-wide crypto provider (ring).
fn signature_algorithms() -> WebPkiSupportedAlgorithms {
    rustls::crypto::ring::default_provider().signature_verification_algorithms
}

/// Server-certificate verifier: pins by fingerprint, and verifies the
/// handshake signature so the server must hold the certificate's key.
#[derive(Debug)]
struct FingerprintVerifier {
    /// Pinned fingerprint in canonical form, or `None` for first contact.
    expected: Option<String>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl FingerprintVerifier {
    fn new(expected: Option<String>) -> Self {
        Self {
            expected: expected.map(|fp| normalize_fingerprint(&fp)),
            algorithms: signature_algorithms(),
        }
    }
}

impl ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
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
                Ok(ServerCertVerified::assertion())
            }
            None => {
                debug!(fingerprint = %actual, "no pin for peer; authorization happens after the handshake");
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Client-certificate verifier: a certificate is mandatory and the client
/// must prove it holds the key. Any self-signed certificate passes this
/// layer; the daemon authorizes the fingerprint after the handshake.
#[derive(Debug)]
struct PeerCertVerifier {
    algorithms: WebPkiSupportedAlgorithms,
}

impl PeerCertVerifier {
    fn new() -> Self {
        Self {
            algorithms: signature_algorithms(),
        }
    }
}

impl ClientCertVerifier for PeerCertVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        debug!(fingerprint = %cert_fingerprint_hex(end_entity), "client presented certificate");
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::sign::CertifiedKey;
    use std::net::SocketAddr;

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

    #[test]
    fn verifiers_offer_real_signature_schemes() {
        let schemes = FingerprintVerifier::new(None).supported_verify_schemes();
        assert!(schemes.contains(&SignatureScheme::ECDSA_NISTP256_SHA256));
        assert_eq!(schemes, PeerCertVerifier::new().supported_verify_schemes());
    }

    /// A certified key that presents `cert_of`'s certificate but signs with
    /// `key_of`'s private key — an impostor replaying a public certificate.
    fn mismatched_key(
        cert_of: &cross_control_certgen::GeneratedCert,
        key_of: &cross_control_certgen::GeneratedCert,
    ) -> Arc<CertifiedKey> {
        let certs = parse_certs(&cert_of.cert_pem).unwrap();
        let key = parse_key(&key_of.key_pem).unwrap();
        let signer = rustls::crypto::ring::sign::any_supported_type(&key).unwrap();
        Arc::new(CertifiedKey::new(certs, signer))
    }

    #[derive(Debug)]
    struct FixedServerCert(Arc<CertifiedKey>);
    impl rustls::server::ResolvesServerCert for FixedServerCert {
        fn resolve(&self, _hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(Arc::clone(&self.0))
        }
    }

    #[derive(Debug)]
    struct FixedClientCert(Arc<CertifiedKey>);
    impl rustls::client::ResolvesClientCert for FixedClientCert {
        fn resolve(
            &self,
            _hints: &[&[u8]],
            _schemes: &[SignatureScheme],
        ) -> Option<Arc<CertifiedKey>> {
            Some(Arc::clone(&self.0))
        }
        fn has_certs(&self) -> bool {
            true
        }
    }

    fn install_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    fn localhost() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    /// Run one handshake between a server and client endpoint. Returns the
    /// client-side result and, if the server accepted, the fingerprint of
    /// the client certificate it saw.
    async fn handshake(
        server: quinn::ServerConfig,
        client: quinn::ClientConfig,
    ) -> (Result<(), String>, Option<String>) {
        let server_ep = quinn::Endpoint::server(server, localhost()).unwrap();
        let addr = server_ep.local_addr().unwrap();
        let accept = tokio::spawn(async move {
            let incoming = server_ep.accept().await?;
            let conn = incoming.await.ok()?;
            let certs = conn
                .peer_identity()?
                .downcast::<Vec<CertificateDer<'static>>>()
                .ok()?;
            let fp = certs.first().map(fingerprint_display);
            // Keep the connection open until the client has finished.
            conn.closed().await;
            fp
        });

        let mut client_ep = quinn::Endpoint::client(localhost()).unwrap();
        client_ep.set_default_client_config(client);
        let result = match client_ep.connect(addr, "cross-control").unwrap().await {
            Ok(conn) => {
                // A rejected client certificate surfaces after the client
                // thinks the handshake is done: give the server a moment.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let outcome = match conn.close_reason() {
                    Some(reason) => Err(reason.to_string()),
                    None => Ok(()),
                };
                conn.close(0u32.into(), b"done");
                outcome
            }
            Err(e) => Err(e.to_string()),
        };
        let seen = tokio::time::timeout(std::time::Duration::from_secs(2), accept)
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        (result, seen)
    }

    #[tokio::test]
    async fn mutual_tls_exposes_client_fingerprint_to_server() {
        install_provider();
        let server = cross_control_certgen::generate_certificate("server").unwrap();
        let client = cross_control_certgen::generate_certificate("client").unwrap();

        let (result, seen) = handshake(
            server_config(&server.cert_pem, &server.key_pem).unwrap(),
            client_config_pinned(
                &client.cert_pem,
                &client.key_pem,
                Some(server.fingerprint.clone()),
            )
            .unwrap(),
        )
        .await;
        assert!(result.is_ok(), "handshake failed: {result:?}");
        assert_eq!(seen.as_deref(), Some(client.fingerprint.as_str()));
    }

    #[tokio::test]
    async fn server_rejects_client_without_certificate() {
        install_provider();
        let server = cross_control_certgen::generate_certificate("server").unwrap();
        let mut tls = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(FingerprintVerifier::new(None)))
            .with_no_client_auth();
        tls.alpn_protocols = vec![ALPN.to_vec()];

        let (result, seen) = handshake(
            server_config(&server.cert_pem, &server.key_pem).unwrap(),
            quic_client_config(tls).unwrap(),
        )
        .await;
        assert!(result.is_err(), "anonymous client must be refused");
        assert_eq!(seen, None);
    }

    /// Regression: the pinned fingerprint is public (it is in mDNS adverts
    /// and config files), so presenting the pinned certificate must not be
    /// enough — the server has to sign with its key.
    #[tokio::test]
    async fn server_replaying_pinned_certificate_is_refused() {
        install_provider();
        let genuine = cross_control_certgen::generate_certificate("genuine").unwrap();
        let impostor = cross_control_certgen::generate_certificate("impostor").unwrap();
        let client = cross_control_certgen::generate_certificate("client").unwrap();

        let mut tls = rustls::ServerConfig::builder()
            .with_client_cert_verifier(Arc::new(PeerCertVerifier::new()))
            .with_cert_resolver(Arc::new(FixedServerCert(mismatched_key(
                &genuine, &impostor,
            ))));
        tls.alpn_protocols = vec![ALPN.to_vec()];

        let (result, _) = handshake(
            quic_server_config(tls).unwrap(),
            client_config_pinned(
                &client.cert_pem,
                &client.key_pem,
                Some(genuine.fingerprint.clone()),
            )
            .unwrap(),
        )
        .await;
        assert!(result.is_err(), "forged server signature was accepted");
    }

    /// Same attack in the other direction: a client claiming a trusted
    /// peer's certificate without its key.
    #[tokio::test]
    async fn client_replaying_trusted_certificate_is_refused() {
        install_provider();
        let server = cross_control_certgen::generate_certificate("server").unwrap();
        let genuine = cross_control_certgen::generate_certificate("genuine").unwrap();
        let impostor = cross_control_certgen::generate_certificate("impostor").unwrap();

        let mut tls = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(FingerprintVerifier::new(None)))
            .with_client_cert_resolver(Arc::new(FixedClientCert(mismatched_key(
                &genuine, &impostor,
            ))));
        tls.alpn_protocols = vec![ALPN.to_vec()];

        let (result, seen) = handshake(
            server_config(&server.cert_pem, &server.key_pem).unwrap(),
            quic_client_config(tls).unwrap(),
        )
        .await;
        assert!(result.is_err(), "forged client signature was accepted");
        assert_eq!(seen, None, "server must not expose an unproven identity");
    }
}
