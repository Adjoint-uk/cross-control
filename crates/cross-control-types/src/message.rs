//! Protocol message types.
//!
//! Messages are exchanged over QUIC streams between cross-control peers.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::clipboard::{ClipboardContent, ClipboardFormat};
use crate::device::{DeviceId, DeviceInfo};
use crate::event::InputEvent;
use crate::machine::MachineId;
use crate::screen::{ScreenEdge, ScreenGeometry};

/// Current protocol version.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Protocol version for compatibility negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Top-level message envelope.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub enum Message {
    Control(ControlMessage),
    Input(InputMessage),
    Clipboard(ClipboardMessage),
}

/// Control-plane messages (bidirectional, stream 0).
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub enum ControlMessage {
    /// Initial handshake from connecting peer.
    Hello {
        version: ProtocolVersion,
        machine_id: MachineId,
        name: String,
        screen: ScreenGeometry,
    },

    /// Response to Hello.
    Welcome {
        version: ProtocolVersion,
        machine_id: MachineId,
        name: String,
        screen: ScreenGeometry,
    },

    /// Announce a new input device.
    DeviceAnnounce(DeviceInfo),

    /// An input device was removed.
    DeviceGone { device_id: DeviceId },

    /// Screen geometry changed (e.g. monitor added/removed).
    ScreenUpdate(ScreenGeometry),

    /// Cursor is crossing to the remote machine.
    Enter {
        /// Which edge the cursor is leaving from.
        edge: ScreenEdge,
        /// Position along the edge (pixels).
        position: u32,
    },

    /// Acknowledge an Enter; remote is ready to receive input.
    EnterAck,

    /// Cursor is returning to the local machine.
    Leave {
        /// Which edge the cursor is entering on.
        edge: ScreenEdge,
        /// Position along the edge (pixels).
        position: u32,
    },

    /// Keepalive ping.
    Ping {
        /// Sequence number for RTT measurement.
        seq: u64,
    },

    /// Keepalive pong.
    Pong {
        /// Echoed sequence number.
        seq: u64,
    },

    /// Graceful disconnect.
    Bye,
}

/// Input data messages (unidirectional, controller -> controlled).
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct InputMessage {
    /// Batch of events for efficiency (typically 1, but may batch at high rates).
    pub device_id: DeviceId,
    pub timestamp_us: u64,
    pub events: Vec<InputEvent>,
}

/// Clipboard synchronisation messages (bidirectional, on demand).
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub enum ClipboardMessage {
    /// Advertise that the clipboard has content available.
    Offer {
        formats: Vec<ClipboardFormat>,
        /// Size in bytes (hint for the receiver).
        size_hint: u64,
    },

    /// Request clipboard content in a specific format.
    Request { format: ClipboardFormat },

    /// Clipboard content payload.
    Data(ClipboardContent),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceCapability;

    fn bincode_roundtrip<T: Encode + Decode<()> + std::fmt::Debug>(value: &T) -> T {
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(value, config).unwrap();
        let (decoded, _): (T, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        decoded
    }

    #[test]
    fn hello_roundtrip() {
        let msg = Message::Control(ControlMessage::Hello {
            version: PROTOCOL_VERSION,
            machine_id: MachineId::new(),
            name: "test-machine".to_string(),
            screen: ScreenGeometry::new(1920, 1080),
        });
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn welcome_roundtrip() {
        let msg = Message::Control(ControlMessage::Welcome {
            version: PROTOCOL_VERSION,
            machine_id: MachineId::new(),
            name: "remote".to_string(),
            screen: ScreenGeometry::new(2560, 1440),
        });
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn device_announce_roundtrip() {
        let msg = Message::Control(ControlMessage::DeviceAnnounce(DeviceInfo {
            id: DeviceId(1),
            name: "Keyboard".to_string(),
            capabilities: vec![DeviceCapability::Keyboard],
        }));
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn enter_leave_roundtrip() {
        let enter = Message::Control(ControlMessage::Enter {
            edge: ScreenEdge::Right,
            position: 540,
        });
        let _decoded = bincode_roundtrip(&enter);

        let leave = Message::Control(ControlMessage::Leave {
            edge: ScreenEdge::Left,
            position: 540,
        });
        let _decoded = bincode_roundtrip(&leave);
    }

    #[test]
    fn input_message_roundtrip() {
        use crate::event::{ButtonState, KeyCode};
        let msg = Message::Input(InputMessage {
            device_id: DeviceId(1),
            timestamp_us: 1_000_000,
            events: vec![
                InputEvent::Key {
                    code: KeyCode::KeyA,
                    state: ButtonState::Pressed,
                },
                InputEvent::Key {
                    code: KeyCode::KeyA,
                    state: ButtonState::Released,
                },
            ],
        });
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn clipboard_offer_roundtrip() {
        let msg = Message::Clipboard(ClipboardMessage::Offer {
            formats: vec![ClipboardFormat::PlainText, ClipboardFormat::Html],
            size_hint: 1024,
        });
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn clipboard_data_roundtrip() {
        let msg = Message::Clipboard(ClipboardMessage::Data(ClipboardContent::text(
            "shared text",
        )));
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn ping_pong_roundtrip() {
        let ping = Message::Control(ControlMessage::Ping { seq: 42 });
        let _decoded = bincode_roundtrip(&ping);

        let pong = Message::Control(ControlMessage::Pong { seq: 42 });
        let _decoded = bincode_roundtrip(&pong);
    }

    #[test]
    fn bye_roundtrip() {
        let msg = Message::Control(ControlMessage::Bye);
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn protocol_version_display() {
        assert_eq!(PROTOCOL_VERSION.to_string(), "0.1");
    }
}
