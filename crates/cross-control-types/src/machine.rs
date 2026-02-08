//! Machine identity types.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a machine in the cross-control network.
///
/// Wraps a UUID v4 but serialises as raw bytes for bincode efficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub struct MachineId(#[bincode(with_serde)] Uuid);

impl MachineId {
    /// Generate a new random machine ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a machine ID from an existing UUID.
    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID.
    #[must_use]
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for MachineId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MachineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_id_unique() {
        let a = MachineId::new();
        let b = MachineId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn machine_id_display() {
        let id = MachineId::new();
        let s = id.to_string();
        assert!(!s.is_empty());
        // UUID v4 format: 8-4-4-4-12
        assert_eq!(s.len(), 36);
    }

    #[test]
    fn machine_id_bincode_roundtrip() {
        let id = MachineId::new();
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&id, config).unwrap();
        let (decoded, _): (MachineId, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn machine_id_serde_roundtrip() {
        let id = MachineId::new();
        let json = serde_json::to_string(&id).unwrap();
        let decoded: MachineId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, decoded);
    }
}
