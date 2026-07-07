//! Live daemon status snapshot, shared with the CLI via a small JSON file.
//!
//! The daemon owns all peer state inside its single-threaded event loop, so
//! there is no shared-memory channel a separate `cross-control status`
//! process could read. Rather than stand up a full IPC socket server, the
//! daemon periodically serialises a [`StatusSnapshot`] to a file in the
//! runtime directory (next to the PID file) and the CLI deserialises it.
//!
//! This mirrors the existing PID-file pattern: cheap, crash-safe (a stale
//! file is simply ignored once the PID is gone), and good enough for a
//! human-facing status readout. A richer query/subscribe IPC channel can
//! replace it later without changing the wire types the CLI renders.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A point-in-time view of the running daemon, written to disk for the CLI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// This machine's configured name.
    pub name: String,
    /// Peer we are currently controlling (sending input to), if any.
    pub controlling: Option<String>,
    /// Peer currently controlling us (sending us input), if any.
    pub controlled_by: Option<String>,
    /// Every peer we currently hold a session with.
    pub peers: Vec<PeerStatus>,
}

/// Per-peer status line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatus {
    /// Peer's advertised name.
    pub name: String,
    /// Peer's machine id (UUID string).
    pub machine_id: String,
    /// Remote socket address of the QUIC connection.
    pub address: String,
    /// Session state (`Idle`, `Controlling`, `Controlled`, …).
    pub state: String,
    /// Last measured round-trip time in milliseconds, if a ping has
    /// completed a full round trip yet.
    pub latency_ms: Option<u64>,
}

impl StatusSnapshot {
    /// Serialise to pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Parse from JSON.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

/// Path of the status snapshot file. Sits alongside the PID file so both
/// share the same lifetime and cleanup story.
pub fn status_file_path() -> PathBuf {
    dirs::runtime_dir()
        .or_else(dirs::state_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("cross-control.status.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_json_round_trip() {
        let snap = StatusSnapshot {
            name: "center".to_string(),
            controlling: Some("laptop-right".to_string()),
            controlled_by: None,
            peers: vec![PeerStatus {
                name: "laptop-right".to_string(),
                machine_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
                address: "192.168.1.42:24800".to_string(),
                state: "Controlling".to_string(),
                latency_ms: Some(3),
            }],
        };
        let json = snap.to_json().unwrap();
        let back = StatusSnapshot::from_json(&json).unwrap();
        assert_eq!(back.name, "center");
        assert_eq!(back.controlling.as_deref(), Some("laptop-right"));
        assert_eq!(back.peers.len(), 1);
        assert_eq!(back.peers[0].latency_ms, Some(3));
    }

    #[test]
    fn empty_snapshot_round_trips() {
        let snap = StatusSnapshot::default();
        let json = snap.to_json().unwrap();
        let back = StatusSnapshot::from_json(&json).unwrap();
        assert!(back.peers.is_empty());
        assert!(back.controlling.is_none());
    }
}
