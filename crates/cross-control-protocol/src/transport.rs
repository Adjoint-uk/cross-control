//! QUIC transport: bind, accept, and connect.

use std::net::SocketAddr;
use std::sync::Arc;

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
    /// Our certificate and key (PEM), presented on every outbound connection.
    identity: Arc<Identity>,
}

struct Identity {
    cert_pem: String,
    key_pem: String,
}

impl QuicTransport {
    /// Bind a QUIC endpoint that can both accept and initiate connections.
    pub fn bind(addr: SocketAddr, cert_pem: &str, key_pem: &str) -> Result<Self, ProtocolError> {
        // Install the default crypto provider if not already done
        let _ = rustls::crypto::ring::default_provider().install_default();

        let server_config = tls::server_config(cert_pem, key_pem)?;
        // Default client config pins nothing; callers that know the peer's
        // fingerprint use `connect_pinned` to enforce it at the TLS layer.
        let client_config = tls::client_config_pinned(cert_pem, key_pem, None)?;

        let mut endpoint = Endpoint::server(server_config, addr)
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        endpoint.set_default_client_config(client_config);

        info!(addr = %addr, "QUIC transport bound");
        Ok(Self {
            endpoint,
            identity: Arc::new(Identity {
                cert_pem: cert_pem.to_string(),
                key_pem: key_pem.to_string(),
            }),
        })
    }

    /// Accept an incoming connection. The peer has presented a certificate
    /// and proved it holds the key; whether that certificate is *trusted* is
    /// for the caller to decide from
    /// [`PeerConnection::peer_fingerprint`].
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

    /// Connect to a remote peer without pinning its certificate. The caller
    /// must check [`PeerConnection::peer_fingerprint`] before trusting it.
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
    /// when `None`, this behaves like [`connect`](Self::connect).
    pub async fn connect_pinned(
        &self,
        addr: SocketAddr,
        server_name: &str,
        expected_fingerprint: Option<String>,
    ) -> Result<PeerConnection, ProtocolError> {
        let client_config = tls::client_config_pinned(
            &self.identity.cert_pem,
            &self.identity.key_pem,
            expected_fingerprint,
        )?;
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
