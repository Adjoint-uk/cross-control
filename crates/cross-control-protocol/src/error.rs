//! Protocol and transport errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("handshake failed: {0}")]
    Handshake(String),

    #[error("incompatible protocol version: remote {remote}, local {local}")]
    VersionMismatch { remote: String, local: String },

    #[error("serialisation error: {0}")]
    Serialization(String),

    #[error("deserialisation error: {0}")]
    Deserialization(String),

    #[error("stream closed unexpectedly")]
    StreamClosed,

    #[error("TLS error: {0}")]
    Tls(String),

    #[error(transparent)]
    Quinn(#[from] quinn::ConnectionError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
