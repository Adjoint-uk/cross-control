//! Zero-config peer discovery for cross-control.
//!
//! The [`Discovery`] trait abstracts over discovery backends. Two ship today:
//!
//! - [`mdns::MdnsDiscovery`] — DNS-SD over multicast for LAN peers
//! - [`aggregator::StaticDiscovery`] — a backend that emits a fixed peer list
//!   from the daemon config
//!
//! [`aggregator::DiscoveryAggregator`] composes backends so the daemon only
//! sees one merged, de-duplicated peer stream.

use async_trait::async_trait;
use cross_control_types::MachineId;

pub mod aggregator;
pub mod error;
pub mod mdns;

pub use aggregator::{DiscoveryAggregator, StaticDiscovery};
pub use error::DiscoveryError;
pub use mdns::MdnsDiscovery;

/// A discovered peer on the network.
#[derive(Debug, Clone)]
pub struct Peer {
    /// Machine identifier.
    pub machine_id: MachineId,
    /// Human-readable name.
    pub name: String,
    /// Network address (host:port).
    pub address: std::net::SocketAddr,
    /// TLS certificate fingerprint (SHA-256, lowercase hex).
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
