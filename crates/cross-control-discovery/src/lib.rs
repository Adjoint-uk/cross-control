//! mDNS/DNS-SD zero-config discovery for cross-control.
//!
//! Defines the [`Discovery`] trait for advertising and browsing cross-control
//! peers on the local network. The mdns-sd backend will be added in Phase 2.

use async_trait::async_trait;
use cross_control_types::MachineId;

pub mod error;

pub use error::DiscoveryError;

/// A discovered peer on the network.
#[derive(Debug, Clone)]
pub struct Peer {
    /// Machine identifier.
    pub machine_id: MachineId,
    /// Human-readable name.
    pub name: String,
    /// Network address (host:port).
    pub address: std::net::SocketAddr,
    /// TLS certificate fingerprint (SHA-256).
    pub fingerprint: Option<String>,
}

/// Network discovery for cross-control peers.
#[async_trait]
pub trait Discovery: Send + 'static {
    /// Start advertising this machine on the network.
    async fn advertise(
        &mut self,
        machine_id: MachineId,
        name: &str,
        port: u16,
    ) -> Result<(), DiscoveryError>;

    /// Stop advertising.
    async fn stop_advertising(&mut self) -> Result<(), DiscoveryError>;

    /// Start browsing for peers, sending discoveries to the returned receiver.
    async fn browse(
        &mut self,
    ) -> Result<tokio::sync::mpsc::Receiver<DiscoveryEvent>, DiscoveryError>;

    /// Stop browsing.
    async fn stop_browsing(&mut self) -> Result<(), DiscoveryError>;
}

/// Events from the discovery subsystem.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// A new peer was found.
    PeerFound(Peer),
    /// A previously known peer disappeared.
    PeerLost(MachineId),
}
