//! Protocol message types.
//!
//! Messages are exchanged over QUIC streams between cross-control peers.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::clipboard::{ClipboardContent, ClipboardFormat};
use crate::device::{DeviceId, DeviceInfo};
use crate::event::InputEvent;
use crate::machine::MachineId;
use crate::screen::{DisplayLayout, ScreenEdge};

/// Current protocol version.
///
/// - 0.2: `Hello`/`Welcome`/`ScreenUpdate` carry a [`DisplayLayout`]
///   (multi-monitor) instead of a single `ScreenGeometry`.
/// - 0.3: `Enter` carries `exit_edges`, positions are edge-relative, and
///   `EnterReject` lets a busy peer refuse a hand-off.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 3 };

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
        layout: DisplayLayout,
    },

    /// Response to Hello.
    Welcome {
        version: ProtocolVersion,
        machine_id: MachineId,
        name: String,
        layout: DisplayLayout,
    },

    /// Announce a new input device.
    DeviceAnnounce(DeviceInfo),

    /// An input device was removed.
    DeviceGone { device_id: DeviceId },

    /// Display layout changed (e.g. monitor added/removed).
    ScreenUpdate(DisplayLayout),

    /// Cursor is crossing to the remote machine.
    Enter {
        /// Which edge of the sender's screen the cursor is leaving from.
        edge: ScreenEdge,
        /// Position along that edge, in pixels from the edge's start (its top
        /// or left end) on the sender's screen. The receiver scales it to its
        /// own resolution.
        position: u32,
        /// Edges of the *receiver's* screen that lead somewhere, according to
        /// the sender's layout. The receiver sends `Leave` only when the cursor
        /// reaches one of these, so it needs no layout config of its own.
        exit_edges: Vec<ScreenEdge>,
    },

    /// Acknowledge an Enter; remote is ready to receive input.
    EnterAck,

    /// Cursor is returning to the local machine.
    Leave {
        /// Which edge of the sender's screen the cursor left from.
        edge: ScreenEdge,
        /// Position along that edge, in pixels from the edge's start on the
        /// sender's screen.
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

    /// Clipboard synchronisation traffic. Travels on the control stream so
    /// the small text-clipboard payload doesn't need its own QUIC stream.
    /// A dedicated clipboard stream is planned for image/large-payload
    /// support (see ROADMAP Phase 2.5).
    Clipboard(ClipboardMessage),

    /// Graceful disconnect.
    Bye,

    /// Refuse an `Enter`: the receiver is already controlling or being
    /// controlled, or lost a simultaneous-hand-off tie-break. The sender
    /// cancels its pending hand-off. Appended last so earlier variants keep
    /// their wire indices.
    EnterReject,
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
            layout: DisplayLayout::single(1920, 1080),
        });
        let _decoded = bincode_roundtrip(&msg);
    }

    #[test]
    fn welcome_roundtrip() {
        let msg = Message::Control(ControlMessage::Welcome {
            version: PROTOCOL_VERSION,
            machine_id: MachineId::new(),
            name: "remote".to_string(),
            layout: DisplayLayout::single(2560, 1440),
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
            exit_edges: vec![ScreenEdge::Left, ScreenEdge::Right],
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
    #[allow(clippy::similar_names)]
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
        assert_eq!(PROTOCOL_VERSION.to_string(), "0.3");
    }
}
