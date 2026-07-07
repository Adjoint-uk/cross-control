//! Peer session management: handshake, enter/leave, device announce.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use cross_control_protocol::{MessageReceiver, MessageSender, PeerConnection};
use cross_control_types::{
    ControlMessage, DeviceId, DeviceInfo, DisplayLayout, InputMessage, MachineId, ProtocolVersion,
    VirtualDeviceId, PROTOCOL_VERSION,
};
use tracing::{debug, info, warn};

use crate::error::DaemonError;
use crate::state::SessionState;

/// A session with a single remote peer.
pub struct PeerSession {
    pub machine_id: MachineId,
    pub name: String,
    pub remote_layout: DisplayLayout,
    pub state: SessionState,
    pub control_tx: MessageSender,
    control_rx: Option<MessageReceiver>,
    pub input_tx: Option<MessageSender>,
    input_rx: Option<MessageReceiver>,
    /// Map from remote device ID to local virtual device ID.
    pub device_map: HashMap<DeviceId, VirtualDeviceId>,
    /// Devices announced by the remote peer.
    pub remote_devices: Vec<DeviceInfo>,
    pub connection: PeerConnection,
    /// Liveness-ping bookkeeping used to derive the round-trip time.
    latency: LatencyTracker,
}

/// Tracks in-flight liveness pings and the most recent round-trip time for a
/// single peer. Split out so the timing logic is unit-testable without a live
/// QUIC connection.
#[derive(Default)]
struct LatencyTracker {
    /// Sequence number → send time for pings awaiting a pong.
    pending: HashMap<u64, Instant>,
    /// Most recent measured round-trip time, if any ping has completed.
    last_rtt: Option<Duration>,
}

impl LatencyTracker {
    /// Record that a ping with `seq` was just sent. Prunes pings older than a
    /// few cycles that never got a reply so the map can't grow unbounded.
    fn record_sent(&mut self, seq: u64) {
        let now = Instant::now();
        self.pending
            .retain(|_, sent| now.duration_since(*sent) < Duration::from_secs(30));
        self.pending.insert(seq, now);
    }

    /// Match a pong to its ping and update the round-trip time. Unknown or
    /// duplicate sequence numbers are ignored.
    fn record_pong(&mut self, seq: u64) {
        if let Some(sent) = self.pending.remove(&seq) {
            self.last_rtt = Some(Instant::now().duration_since(sent));
        }
    }
}

impl PeerSession {
    /// Create a new session from an accepted or initiated connection.
    pub fn new(
        connection: PeerConnection,
        control_tx: MessageSender,
        control_rx: MessageReceiver,
    ) -> Self {
        Self {
            machine_id: MachineId::default(),
            name: String::new(),
            remote_layout: DisplayLayout::default(),
            state: SessionState::Connected,
            control_tx,
            control_rx: Some(control_rx),
            input_tx: None,
            input_rx: None,
            device_map: HashMap::new(),
            remote_devices: Vec::new(),
            connection,
            latency: LatencyTracker::default(),
        }
    }

    /// Record that a liveness `Ping` with `seq` was just sent, so its `Pong`
    /// can be timed.
    pub fn record_ping_sent(&mut self, seq: u64) {
        self.latency.record_sent(seq);
    }

    /// Match an incoming `Pong` to its `Ping` and update the round-trip time.
    /// Unknown or duplicate sequence numbers are ignored.
    pub fn record_pong(&mut self, seq: u64) {
        self.latency.record_pong(seq);
    }

    /// The most recent round-trip time, if a ping has completed.
    pub fn last_rtt(&self) -> Option<Duration> {
        self.latency.last_rtt
    }

    /// Take ownership of the control receiver for spawning a reader task.
    /// Returns `None` if already taken.
    pub fn take_control_rx(&mut self) -> Option<MessageReceiver> {
        self.control_rx.take()
    }

    /// Take ownership of the input receiver for spawning a reader task.
    /// Returns `None` if already taken or not yet set.
    pub fn take_input_rx(&mut self) -> Option<MessageReceiver> {
        self.input_rx.take()
    }

    /// Perform the initiator side of the handshake: send Hello, receive Welcome.
    ///
    /// Must be called before `take_control_rx()` — uses the `control_rx` directly.
    pub async fn handshake_initiator(
        &mut self,
        our_id: MachineId,
        our_name: &str,
        our_layout: &DisplayLayout,
    ) -> Result<(), DaemonError> {
        let hello = ControlMessage::Hello {
            version: PROTOCOL_VERSION,
            machine_id: our_id,
            name: our_name.to_string(),
            layout: our_layout.clone(),
        };
        self.control_tx.send(&hello).await?;
        self.state = SessionState::HelloSent;
        debug!("sent Hello");

        let rx = self
            .control_rx
            .as_mut()
            .expect("control_rx must exist during handshake");
        let welcome: ControlMessage = rx.recv().await?.ok_or_else(|| {
            DaemonError::Protocol(cross_control_protocol::ProtocolError::StreamClosed)
        })?;

        match welcome {
            ControlMessage::Welcome {
                version,
                machine_id,
                name,
                layout,
            } => {
                verify_version(version)?;
                self.machine_id = machine_id;
                self.name.clone_from(&name);
                self.remote_layout = layout;
                self.state = SessionState::Idle;
                info!(peer = %name, id = %machine_id, "handshake complete (initiator)");
                Ok(())
            }
            other => Err(DaemonError::Protocol(
                cross_control_protocol::ProtocolError::Handshake(format!(
                    "expected Welcome, got {other:?}"
                )),
            )),
        }
    }

    /// Perform the responder side of the handshake: receive Hello, send Welcome.
    ///
    /// Must be called before `take_control_rx()` — uses the `control_rx` directly.
    pub async fn handshake_responder(
        &mut self,
        our_id: MachineId,
        our_name: &str,
        our_layout: &DisplayLayout,
    ) -> Result<(), DaemonError> {
        let rx = self
            .control_rx
            .as_mut()
            .expect("control_rx must exist during handshake");
        let hello: ControlMessage = rx.recv().await?.ok_or_else(|| {
            DaemonError::Protocol(cross_control_protocol::ProtocolError::StreamClosed)
        })?;

        match hello {
            ControlMessage::Hello {
                version,
                machine_id,
                name,
                layout,
            } => {
                verify_version(version)?;
                self.machine_id = machine_id;
                self.name.clone_from(&name);
                self.remote_layout = layout;

                let welcome = ControlMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    machine_id: our_id,
                    name: our_name.to_string(),
                    layout: our_layout.clone(),
                };
                self.control_tx.send(&welcome).await?;
                self.state = SessionState::Idle;
                info!(peer = %name, id = %machine_id, "handshake complete (responder)");
                Ok(())
            }
            other => Err(DaemonError::Protocol(
                cross_control_protocol::ProtocolError::Handshake(format!(
                    "expected Hello, got {other:?}"
                )),
            )),
        }
    }

    /// Send a `DeviceAnnounce` for each of our devices.
    pub async fn announce_devices(&mut self, devices: &[DeviceInfo]) -> Result<(), DaemonError> {
        for device in devices {
            let msg = ControlMessage::DeviceAnnounce(device.clone());
            self.control_tx.send(&msg).await?;
            debug!(device = %device.name, "announced device");
        }
        Ok(())
    }

    /// Send Enter and open input stream (non-blocking — `EnterAck` handled by event loop).
    pub async fn send_enter(
        &mut self,
        edge: cross_control_types::ScreenEdge,
        position: u32,
    ) -> Result<(), DaemonError> {
        if !self.state.can_enter_controlling() {
            return Err(DaemonError::Protocol(
                cross_control_protocol::ProtocolError::Handshake(format!(
                    "cannot Enter from state {}",
                    self.state
                )),
            ));
        }

        // Open input stream BEFORE sending Enter so it's available when
        // the remote calls accept_input_stream() upon receiving Enter.
        let input_tx = self.connection.open_input_stream().await?;
        self.input_tx = Some(input_tx);

        let enter = ControlMessage::Enter { edge, position };
        self.control_tx.send(&enter).await?;

        // Transition state so duplicate send_enter calls are rejected
        self.state = SessionState::Controlling;
        debug!("sent Enter, waiting for EnterAck via event loop");
        Ok(())
    }

    /// Handle an incoming Enter from the remote peer: send `EnterAck`.
    ///
    /// Sends `EnterAck` immediately. The input stream must be accepted
    /// separately via [`accept_input_stream`] (typically spawned as a task).
    pub async fn handle_enter(&mut self) -> Result<(), DaemonError> {
        if !self.state.can_enter_controlled() {
            return Err(DaemonError::Protocol(
                cross_control_protocol::ProtocolError::Handshake(format!(
                    "cannot be controlled from state {}",
                    self.state
                )),
            ));
        }

        self.control_tx.send(&ControlMessage::EnterAck).await?;
        self.state = SessionState::Controlled;
        info!(peer = %self.name, "now being controlled by remote");
        Ok(())
    }

    /// Transition to Controlling state (called when `EnterAck` received via event loop).
    pub fn set_controlling(&mut self) {
        self.state = SessionState::Controlling;
        info!(peer = %self.name, "now controlling remote");
    }

    /// Send Leave message and return to Idle.
    pub async fn leave(
        &mut self,
        edge: cross_control_types::ScreenEdge,
        position: u32,
    ) -> Result<(), DaemonError> {
        let leave = ControlMessage::Leave { edge, position };
        self.control_tx.send(&leave).await?;
        self.input_tx = None;
        self.state = SessionState::Idle;
        info!(peer = %self.name, "left remote control");
        Ok(())
    }

    /// Handle an incoming Leave from the remote peer.
    pub fn handle_leave(&mut self) {
        self.input_rx = None;
        self.state = SessionState::Idle;
        info!(peer = %self.name, "remote released control");
    }

    /// Send input events to the remote peer.
    pub async fn send_input(&mut self, msg: &InputMessage) -> Result<(), DaemonError> {
        if let Some(tx) = &mut self.input_tx {
            tx.send(msg).await?;
            Ok(())
        } else {
            warn!("attempted to send input without open input stream");
            Ok(())
        }
    }

    /// Send Bye and close the connection.
    pub async fn disconnect(&mut self) -> Result<(), DaemonError> {
        self.state = SessionState::Disconnecting;
        let _ = self.control_tx.send(&ControlMessage::Bye).await;
        self.connection.close();
        info!(peer = %self.name, "disconnected");
        Ok(())
    }
}

fn verify_version(remote: ProtocolVersion) -> Result<(), DaemonError> {
    if remote.major != PROTOCOL_VERSION.major {
        return Err(DaemonError::Protocol(
            cross_control_protocol::ProtocolError::VersionMismatch {
                remote: remote.to_string(),
                local: PROTOCOL_VERSION.to_string(),
            },
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_records_round_trip() {
        let mut lt = LatencyTracker::default();
        assert!(lt.last_rtt.is_none());
        lt.record_sent(1);
        lt.record_pong(1);
        assert!(lt.last_rtt.is_some());
        // The ping is consumed once matched.
        assert!(lt.pending.is_empty());
    }

    #[test]
    fn pong_for_unknown_seq_is_ignored() {
        let mut lt = LatencyTracker::default();
        lt.record_sent(1);
        // A pong for a seq we never sent leaves state untouched.
        lt.record_pong(99);
        assert!(lt.last_rtt.is_none());
        assert_eq!(lt.pending.len(), 1);
    }

    #[test]
    fn duplicate_pong_is_ignored() {
        let mut lt = LatencyTracker::default();
        lt.record_sent(1);
        lt.record_pong(1);
        let first = lt.last_rtt;
        // A second pong for the same seq has no matching ping to consume.
        lt.record_pong(1);
        assert_eq!(lt.last_rtt, first);
    }
}
