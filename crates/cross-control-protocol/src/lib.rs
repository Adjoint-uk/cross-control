//! QUIC transport layer and wire protocol for cross-control.
//!
//! This crate handles QUIC connection management (via quinn), message
//! serialisation/deserialisation (via bincode v2), and the protocol state
//! machine for handshake and stream management.

pub mod connection;
pub mod error;
pub mod tls;
pub mod transport;
pub mod wire;

pub use connection::{MessageReceiver, MessageSender, PeerConnection};
pub use error::ProtocolError;
pub use transport::QuicTransport;
