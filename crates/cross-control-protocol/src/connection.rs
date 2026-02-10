//! QUIC connection and stream framing.

use std::net::SocketAddr;

use bincode::{Decode, Encode};
use quinn::{Connection, RecvStream, SendStream};
use tracing::trace;

use crate::error::ProtocolError;
use crate::wire::MAX_MESSAGE_SIZE;

/// A connection to a remote cross-control peer.
#[derive(Clone)]
pub struct PeerConnection {
    connection: Connection,
}

impl PeerConnection {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    /// Get the remote address of this connection.
    pub fn remote_address(&self) -> SocketAddr {
        self.connection.remote_address()
    }

    /// Open a bidirectional stream (for control messages).
    pub async fn open_control_stream(
        &self,
    ) -> Result<(MessageSender, MessageReceiver), ProtocolError> {
        let (send, recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        Ok((MessageSender::new(send), MessageReceiver::new(recv)))
    }

    /// Accept a bidirectional stream (for control messages).
    pub async fn accept_control_stream(
        &self,
    ) -> Result<(MessageSender, MessageReceiver), ProtocolError> {
        let (send, recv) = self
            .connection
            .accept_bi()
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        Ok((MessageSender::new(send), MessageReceiver::new(recv)))
    }

    /// Open a unidirectional stream (for input events, controller -> controlled).
    pub async fn open_input_stream(&self) -> Result<MessageSender, ProtocolError> {
        let send = self
            .connection
            .open_uni()
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        Ok(MessageSender::new(send))
    }

    /// Accept a unidirectional stream (for input events, controller -> controlled).
    pub async fn accept_input_stream(&self) -> Result<MessageReceiver, ProtocolError> {
        let recv = self
            .connection
            .accept_uni()
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        Ok(MessageReceiver::new(recv))
    }

    /// Close the connection gracefully.
    pub fn close(&self) {
        self.connection.close(quinn::VarInt::from_u32(0), b"bye");
    }
}

/// Sends length-prefixed bincode messages over a QUIC send stream.
pub struct MessageSender {
    stream: SendStream,
}

impl MessageSender {
    fn new(stream: SendStream) -> Self {
        Self { stream }
    }

    /// Send a message, encoding it as length-prefixed bincode.
    pub async fn send<T: Encode>(&mut self, msg: &T) -> Result<(), ProtocolError> {
        let config = bincode::config::standard();
        let payload = bincode::encode_to_vec(msg, config)
            .map_err(|e| ProtocolError::Serialization(e.to_string()))?;

        let len = u32::try_from(payload.len())
            .map_err(|_| ProtocolError::Serialization("message too large".to_string()))?;

        if len > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::Serialization(format!(
                "message size {len} exceeds maximum {MAX_MESSAGE_SIZE}"
            )));
        }

        self.stream
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        self.stream
            .write_all(&payload)
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;

        trace!(len, "sent message");
        Ok(())
    }

    /// Finish the stream (signal no more data).
    pub fn finish(mut self) -> Result<(), ProtocolError> {
        self.stream
            .finish()
            .map_err(|e| ProtocolError::Connection(e.to_string()))
    }
}

/// Receives length-prefixed bincode messages from a QUIC recv stream.
pub struct MessageReceiver {
    stream: RecvStream,
}

impl MessageReceiver {
    fn new(stream: RecvStream) -> Self {
        Self { stream }
    }

    /// Receive and decode a message.
    ///
    /// Returns `None` if the stream has been cleanly closed by the peer.
    pub async fn recv<T: Decode<()>>(&mut self) -> Result<Option<T>, ProtocolError> {
        // Read 4-byte length prefix
        let mut len_buf = [0u8; 4];
        match self.stream.read_exact(&mut len_buf).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => return Ok(None),
            Err(quinn::ReadExactError::ReadError(e)) => {
                return Err(ProtocolError::Connection(e.to_string()));
            }
        }

        let len = u32::from_be_bytes(len_buf);
        if len > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::Deserialization(format!(
                "message size {len} exceeds maximum {MAX_MESSAGE_SIZE}"
            )));
        }

        let mut payload = vec![0u8; len as usize];
        match self.stream.read_exact(&mut payload).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => {
                return Err(ProtocolError::StreamClosed);
            }
            Err(quinn::ReadExactError::ReadError(e)) => {
                return Err(ProtocolError::Connection(e.to_string()));
            }
        }

        let config = bincode::config::standard();
        let (msg, _) = bincode::decode_from_slice(&payload, config)
            .map_err(|e| ProtocolError::Deserialization(e.to_string()))?;

        trace!(len, "received message");
        Ok(Some(msg))
    }
}
