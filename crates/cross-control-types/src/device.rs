//! Device descriptor types.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Opaque ID for a physical input device on the source machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub struct DeviceId(pub u32);

/// Opaque ID for a virtual input device on the destination machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub struct VirtualDeviceId(pub u32);

/// Describes a physical input device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct DeviceInfo {
    /// Local device ID.
    pub id: DeviceId,
    /// Human-readable name (e.g. "Logitech MX Master 3").
    pub name: String,
    /// What this device can do.
    pub capabilities: Vec<DeviceCapability>,
}

/// What kind of input a device supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum DeviceCapability {
    Keyboard,
    RelativeMouse,
    AbsoluteMouse,
    Scroll,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_info_roundtrip() {
        let info = DeviceInfo {
            id: DeviceId(7),
            name: "Test Keyboard".to_string(),
            capabilities: vec![DeviceCapability::Keyboard],
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&info, config).unwrap();
        let (decoded, _): (DeviceInfo, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(info, decoded);
    }

    #[test]
    fn device_info_multi_capability() {
        let info = DeviceInfo {
            id: DeviceId(1),
            name: "Gaming Mouse".to_string(),
            capabilities: vec![DeviceCapability::RelativeMouse, DeviceCapability::Scroll],
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&info, config).unwrap();
        let (decoded, _): (DeviceInfo, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(info, decoded);
    }
}
