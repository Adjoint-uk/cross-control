//! Core daemon orchestration.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cross_control_clipboard::ClipboardProvider;
use cross_control_discovery::{
    Discovery, DiscoveryAggregator, DiscoveryEvent, MdnsDiscovery, Peer, StaticDiscovery,
};
use cross_control_input::{InputCapture, InputEmulation};
use cross_control_protocol::tls::normalize_fingerprint;
use cross_control_protocol::{PeerConnection, QuicTransport};
use cross_control_types::{
    ButtonState, CapturedEvent, ClipboardFormat, ClipboardMessage, ControlMessage, DeviceInfo,
    DisplayLayout, InputEvent, InputMessage, KeyCode, MachineId, MouseButton, ScreenEdge,
    ScreenGeometry, VirtualDeviceId,
};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::DaemonError;
use crate::session::PeerSession;
use crate::status::{status_file_path, PeerStatus, StatusSnapshot};
use crate::trust::{TrustError, TrustStore};

/// How often the daemon pings each peer for latency and writes the status
/// snapshot file the CLI reads.
const STATUS_INTERVAL: Duration = Duration::from_secs(2);

/// How long to wait for `EnterAck` before abandoning a hand-off.
const ENTER_TIMEOUT: Duration = Duration::from_secs(3);

/// After a refused or timed-out hand-off, how long before we try again, so a
/// busy peer is not sent an `Enter` on every mouse movement at the edge.
const ENTER_COOLDOWN: Duration = Duration::from_secs(1);

/// All four screen edges, in a fixed order.
const ALL_EDGES: [ScreenEdge; 4] = [
    ScreenEdge::Left,
    ScreenEdge::Right,
    ScreenEdge::Top,
    ScreenEdge::Bottom,
];

/// A key or mouse button that is currently held down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HeldInput {
    Key(KeyCode),
    Button(MouseButton),
}

impl HeldInput {
    /// The held input and its new state, if `event` is a key or button event.
    fn from_event(event: &InputEvent) -> Option<(Self, ButtonState)> {
        match event {
            InputEvent::Key { code, state } => Some((Self::Key(*code), *state)),
            InputEvent::MouseButton { button, state } => Some((Self::Button(*button), *state)),
            _ => None,
        }
    }

    /// The event that releases this input.
    fn release_event(self) -> InputEvent {
        match self {
            Self::Key(code) => InputEvent::Key {
                code,
                state: ButtonState::Released,
            },
            Self::Button(button) => InputEvent::MouseButton {
                button,
                state: ButtonState::Released,
            },
        }
    }
}

/// Track presses and releases into a held-input set.
fn track_held(held: &mut HashSet<HeldInput>, event: &InputEvent) {
    if let Some((input, state)) = HeldInput::from_event(event) {
        match state {
            ButtonState::Pressed => {
                held.insert(input);
            }
            ButtonState::Released => {
                held.remove(&input);
            }
        }
    }
}

/// Scale a position along an edge from a source length to a target length,
/// clamped to the target range.
fn scale_along_edge(position: u32, source_len: u32, target_len: u32) -> u32 {
    let max = target_len.saturating_sub(1);
    if source_len == 0 {
        return position.min(max);
    }
    let scaled = u64::from(position) * u64::from(target_len) / u64::from(source_len);
    u32::try_from(scaled).unwrap_or(u32::MAX).min(max)
}

/// The last pixel on each axis of `screen` (inclusive).
fn screen_max(screen: &ScreenGeometry) -> (i32, i32) {
    let last = |origin: i32, len: u32| {
        origin
            .saturating_add(i32::try_from(len).unwrap_or(i32::MAX))
            .saturating_sub(1)
            .max(origin)
    };
    (last(screen.x, screen.width), last(screen.y, screen.height))
}

/// Clamp a point to `screen`, which may start anywhere (multi-monitor layouts
/// can have negative origins).
fn clamp_point(screen: &ScreenGeometry, x: i32, y: i32) -> (i32, i32) {
    let (x1, y1) = screen_max(screen);
    (x.clamp(screen.x, x1), y.clamp(screen.y, y1))
}

/// Length of a screen's edge: height for left/right edges, width for top/bottom.
fn edge_len(screen: &ScreenGeometry, edge: ScreenEdge) -> u32 {
    match edge {
        ScreenEdge::Left | ScreenEdge::Right => screen.height,
        ScreenEdge::Top | ScreenEdge::Bottom => screen.width,
    }
}

/// Events processed by the daemon's main loop.
pub enum DaemonEvent {
    /// A new peer connected (inbound) — handed off to a background handshake task.
    IncomingConnection(PeerConnection),
    /// A captured local input event.
    CapturedInput(CapturedEvent),
    /// A control message from a peer. `conn_id` identifies the connection
    /// it arrived on ([`PeerSession::conn_id`]); events from a connection
    /// that has since been replaced are dropped.
    PeerControl {
        machine_id: MachineId,
        conn_id: u64,
        msg: ControlMessage,
    },
    /// An input message from a peer.
    PeerInput {
        machine_id: MachineId,
        conn_id: u64,
        msg: InputMessage,
    },
    /// A peer's connection closed.
    PeerDisconnected { machine_id: MachineId, conn_id: u64 },
    /// A fully handshaked session is ready (from a background task). Boxed
    /// because `PeerSession` is much larger than the other variants.
    SessionReady { session: Box<PeerSession> },
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
    /// This machine's full monitor layout, sent to peers in the handshake.
    layout: DisplayLayout,
    /// Bounding box of `layout` — the combined desktop used for all edge
    /// detection and cursor clamping.
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
    /// Peer we sent `Enter` to and when, while we wait for `EnterAck`.
    pending_enter: Option<(MachineId, Instant)>,
    /// No new hand-off is started before this instant (see [`ENTER_COOLDOWN`]).
    enter_cooldown_until: Option<Instant>,
    /// Keys and buttons physically held on this machine. A hand-off waits
    /// until none are held: grabbing the devices mid-press would leave the
    /// key stuck down in local applications.
    local_held: HashSet<HeldInput>,
    /// Keys and buttons we injected on behalf of the controlling peer and
    /// have not released yet, per virtual device. Released when control
    /// leaves, so nothing stays pressed.
    injected_held: HashMap<VirtualDeviceId, HashSet<HeldInput>>,
    /// While controlled: the edges of our screen that lead somewhere, as told
    /// by the controller in `Enter`. Reaching one sends `Leave`.
    exit_edges: Vec<ScreenEdge>,
    /// Status broadcast channel.
    status_tx: watch::Sender<DaemonStatus>,
    /// Full screen adjacency graph: `(screen_name, edge) → neighbor_name`.
    adjacency: HashMap<(String, ScreenEdge), String>,
    /// Our own TLS cert fingerprint (lowercase hex), advertised via mDNS so
    /// peers can TOFU-pin before connecting. `None` skips the fingerprint
    /// TXT record.
    local_fingerprint: Option<String>,
    /// Remote addresses we currently have a session with or an in-flight
    /// outbound handshake to. Used to suppress duplicate dial attempts when
    /// a peer surfaces via more than one discovery backend.
    dialed_addrs: Arc<Mutex<HashSet<SocketAddr>>>,
    /// Local clipboard backend, optional so headless or test daemons can
    /// skip it. When `Some`, the daemon syncs the text clipboard on each
    /// control hand-off.
    clipboard: Option<Box<dyn ClipboardProvider>>,
    /// Latest status snapshot, shared with the file-writer task that the CLI
    /// reads. Updated on peer, focus, and latency changes.
    status_snapshot: Arc<Mutex<StatusSnapshot>>,
    /// Monotonic sequence for liveness pings.
    next_ping_seq: u64,
    /// Handle to the background task that persists the status snapshot, so it
    /// can be stopped cleanly on shutdown before the file is removed.
    status_writer: Option<tokio::task::JoinHandle<()>>,
    /// Which peers may connect (config pins and `known_peers`).
    trust: TrustStore,
}

/// Everything a background handshake task needs, cloned out of the daemon so
/// the task does not borrow it.
#[derive(Clone)]
struct HandshakeCtx {
    our_id: MachineId,
    our_name: String,
    our_layout: DisplayLayout,
    local_devices: Vec<DeviceInfo>,
    trust: TrustStore,
    event_tx: mpsc::Sender<DaemonEvent>,
}

impl HandshakeCtx {
    /// The authorization check for a handshake on `conn`: the peer's proven
    /// certificate fingerprint must be trusted for the name it announces.
    fn authorizer(&self, conn: &PeerConnection) -> impl FnOnce(&str) -> Result<(), DaemonError> {
        let trust = self.trust.clone();
        let fingerprint = conn.peer_fingerprint();
        move |name| {
            let fingerprint = fingerprint.ok_or(TrustError::NoCertificate)?;
            let how = trust.authorize(name, &fingerprint)?;
            debug!(peer = %name, ?how, "peer authorized");
            Ok(())
        }
    }
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
        let layout = config.display_layout();
        let screen = layout.bounding_box();
        let trust =
            TrustStore::from_config(&config, crate::setup::config_dir().join("known_peers"));
        let (event_tx, event_rx) = mpsc::channel(1024);
        let cursor_x = screen
            .x
            .saturating_add(i32::try_from(screen.width / 2).unwrap_or(960));
        let cursor_y = screen
            .y
            .saturating_add(i32::try_from(screen.height / 2).unwrap_or(540));
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
            layout,
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
            pending_enter: None,
            enter_cooldown_until: None,
            local_held: HashSet::new(),
            injected_held: HashMap::new(),
            exit_edges: Vec::new(),
            status_tx,
            adjacency,
            local_fingerprint: None,
            dialed_addrs: Arc::new(Mutex::new(HashSet::new())),
            clipboard: None,
            status_snapshot: Arc::new(Mutex::new(StatusSnapshot::default())),
            next_ping_seq: 0,
            status_writer: None,
            trust,
        }
    }

    fn handshake_ctx(&self) -> HandshakeCtx {
        HandshakeCtx {
            our_id: self.machine_id,
            our_name: self.config.identity.name.clone(),
            our_layout: self.layout.clone(),
            local_devices: self.local_devices.clone(),
            trust: self.trust.clone(),
            event_tx: self.event_tx.clone(),
        }
    }

    /// Get a clone of the event sender for feeding events into the daemon.
    pub fn event_sender(&self) -> mpsc::Sender<DaemonEvent> {
        self.event_tx.clone()
    }

    /// Set the local TLS cert fingerprint (lowercase hex) to advertise via
    /// mDNS. Call this once at startup, after cert generation. If unset, the
    /// mDNS advert omits the fingerprint TXT, which downgrades peers to
    /// blind first-contact TOFU.
    pub fn set_local_fingerprint(&mut self, fingerprint: String) {
        self.local_fingerprint = Some(fingerprint);
    }

    /// Install a clipboard backend. When set, the daemon syncs the local
    /// text clipboard to the remote peer on every control hand-off
    /// (controller → controlled). Skip this call to run without clipboard
    /// sync — handy in headless tests.
    pub fn set_clipboard(&mut self, clipboard: Box<dyn ClipboardProvider>) {
        self.clipboard = Some(clipboard);
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
            let ctx = self.handshake_ctx();
            tokio::spawn(async move {
                loop {
                    match transport.accept().await {
                        Ok(conn) => spawn_inbound_handshake(conn, ctx.clone()),
                        Err(e) => {
                            debug!(error = %e, "accept loop ending");
                            break;
                        }
                    }
                }
            });
        }

        // Start discovery — mDNS for LAN peers, StaticDiscovery for any
        // config screens with an explicit address. The aggregator merges the
        // two streams and dedupes by MachineId; the daemon dedupes by
        // SocketAddr below so duplicate dial attempts are suppressed even
        // when MachineIds differ (e.g. static config emits placeholder ids
        // until handshake confirms the real one).
        let mut aggregator = DiscoveryAggregator::new();

        // Static peers from config.
        let static_entries: Vec<(String, SocketAddr, Option<String>)> = self
            .config
            .screens
            .iter()
            .filter_map(|sc| {
                let addr_str = sc.address.as_ref()?;
                let addr: SocketAddr = addr_str
                    .parse()
                    .or_else(|_| format!("{addr_str}:{}", self.config.daemon.port).parse())
                    .ok()?;
                Some((sc.name.clone(), addr, sc.fingerprint.clone()))
            })
            .collect();
        if !static_entries.is_empty() {
            aggregator.push(Box::new(StaticDiscovery::from_addresses(static_entries)));
        }

        // mDNS backend — gated on the daemon.discovery config flag so a user
        // can disable multicast for networks where it's blocked or undesired.
        if self.config.daemon.discovery {
            let mdns = if let Some(fp) = self.local_fingerprint.clone() {
                MdnsDiscovery::with_fingerprint(fp)
            } else {
                MdnsDiscovery::new()
            };
            match mdns {
                Ok(backend) => aggregator.push(Box::new(backend)),
                Err(e) => warn!(error = %e, "mDNS backend unavailable, continuing without it"),
            }
        }

        if !aggregator.is_empty() {
            if let Err(e) = aggregator
                .advertise(
                    self.machine_id,
                    &self.config.identity.name,
                    transport_local.port(),
                )
                .await
            {
                warn!(error = %e, "discovery advertise failed");
            }
            match aggregator.browse().await {
                Ok(mut rx) => {
                    let transport = self.transport.clone();
                    let ctx = self.handshake_ctx();
                    let dialed = Arc::clone(&self.dialed_addrs);
                    tokio::spawn(async move {
                        while let Some(event) = rx.recv().await {
                            match event {
                                DiscoveryEvent::PeerFound(peer) => {
                                    if !claim_dial(&dialed, peer.address) {
                                        debug!(
                                            address = %peer.address,
                                            "skipping duplicate dial"
                                        );
                                        continue;
                                    }
                                    spawn_outbound_handshake(
                                        transport.clone(),
                                        peer,
                                        ctx.clone(),
                                        Arc::clone(&dialed),
                                    );
                                }
                                DiscoveryEvent::PeerLost(id) => {
                                    debug!(peer = %id, "discovery: peer lost");
                                }
                            }
                        }
                    });
                }
                Err(e) => warn!(error = %e, "discovery browse failed"),
            }
        }

        info!("daemon running");
        self.broadcast_status();
        self.update_snapshot();

        // Persist the status snapshot to disk on a timer so a separate
        // `cross-control status` process can read live peer/latency/focus
        // state. Runs as its own task; stopped in `shutdown()`.
        {
            let snapshot = Arc::clone(&self.status_snapshot);
            self.status_writer = Some(tokio::spawn(async move {
                let path = status_file_path();
                let mut ticker = tokio::time::interval(STATUS_INTERVAL);
                loop {
                    ticker.tick().await;
                    let json = snapshot.lock().ok().and_then(|s| s.to_json().ok());
                    if let Some(json) = json {
                        let _ = std::fs::write(&path, json);
                    }
                }
            }));
        }

        // Ping peers on the same cadence so latency stays fresh. Ticks are
        // handled on the event loop itself, keeping all session mutation
        // single-owner. The first tick fires immediately — swallow it.
        let mut ping_interval = tokio::time::interval(STATUS_INTERVAL);
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ping_interval.tick().await;

        // Main event loop — event-driven, with a periodic ping tick. Never
        // blocks on I/O.
        loop {
            tokio::select! {
                maybe_event = self.event_rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            if self.handle_event(event).await {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = ping_interval.tick() => {
                    self.expire_pending_enter();
                    self.send_pings().await;
                }
            }
        }

        self.shutdown().await
    }

    /// Send a liveness ping to every peer and record the send time so the
    /// round-trip time can be computed when the `Pong` returns. Refreshes the
    /// status snapshot afterward.
    async fn send_pings(&mut self) {
        let seq = self.next_ping_seq;
        self.next_ping_seq = self.next_ping_seq.wrapping_add(1);
        for session in self.sessions.values_mut() {
            if session
                .control_tx
                .send(&ControlMessage::Ping { seq })
                .await
                .is_ok()
            {
                session.record_ping_sent(seq);
            }
        }
        self.update_snapshot();
    }

    /// Rebuild the shared status snapshot from current sessions and focus.
    /// Cheap — a handful of peers — and called only on peer/focus/latency
    /// transitions, never on the mouse-move hot path.
    fn update_snapshot(&self) {
        let peers = self
            .sessions
            .values()
            .map(|s| PeerStatus {
                name: s.name.clone(),
                machine_id: s.machine_id.to_string(),
                address: s.connection.remote_address().to_string(),
                state: s.state.to_string(),
                latency_ms: s
                    .last_rtt()
                    .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            })
            .collect();
        let name_of = |id: MachineId| self.sessions.get(&id).map(|s| s.name.clone());
        let snapshot = StatusSnapshot {
            name: self.config.identity.name.clone(),
            controlling: self.controlling.and_then(name_of),
            controlled_by: self.controlled_by.and_then(name_of),
            peers,
        };
        if let Ok(mut snap) = self.status_snapshot.lock() {
            *snap = snapshot;
        }
    }

    /// Handle a single daemon event. Returns `true` if the daemon should shut down.
    async fn handle_event(&mut self, event: DaemonEvent) -> bool {
        match event {
            DaemonEvent::CapturedInput(captured) => {
                self.handle_captured_input(captured).await;
            }
            DaemonEvent::PeerControl {
                machine_id,
                conn_id,
                msg,
            } => {
                if self.is_current(machine_id, conn_id) {
                    self.handle_peer_control(machine_id, msg).await;
                }
            }
            DaemonEvent::PeerInput {
                machine_id,
                conn_id,
                msg,
            } => {
                if self.is_current(machine_id, conn_id) {
                    self.handle_peer_input(machine_id, msg).await;
                }
            }
            DaemonEvent::PeerDisconnected {
                machine_id,
                conn_id,
            } => {
                if self.is_current(machine_id, conn_id) {
                    self.handle_peer_disconnected(machine_id).await;
                } else {
                    debug!(peer = %machine_id, conn_id, "ignoring disconnect of a replaced connection");
                }
            }
            DaemonEvent::SessionReady { session } => {
                self.handle_session_ready(*session).await;
            }
            DaemonEvent::Shutdown => {
                info!("shutting down");
                return true;
            }
            DaemonEvent::IncomingConnection(conn) => {
                spawn_inbound_handshake(conn, self.handshake_ctx());
            }
        }
        self.broadcast_status();
        // Refresh the CLI-facing snapshot after every transition. Cheap (a
        // few peers) and keeps peers/latency/focus current without touching
        // the file — the writer task persists it on its own timer.
        self.update_snapshot();
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

    /// Whether `conn_id` is the live connection to `machine_id`.
    fn is_current(&self, machine_id: MachineId, conn_id: u64) -> bool {
        self.sessions
            .get(&machine_id)
            .is_some_and(|s| s.conn_id == conn_id)
    }

    async fn handle_session_ready(&mut self, session: PeerSession) {
        let peer_id = session.machine_id;
        let peer_name = session.name.clone();

        if peer_id == self.machine_id {
            warn!(peer = %peer_name, "peer claims our own machine id; dropping connection");
            session.connection.close_with_reason("machine id collision");
            return;
        }

        if let Some(existing) = self.sessions.get(&peer_id) {
            // A machine id is only as good as the certificate behind it: an
            // authorized peer must not take over another peer's session.
            if existing.fingerprint != session.fingerprint {
                warn!(
                    peer = %peer_name,
                    id = %peer_id,
                    "a different certificate claims an existing peer's machine id; dropping it"
                );
                session.connection.close_with_reason("machine id in use");
                return;
            }
            // Two connections to the same peer, usually because both sides
            // dialed at once.
            let keep_new = keep_new_connection(
                self.machine_id,
                peer_id,
                existing.initiated_by,
                session.initiated_by,
            );
            if !keep_new {
                debug!(peer = %peer_name, "duplicate connection; keeping the existing one");
                session.connection.close_with_reason("duplicate connection");
                return;
            }
            debug!(peer = %peer_name, "duplicate connection; replacing the existing one");
            let old = existing.connection.clone();
            self.handle_peer_disconnected(peer_id).await;
            old.close_with_reason("superseded by a newer connection");
        }

        let remote_addr = session.connection.remote_address();
        // Reserve the address so subsequent discovery events for this peer
        // don't trigger a redundant outbound dial. Released on disconnect.
        let _ = claim_dial(&self.dialed_addrs, remote_addr);
        self.sessions.insert(peer_id, session);
        self.spawn_control_reader(peer_id);
        info!(peer = %peer_name, id = %peer_id, "session established");
    }

    fn spawn_control_reader(&mut self, peer_id: MachineId) {
        let Some(session) = self.sessions.get_mut(&peer_id) else {
            return;
        };
        let conn_id = session.conn_id;
        let mut control_rx = session
            .take_control_rx()
            .expect("control_rx should exist after handshake");
        let event_tx = self.event_tx.clone();
        let disconnected = DaemonEvent::PeerDisconnected {
            machine_id: peer_id,
            conn_id,
        };
        tokio::spawn(async move {
            loop {
                match control_rx.recv::<ControlMessage>().await {
                    Ok(Some(msg)) => {
                        if event_tx
                            .send(DaemonEvent::PeerControl {
                                machine_id: peer_id,
                                conn_id,
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
                        let _ = event_tx.send(disconnected).await;
                        break;
                    }
                    Err(e) => {
                        debug!(peer = %peer_id, error = %e, "control reader error");
                        let _ = event_tx.send(disconnected).await;
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
        let conn_id = session.conn_id;
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            match connection.accept_input_stream().await {
                Ok(input_rx) => {
                    debug!(peer = %peer_id, "accepted input stream from controller");
                    Self::spawn_input_reader_task(event_tx, input_rx, peer_id, conn_id);
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
        conn_id: u64,
    ) {
        tokio::spawn(async move {
            loop {
                match input_rx.recv::<InputMessage>().await {
                    Ok(Some(msg)) => {
                        if event_tx
                            .send(DaemonEvent::PeerInput {
                                machine_id: peer_id,
                                conn_id,
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
        self.update_hotkey_state(&captured.event);
        track_held(&mut self.local_held, &captured.event);
        self.expire_pending_enter();

        // The release hotkey always wins: it ends control, and also abandons
        // a hand-off that is still waiting for its ack.
        if self.is_release_hotkey_pressed()
            && (self.controlling.is_some() || self.pending_enter.is_some())
        {
            self.release_control().await;
            return;
        }

        // If we're controlling a remote, forward the event.
        if let Some(peer_id) = self.controlling {
            let msg = InputMessage {
                device_id: captured.device_id,
                timestamp_us: captured.timestamp_us,
                events: vec![captured.event],
            };
            let sent = match self.sessions.get_mut(&peer_id) {
                Some(session) => session.send_input(&msg).await,
                None => Ok(()),
            };
            if let Err(e) = sent {
                warn!(peer = %peer_id, error = %e, "failed to send input to peer; taking control back");
                self.drop_control(peer_id).await;
            }
            return;
        }

        // Track the virtual cursor for barrier detection.
        if let InputEvent::MouseMove { dx, dy } = &captured.event {
            self.move_cursor(*dx, *dy);
            if let Some((peer_id, edge, position)) = self.check_barrier_crossing() {
                self.initiate_control(peer_id, edge, position, false).await;
            }
        }
    }

    /// Move the virtual cursor by a relative amount, saturating instead of
    /// overflowing (deltas can come from the network), then clamp it to the
    /// combined desktop.
    fn move_cursor(&mut self, dx: i32, dy: i32) {
        self.cursor_x = self.cursor_x.saturating_add(dx);
        self.cursor_y = self.cursor_y.saturating_add(dy);
        self.clamp_cursor();
    }

    /// The last pixel on each axis of the combined desktop.
    fn screen_max(&self) -> (i32, i32) {
        screen_max(&self.screen)
    }

    /// Clamp the virtual cursor to the desktop, which may not start at (0, 0).
    fn clamp_cursor(&mut self) {
        (self.cursor_x, self.cursor_y) = clamp_point(&self.screen, self.cursor_x, self.cursor_y);
    }

    /// The cursor's offset along `edge`, measured from the edge's start (its
    /// top or left end).
    fn edge_offset(&self, edge: ScreenEdge) -> u32 {
        let off = match edge {
            ScreenEdge::Left | ScreenEdge::Right => self.cursor_y.saturating_sub(self.screen.y),
            ScreenEdge::Top | ScreenEdge::Bottom => self.cursor_x.saturating_sub(self.screen.x),
        };
        u32::try_from(off).unwrap_or(0)
    }

    /// Put the cursor on `edge`, at a position given along a peer's edge of
    /// length `source_len`, scaled to our own resolution.
    fn place_at_edge(&mut self, edge: ScreenEdge, position: u32, source_len: u32) {
        let offset = scale_along_edge(position, source_len, edge_len(&self.screen, edge));
        let offset = i32::try_from(offset).unwrap_or(0);
        let (x1, y1) = self.screen_max();
        let (x0, y0) = (self.screen.x, self.screen.y);
        (self.cursor_x, self.cursor_y) = match edge {
            ScreenEdge::Left => (x0, y0.saturating_add(offset)),
            ScreenEdge::Right => (x1, y0.saturating_add(offset)),
            ScreenEdge::Top => (x0.saturating_add(offset), y0),
            ScreenEdge::Bottom => (x0.saturating_add(offset), y1),
        };
        self.clamp_cursor();
    }

    /// Put the cursor back in the middle of the desktop.
    fn center_cursor(&mut self) {
        self.cursor_x = self
            .screen
            .x
            .saturating_add(i32::try_from(self.screen.width / 2).unwrap_or(0));
        self.cursor_y = self
            .screen
            .y
            .saturating_add(i32::try_from(self.screen.height / 2).unwrap_or(0));
    }

    /// Length of a peer's `edge`, from the layout it sent in the handshake.
    fn peer_edge_len(&self, peer_id: MachineId, edge: ScreenEdge) -> u32 {
        self.sessions
            .get(&peer_id)
            .map_or(0, |s| edge_len(&s.remote_layout.bounding_box(), edge))
    }

    /// Edges of `peer_name`'s screen that lead to another screen in our layout.
    fn exit_edges_for(&self, peer_name: &str) -> Vec<ScreenEdge> {
        ALL_EDGES
            .into_iter()
            .filter(|edge| self.adjacency.contains_key(&(peer_name.to_string(), *edge)))
            .collect()
    }

    /// True when we are neither controlling, controlled, nor mid-hand-off.
    fn is_idle(&self) -> bool {
        self.controlling.is_none() && self.controlled_by.is_none() && self.pending_enter.is_none()
    }

    fn check_barrier_crossing(&self) -> Option<(MachineId, ScreenEdge, u32)> {
        for (peer_id, session) in &self.sessions {
            for screen_config in &self.config.screens {
                if screen_config.name == session.name {
                    let edge = screen_config.position.local_edge();
                    if self.screen.is_at_edge(self.cursor_x, self.cursor_y, edge) {
                        return Some((*peer_id, edge, self.edge_offset(edge)));
                    }
                }
            }
        }
        None
    }

    /// Start a hand-off to `peer_id` by sending `Enter`.
    ///
    /// Refused unless we are idle and out of the cool-down. Unless
    /// `multi_hop` (continuing a crossing that is already under way), it also
    /// waits until no key or button is held, so grabbing the devices cannot
    /// leave a key stuck down locally.
    async fn initiate_control(
        &mut self,
        peer_id: MachineId,
        edge: ScreenEdge,
        position: u32,
        multi_hop: bool,
    ) {
        if !self.is_idle() {
            return;
        }
        if self
            .enter_cooldown_until
            .is_some_and(|until| Instant::now() < until)
        {
            return;
        }
        if !multi_hop && !self.local_held.is_empty() {
            debug!(
                held = self.local_held.len(),
                "not crossing while a key or button is held"
            );
            return;
        }
        let Some(peer_name) = self.sessions.get(&peer_id).map(|s| s.name.clone()) else {
            return;
        };
        let exit_edges = self.exit_edges_for(&peer_name);

        info!(peer = %peer_name, ?edge, position, "initiating control");
        let Some(session) = self.sessions.get_mut(&peer_id) else {
            return;
        };
        match session.send_enter(edge, position, exit_edges).await {
            Ok(()) => {
                self.pending_enter = Some((peer_id, Instant::now()));
            }
            Err(e) => {
                warn!(peer = %peer_name, error = %e, "failed to send Enter");
                self.enter_cooldown_until = Some(Instant::now() + ENTER_COOLDOWN);
            }
        }
    }

    /// Abandon a pending hand-off if its ack is overdue.
    fn expire_pending_enter(&mut self) {
        if let Some((peer_id, sent_at)) = self.pending_enter {
            if sent_at.elapsed() >= ENTER_TIMEOUT {
                warn!(peer = %peer_id, "no EnterAck within {ENTER_TIMEOUT:?}; abandoning hand-off");
                self.cancel_pending_enter();
            }
        }
    }

    /// Forget the pending hand-off and start the cool-down. If the peer acks
    /// it later anyway, that ack is answered with `Leave`.
    fn cancel_pending_enter(&mut self) {
        if let Some((peer_id, _)) = self.pending_enter.take() {
            if let Some(session) = self.sessions.get_mut(&peer_id) {
                session.cancel_enter();
            }
            self.enter_cooldown_until = Some(Instant::now() + ENTER_COOLDOWN);
        }
    }

    /// Stop controlling `peer_id` without telling it (the link failed): reset
    /// the session and give the local devices back.
    async fn drop_control(&mut self, peer_id: MachineId) {
        if self.controlling == Some(peer_id) {
            self.controlling = None;
            if let Some(session) = self.sessions.get_mut(&peer_id) {
                session.reset_to_idle();
            }
            let _ = self.capture.release().await;
            self.center_cursor();
        }
    }

    /// Take control back from the remote (release hotkey), or abandon a
    /// hand-off still awaiting its ack.
    async fn release_control(&mut self) {
        self.cancel_pending_enter();
        if let Some(peer_id) = self.controlling.take() {
            info!(peer = %peer_id, "releasing control");
            if let Some(session) = self.sessions.get_mut(&peer_id) {
                // The edge and position are not used for routing here: the
                // remote just stops being controlled.
                let _ = session.leave(ScreenEdge::Left, 0).await;
            }
            let _ = self.capture.release().await;
            self.center_cursor();
        }
    }

    /// Release every key and button we injected for the controlling peer.
    async fn release_injected(&mut self) {
        let held: Vec<_> = self.injected_held.drain().collect();
        for (virtual_id, inputs) in held {
            for input in inputs {
                if let Err(e) = self
                    .emulation
                    .inject(virtual_id, input.release_event())
                    .await
                {
                    debug!(error = %e, "failed to release injected input");
                }
            }
        }
    }

    /// Stop being controlled: release injected input and forget the
    /// controller's exit edges.
    async fn end_controlled(&mut self) {
        self.release_injected().await;
        self.controlled_by = None;
        self.entry_edge = None;
        self.exit_edges.clear();
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
            ControlMessage::Enter {
                edge,
                position,
                exit_edges,
            } => {
                self.handle_enter(machine_id, edge, position, exit_edges)
                    .await;
            }
            ControlMessage::EnterAck => {
                let pending = self.pending_enter.map(|(id, _)| id);
                if pending == Some(machine_id) {
                    self.pending_enter = None;
                    let acked = self
                        .sessions
                        .get_mut(&machine_id)
                        .is_some_and(PeerSession::set_controlling);
                    if acked {
                        info!(peer = %machine_id, "EnterAck: now controlling");
                        self.controlling = Some(machine_id);
                        // Stop local applications from seeing the input we
                        // now forward. The release hotkey still works:
                        // capture keeps delivering events to us.
                        if let Err(e) = self.capture.grab().await {
                            warn!(error = %e, "failed to grab input devices");
                        }
                        self.offer_clipboard(machine_id).await;
                        return;
                    }
                }
                // An ack we did not ask for (or one that arrived after we gave
                // up): tell the peer to stand down so it is not left
                // believing we control it.
                warn!(peer = %machine_id, "unsolicited EnterAck; asking peer to stand down");
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    let _ = session.leave(ScreenEdge::Left, 0).await;
                    session.reset_to_idle();
                }
            }
            ControlMessage::EnterReject => {
                if self.pending_enter.map(|(id, _)| id) == Some(machine_id) {
                    info!(peer = %machine_id, "peer refused hand-off");
                    self.cancel_pending_enter();
                }
            }
            ControlMessage::Leave { edge, position } => {
                self.handle_leave(machine_id, edge, position).await;
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
                if let Some(session) = self.sessions.get_mut(&machine_id) {
                    session.record_pong(seq);
                }
            }
            ControlMessage::Clipboard(cb_msg) => {
                self.handle_clipboard_message(machine_id, cb_msg).await;
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

    /// Read the local clipboard and send an `Offer` to the given peer.
    /// Silent no-op if no clipboard backend is installed, the clipboard is
    /// empty, or the peer session is gone.
    async fn offer_clipboard(&mut self, peer_id: MachineId) {
        let Some(clipboard) = self.clipboard.as_ref() else {
            return;
        };
        let formats = match clipboard.available_formats().await {
            Ok(f) if !f.is_empty() => f,
            Ok(_) => return,
            Err(e) => {
                debug!(error = %e, "clipboard available_formats failed; skipping offer");
                return;
            }
        };
        let size_hint = clipboard.get().await.map_or(0, |c| c.size() as u64);
        let offer = ControlMessage::Clipboard(ClipboardMessage::Offer { formats, size_hint });
        if let Some(session) = self.sessions.get_mut(&peer_id) {
            if let Err(e) = session.control_tx.send(&offer).await {
                debug!(peer = %peer_id, error = %e, "failed to send clipboard offer");
            }
        }
    }

    async fn handle_clipboard_message(&mut self, peer_id: MachineId, msg: ClipboardMessage) {
        match msg {
            ClipboardMessage::Offer { formats, size_hint } => {
                debug!(peer = %peer_id, ?formats, size_hint, "received clipboard offer");
                // Request the richest format the peer offers that we know how
                // to apply: an image beats HTML, HTML beats plain text.
                let Some(format) = richest_format(&formats) else {
                    debug!(peer = %peer_id, "offer had no format we support");
                    return;
                };
                let request = ControlMessage::Clipboard(ClipboardMessage::Request { format });
                if let Some(session) = self.sessions.get_mut(&peer_id) {
                    if let Err(e) = session.control_tx.send(&request).await {
                        debug!(peer = %peer_id, error = %e, "failed to send clipboard request");
                    }
                }
            }
            ClipboardMessage::Request { format } => {
                let Some(clipboard) = self.clipboard.as_ref() else {
                    return;
                };
                let content = match clipboard.get_format(format).await {
                    Ok(c) => c,
                    Err(e) => {
                        debug!(?format, error = %e, "clipboard get failed; ignoring request");
                        return;
                    }
                };
                // Enforce the configured size cap before putting a large
                // payload on the wire. Streaming large images is a follow-up;
                // for now oversized content is simply not synced.
                let max = self.config.clipboard.max_size;
                if content.size() > max {
                    warn!(
                        peer = %peer_id,
                        size = content.size(),
                        max,
                        ?format,
                        "clipboard content exceeds max_size; not sending"
                    );
                    return;
                }
                let data = ControlMessage::Clipboard(ClipboardMessage::Data(content));
                if let Some(session) = self.sessions.get_mut(&peer_id) {
                    if let Err(e) = session.control_tx.send(&data).await {
                        debug!(peer = %peer_id, error = %e, "failed to send clipboard data");
                    }
                }
            }
            ClipboardMessage::Data(content) => {
                let max = self.config.clipboard.max_size;
                if content.size() > max {
                    warn!(
                        peer = %peer_id,
                        size = content.size(),
                        max,
                        "received oversized clipboard content; rejecting"
                    );
                    return;
                }
                let Some(clipboard) = self.clipboard.as_mut() else {
                    return;
                };
                debug!(peer = %peer_id, size = content.size(), format = ?content.format, "received clipboard data");
                if let Err(e) = clipboard.set(content).await {
                    warn!(peer = %peer_id, error = %e, "failed to set local clipboard");
                }
            }
        }
    }

    /// A peer wants to hand control to us.
    ///
    /// Accepted only when we are idle. If we are mid-hand-off *to that same
    /// peer* (both pushed at once), the lower `MachineId` keeps its hand-off
    /// and the other side yields, so exactly one direction wins. Anything else
    /// is answered with `EnterReject` so the sender does not wait for an ack
    /// that will never come.
    async fn handle_enter(
        &mut self,
        machine_id: MachineId,
        edge: ScreenEdge,
        position: u32,
        exit_edges: Vec<ScreenEdge>,
    ) {
        info!(peer = %machine_id, ?edge, position, "peer entering");

        if self.pending_enter.map(|(id, _)| id) == Some(machine_id) {
            if self.machine_id.as_uuid() < machine_id.as_uuid() {
                debug!(peer = %machine_id, "simultaneous hand-off: keeping ours");
                self.reject_enter(machine_id).await;
                return;
            }
            debug!(peer = %machine_id, "simultaneous hand-off: yielding");
            self.pending_enter = None;
            if let Some(session) = self.sessions.get_mut(&machine_id) {
                session.cancel_enter();
            }
        }

        if !self.is_idle() {
            debug!(peer = %machine_id, "busy; refusing hand-off");
            self.reject_enter(machine_id).await;
            return;
        }

        let source_len = self.peer_edge_len(machine_id, edge);
        let Some(session) = self.sessions.get_mut(&machine_id) else {
            return;
        };
        if let Err(e) = session.handle_enter().await {
            warn!(peer = %machine_id, error = %e, "cannot accept hand-off");
            self.reject_enter(machine_id).await;
            return;
        }

        self.controlled_by = Some(machine_id);
        self.exit_edges = exit_edges;
        // `edge` is the controller's exit edge; the cursor enters on our
        // opposite edge. Suppress Leave on that edge until the cursor moves
        // off it, or it would bounce straight back.
        let entry_edge = edge.opposite();
        self.entry_edge = Some(entry_edge);
        self.place_at_edge(entry_edge, position, source_len);
        // The initiator opened a uni stream before sending Enter, but QUIC may
        // not have delivered the stream frame yet: accept it asynchronously.
        self.spawn_accept_input_stream(machine_id);
    }

    async fn reject_enter(&mut self, machine_id: MachineId) {
        if let Some(session) = self.sessions.get_mut(&machine_id) {
            if let Err(e) = session.control_tx.send(&ControlMessage::EnterReject).await {
                debug!(peer = %machine_id, error = %e, "failed to send EnterReject");
            }
        }
    }

    /// A peer reports that the cursor left its screen (or that it stopped
    /// being controlled by us, after our release hotkey).
    async fn handle_leave(&mut self, machine_id: MachineId, edge: ScreenEdge, position: u32) {
        if let Some(session) = self.sessions.get_mut(&machine_id) {
            session.handle_leave();
        }
        if self.pending_enter.map(|(id, _)| id) == Some(machine_id) {
            self.pending_enter = None;
        }
        if self.controlled_by == Some(machine_id) {
            self.end_controlled().await;
        }
        if self.controlling != Some(machine_id) {
            return;
        }

        info!(peer = %machine_id, ?edge, position, "peer sent Leave");
        self.controlling = None;
        let _ = self.capture.release().await;

        let source_len = self.peer_edge_len(machine_id, edge);
        // Where does the cursor go next? The adjacency map knows what lies
        // beyond each edge of the peer's screen.
        let my_name = self.config.identity.name.clone();
        let next_peer = self
            .sessions
            .get(&machine_id)
            .and_then(|s| self.adjacency.get(&(s.name.clone(), edge)))
            .filter(|target| **target != my_name)
            .and_then(|target| {
                self.sessions
                    .iter()
                    .find(|(_, s)| s.name == *target)
                    .map(|(id, _)| *id)
            });

        if let Some(next_peer_id) = next_peer {
            // Multi-hop: carry the position through in our own coordinates.
            let position = scale_along_edge(position, source_len, edge_len(&self.screen, edge));
            info!(next_peer = %next_peer_id, ?edge, position, "multi-hop: transferring control");
            self.initiate_control(next_peer_id, edge, position, true)
                .await;
            if self.pending_enter.is_none() {
                // The hop could not start: bring the cursor home instead.
                self.place_at_edge(edge.opposite(), position, edge_len(&self.screen, edge));
            }
        } else {
            // The cursor comes back to us on the edge facing the peer.
            self.place_at_edge(edge.opposite(), position, source_len);
        }
    }

    async fn handle_peer_input(&mut self, machine_id: MachineId, msg: InputMessage) {
        if self.controlled_by != Some(machine_id) {
            debug!(peer = %machine_id, controlled_by = ?self.controlled_by, "ignoring input from non-controlling peer");
            return;
        }

        for event in &msg.events {
            if let InputEvent::MouseMove { dx, dy } = event {
                self.move_cursor(*dx, *dy);
            }
        }

        // Clear the entry-edge suppression once the cursor moves off it, so a
        // deliberate return to that edge still sends Leave later.
        if let Some(entry) = self.entry_edge {
            if !self.screen.is_at_edge(self.cursor_x, self.cursor_y, entry) {
                self.entry_edge = None;
            }
        }

        // Reaching an edge the controller says leads somewhere hands the
        // cursor back; it decides where it goes (multi-hop or home).
        let exit = self.exit_edges.iter().copied().find(|edge| {
            self.entry_edge != Some(*edge)
                && self.screen.is_at_edge(self.cursor_x, self.cursor_y, *edge)
        });
        if let Some(edge) = exit {
            let position = self.edge_offset(edge);
            info!(peer = %machine_id, ?edge, position, "cursor hit edge, sending Leave");
            self.end_controlled().await;
            if let Some(session) = self.sessions.get_mut(&machine_id) {
                let _ = session.leave(edge, position).await;
            }
            return;
        }

        let Some(&virtual_id) = self
            .sessions
            .get(&machine_id)
            .and_then(|s| s.device_map.get(&msg.device_id))
        else {
            debug!(peer = %machine_id, device_id = ?msg.device_id, "no virtual device for input device");
            return;
        };
        for event in msg.events {
            track_held(self.injected_held.entry(virtual_id).or_default(), &event);
            if let Err(e) = self.emulation.inject(virtual_id, event).await {
                warn!(error = %e, "failed to inject event");
            }
        }
    }

    async fn handle_peer_disconnected(&mut self, machine_id: MachineId) {
        if self.controlling == Some(machine_id) {
            self.controlling = None;
            let _ = self.capture.release().await;
            self.center_cursor();
        }
        if self.pending_enter.map(|(id, _)| id) == Some(machine_id) {
            self.pending_enter = None;
        }
        if self.controlled_by == Some(machine_id) {
            self.end_controlled().await;
        }

        if let Some(mut session) = self.sessions.remove(&machine_id) {
            // Release the dial slot so this peer can be re-dialed when
            // discovery surfaces it again.
            release_dial(&self.dialed_addrs, session.connection.remote_address());
            for (_, virtual_id) in session.device_map.drain() {
                self.injected_held.remove(&virtual_id);
                let _ = self.emulation.destroy_device(virtual_id).await;
            }
            info!(peer = %session.name, "peer session removed");
        }
    }

    async fn shutdown(&mut self) -> Result<(), DaemonError> {
        info!("daemon shutting down");

        // Stop the status writer before removing its file so it can't
        // recreate it after cleanup.
        if let Some(writer) = self.status_writer.take() {
            writer.abort();
        }
        let _ = std::fs::remove_file(status_file_path());

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

/// Pick the richest clipboard format from an offer that we can apply locally:
/// image first, then HTML, then plain text. Returns `None` if the offer
/// contains no format we recognise.
fn richest_format(formats: &[ClipboardFormat]) -> Option<ClipboardFormat> {
    [
        ClipboardFormat::Png,
        ClipboardFormat::Html,
        ClipboardFormat::PlainText,
    ]
    .into_iter()
    .find(|f| formats.contains(f))
}

/// Given an existing connection to a peer and a new one, whether to keep the
/// new one. Both machines evaluate this independently, possibly seeing the
/// two connections in opposite orders, and must keep the same connection:
/// the one opened by the lower machine id. A reconnection from the same side
/// replaces the old (presumably dead) connection.
fn keep_new_connection(
    us: MachineId,
    peer: MachineId,
    existing_initiator: MachineId,
    new_initiator: MachineId,
) -> bool {
    existing_initiator == new_initiator
        || new_initiator.as_uuid() == us.as_uuid().min(peer.as_uuid())
}

/// Try to reserve a `SocketAddr` for an outbound dial. Returns `true` if this
/// caller now owns the dial; `false` if another task got there first.
fn claim_dial(dialed: &Arc<Mutex<HashSet<SocketAddr>>>, addr: SocketAddr) -> bool {
    match dialed.lock() {
        Ok(mut set) => set.insert(addr),
        // Lock poisoning shouldn't happen here, but if it does, the safe
        // call is to skip the dial rather than risk a duplicate.
        Err(_) => false,
    }
}

/// Release a previously-claimed dial slot so the address can be retried
/// (e.g. after a failed handshake or a peer disconnect).
fn release_dial(dialed: &Arc<Mutex<HashSet<SocketAddr>>>, addr: SocketAddr) {
    if let Ok(mut set) = dialed.lock() {
        set.remove(&addr);
    }
}

/// Spawn a background task that runs the responder handshake on an
/// accepted connection and hands the session to the daemon.
fn spawn_inbound_handshake(conn: PeerConnection, ctx: HandshakeCtx) {
    tokio::spawn(async move {
        let remote = conn.remote_address();
        match perform_handshake_responder(conn, &ctx).await {
            Ok(session) => {
                info!(peer = %session.name, remote = %remote, "inbound handshake complete");
                let _ = ctx
                    .event_tx
                    .send(DaemonEvent::SessionReady {
                        session: Box::new(session),
                    })
                    .await;
            }
            Err(e) => warn!(remote = %remote, error = %e, "inbound handshake failed"),
        }
    });
}

/// Spawn a background task that connects to `peer`, completes the initiator
/// handshake, and forwards the resulting [`PeerSession`] back to the daemon
/// via [`DaemonEvent::SessionReady`]. Releases the dial slot on failure.
///
/// The certificate is pinned to what the trust store holds for the peer's
/// name. A fingerprint from discovery (mDNS TXT) is not used for that: anyone
/// on the LAN can advertise one, so it is only a hint.
fn spawn_outbound_handshake(
    transport: QuicTransport,
    peer: Peer,
    ctx: HandshakeCtx,
    dialed: Arc<Mutex<HashSet<SocketAddr>>>,
) {
    tokio::spawn(async move {
        let addr = peer.address;
        let pin = ctx.trust.pin_for(&peer.name);
        if let (Some(hint), Some(pin)) = (&peer.fingerprint, &pin) {
            if normalize_fingerprint(hint) != normalize_fingerprint(pin) {
                debug!(peer = %peer.name, "advertised fingerprint differs from the trusted one");
            }
        }
        let result = match transport.connect_pinned(addr, "cross-control", pin).await {
            Ok(conn) => perform_handshake_initiator(conn, &ctx).await,
            Err(e) => Err(e.into()),
        };
        match result {
            Ok(session) => {
                info!(peer = %session.name, address = %addr, "outbound handshake complete");
                if ctx
                    .event_tx
                    .send(DaemonEvent::SessionReady {
                        session: Box::new(session),
                    })
                    .await
                    .is_err()
                {
                    release_dial(&dialed, addr);
                }
            }
            Err(e) => {
                warn!(peer = %peer.name, address = %addr, error = %e, "outbound handshake failed");
                release_dial(&dialed, addr);
            }
        }
    });
}

/// Close `conn` after a failed handshake, telling the peer why when it was
/// refused authorization.
fn close_failed(conn: &PeerConnection, error: &DaemonError) {
    let reason = match error {
        DaemonError::Untrusted(_) => "not authorized",
        _ => "handshake failed",
    };
    conn.close_with_reason(reason);
}

/// Responder handshake (accept bidi stream, read Hello, authorize, send
/// Welcome, announce devices).
async fn perform_handshake_responder(
    conn: PeerConnection,
    ctx: &HandshakeCtx,
) -> Result<PeerSession, DaemonError> {
    let authorize = ctx.authorizer(&conn);
    let (control_tx, control_rx) = conn.accept_control_stream().await?;
    let mut session = PeerSession::new(conn.clone(), control_tx, control_rx);
    let result = async {
        session
            .handshake_responder(ctx.our_id, &ctx.our_name, &ctx.our_layout, authorize)
            .await?;
        session.announce_devices(&ctx.local_devices).await
    }
    .await;
    if let Err(e) = result {
        close_failed(&conn, &e);
        return Err(e);
    }
    Ok(session)
}

/// Initiator handshake (open bidi stream, send Hello, read Welcome,
/// authorize, announce devices).
async fn perform_handshake_initiator(
    conn: PeerConnection,
    ctx: &HandshakeCtx,
) -> Result<PeerSession, DaemonError> {
    let authorize = ctx.authorizer(&conn);
    let (control_tx, control_rx) = conn.open_control_stream().await?;
    let mut session = PeerSession::new(conn.clone(), control_tx, control_rx);
    let result = async {
        session
            .handshake_initiator(ctx.our_id, &ctx.our_name, &ctx.our_layout, authorize)
            .await?;
        session.announce_devices(&ctx.local_devices).await
    }
    .await;
    if let Err(e) = result {
        close_failed(&conn, &e);
        return Err(e);
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_keep_the_same_duplicate_connection() {
        for _ in 0..50 {
            let (a, b) = (MachineId::new(), MachineId::new());
            // Connections opened by A and by B. Each side may see either one
            // first; whichever arrives second is "new".
            for (first, second) in [(a, b), (b, a)] {
                for (a_first, b_first) in [(first, second), (second, first)] {
                    let a_sees_second = if a_first == a { b } else { a };
                    let b_sees_second = if b_first == a { b } else { a };
                    let a_keeps = if keep_new_connection(a, b, a_first, a_sees_second) {
                        a_sees_second
                    } else {
                        a_first
                    };
                    let b_keeps = if keep_new_connection(b, a, b_first, b_sees_second) {
                        b_sees_second
                    } else {
                        b_first
                    };
                    assert_eq!(a_keeps, b_keeps, "sides disagree");
                    let lower = if a.as_uuid() < b.as_uuid() { a } else { b };
                    assert_eq!(a_keeps, lower, "must keep the lower id's connection");
                }
            }
            // A reconnection from the same side replaces the old one.
            assert!(keep_new_connection(a, b, b, b));
            assert!(keep_new_connection(a, b, a, a));
        }
    }

    #[test]
    fn scale_maps_between_resolutions() {
        // Halfway down a 1080-high edge is halfway down a 1440-high edge.
        assert_eq!(scale_along_edge(540, 1080, 1440), 720);
        // Clamped to the target edge.
        assert_eq!(scale_along_edge(5000, 1080, 1080), 1079);
        // Unknown source length: keep the position, still clamped.
        assert_eq!(scale_along_edge(100, 0, 1080), 100);
        assert_eq!(scale_along_edge(5000, 0, 1080), 1079);
    }

    #[test]
    fn clamp_respects_negative_origin() {
        // A monitor to the left of the primary: the desktop spans x = -1920..1919.
        let screen = ScreenGeometry {
            width: 3840,
            height: 1080,
            x: -1920,
            y: 0,
        };
        assert_eq!(clamp_point(&screen, -5000, 500), (-1920, 500));
        assert_eq!(clamp_point(&screen, 5000, 500), (1919, 500));
        // The left edge is reachable and detected as the left edge.
        let (x, y) = clamp_point(&screen, -5000, 500);
        assert!(screen.is_at_edge(x, y, ScreenEdge::Left));
        // Mid-desktop is not the right edge (the old [0, width) clamp put the
        // "right edge" at x = 3839, unreachable, and the left at 0, mid-desktop).
        assert!(!screen.is_at_edge(0, 500, ScreenEdge::Left));
    }

    #[test]
    fn held_input_tracking_and_release() {
        let mut held = HashSet::new();
        let down = InputEvent::Key {
            code: KeyCode::KeyA,
            state: ButtonState::Pressed,
        };
        let up = InputEvent::Key {
            code: KeyCode::KeyA,
            state: ButtonState::Released,
        };
        track_held(&mut held, &down);
        assert!(held.contains(&HeldInput::Key(KeyCode::KeyA)));
        assert_eq!(HeldInput::Key(KeyCode::KeyA).release_event(), up);
        track_held(&mut held, &up);
        assert_eq!(held, HashSet::new());
        // Mouse moves are not held inputs.
        track_held(&mut held, &InputEvent::MouseMove { dx: 1, dy: 1 });
        assert_eq!(held, HashSet::new());
    }
}
