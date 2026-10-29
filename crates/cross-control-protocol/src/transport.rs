//! QUIC transport: bind, accept, and connect.

use std::net::SocketAddr;

use quinn::Endpoint;
use tracing::{debug, info};

use crate::connection::PeerConnection;
use crate::error::ProtocolError;
use crate::tls;

/// QUIC transport layer for cross-control.
///
/// A single endpoint acts as both server (accepting connections) and client
/// (connecting to peers).
#[derive(Clone)]
pub struct QuicTransport {
    endpoint: Endpoint,
}

impl QuicTransport {
    /// Bind a QUIC endpoint that can both accept and initiate connections.
    pub fn bind(addr: SocketAddr, cert_pem: &str, key_pem: &str) -> Result<Self, ProtocolError> {
        // Install the default crypto provider if not already done
        let _ = rustls::crypto::ring::default_provider().install_default();

        let server_config = tls::server_config(cert_pem, key_pem)?;
        // Default client config pins nothing (first-contact TOFU); callers that
        // know the peer's fingerprint use `connect_pinned` to enforce it.
        let client_config = tls::client_config_pinned(None)?;

        let mut endpoint = Endpoint::server(server_config, addr)
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        endpoint.set_default_client_config(client_config);

        info!(addr = %addr, "QUIC transport bound");
        Ok(Self { endpoint })
    }

    /// Accept an incoming connection.
    pub async fn accept(&self) -> Result<PeerConnection, ProtocolError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| ProtocolError::Connection("endpoint closed".to_string()))?;

        let connection = incoming
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;

        let remote = connection.remote_address();
        debug!(remote = %remote, "accepted connection");
        Ok(PeerConnection::new(connection))
    }

    /// Connect to a remote peer, using the default (first-contact) client
    /// config — the presented certificate is accepted whatever its fingerprint.
    pub async fn connect(
        &self,
        addr: SocketAddr,
        server_name: &str,
    ) -> Result<PeerConnection, ProtocolError> {
        let connection = self
            .endpoint
            .connect(addr, server_name)
            .map_err(|e| ProtocolError::Connection(e.to_string()))?
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;

        debug!(remote = %addr, "connected to peer");
        Ok(PeerConnection::new(connection))
    }

    /// Connect to a remote peer, pinning its certificate by SHA-256
    /// fingerprint. When `expected_fingerprint` is `Some`, the TLS handshake
    /// fails unless the peer presents a certificate with that fingerprint;
    /// when `None`, this behaves like [`connect`](Self::connect) (first-contact
    /// TOFU).
    pub async fn connect_pinned(
        &self,
        addr: SocketAddr,
        server_name: &str,
        expected_fingerprint: Option<String>,
    ) -> Result<PeerConnection, ProtocolError> {
        let client_config = tls::client_config_pinned(expected_fingerprint)?;
        let connection = self
            .endpoint
            .connect_with(client_config, addr, server_name)
            .map_err(|e| ProtocolError::Connection(e.to_string()))?
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;

        debug!(remote = %addr, "connected to peer (pinned)");
        Ok(PeerConnection::new(connection))
    }

    /// Get the local address this transport is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, ProtocolError> {
        self.endpoint
            .local_addr()
            .map_err(|e| ProtocolError::Connection(e.to_string()))
    }

    /// Gracefully shut down the transport.
    pub fn close(&self) {
        self.endpoint.close(quinn::VarInt::from_u32(0), b"shutdown");
        info!("QUIC transport closed");
    }
}
