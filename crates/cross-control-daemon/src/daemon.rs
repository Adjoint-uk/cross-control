//! Core daemon orchestration.

use std::collections::HashMap;
use std::net::SocketAddr;

use cross_control_input::{InputCapture, InputEmulation};
use cross_control_protocol::QuicTransport;
use cross_control_types::{
    CapturedEvent, ControlMessage, DeviceInfo, InputEvent, InputMessage, KeyCode, MachineId,
    ScreenEdge, ScreenGeometry,
};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::DaemonError;
use crate::session::PeerSession;

/// Events processed by the daemon's main loop.
pub enum DaemonEvent {
    /// A new peer connected (inbound) — handed off to a background handshake task.
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
    /// A fully handshaked session is ready (from a background task).
    SessionReady { session: PeerSession },
    /// Shutdown signal.
    Shutdown,
}

/// Observable daemon status (via watch channel).
#[derive(Debug, Clone)]
pub struct DaemonStatus {
    pub controlling: Option<MachineId>,
    pub controlled_by: Option<MachineId>,
    pub session_count: usize,
    pub cursor_x: i32,
    pub cursor_y: i32,
}

impl Default for DaemonStatus {
    fn default() -> Self {
        Self {
            controlling: None,
            controlled_by: None,
            session_count: 0,
            cursor_x: 960,
            cursor_y: 540,
        }
    }
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
    /// The edge the cursor entered from when we are being controlled.
    /// Suppresses Leave checks on this edge until the cursor moves away,
    /// preventing an immediate bounce-back when the cursor starts AT the
    /// entry edge.
    entry_edge: Option<ScreenEdge>,
    /// Hotkey state tracking: set of currently pressed keys.
    hotkey_pressed: Vec<KeyCode>,
    /// Status broadcast channel.
    status_tx: watch::Sender<DaemonStatus>,
    /// Full screen adjacency graph: `(screen_name, edge) → neighbor_name`.
    adjacency: HashMap<(String, ScreenEdge), String>,
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
        let cursor_x = i32::try_from(screen.width / 2).unwrap_or(960);
        let cursor_y = i32::try_from(screen.height / 2).unwrap_or(540);
        let (status_tx, _) = watch::channel(DaemonStatus {
            cursor_x,
            cursor_y,
            ..DaemonStatus::default()
        });

        // Build the full adjacency map.
        // 1) From config.screens: our own direct neighbors.
        let my_name = config.identity.name.clone();
        let mut adjacency: HashMap<(String, ScreenEdge), String> = HashMap::new();
        for sc in &config.screens {
            let edge = sc.position.local_edge();
            adjacency.insert((my_name.clone(), edge), sc.name.clone());
            // Auto-generate inverse: neighbor → opposite edge → us
            adjacency.insert((sc.name.clone(), edge.opposite()), my_name.clone());
        }
        // 2) From config.screen_adjacency: remote edges.
        for adj in &config.screen_adjacency {
            let edge = adj.position.local_edge();
            adjacency.insert((adj.screen.clone(), edge), adj.neighbor.clone());
            // Auto-generate inverse
            adjacency.insert((adj.neighbor.clone(), edge.opposite()), adj.screen.clone());
        }

        Self {
            cursor_x,
            cursor_y,
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
            entry_edge: None,
            hotkey_pressed: Vec::new(),
            status_tx,
            adjacency,
        }
    }

    /// Get a clone of the event sender for feeding events into the daemon.
    pub fn event_sender(&self) -> mpsc::Sender<DaemonEvent> {
        self.event_tx.clone()
    }

    /// Get a status watch receiver for observing daemon state changes.
    pub fn status_receiver(&self) -> watch::Receiver<DaemonStatus> {
        self.status_tx.subscribe()
    }

    /// Run the daemon event loop.
    #[allow(clippy::too_many_lines)]
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

        // Spawn accept loop as a background task. Each accepted connection
        // gets its own handshake task so the event loop never blocks.
        {
            let transport = self.transport.clone();
            let event_tx = self.event_tx.clone();
            let our_id = self.machine_id;
            let our_name = self.config.identity.name.clone();
            let our_screen = self.screen.clone();
            let local_devices = self.local_devices.clone();
            tokio::spawn(async move {
                loop {
                    match transport.accept().await {
                        Ok(conn) => {
                            let tx = event_tx.clone();
                            let name = our_name.clone();
                            let screen = our_screen.clone();
                            let devs = local_devices.clone();
                            tokio::spawn(async move {
                                let remote = conn.remote_address();
                                match perform_handshake_responder(
                                    conn, our_id, &name, &screen, &devs,
                                )
                                .await
                                {
                                    Ok(session) => {
                                        info!(
                                            peer = %session.name,
                                            remote = %remote,
                                            "inbound handshake complete"
                                        );
                                        let _ =
                                            tx.send(DaemonEvent::SessionReady { session }).await;
                                    }
                                    Err(e) => {
                                        warn!(
                                            remote = %remote,
                                            error = %e,
                                            "inbound handshake failed"
                                        );
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            debug!(error = %e, "accept loop ending");
                            break;
                        }
                    }
                }
            });
        }

        // Spawn outbound connection + handshake tasks. Each task connects,
        // completes the handshake, then sends the ready session back.
        for sc in &self.config.screens {
            if let Some(addr_str) = &sc.address {
                let addr: Option<SocketAddr> = addr_str
                    .parse()
                    .or_else(|_| format!("{addr_str}:{}", self.config.daemon.port).parse())
                    .ok();
                if let Some(addr) = addr {
                    let transport = self.transport.clone();
                    let event_tx = self.event_tx.clone();
                    let peer_name = sc.name.clone();
                    let our_id = self.machine_id;
                    let our_name = self.config.identity.name.clone();
                    let our_screen = self.screen.clone();
                    let local_devices = self.local_devices.clone();
                    tokio::spawn(async move {
                        match transport.connect(addr, "cross-control").await {
                            Ok(conn) => {
                                match perform_handshake_initiator(
                                    conn,
                                    our_id,
                                    &our_name,
                                    &our_screen,
                                    &local_devices,
                                )
                                .await
                                {
                                    Ok(session) => {
                                        info!(
                                            peer = %session.name,
                                            address = %addr,
                                            "outbound handshake complete"
                                        );
                                        let _ = event_tx
                                            .send(DaemonEvent::SessionReady { session })
                                            .await;
                                    }
                                    Err(e) => {
                                        warn!(
                                            peer = %peer_name,
                                            address = %addr,
                                            error = %e,
                                            "outbound handshake failed"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    address = %addr,
                                    error = %e,
                                    "failed to connect to peer"
                                );
                            }
                        }
                    });
                }
            }
        }

        info!("daemon running");
        self.broadcast_status();

        // Main event loop — purely event-driven, never blocks on I/O.
        while let Some(event) = self.event_rx.recv().await {
            if self.handle_event(event).await {
                break;
            }
        }

        self.shutdown().await
    }

    /// Handle a single daemon event. Returns `true` if the daemon should shut down.
    async fn handle_event(&mut self, event: DaemonEvent) -> bool {
        match event {
            DaemonEvent::CapturedInput(captured) => {
                self.handle_captured_input(captured).await;
            }
            DaemonEvent::PeerControl { machine_id, msg } => {
                self.handle_peer_control(machine_id, msg).await;
            }
            DaemonEvent::PeerInput { machine_id, msg } => {
                self.handle_peer_input(machine_id, msg).await;
            }
            DaemonEvent::PeerDisconnected(machine_id) => {
                self.handle_peer_disconnected(machine_id).await;
            }
            DaemonEvent::SessionReady { session } => {
                self.handle_session_ready(session);
            }
            DaemonEvent::Shutdown => {
                info!("shutting down");
                return true;
            }
            DaemonEvent::IncomingConnection(conn) => {
                // Spawn handshake in background so we don't block the event loop.
                let tx = self.event_tx.clone();
                let our_id = self.machine_id;
                let our_name = self.config.identity.name.clone();
                let our_screen = self.screen.clone();
                let local_devices = self.local_devices.clone();
                tokio::spawn(async move {
                    match perform_handshake_responder(
                        conn,
                        our_id,
                        &our_name,
                        &our_screen,
                        &local_devices,
                    )
                    .await
                    {
                        Ok(session) => {
                            let _ = tx.send(DaemonEvent::SessionReady { session }).await;
                        }
                        Err(e) => {
                            warn!(error = %e, "incoming connection handshake failed");
                        }
                    }
                });
            }
        }
        self.broadcast_status();
        false
    }

    fn broadcast_status(&self) {
        let _ = self.status_tx.send(DaemonStatus {
            controlling: self.controlling,
            controlled_by: self.controlled_by,
            session_count: self.sessions.len(),
            cursor_x: self.cursor_x,
            cursor_y: self.cursor_y,
        });
    }

    fn handle_session_ready(&mut self, session: PeerSession) {
        let peer_id = session.machine_id;
        let peer_name = session.name.clone();
        self.sessions.insert(peer_id, session);
        self.spawn_control_reader(peer_id);
        info!(peer = %peer_name, id = %peer_id, "session established");
    }

    fn spawn_control_reader(&mut self, peer_id: MachineId) {
        let mut control_rx = self
            .sessions
            .get_mut(&peer_id)
            .and_then(PeerSession::take_control_rx)
            .expect("control_rx should exist after handshake");
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            loop {
                match control_rx.recv::<ControlMessage>().await {
                    Ok(Some(msg)) => {
                        if event_tx
                            .send(DaemonEvent::PeerControl {
                                machine_id: peer_id,
                                msg,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => {
                        // Stream closed cleanly
                        let _ = event_tx.send(DaemonEvent::PeerDisconnected(peer_id)).await;
                        break;
                    }
                    Err(e) => {
                        debug!(peer = %peer_id, error = %e, "control reader error");
                        let _ = event_tx.send(DaemonEvent::PeerDisconnected(peer_id)).await;
                        break;
                    }
                }
            }
        });
    }

    /// Accept the unidirectional input stream from the remote peer, then start
    /// reading input messages from it. This runs as a spawned task because the
    /// QUIC stream may not be visible to `accept_uni` until the remote sends
    /// data on it.
    fn spawn_accept_input_stream(&self, peer_id: MachineId) {
        let Some(session) = self.sessions.get(&peer_id) else {
            return;
        };
        let connection = session.connection.clone();
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            match connection.accept_input_stream().await {
                Ok(input_rx) => {
                    debug!(peer = %peer_id, "accepted input stream from controller");
                    Self::spawn_input_reader_task(event_tx, input_rx, peer_id);
                }
                Err(e) => {
                    warn!(peer = %peer_id, error = %e, "failed to accept input stream");
                }
            }
        });
    }

    fn spawn_input_reader_task(
        event_tx: mpsc::Sender<DaemonEvent>,
        mut input_rx: cross_control_protocol::MessageReceiver,
        peer_id: MachineId,
    ) {
        tokio::spawn(async move {
            loop {
                match input_rx.recv::<InputMessage>().await {
                    Ok(Some(msg)) => {
                        if event_tx
                            .send(DaemonEvent::PeerInput {
                                machine_id: peer_id,
                                msg,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        debug!(peer = %peer_id, error = %e, "input reader error");
                        break;
                    }
                }
            }
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
                debug!(peer = %peer_id, device = ?msg.device_id, "forwarding input to peer");
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
            match session.send_enter(edge, position).await {
                Ok(()) => {
                    // Don't set controlling yet — wait for EnterAck via event loop
                    info!(peer = %peer_id, "Enter sent, awaiting EnterAck");
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

    #[allow(clippy::too_many_lines)]
    async fn handle_peer_control(&mut self, machine_id: MachineId, msg: ControlMessage) {
        match msg {
            ControlMessage::Enter { edge, position } => {
                info!(peer = %machine_id, ?edge, position, "peer entering");
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    match session.handle_enter().await {
                        Ok(()) => {
                            self.controlled_by = Some(machine_id);
                            // The edge in Enter is the exit edge on the controller's
                            // screen. We need the opposite edge — where the cursor
                            // enters our screen.
                            let entry_edge = edge.opposite();
                            self.entry_edge = Some(entry_edge);
                            let pos = i32::try_from(position).unwrap_or(0);
                            match entry_edge {
                                ScreenEdge::Left => {
                                    self.cursor_x = 0;
                                    self.cursor_y = pos;
                                }
                                ScreenEdge::Right => {
                                    let w = i32::try_from(self.screen.width).unwrap_or(1920);
                                    self.cursor_x = w - 1;
                                    self.cursor_y = pos;
                                }
                                ScreenEdge::Top => {
                                    self.cursor_x = pos;
                                    self.cursor_y = 0;
                                }
                                ScreenEdge::Bottom => {
                                    let h = i32::try_from(self.screen.height).unwrap_or(1080);
                                    self.cursor_x = pos;
                                    self.cursor_y = h - 1;
                                }
                            }
                            // Accept input stream asynchronously — the initiator
                            // opened a uni stream but QUIC may not have delivered
                            // the stream frame yet.
                            self.spawn_accept_input_stream(machine_id);
                        }
                        Err(e) => {
                            warn!(error = %e, "failed to handle Enter");
                        }
                    }
                }
            }
            ControlMessage::EnterAck => {
                info!(peer = %machine_id, "received EnterAck");
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    session.set_controlling();
                }
                self.controlling = Some(machine_id);
            }
            ControlMessage::Leave { edge, position } => {
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    session.handle_leave();
                }
                if self.controlled_by == Some(machine_id) {
                    self.controlled_by = None;
                    self.entry_edge = None;
                }
                // If we were controlling this peer, check adjacency map for
                // multi-hop: maybe the cursor should go to another screen
                // rather than returning to us.
                if self.controlling == Some(machine_id) {
                    info!(peer = %machine_id, ?edge, position, "peer sent Leave");
                    self.controlling = None;
                    let _ = self.capture.release().await;

                    // Look up the leaving peer's name
                    let peer_name = self.sessions.get(&machine_id).map(|s| s.name.clone());

                    // Check adjacency map: where should the cursor go?
                    let next_target = peer_name
                        .as_ref()
                        .and_then(|name| self.adjacency.get(&(name.clone(), edge)).cloned());

                    // If the next target is us (local machine), fall through
                    // to the default cursor-return behavior.
                    let my_name = self.config.identity.name.clone();
                    let next_target = next_target.filter(|t| *t != my_name);

                    // Try to find the MachineId for the next target
                    let next_peer = next_target.and_then(|target_name| {
                        self.sessions
                            .iter()
                            .find(|(_, s)| s.name == target_name)
                            .map(|(id, _)| *id)
                    });

                    if let Some(next_peer_id) = next_peer {
                        // Multi-hop: transfer control to the next screen
                        info!(
                            next_peer = %next_peer_id,
                            ?edge,
                            position,
                            "multi-hop: transferring control to next screen"
                        );
                        self.initiate_control(next_peer_id, edge, position).await;
                    } else {
                        // No multi-hop target — cursor returns to us.
                        // Place cursor at the opposite edge.
                        let return_edge = edge.opposite();
                        let pos = i32::try_from(position).unwrap_or(0);
                        let width = i32::try_from(self.screen.width).unwrap_or(1920);
                        let height = i32::try_from(self.screen.height).unwrap_or(1080);
                        match return_edge {
                            ScreenEdge::Left => {
                                self.cursor_x = 0;
                                self.cursor_y = pos;
                            }
                            ScreenEdge::Right => {
                                self.cursor_x = width - 1;
                                self.cursor_y = pos;
                            }
                            ScreenEdge::Top => {
                                self.cursor_x = pos;
                                self.cursor_y = 0;
                            }
                            ScreenEdge::Bottom => {
                                self.cursor_x = pos;
                                self.cursor_y = height - 1;
                            }
                        }
                    }
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
            warn!(peer = %machine_id, controlled_by = ?self.controlled_by, "received input from non-controlling peer");
            return;
        }

        // Track cursor position from remote input for barrier detection
        for event in &msg.events {
            if let InputEvent::MouseMove { dx, dy } = event {
                self.cursor_x += dx;
                self.cursor_y += dy;
                let width = i32::try_from(self.screen.width).unwrap_or(i32::MAX);
                let height = i32::try_from(self.screen.height).unwrap_or(i32::MAX);
                self.cursor_x = self.cursor_x.clamp(0, width - 1);
                self.cursor_y = self.cursor_y.clamp(0, height - 1);
            }
        }

        // Clear the entry_edge suppression once the cursor moves away from
        // the entry edge. This prevents immediate bounce-back when the cursor
        // starts AT the entry edge, while still allowing Leave if the user
        // deliberately moves back to it later.
        if let Some(entry) = self.entry_edge {
            if !self.screen.is_at_edge(self.cursor_x, self.cursor_y, entry) {
                self.entry_edge = None;
            }
        }

        // Check if cursor has hit ANY screen edge — if so, send Leave to
        // the controller. The Leave message includes the exit edge so the
        // controller can decide where to route the cursor (multi-hop via
        // adjacency map, or return to itself).
        if let Some(controller_id) = self.controlled_by {
            for screen_config in &self.config.screens {
                let edge = screen_config.position.local_edge();
                // Skip the entry edge while cursor is still on it (suppression).
                if self.entry_edge == Some(edge) {
                    continue;
                }
                if self.screen.is_at_edge(self.cursor_x, self.cursor_y, edge) {
                    let position = match edge {
                        ScreenEdge::Left | ScreenEdge::Right => {
                            u32::try_from(self.cursor_y).unwrap_or(0)
                        }
                        ScreenEdge::Top | ScreenEdge::Bottom => {
                            u32::try_from(self.cursor_x).unwrap_or(0)
                        }
                    };
                    info!(
                        peer = %controller_id,
                        ?edge,
                        neighbor = %screen_config.name,
                        position,
                        "cursor hit edge, sending Leave"
                    );
                    if let Some(session) = self.sessions.get_mut(&controller_id) {
                        let _ = session.leave(edge, position).await;
                    }
                    self.controlled_by = None;
                    self.entry_edge = None;
                    return;
                }
            }
        }

        if let Some(session) = self.sessions.get(&machine_id) {
            if let Some(&virtual_id) = session.device_map.get(&msg.device_id) {
                for event in &msg.events {
                    if let Err(e) = self.emulation.inject(virtual_id, event.clone()).await {
                        warn!(error = %e, "failed to inject event");
                    }
                }
            } else {
                debug!(peer = %machine_id, device_id = ?msg.device_id, "no virtual device for input device");
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
            self.entry_edge = None;
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

/// Perform a responder handshake in a background task (accept bidi stream,
/// read Hello, send Welcome, announce devices).
async fn perform_handshake_responder(
    conn: cross_control_protocol::PeerConnection,
    our_id: MachineId,
    our_name: &str,
    our_screen: &ScreenGeometry,
    local_devices: &[DeviceInfo],
) -> Result<PeerSession, DaemonError> {
    let (control_tx, control_rx) = conn.accept_control_stream().await?;
    let mut session = PeerSession::new(conn, control_tx, control_rx);
    session
        .handshake_responder(our_id, our_name, our_screen)
        .await?;
    session.announce_devices(local_devices).await?;
    Ok(session)
}

/// Perform an initiator handshake in a background task (open bidi stream,
/// send Hello, read Welcome, announce devices).
async fn perform_handshake_initiator(
    conn: cross_control_protocol::PeerConnection,
    our_id: MachineId,
    our_name: &str,
    our_screen: &ScreenGeometry,
    local_devices: &[DeviceInfo],
) -> Result<PeerSession, DaemonError> {
    let (control_tx, control_rx) = conn.open_control_stream().await?;
    let mut session = PeerSession::new(conn, control_tx, control_rx);
    session
        .handshake_initiator(our_id, our_name, our_screen)
        .await?;
    session.announce_devices(local_devices).await?;
    Ok(session)
}
