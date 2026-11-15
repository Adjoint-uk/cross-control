//! Peer session management: handshake, enter/leave, device announce.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cross_control_protocol::{MessageReceiver, MessageSender, PeerConnection, QueuedSender};
use cross_control_types::{
    ControlMessage, DeviceId, DeviceInfo, DisplayLayout, InputMessage, MachineId, ProtocolVersion,
    VirtualDeviceId, PROTOCOL_VERSION,
};
use tracing::{debug, info, warn};

use crate::error::DaemonError;
use crate::state::SessionState;

/// Control messages that may wait for a peer before it counts as stalled.
/// Control traffic is light, but a clipboard payload can be large, so this
/// is kept small.
const CONTROL_QUEUE: usize = 64;
/// Input messages that may wait for a peer. Mouse movement is bursty.
const INPUT_QUEUE: usize = 1024;

/// Source of [`PeerSession::conn_id`].
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

/// A session with a single remote peer.
pub struct PeerSession {
    /// Unique per connection, never reused. Events from reader tasks carry
    /// it so those of a replaced connection can be told apart and dropped.
    pub conn_id: u64,
    pub machine_id: MachineId,
    pub name: String,
    pub remote_layout: DisplayLayout,
    pub state: SessionState,
    /// Control messages, written by a background task so a slow peer
    /// cannot block the daemon.
    pub control_tx: QueuedSender,
    control_rx: Option<MessageReceiver>,
    /// Input events while we control the peer, written the same way.
    pub input_tx: Option<QueuedSender>,
    input_rx: Option<MessageReceiver>,
    /// Map from remote device ID to local virtual device ID.
    pub device_map: HashMap<DeviceId, VirtualDeviceId>,
    /// Devices announced by the remote peer.
    pub remote_devices: Vec<DeviceInfo>,
    pub connection: PeerConnection,
    /// SHA-256 fingerprint of the peer's certificate (display form), as
    /// proven in the TLS handshake. `None` only if it presented none.
    pub fingerprint: Option<String>,
    /// The machine that opened this connection. When two connections to the
    /// same peer race, both sides keep the one opened by the lower id.
    pub initiated_by: MachineId,
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
        let fingerprint = connection.peer_fingerprint();
        Self {
            conn_id: NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed),
            machine_id: MachineId::default(),
            name: String::new(),
            remote_layout: DisplayLayout::default(),
            state: SessionState::Connected,
            control_tx: control_tx.into_queued(CONTROL_QUEUE),
            control_rx: Some(control_rx),
            input_tx: None,
            input_rx: None,
            device_map: HashMap::new(),
            remote_devices: Vec::new(),
            connection,
            fingerprint,
            initiated_by: MachineId::default(),
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
    /// `authorize` is called with the name the peer announces; an error
    /// aborts the handshake.
    ///
    /// Must be called before `take_control_rx()` — uses the `control_rx` directly.
    pub async fn handshake_initiator(
        &mut self,
        our_id: MachineId,
        our_name: &str,
        our_layout: &DisplayLayout,
        authorize: impl FnOnce(&str) -> Result<(), DaemonError>,
    ) -> Result<(), DaemonError> {
        self.initiated_by = our_id;
        let hello = ControlMessage::Hello {
            version: PROTOCOL_VERSION,
            machine_id: our_id,
            name: our_name.to_string(),
            layout: our_layout.clone(),
        };
        self.control_tx.send(&hello)?;
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
                authorize(&name)?;
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
    /// `authorize` is called with the name the peer announces, before we send
    /// anything; an error aborts the handshake.
    ///
    /// Must be called before `take_control_rx()` — uses the `control_rx` directly.
    pub async fn handshake_responder(
        &mut self,
        our_id: MachineId,
        our_name: &str,
        our_layout: &DisplayLayout,
        authorize: impl FnOnce(&str) -> Result<(), DaemonError>,
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
                // Decide before revealing anything about ourselves.
                authorize(&name)?;
                self.machine_id = machine_id;
                self.initiated_by = machine_id;
                self.name.clone_from(&name);
                self.remote_layout = layout;

                let welcome = ControlMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    machine_id: our_id,
                    name: our_name.to_string(),
                    layout: our_layout.clone(),
                };
                self.control_tx.send(&welcome)?;
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
    pub fn announce_devices(&mut self, devices: &[DeviceInfo]) -> Result<(), DaemonError> {
        for device in devices {
            let msg = ControlMessage::DeviceAnnounce(device.clone());
            self.control_tx.send(&msg)?;
            debug!(device = %device.name, "announced device");
        }
        Ok(())
    }

    /// Send `Enter` and open the input stream. The session moves to
    /// [`SessionState::PendingEnter`]; the event loop moves it on to
    /// `Controlling` on `EnterAck`, or back to `Idle` via [`cancel_enter`].
    ///
    /// [`cancel_enter`]: Self::cancel_enter
    pub async fn send_enter(
        &mut self,
        edge: cross_control_types::ScreenEdge,
        position: u32,
        exit_edges: Vec<cross_control_types::ScreenEdge>,
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
        self.input_tx = Some(input_tx.into_queued(INPUT_QUEUE));

        let enter = ControlMessage::Enter {
            edge,
            position,
            exit_edges,
        };
        if let Err(e) = self.control_tx.send(&enter) {
            self.input_tx = None;
            return Err(e.into());
        }

        self.state = SessionState::PendingEnter;
        debug!("sent Enter, waiting for EnterAck");
        Ok(())
    }

    /// Abandon a pending `Enter` (rejected, timed out, or yielded in a
    /// tie-break) and return to `Idle`. No-op in any other state.
    pub fn cancel_enter(&mut self) {
        if self.state == SessionState::PendingEnter {
            self.input_tx = None;
            self.state = SessionState::Idle;
            debug!(peer = %self.name, "pending Enter cancelled");
        }
    }

    /// Handle an incoming Enter from the remote peer: send `EnterAck`.
    ///
    /// Sends `EnterAck` immediately. The input stream must be accepted
    /// separately via [`PeerConnection::accept_input_stream`] (typically spawned
    /// as a task).
    pub fn handle_enter(&mut self) -> Result<(), DaemonError> {
        if !self.state.can_enter_controlled() {
            return Err(DaemonError::Protocol(
                cross_control_protocol::ProtocolError::Handshake(format!(
                    "cannot be controlled from state {}",
                    self.state
                )),
            ));
        }

        self.control_tx.send(&ControlMessage::EnterAck)?;
        self.state = SessionState::Controlled;
        info!(peer = %self.name, "now being controlled by remote");
        Ok(())
    }

    /// Transition to Controlling (called when `EnterAck` arrives for a
    /// pending `Enter`). Returns `false` — and changes nothing — if no Enter
    /// was pending, so an unsolicited ack can be refused.
    pub fn set_controlling(&mut self) -> bool {
        if self.state != SessionState::PendingEnter {
            return false;
        }
        self.state = SessionState::Controlling;
        info!(peer = %self.name, "now controlling remote");
        true
    }

    /// Return to `Idle` from any active state without sending anything (used
    /// when the peer refuses, or when a send fails).
    pub fn reset_to_idle(&mut self) {
        if matches!(
            self.state,
            SessionState::PendingEnter | SessionState::Controlling | SessionState::Controlled
        ) {
            self.input_tx = None;
            self.input_rx = None;
            self.state = SessionState::Idle;
        }
    }

    /// Send Leave message and return to Idle.
    pub fn leave(
        &mut self,
        edge: cross_control_types::ScreenEdge,
        position: u32,
    ) -> Result<(), DaemonError> {
        let leave = ControlMessage::Leave { edge, position };
        self.control_tx.send(&leave)?;
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
    pub fn send_input(&mut self, msg: &InputMessage) -> Result<(), DaemonError> {
        if let Some(tx) = &self.input_tx {
            tx.send(msg)?;
            Ok(())
        } else {
            warn!("attempted to send input without open input stream");
            Ok(())
        }
    }

    /// Send Bye and close the connection.
    pub fn disconnect(&mut self) -> Result<(), DaemonError> {
        self.state = SessionState::Disconnecting;
        let _ = self.control_tx.send(&ControlMessage::Bye);
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
