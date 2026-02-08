//! Peer session management: handshake, enter/leave, device announce.

use std::collections::HashMap;

use cross_control_protocol::{MessageReceiver, MessageSender, PeerConnection};
use cross_control_types::{
    ControlMessage, DeviceId, DeviceInfo, InputMessage, MachineId, ProtocolVersion, ScreenGeometry,
    VirtualDeviceId, PROTOCOL_VERSION,
};
use tracing::{debug, info, warn};

use crate::error::DaemonError;
use crate::state::SessionState;

/// A session with a single remote peer.
pub struct PeerSession {
    pub machine_id: MachineId,
    pub name: String,
    pub remote_screen: ScreenGeometry,
    pub state: SessionState,
    pub control_tx: MessageSender,
    pub control_rx: MessageReceiver,
    pub input_tx: Option<MessageSender>,
    pub input_rx: Option<MessageReceiver>,
    /// Map from remote device ID to local virtual device ID.
    pub device_map: HashMap<DeviceId, VirtualDeviceId>,
    /// Devices announced by the remote peer.
    pub remote_devices: Vec<DeviceInfo>,
    connection: PeerConnection,
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
            remote_screen: ScreenGeometry::new(1920, 1080),
            state: SessionState::Connected,
            control_tx,
            control_rx,
            input_tx: None,
            input_rx: None,
            device_map: HashMap::new(),
            remote_devices: Vec::new(),
            connection,
        }
    }

    /// Perform the initiator side of the handshake: send Hello, receive Welcome.
    pub async fn handshake_initiator(
        &mut self,
        our_id: MachineId,
        our_name: &str,
        our_screen: &ScreenGeometry,
    ) -> Result<(), DaemonError> {
        let hello = ControlMessage::Hello {
            version: PROTOCOL_VERSION,
            machine_id: our_id,
            name: our_name.to_string(),
            screen: our_screen.clone(),
        };
        self.control_tx.send(&hello).await?;
        self.state = SessionState::HelloSent;
        debug!("sent Hello");

        let welcome: ControlMessage = self.control_rx.recv().await?.ok_or_else(|| {
            DaemonError::Protocol(cross_control_protocol::ProtocolError::StreamClosed)
        })?;

        match welcome {
            ControlMessage::Welcome {
                version,
                machine_id,
                name,
                screen,
            } => {
                verify_version(version)?;
                self.machine_id = machine_id;
                self.name.clone_from(&name);
                self.remote_screen = screen;
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
    pub async fn handshake_responder(
        &mut self,
        our_id: MachineId,
        our_name: &str,
        our_screen: &ScreenGeometry,
    ) -> Result<(), DaemonError> {
        let hello: ControlMessage = self.control_rx.recv().await?.ok_or_else(|| {
            DaemonError::Protocol(cross_control_protocol::ProtocolError::StreamClosed)
        })?;

        match hello {
            ControlMessage::Hello {
                version,
                machine_id,
                name,
                screen,
            } => {
                verify_version(version)?;
                self.machine_id = machine_id;
                self.name.clone_from(&name);
                self.remote_screen = screen;

                let welcome = ControlMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    machine_id: our_id,
                    name: our_name.to_string(),
                    screen: our_screen.clone(),
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

    /// Initiate an Enter: send Enter, open input stream, await `EnterAck`.
    pub async fn enter(
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

        let enter = ControlMessage::Enter { edge, position };
        self.control_tx.send(&enter).await?;

        let input_tx = self.connection.open_input_stream().await?;
        self.input_tx = Some(input_tx);

        // Wait for EnterAck
        let ack: ControlMessage = self.control_rx.recv().await?.ok_or_else(|| {
            DaemonError::Protocol(cross_control_protocol::ProtocolError::StreamClosed)
        })?;

        match ack {
            ControlMessage::EnterAck => {
                self.state = SessionState::Controlling;
                info!(peer = %self.name, "now controlling remote");
                Ok(())
            }
            other => Err(DaemonError::Protocol(
                cross_control_protocol::ProtocolError::Handshake(format!(
                    "expected EnterAck, got {other:?}"
                )),
            )),
        }
    }

    /// Handle an incoming Enter from the remote peer: accept input stream, send `EnterAck`.
    pub async fn handle_enter(&mut self) -> Result<(), DaemonError> {
        if !self.state.can_enter_controlled() {
            return Err(DaemonError::Protocol(
                cross_control_protocol::ProtocolError::Handshake(format!(
                    "cannot be controlled from state {}",
                    self.state
                )),
            ));
        }

        let input_rx = self.connection.accept_input_stream().await?;
        self.input_rx = Some(input_rx);

        self.control_tx.send(&ControlMessage::EnterAck).await?;
        self.state = SessionState::Controlled;
        info!(peer = %self.name, "now being controlled by remote");
        Ok(())
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

    /// Receive input events from the remote peer.
    pub async fn recv_input(&mut self) -> Result<Option<InputMessage>, DaemonError> {
        if let Some(rx) = &mut self.input_rx {
            let msg = rx.recv().await?;
            Ok(msg)
        } else {
            Ok(None)
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
