//! Session state machine.

/// State of a peer session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// TCP/QUIC connected but handshake not started.
    Connected,
    /// Hello sent, waiting for Welcome.
    HelloSent,
    /// Handshake complete, idle — no input being forwarded.
    Idle,
    /// This machine is controlling the remote (sending input).
    Controlling,
    /// This machine is being controlled by the remote (receiving input).
    Controlled,
    /// Disconnecting gracefully.
    Disconnecting,
}

impl SessionState {
    /// Whether we can transition to the Controlling state.
    pub fn can_enter_controlling(self) -> bool {
        self == Self::Idle
    }

    /// Whether we can transition to the Controlled state.
    pub fn can_enter_controlled(self) -> bool {
        self == Self::Idle
    }

    /// Whether we are actively forwarding or receiving input.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Controlling | Self::Controlled)
    }
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected => write!(f, "Connected"),
            Self::HelloSent => write!(f, "HelloSent"),
            Self::Idle => write!(f, "Idle"),
            Self::Controlling => write!(f, "Controlling"),
            Self::Controlled => write!(f, "Controlled"),
            Self::Disconnecting => write!(f, "Disconnecting"),
        }
    }
}
