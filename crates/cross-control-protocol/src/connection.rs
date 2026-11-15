//! QUIC connection and stream framing.

use std::net::SocketAddr;

use bincode::{Decode, Encode};
use quinn::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;
use tracing::trace;

use crate::error::ProtocolError;
use crate::wire::{decode_message, encode_payload, MAX_MESSAGE_SIZE};

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

    /// Close the connection with an application reason.
    pub fn close_with_reason(&self, reason: &str) {
        self.connection
            .close(quinn::VarInt::from_u32(1), reason.as_bytes());
    }

    /// The SHA-256 fingerprint of the peer's TLS certificate, in display form
    /// (`SHA256:aa:bb:…`). `None` if the peer presented no certificate. Used
    /// by `pair` to show the user a fingerprint to pin.
    pub fn peer_fingerprint(&self) -> Option<String> {
        let identity = self.connection.peer_identity()?;
        let certs = identity
            .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
            .ok()?;
        certs.first().map(crate::tls::fingerprint_display)
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
        let payload = frame_payload(msg)?;
        self.send_payload(&payload).await
    }

    /// Hand this stream to a background writer task. Messages are then
    /// queued without waiting for the peer; see [`QueuedSender`].
    #[must_use]
    pub fn into_queued(self, capacity: usize) -> QueuedSender {
        QueuedSender::spawn(self, capacity)
    }

    async fn send_payload(&mut self, payload: &[u8]) -> Result<(), ProtocolError> {
        let len = u32::try_from(payload.len())
            .map_err(|_| ProtocolError::Serialization("message too large".to_string()))?;
        self.stream
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| ProtocolError::Connection(e.to_string()))?;
        self.stream
            .write_all(payload)
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

/// Encode `msg` and check it fits in one frame.
fn frame_payload<T: Encode>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = encode_payload(msg)?;
    if payload.len() > MAX_MESSAGE_SIZE as usize {
        return Err(ProtocolError::Serialization(format!(
            "message size {} exceeds maximum {MAX_MESSAGE_SIZE}",
            payload.len()
        )));
    }
    Ok(payload)
}

/// A send stream drained by a background task.
///
/// [`send`](Self::send) encodes the message and queues it without waiting
/// for the network, so a peer that stops reading cannot stall the caller
/// (e.g. the daemon's event loop). When the queue is full the peer is not
/// keeping up and `send` fails with [`ProtocolError::Backpressure`]; the
/// caller decides whether to drop the message or the peer.
pub struct QueuedSender {
    tx: mpsc::Sender<Vec<u8>>,
    writer: tokio::task::JoinHandle<()>,
}

impl QueuedSender {
    fn spawn(mut sender: MessageSender, capacity: usize) -> Self {
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(capacity.max(1));
        let writer = tokio::spawn(async move {
            while let Some(payload) = rx.recv().await {
                if let Err(e) = sender.send_payload(&payload).await {
                    trace!(error = %e, "queued writer stopping");
                    return;
                }
            }
            // All senders dropped: everything queued has been written.
            let _ = sender.finish();
        });
        Self { tx, writer }
    }

    /// Encode `msg` and queue it for sending.
    pub fn send<T: Encode>(&self, msg: &T) -> Result<(), ProtocolError> {
        let payload = frame_payload(msg)?;
        self.tx.try_send(payload).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => ProtocolError::Backpressure,
            mpsc::error::TrySendError::Closed(_) => ProtocolError::StreamClosed,
        })
    }

    /// Stop accepting messages and wait (at most `timeout`) for the queued
    /// ones to be written and the stream finished.
    pub async fn finish(self, timeout: std::time::Duration) {
        let Self { tx, writer } = self;
        drop(tx);
        let _ = tokio::time::timeout(timeout, writer).await;
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

        let msg = decode_message(&payload)?;

        trace!(len, "received message");
        Ok(Some(msg))
    }
}
