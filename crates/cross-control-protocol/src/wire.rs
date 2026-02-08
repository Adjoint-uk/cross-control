//! Wire format: length-prefixed bincode v2 frames.
//!
//! Each message on the wire is:
//!   [4 bytes big-endian length][bincode v2 payload]

use bincode::{Decode, Encode};

use crate::error::ProtocolError;

/// Maximum message size (1 MiB). Prevents allocation bombs.
pub const MAX_MESSAGE_SIZE: u32 = 1024 * 1024;

/// Encode a message to a length-prefixed byte vector.
pub fn encode_message<T: Encode>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    let config = bincode::config::standard();
    let payload = bincode::encode_to_vec(msg, config)
        .map_err(|e| ProtocolError::Serialization(e.to_string()))?;

    let len = u32::try_from(payload.len())
        .map_err(|_| ProtocolError::Serialization("message too large".to_string()))?;

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Decode a message from a bincode v2 payload (without the length prefix).
pub fn decode_message<T: Decode<()>>(payload: &[u8]) -> Result<T, ProtocolError> {
    let config = bincode::config::standard();
    let (msg, _) = bincode::decode_from_slice(payload, config)
        .map_err(|e| ProtocolError::Deserialization(e.to_string()))?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cross_control_types::message::{ControlMessage, Message, PROTOCOL_VERSION};
    use cross_control_types::screen::ScreenGeometry;
    use cross_control_types::MachineId;

    #[test]
    fn encode_decode_roundtrip() {
        let msg = Message::Control(ControlMessage::Hello {
            version: PROTOCOL_VERSION,
            machine_id: MachineId::new(),
            name: "test".to_string(),
            screen: ScreenGeometry::new(1920, 1080),
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
}
