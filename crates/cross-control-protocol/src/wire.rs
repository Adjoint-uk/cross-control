//! Wire format: length-prefixed bincode v2 frames.
//!
//! Each message on the wire is:
//!   [4 bytes big-endian length][bincode v2 payload]

use bincode::{Decode, Encode};

use crate::error::ProtocolError;

/// Maximum message size (1 MiB). Frames longer than this are rejected before
/// they are read.
pub const MAX_MESSAGE_SIZE: u32 = 1024 * 1024;

/// [`MAX_MESSAGE_SIZE`] as a `usize`, for bincode's decode limit.
const MAX_MESSAGE_BYTES: usize = MAX_MESSAGE_SIZE as usize;

/// The bincode configuration for every message on the wire.
///
/// The limit matters on decode: bincode otherwise trusts the length a peer
/// claims for a `String` or `Vec` and tries to allocate it up front, so a
/// ten-byte frame claiming a terabyte-long string aborts the process. With a
/// limit, each claimed length is checked against the budget before anything is
/// allocated. The limit does not change the encoding.
fn wire_config() -> impl bincode::config::Config {
    bincode::config::standard().with_limit::<MAX_MESSAGE_BYTES>()
}

/// Encode a message to a length-prefixed byte vector.
pub fn encode_message<T: Encode>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = encode_payload(msg)?;
    let len = u32::try_from(payload.len())
        .map_err(|_| ProtocolError::Serialization("message too large".to_string()))?;

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Encode a message payload (without the length prefix).
pub(crate) fn encode_payload<T: Encode>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    bincode::encode_to_vec(msg, wire_config())
        .map_err(|e| ProtocolError::Serialization(e.to_string()))
}

/// Decode a message from a bincode v2 payload (without the length prefix).
///
/// Safe on untrusted input: claimed lengths are bounded by
/// [`MAX_MESSAGE_SIZE`] before any allocation.
pub fn decode_message<T: Decode<()>>(payload: &[u8]) -> Result<T, ProtocolError> {
    let (msg, _) = bincode::decode_from_slice(payload, wire_config())
        .map_err(|e| ProtocolError::Deserialization(e.to_string()))?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cross_control_types::message::{ControlMessage, Message, PROTOCOL_VERSION};
    use cross_control_types::screen::DisplayLayout;
    use cross_control_types::MachineId;

    #[test]
    fn encode_decode_roundtrip() {
        let msg = Message::Control(ControlMessage::Hello {
            version: PROTOCOL_VERSION,
            machine_id: MachineId::new(),
            name: "test".to_string(),
            layout: DisplayLayout::single(1920, 1080),
        });

        let bytes = encode_message(&msg).unwrap();
        // First 4 bytes are length
        let len = u32::from_be_bytes(bytes[..4].try_into().unwrap());
        assert_eq!(len as usize, bytes.len() - 4);

        let decoded: Message = decode_message(&bytes[4..]).unwrap();
        // Just verify it decoded without error
        match decoded {
            Message::Control(ControlMessage::Hello { name, .. }) => {
                assert_eq!(name, "test");
            }
            _ => panic!("unexpected message type"),
        }
    }

    #[test]
    fn ping_pong_wire_roundtrip() {
        let msg = Message::Control(ControlMessage::Ping { seq: 12345 });
        let bytes = encode_message(&msg).unwrap();
        let decoded: Message = decode_message(&bytes[4..]).unwrap();
        match decoded {
            Message::Control(ControlMessage::Ping { seq }) => assert_eq!(seq, 12345),
            _ => panic!("unexpected message type"),
        }
    }

    /// bincode's varint encoding of a u64 length: a 0xFD marker, then 8 bytes LE.
    fn hostile_len_prefix(claimed: u64) -> Vec<u8> {
        let mut v = vec![0xFD];
        v.extend_from_slice(&claimed.to_le_bytes());
        v
    }

    #[test]
    fn hostile_string_length_is_rejected_not_allocated() {
        // A ~10-byte frame claiming a 1 TiB string. Without a decode limit,
        // bincode tries to allocate it and the process aborts.
        let payload = hostile_len_prefix(1 << 40);
        let result: Result<String, _> = decode_message(&payload);
        assert!(result.is_err());
    }

    #[test]
    fn hostile_vec_length_is_rejected_not_allocated() {
        let payload = hostile_len_prefix(1 << 40);
        let result: Result<Vec<u64>, _> = decode_message(&payload);
        assert!(result.is_err());
    }

    #[test]
    fn length_just_over_the_limit_is_rejected() {
        let payload = hostile_len_prefix(u64::from(MAX_MESSAGE_SIZE) + 1);
        let result: Result<Vec<u8>, _> = decode_message(&payload);
        assert!(result.is_err());
    }

    /// Deterministic xorshift generator, so failures reproduce.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn byte(&mut self) -> u8 {
            self.next().to_le_bytes()[0]
        }
        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % n as u64).unwrap()
        }
    }

    /// Valid encodings of a spread of messages, as mutation seeds.
    fn seed_payloads() -> Vec<Vec<u8>> {
        use cross_control_types::{
            ClipboardContent, ClipboardMessage, DeviceCapability, DeviceId, DeviceInfo, InputEvent,
            InputMessage, KeyCode, ScreenEdge,
        };
        let messages = [
            Message::Control(ControlMessage::Hello {
                version: PROTOCOL_VERSION,
                machine_id: MachineId::new(),
                name: "seed".to_string(),
                layout: DisplayLayout::single(1920, 1080),
            }),
            Message::Control(ControlMessage::Enter {
                edge: ScreenEdge::Right,
                position: 540,
                exit_edges: vec![ScreenEdge::Left, ScreenEdge::Top],
            }),
            Message::Control(ControlMessage::DeviceAnnounce(DeviceInfo {
                id: DeviceId(7),
                name: "kbd".to_string(),
                capabilities: vec![DeviceCapability::Keyboard],
            })),
            Message::Control(ControlMessage::Clipboard(ClipboardMessage::Data(
                ClipboardContent::html("<b>x</b>"),
            ))),
            Message::Input(InputMessage {
                device_id: DeviceId(1),
                timestamp_us: 42,
                events: vec![
                    InputEvent::MouseMove { dx: -3, dy: 9 },
                    InputEvent::Key {
                        code: KeyCode::KeyA,
                        state: cross_control_types::ButtonState::Pressed,
                    },
                ],
            }),
        ];
        messages
            .iter()
            .map(|m| encode_payload(m).unwrap())
            .collect()
    }

    /// Whatever bytes a peer sends, decoding returns `Ok` or `Err`: it never
    /// panics, aborts, or allocates beyond the frame limit. A cheap,
    /// always-on complement to the cargo-fuzz targets in `fuzz/`.
    #[test]
    fn decoder_survives_random_and_mutated_input() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        let seeds = seed_payloads();
        for seed in &seeds {
            let _: Message = decode_message(seed).expect("seed must decode");
        }

        for _ in 0..50_000 {
            let len = rng.below(96);
            let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
            let _ = decode_message::<Message>(&bytes);
        }

        for _ in 0..50_000 {
            let mut bytes = seeds[rng.below(seeds.len())].clone();
            match rng.below(4) {
                0 => {
                    let i = rng.below(bytes.len());
                    bytes[i] = rng.byte();
                }
                1 => bytes.truncate(rng.below(bytes.len())),
                2 => {
                    // Splice in a huge varint length somewhere.
                    let i = rng.below(bytes.len());
                    let claimed = rng.next() | (1 << 40);
                    bytes.splice(i..i, hostile_len_prefix(claimed));
                }
                _ => {
                    let i = rng.below(bytes.len());
                    bytes.insert(i, rng.byte());
                }
            }
            let _ = decode_message::<Message>(&bytes);
        }
    }
}
