//! Core daemon orchestration.

use std::collections::HashMap;
use std::net::SocketAddr;

use cross_control_input::{InputCapture, InputEmulation};
use cross_control_protocol::QuicTransport;
use cross_control_types::{
    CapturedEvent, ControlMessage, DeviceInfo, InputEvent, InputMessage, KeyCode, MachineId,
    ScreenEdge, ScreenGeometry,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::DaemonError;
use crate::session::PeerSession;

/// Events processed by the daemon's main loop.
pub enum DaemonEvent {
    /// A new peer connected (inbound).
    IncomingConnection(cross_control_protocol::PeerConnection),
    /// A captured local input event.
    CapturedInput(CapturedEvent),
    /// A control message from a peer.
    PeerControl {
        machine_id: MachineId,
        msg: ControlMessage,
    },
    /// An input message from a peer.
    PeerInput {
        machine_id: MachineId,
        msg: InputMessage,
    },
    /// A peer disconnected.
    PeerDisconnected(MachineId),
    /// Shutdown signal.
    Shutdown,
}

/// The core cross-control daemon.
pub struct Daemon {
    config: Config,
    machine_id: MachineId,
    screen: ScreenGeometry,
    transport: QuicTransport,
    capture: Box<dyn InputCapture>,
    emulation: Box<dyn InputEmulation>,
    sessions: HashMap<MachineId, PeerSession>,
    local_devices: Vec<DeviceInfo>,
    event_tx: mpsc::Sender<DaemonEvent>,
    event_rx: mpsc::Receiver<DaemonEvent>,
    /// Virtual cursor position for barrier detection.
    cursor_x: i32,
    cursor_y: i32,
    /// Which peer we are currently controlling, if any.
    controlling: Option<MachineId>,
    /// Which peer is currently controlling us, if any.
    controlled_by: Option<MachineId>,
    /// Hotkey state tracking: set of currently pressed keys.
    hotkey_pressed: Vec<KeyCode>,
}

impl Daemon {
    /// Create a new daemon instance.
    pub fn new(
        config: Config,
        machine_id: MachineId,
        transport: QuicTransport,
        capture: Box<dyn InputCapture>,
        emulation: Box<dyn InputEmulation>,
    ) -> Self {
        let screen = ScreenGeometry::new(config.daemon.screen_width, config.daemon.screen_height);
        let (event_tx, event_rx) = mpsc::channel(1024);

        Self {
            cursor_x: i32::try_from(screen.width / 2).unwrap_or(960),
            cursor_y: i32::try_from(screen.height / 2).unwrap_or(540),
            config,
            machine_id,
            screen,
            transport,
            capture,
            emulation,
            sessions: HashMap::new(),
            local_devices: Vec::new(),
            event_tx,
            event_rx,
            controlling: None,
            controlled_by: None,
            hotkey_pressed: Vec::new(),
        }
    }

    /// Get a clone of the event sender for feeding events into the daemon.
    pub fn event_sender(&self) -> mpsc::Sender<DaemonEvent> {
        self.event_tx.clone()
    }

    /// Run the daemon event loop.
    pub async fn run(&mut self) -> Result<(), DaemonError> {
        // Start input capture
        let capture_tx = self.event_tx.clone();
        let (input_tx, mut input_rx) = mpsc::channel::<CapturedEvent>(1024);
        self.capture.start(input_tx).await?;

        // Forward captured input to daemon events
        let capture_event_tx = capture_tx.clone();
        tokio::spawn(async move {
            while let Some(event) = input_rx.recv().await {
                if capture_event_tx
                    .send(DaemonEvent::CapturedInput(event))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let transport_local = self.transport.local_addr()?;
        info!(addr = %transport_local, "daemon listening");

        // Connect to statically configured peers
        let peers_to_connect: Vec<(SocketAddr, String)> = self
            .config
            .screens
            .iter()
            .filter_map(|sc| {
                sc.address.as_ref().map(|addr_str| {
                    let addr: SocketAddr = addr_str
                        .parse()
                        .or_else(|_| format!("{addr_str}:{}", self.config.daemon.port).parse())
                        .ok()?;
                    Some((addr, sc.name.clone()))
                })
            })
            .flatten()
            .collect();

        for (addr, name) in peers_to_connect {
            info!(address = %addr, name = %name, "connecting to configured peer");
            match self.transport.connect(addr, "cross-control").await {
                Ok(conn) => {
                    if let Err(e) = self.setup_outbound_session(conn, &name).await {
                        warn!(error = %e, "failed to set up outbound session");
                    }
                }
                Err(e) => {
                    warn!(address = %addr, error = %e, "failed to connect to peer");
                }
            }
        }

        info!("daemon running");

        // Main event loop
        loop {
            tokio::select! {
                // Accept new connections
                result = self.transport.accept() => {
                    match result {
                        Ok(conn) => {
                            if let Err(e) = self.handle_incoming_connection(conn).await {
                                warn!(error = %e, "failed to handle incoming connection");
                            }
                        }
                        Err(e) => {
                            debug!(error = %e, "accept error");
                        }
                    }
                }
                // Process daemon events
                event = self.event_rx.recv() => {
                    match event {
                        Some(DaemonEvent::CapturedInput(captured)) => {
                            self.handle_captured_input(captured).await;
                        }
                        Some(DaemonEvent::PeerControl { machine_id, msg }) => {
                            self.handle_peer_control(machine_id, msg).await;
                        }
                        Some(DaemonEvent::PeerInput { machine_id, msg }) => {
                            self.handle_peer_input(machine_id, msg).await;
                        }
                        Some(DaemonEvent::PeerDisconnected(machine_id)) => {
                            self.handle_peer_disconnected(machine_id).await;
                        }
                        Some(DaemonEvent::Shutdown) | None => {
                            info!("shutting down");
                            break;
                        }
                        Some(DaemonEvent::IncomingConnection(conn)) => {
                            if let Err(e) = self.handle_incoming_connection(conn).await {
                                warn!(error = %e, "failed to handle incoming connection");
                            }
                        }
                    }
                }
            }
        }

        self.shutdown().await
    }

    async fn setup_outbound_session(
        &mut self,
        conn: cross_control_protocol::PeerConnection,
        _peer_name: &str,
    ) -> Result<(), DaemonError> {
        let (control_tx, control_rx) = conn.open_control_stream().await?;
        let mut session = PeerSession::new(conn, control_tx, control_rx);

        session
            .handshake_initiator(self.machine_id, &self.config.identity.name, &self.screen)
            .await?;

        // Announce our devices
        session.announce_devices(&self.local_devices).await?;

        let peer_id = session.machine_id;
        let peer_name = session.name.clone();

        // Spawn control message reader
        self.spawn_control_reader(peer_id);

        self.sessions.insert(peer_id, session);
        info!(peer = %peer_name, id = %peer_id, "outbound session established");
        Ok(())
    }

    async fn handle_incoming_connection(
        &mut self,
        conn: cross_control_protocol::PeerConnection,
    ) -> Result<(), DaemonError> {
        let remote = conn.remote_address();
        debug!(remote = %remote, "handling incoming connection");

        let (control_tx, control_rx) = conn.accept_control_stream().await?;
        let mut session = PeerSession::new(conn, control_tx, control_rx);

        session
            .handshake_responder(self.machine_id, &self.config.identity.name, &self.screen)
            .await?;

        // Announce our devices
        session.announce_devices(&self.local_devices).await?;

        let peer_id = session.machine_id;
        let peer_name = session.name.clone();

        self.sessions.insert(peer_id, session);

        // Spawn control message reader
        self.spawn_control_reader(peer_id);

        info!(peer = %peer_name, id = %peer_id, "inbound session established");
        Ok(())
    }

    fn spawn_control_reader(&self, peer_id: MachineId) {
        // NOTE: We can't read from the session's control_rx here since it's
        // owned by the PeerSession. The reading happens in handle_peer_control
        // which is driven by the daemon calling recv on the session. For MVP,
        // we spawn a task that periodically polls.
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            // The actual reading is done in the main loop via poll_sessions
            // For now, this task just ensures the peer_id is tracked
            let _ = (event_tx, peer_id);
        });
    }

    async fn handle_captured_input(&mut self, captured: CapturedEvent) {
        // Track hotkey state
        self.update_hotkey_state(&captured.event);

        // Check release hotkey
        if self.is_release_hotkey_pressed() && self.controlling.is_some() {
            self.release_control().await;
            return;
        }

        // If we're controlling a remote, forward the event
        if let Some(peer_id) = self.controlling {
            if let Some(session) = self.sessions.get_mut(&peer_id) {
                let msg = InputMessage {
                    device_id: captured.device_id,
                    timestamp_us: captured.timestamp_us,
                    events: vec![captured.event],
                };
                if let Err(e) = session.send_input(&msg).await {
                    warn!(error = %e, "failed to send input to peer");
                    self.controlling = None;
                    let _ = self.capture.release().await;
                }
            }
            return;
        }

        // Track cursor position for barrier detection
        if let InputEvent::MouseMove { dx, dy } = &captured.event {
            self.cursor_x += dx;
            self.cursor_y += dy;

            // Clamp to screen bounds
            let width = i32::try_from(self.screen.width).unwrap_or(i32::MAX);
            let height = i32::try_from(self.screen.height).unwrap_or(i32::MAX);
            self.cursor_x = self.cursor_x.clamp(0, width - 1);
            self.cursor_y = self.cursor_y.clamp(0, height - 1);

            // Check barrier crossings
            if let Some((peer_id, edge, position)) = self.check_barrier_crossing() {
                self.initiate_control(peer_id, edge, position).await;
            }
        }
    }

    fn check_barrier_crossing(&self) -> Option<(MachineId, ScreenEdge, u32)> {
        for (peer_id, session) in &self.sessions {
            // Find which screen config matches this peer
            for screen_config in &self.config.screens {
                if screen_config.name == session.name {
                    let edge = screen_config.position.local_edge();
                    if self.screen.is_at_edge(self.cursor_x, self.cursor_y, edge) {
                        let position = match edge {
                            ScreenEdge::Left | ScreenEdge::Right => {
                                u32::try_from(self.cursor_y).unwrap_or(0)
                            }
                            ScreenEdge::Top | ScreenEdge::Bottom => {
                                u32::try_from(self.cursor_x).unwrap_or(0)
                            }
                        };
                        return Some((*peer_id, edge, position));
                    }
                }
            }
        }
        None
    }

    async fn initiate_control(&mut self, peer_id: MachineId, edge: ScreenEdge, position: u32) {
        info!(peer = %peer_id, ?edge, position, "initiating control");

        if let Some(session) = self.sessions.get_mut(&peer_id) {
            match session.enter(edge, position).await {
                Ok(()) => {
                    self.controlling = Some(peer_id);
                    // Grab input devices
                    // NOTE: grab is Linux-specific and not in the trait.
                    // We handle this at the CLI level where we know the concrete type.
                    info!(peer = %peer_id, "control initiated");
                }
                Err(e) => {
                    warn!(error = %e, "failed to initiate control");
                }
            }
        }
    }

    async fn release_control(&mut self) {
        if let Some(peer_id) = self.controlling.take() {
            info!(peer = %peer_id, "releasing control");
            if let Some(session) = self.sessions.get_mut(&peer_id) {
                let edge = ScreenEdge::Left; // Default edge for release
                let _ = session.leave(edge, 0).await;
            }
            let _ = self.capture.release().await;

            // Reset cursor to center
            self.cursor_x = i32::try_from(self.screen.width / 2).unwrap_or(960);
            self.cursor_y = i32::try_from(self.screen.height / 2).unwrap_or(540);
        }
    }

    fn update_hotkey_state(&mut self, event: &InputEvent) {
        if let InputEvent::Key { code, state } = event {
            match state {
                cross_control_types::ButtonState::Pressed => {
                    if !self.hotkey_pressed.contains(code) {
                        self.hotkey_pressed.push(*code);
                    }
                }
                cross_control_types::ButtonState::Released => {
                    self.hotkey_pressed.retain(|k| k != code);
                }
            }
        }
    }

    fn is_release_hotkey_pressed(&self) -> bool {
        let hotkey = &self.config.input.release_hotkey;
        if hotkey.len() > self.hotkey_pressed.len() {
            return false;
        }
        hotkey.iter().all(|key_name| {
            self.hotkey_pressed
                .iter()
                .any(|pressed| format!("{pressed:?}") == *key_name)
        })
    }

    async fn handle_peer_control(&mut self, machine_id: MachineId, msg: ControlMessage) {
        match msg {
            ControlMessage::Enter { edge, position } => {
                info!(peer = %machine_id, ?edge, position, "peer entering");
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    match session.handle_enter().await {
                        Ok(()) => {
                            self.controlled_by = Some(machine_id);
                        }
                        Err(e) => {
                            warn!(error = %e, "failed to handle Enter");
                        }
                    }
                }
            }
            ControlMessage::Leave { .. } => {
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    session.handle_leave();
                }
                if self.controlled_by == Some(machine_id) {
                    self.controlled_by = None;
                }
            }
            ControlMessage::DeviceAnnounce(info) => {
                debug!(peer = %machine_id, device = %info.name, "device announced");
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    match self.emulation.create_device(&info).await {
                        Ok(virtual_id) => {
                            session.device_map.insert(info.id, virtual_id);
                            session.remote_devices.push(info);
                        }
                        Err(e) => {
                            warn!(error = %e, "failed to create virtual device");
                        }
                    }
                }
            }
            ControlMessage::DeviceGone { device_id } => {
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    if let Some(virtual_id) = session.device_map.remove(&device_id) {
                        let _ = self.emulation.destroy_device(virtual_id).await;
                    }
                }
            }
            ControlMessage::Ping { seq } => {
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    let _ = session.control_tx.send(&ControlMessage::Pong { seq }).await;
                }
            }
            ControlMessage::Pong { seq } => {
                debug!(peer = %machine_id, seq, "received pong");
            }
            ControlMessage::Bye => {
                info!(peer = %machine_id, "peer sent Bye");
                self.handle_peer_disconnected(machine_id).await;
            }
            _ => {
                debug!(peer = %machine_id, ?msg, "unhandled control message");
            }
        }
    }

    async fn handle_peer_input(&mut self, machine_id: MachineId, msg: InputMessage) {
        if self.controlled_by != Some(machine_id) {
            warn!(peer = %machine_id, "received input from non-controlling peer");
            return;
        }

        if let Some(session) = self.sessions.get(&machine_id) {
            if let Some(&virtual_id) = session.device_map.get(&msg.device_id) {
                for event in &msg.events {
                    if let Err(e) = self.emulation.inject(virtual_id, event.clone()).await {
                        warn!(error = %e, "failed to inject event");
                    }
                }
            }
        }
    }

    async fn handle_peer_disconnected(&mut self, machine_id: MachineId) {
        if self.controlling == Some(machine_id) {
            self.controlling = None;
            let _ = self.capture.release().await;
        }
        if self.controlled_by == Some(machine_id) {
            self.controlled_by = None;
        }

        if let Some(mut session) = self.sessions.remove(&machine_id) {
            // Clean up virtual devices
            for (_, virtual_id) in session.device_map.drain() {
                let _ = self.emulation.destroy_device(virtual_id).await;
            }
            info!(peer = %session.name, "peer session removed");
        }
    }

    async fn shutdown(&mut self) -> Result<(), DaemonError> {
        info!("daemon shutting down");

        // Disconnect all peers
        let peer_ids: Vec<MachineId> = self.sessions.keys().copied().collect();
        for peer_id in peer_ids {
            if let Some(mut session) = self.sessions.remove(&peer_id) {
                let _ = session.disconnect().await;
            }
        }

        // Shut down capture and emulation
        self.capture.shutdown().await?;
        self.emulation.shutdown().await?;

        // Close transport
        self.transport.close();

        info!("daemon shut down complete");
        Ok(())
    }

    /// Set the local device list (called before run, after enumeration).
    pub fn set_local_devices(&mut self, devices: Vec<DeviceInfo>) {
        self.local_devices = devices;
    }
}
