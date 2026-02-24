//! Integration tests exercising the full daemon event loop on loopback.

use std::net::SocketAddr;
use std::time::Duration;

use cross_control_daemon::config::{Config, DaemonConfig, IdentityConfig, ScreenAdjacency, ScreenConfig};
use cross_control_daemon::{Daemon, DaemonEvent, DaemonStatus};
use cross_control_input::mock::{MockCapture, MockEmulation, MockEmulationHandle};
use cross_control_types::{
    ButtonState, CapturedEvent, DeviceCapability, DeviceId, DeviceInfo, InputEvent, KeyCode,
    MachineId, Position,
};
use tokio::sync::{mpsc, watch};
use tracing_subscriber::EnvFilter;

/// Everything needed to run a two-daemon test.
#[allow(dead_code)]
struct TestPair {
    // Daemon A (initiator / left)
    feed_a: mpsc::Sender<CapturedEvent>,
    emulation_a: MockEmulationHandle,
    status_a: watch::Receiver<DaemonStatus>,
    shutdown_a: mpsc::Sender<DaemonEvent>,

    // Daemon B (responder / right)
    feed_b: mpsc::Sender<CapturedEvent>,
    emulation_b: MockEmulationHandle,
    status_b: watch::Receiver<DaemonStatus>,
    shutdown_b: mpsc::Sender<DaemonEvent>,

    // Join handles
    handle_a: tokio::task::JoinHandle<()>,
    handle_b: tokio::task::JoinHandle<()>,
}

impl TestPair {
    async fn shutdown(self) {
        let _ = self.shutdown_a.send(DaemonEvent::Shutdown).await;
        let _ = self.shutdown_b.send(DaemonEvent::Shutdown).await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.handle_a).await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.handle_b).await;
    }
}

fn test_devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            id: DeviceId(1),
            name: "Test Keyboard".to_string(),
            capabilities: vec![DeviceCapability::Keyboard],
        },
        DeviceInfo {
            id: DeviceId(2),
            name: "Test Mouse".to_string(),
            capabilities: vec![DeviceCapability::RelativeMouse, DeviceCapability::Scroll],
        },
    ]
}

/// Set up two daemons on loopback.
///
/// Daemon A has a screen "machine-b" at `Position::Right` pointing at B.
/// Daemon B has a screen "machine-a" at `Position::Left` pointing at A.
///
/// A initiates the outbound connection to B.
async fn setup_pair() -> TestPair {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_a = cross_control_certgen::generate_certificate("localhost").unwrap();
    let cert_b = cross_control_certgen::generate_certificate("localhost").unwrap();

    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let transport_a =
        cross_control_protocol::QuicTransport::bind(bind, &cert_a.cert_pem, &cert_a.key_pem)
            .unwrap();
    let transport_b =
        cross_control_protocol::QuicTransport::bind(bind, &cert_b.cert_pem, &cert_b.key_pem)
            .unwrap();

    let addr_b = transport_b.local_addr().unwrap();

    let machine_id_a = MachineId::new();
    let machine_id_b = MachineId::new();

    // Config for daemon A: knows about B at Position::Right
    let config_a = Config {
        daemon: DaemonConfig {
            screen_width: 1920,
            screen_height: 1080,
            ..DaemonConfig::default()
        },
        identity: IdentityConfig {
            name: "machine-a".to_string(),
        },
        screens: vec![ScreenConfig {
            name: "machine-b".to_string(),
            address: Some(addr_b.to_string()),
            position: Position::Right,
            fingerprint: None,
        }],
        ..Config::default()
    };

    // Config for daemon B: knows about A at Position::Left (no address — A connects to B)
    let config_b = Config {
        daemon: DaemonConfig {
            screen_width: 1920,
            screen_height: 1080,
            ..DaemonConfig::default()
        },
        identity: IdentityConfig {
            name: "machine-b".to_string(),
        },
        screens: vec![ScreenConfig {
            name: "machine-a".to_string(),
            address: None,
            position: Position::Left,
            fingerprint: None,
        }],
        ..Config::default()
    };

    // Mock backends for A
    let (capture_a, feed_a) = MockCapture::new();
    let emulation_a_backend = MockEmulation::new();
    let emulation_a = emulation_a_backend.handle();

    // Mock backends for B
    let (capture_b, feed_b) = MockCapture::new();
    let emulation_b_backend = MockEmulation::new();
    let emulation_b = emulation_b_backend.handle();

    // Build daemons
    let mut daemon_a = Daemon::new(
        config_a,
        machine_id_a,
        transport_a,
        Box::new(capture_a),
        Box::new(emulation_a_backend),
    );
    daemon_a.set_local_devices(test_devices());
    let status_a = daemon_a.status_receiver();
    let shutdown_a = daemon_a.event_sender();

    let mut daemon_b = Daemon::new(
        config_b,
        machine_id_b,
        transport_b,
        Box::new(capture_b),
        Box::new(emulation_b_backend),
    );
    daemon_b.set_local_devices(test_devices());
    let status_b = daemon_b.status_receiver();
    let shutdown_b = daemon_b.event_sender();

    // Spawn daemons — B first (it's the server), then A (connects to B)
    let handle_b = tokio::spawn(async move {
        if let Err(e) = daemon_b.run().await {
            eprintln!("daemon B error: {e}");
        }
    });

    // Small delay to let B start accepting
    tokio::time::sleep(Duration::from_millis(50)).await;

    let handle_a = tokio::spawn(async move {
        if let Err(e) = daemon_a.run().await {
            eprintln!("daemon A error: {e}");
        }
    });

    TestPair {
        feed_a,
        emulation_a,
        status_a,
        shutdown_a,
        feed_b,
        emulation_b,
        status_b,
        shutdown_b,
        handle_a,
        handle_b,
    }
}

/// Wait for a condition on a status receiver with timeout.
async fn wait_for_status(
    rx: &mut watch::Receiver<DaemonStatus>,
    timeout: Duration,
    pred: impl Fn(&DaemonStatus) -> bool,
) -> Result<DaemonStatus, &'static str> {
    tokio::time::timeout(timeout, async {
        loop {
            {
                let status = rx.borrow_and_update().clone();
                if pred(&status) {
                    return Ok(status);
                }
            }
            if rx.changed().await.is_err() {
                return Err("watch closed");
            }
        }
    })
    .await
    .map_err(|_| "timeout")?
}

#[tokio::test]
async fn test_handshake() {
    let mut pair = setup_pair().await;

    // Wait for both daemons to have 1 session
    let status_a = wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("daemon A should establish session");

    let status_b = wait_for_status(&mut pair.status_b, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("daemon B should establish session");

    assert_eq!(status_a.session_count, 1);
    assert_eq!(status_b.session_count, 1);
    assert!(status_a.controlling.is_none());
    assert!(status_b.controlling.is_none());

    pair.shutdown().await;
}

#[tokio::test]
async fn test_device_announce() {
    let mut pair = setup_pair().await;

    // Wait for handshake
    wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("handshake A");

    wait_for_status(&mut pair.status_b, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("handshake B");

    // Give device announces time to be processed
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Daemon B should have created virtual devices matching A's local devices
    let b_devices = pair.emulation_b.devices();
    assert!(
        b_devices.len() >= 2,
        "daemon B should have created virtual devices for A's keyboard and mouse, got {}",
        b_devices.len()
    );

    // Daemon A should have created virtual devices matching B's local devices
    let a_devices = pair.emulation_a.devices();
    assert!(
        a_devices.len() >= 2,
        "daemon A should have created virtual devices for B's keyboard and mouse, got {}",
        a_devices.len()
    );

    pair.shutdown().await;
}

#[tokio::test]
async fn test_enter_leave_flow() {
    let mut pair = setup_pair().await;

    // Wait for handshake
    wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("handshake A");

    wait_for_status(&mut pair.status_b, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("handshake B");

    // Give device announces time to process
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Push cursor to the right edge by sending large mouse move events
    for _ in 0..5 {
        let event = CapturedEvent {
            device_id: DeviceId(2),
            timestamp_us: 1000,
            event: InputEvent::MouseMove { dx: 500, dy: 0 },
        };
        pair.feed_a.send(event).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Wait for daemon A to report controlling
    let status = wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("daemon A should be controlling");

    assert!(status.controlling.is_some());

    // B should report being controlled
    let status_b = wait_for_status(&mut pair.status_b, Duration::from_secs(5), |s| {
        s.controlled_by.is_some()
    })
    .await
    .expect("daemon B should be controlled");

    assert!(status_b.controlled_by.is_some());

    // Release via hotkey: LeftCtrl + LeftShift + Escape
    let hotkey_events = [
        InputEvent::Key {
            code: KeyCode::LeftCtrl,
            state: ButtonState::Pressed,
        },
        InputEvent::Key {
            code: KeyCode::LeftShift,
            state: ButtonState::Pressed,
        },
        InputEvent::Key {
            code: KeyCode::Escape,
            state: ButtonState::Pressed,
        },
    ];
    for event in hotkey_events {
        let captured = CapturedEvent {
            device_id: DeviceId(1),
            timestamp_us: 2000,
            event,
        };
        pair.feed_a.send(captured).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Wait for daemon A to release control
    let status = wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.controlling.is_none()
    })
    .await
    .expect("daemon A should release control");

    assert!(status.controlling.is_none());

    pair.shutdown().await;
}

#[tokio::test]
async fn test_input_forwarding() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("debug"))
        .with_test_writer()
        .try_init();
    let mut pair = setup_pair().await;

    // Wait for handshake
    wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("handshake A");

    wait_for_status(&mut pair.status_b, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("handshake B");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Enter controlling state by pushing cursor right
    for _ in 0..5 {
        let event = CapturedEvent {
            device_id: DeviceId(2),
            timestamp_us: 1000,
            event: InputEvent::MouseMove { dx: 500, dy: 0 },
        };
        pair.feed_a.send(event).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Wait for controlling state
    wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("should be controlling");

    // Wait for B to confirm controlled_by
    wait_for_status(&mut pair.status_b, Duration::from_secs(5), |s| {
        s.controlled_by.is_some()
    })
    .await
    .expect("B should be controlled");

    // Give the input reader time to be fully established
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send multiple key events with delays to ensure delivery
    for i in 0..5 {
        let key_event = CapturedEvent {
            device_id: DeviceId(1),
            timestamp_us: 3000 + u64::try_from(i).unwrap_or(0),
            event: InputEvent::Key {
                code: KeyCode::KeyA,
                state: ButtonState::Pressed,
            },
        };
        pair.feed_a.send(key_event).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Wait for B's emulation to receive the injected event
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let events = pair.emulation_b.injected_events();
            if events.iter().any(|e| {
                matches!(
                    &e.event,
                    InputEvent::Key {
                        code: KeyCode::KeyA,
                        ..
                    }
                )
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("daemon B should receive KeyA injection");

    pair.shutdown().await;
}

#[tokio::test]
async fn test_hotkey_release() {
    let mut pair = setup_pair().await;

    // Wait for handshake
    wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await
    .expect("handshake A");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Enter controlling state
    for _ in 0..5 {
        let event = CapturedEvent {
            device_id: DeviceId(2),
            timestamp_us: 1000,
            event: InputEvent::MouseMove { dx: 500, dy: 0 },
        };
        pair.feed_a.send(event).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("should be controlling");

    // Send the release hotkey combo
    let hotkey_events = [
        InputEvent::Key {
            code: KeyCode::LeftCtrl,
            state: ButtonState::Pressed,
        },
        InputEvent::Key {
            code: KeyCode::LeftShift,
            state: ButtonState::Pressed,
        },
        InputEvent::Key {
            code: KeyCode::Escape,
            state: ButtonState::Pressed,
        },
    ];
    for event in hotkey_events {
        let captured = CapturedEvent {
            device_id: DeviceId(1),
            timestamp_us: 4000,
            event,
        };
        pair.feed_a.send(captured).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Daemon A should release control
    let status_a = wait_for_status(&mut pair.status_a, Duration::from_secs(5), |s| {
        s.controlling.is_none()
    })
    .await
    .expect("daemon A should release");

    assert!(status_a.controlling.is_none());

    // Daemon B should return to idle
    let status_b = wait_for_status(&mut pair.status_b, Duration::from_secs(5), |s| {
        s.controlled_by.is_none()
    })
    .await
    .expect("daemon B should return to idle");

    assert!(status_b.controlled_by.is_none());

    pair.shutdown().await;
}

// ---------------------------------------------------------------------------
// Multi-daemon test infrastructure
// ---------------------------------------------------------------------------

/// Handles for an N-daemon test cluster.
struct TestCluster {
    feeds: Vec<mpsc::Sender<CapturedEvent>>,
    statuses: Vec<watch::Receiver<DaemonStatus>>,
    shutdowns: Vec<mpsc::Sender<DaemonEvent>>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl TestCluster {
    async fn shutdown(self) {
        for tx in &self.shutdowns {
            let _ = tx.send(DaemonEvent::Shutdown).await;
        }
        for h in self.handles {
            let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
        }
    }

    /// Push cursor on daemon `idx` in a direction until it enters controlling state.
    async fn push_cursor_to_edge(
        &mut self,
        idx: usize,
        dx: i32,
        dy: i32,
    ) {
        for _ in 0..10 {
            let event = CapturedEvent {
                device_id: DeviceId(2),
                timestamp_us: 1000,
                event: InputEvent::MouseMove { dx, dy },
            };
            self.feeds[idx].send(event).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Descriptor for one daemon in a cluster.
struct DaemonSpec {
    name: String,
    screens: Vec<ScreenConfig>,
    screen_adjacency: Vec<ScreenAdjacency>,
}

/// Set up N daemons on loopback. Returns the cluster and addresses.
/// `build_specs` receives the bound addresses and returns a spec per daemon.
async fn setup_cluster<F>(n: usize, build_specs: F) -> TestCluster
where
    F: FnOnce(&[SocketAddr]) -> Vec<DaemonSpec>,
{
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Bind all transports first so we know the addresses.
    let mut transports = Vec::new();
    let mut addrs = Vec::new();
    for _ in 0..n {
        let cert = cross_control_certgen::generate_certificate("localhost").unwrap();
        let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let transport =
            cross_control_protocol::QuicTransport::bind(bind, &cert.cert_pem, &cert.key_pem)
                .unwrap();
        addrs.push(transport.local_addr().unwrap());
        transports.push(transport);
    }

    let specs = build_specs(&addrs);
    assert_eq!(specs.len(), n);

    let mut feeds = Vec::new();
    let mut statuses = Vec::new();
    let mut shutdowns = Vec::new();
    let mut handles = Vec::new();

    for (i, (transport, spec)) in transports.into_iter().zip(specs).enumerate() {
        let (capture, feed) = MockCapture::new();
        let emu = MockEmulation::new();

        let config = Config {
            daemon: DaemonConfig {
                screen_width: 1920,
                screen_height: 1080,
                ..DaemonConfig::default()
            },
            identity: IdentityConfig {
                name: spec.name.clone(),
            },
            screens: spec.screens,
            screen_adjacency: spec.screen_adjacency,
            ..Config::default()
        };

        let mut daemon = Daemon::new(
            config,
            MachineId::new(),
            transport,
            Box::new(capture),
            Box::new(emu),
        );
        daemon.set_local_devices(test_devices());
        statuses.push(daemon.status_receiver());
        shutdowns.push(daemon.event_sender());
        feeds.push(feed);

        let name = spec.name;
        let handle = tokio::spawn(async move {
            if let Err(e) = daemon.run().await {
                eprintln!("daemon {name} (idx {i}) error: {e}");
            }
        });
        handles.push(handle);
    }

    // Wait for all daemons to reach expected session counts.
    // Each daemon with outbound addresses will connect; each accept completes.
    // Give a generous timeout.
    // We don't know expected counts here, so just wait for at least 1 session each.
    // The caller can do more specific waits.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let all_connected = statuses
                .iter()
                .all(|s| s.borrow().session_count >= 1);
            if all_connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("all daemons should establish at least 1 session");

    // Let device announcements propagate.
    tokio::time::sleep(Duration::from_millis(200)).await;

    TestCluster {
        feeds,
        statuses,
        shutdowns,
        handles,
    }
}

// ---------------------------------------------------------------------------
// Three-screen tests: A (center), B (above), C (right)
// ---------------------------------------------------------------------------

/// Set up: A connects to B (above) and C (right).
/// A knows the full graph via screen_adjacency.
///
///        B
///        |
///    A ——— C
async fn setup_three_screens() -> TestCluster {
    setup_cluster(3, |addrs| {
        vec![
            DaemonSpec {
                name: "A".into(),
                screens: vec![
                    ScreenConfig {
                        name: "B".into(),
                        address: Some(addrs[1].to_string()),
                        position: Position::Above,
                        fingerprint: None,
                    },
                    ScreenConfig {
                        name: "C".into(),
                        address: Some(addrs[2].to_string()),
                        position: Position::Right,
                        fingerprint: None,
                    },
                ],
                screen_adjacency: vec![],
            },
            DaemonSpec {
                name: "B".into(),
                screens: vec![ScreenConfig {
                    name: "A".into(),
                    address: None,
                    position: Position::Below,
                    fingerprint: None,
                }],
                screen_adjacency: vec![],
            },
            DaemonSpec {
                name: "C".into(),
                screens: vec![ScreenConfig {
                    name: "A".into(),
                    address: None,
                    position: Position::Left,
                    fingerprint: None,
                }],
                screen_adjacency: vec![],
            },
        ]
    })
    .await
}

#[tokio::test]
async fn test_three_screens_a_to_b_above() {
    let mut cluster = setup_three_screens().await;

    // Wait for A to have 2 sessions (B and C).
    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.session_count >= 2
    })
    .await
    .expect("A should have 2 sessions");

    // Push A's cursor upward to cross into B.
    cluster.push_cursor_to_edge(0, 0, -500).await;

    // A should now be controlling.
    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("A should be controlling B");

    // B should be controlled.
    wait_for_status(&mut cluster.statuses[1], Duration::from_secs(5), |s| {
        s.controlled_by.is_some()
    })
    .await
    .expect("B should be controlled by A");

    // C should be unaffected.
    let status_c = cluster.statuses[2].borrow().clone();
    assert!(status_c.controlling.is_none());
    assert!(status_c.controlled_by.is_none());

    cluster.shutdown().await;
}

#[tokio::test]
async fn test_three_screens_a_to_c_right() {
    let mut cluster = setup_three_screens().await;

    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.session_count >= 2
    })
    .await
    .expect("A should have 2 sessions");

    // Push A's cursor right to cross into C.
    cluster.push_cursor_to_edge(0, 500, 0).await;

    // A should now be controlling.
    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("A should be controlling C");

    // C should be controlled.
    wait_for_status(&mut cluster.statuses[2], Duration::from_secs(5), |s| {
        s.controlled_by.is_some()
    })
    .await
    .expect("C should be controlled by A");

    // B should be unaffected.
    let status_b = cluster.statuses[1].borrow().clone();
    assert!(status_b.controlling.is_none());
    assert!(status_b.controlled_by.is_none());

    cluster.shutdown().await;
}

#[tokio::test]
async fn test_three_screens_cursor_returns_from_b_to_a() {
    let mut cluster = setup_three_screens().await;

    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.session_count >= 2
    })
    .await
    .expect("A should have 2 sessions");

    // Push cursor up into B.
    cluster.push_cursor_to_edge(0, 0, -500).await;

    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("A should be controlling B");

    // Now A is controlling B. Push cursor down — B should send Leave
    // (cursor hits B's bottom edge where A lives) and control returns to A.
    // We inject mouse moves into A's capture (A forwards them to B).
    for _ in 0..10 {
        let event = CapturedEvent {
            device_id: DeviceId(2),
            timestamp_us: 2000,
            event: InputEvent::MouseMove { dx: 0, dy: 500 },
        };
        cluster.feeds[0].send(event).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // A should release control (B sent Leave back).
    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.controlling.is_none()
    })
    .await
    .expect("A should release control when cursor returns from B");

    cluster.shutdown().await;
}

// ---------------------------------------------------------------------------
// Four-screen multi-hop test: A→right→B→below→C via adjacency
// ---------------------------------------------------------------------------

/// Layout:
///   A — B
///   |   |
///   +   C
///
/// A connects to B (right) and C (below-right, via Below for session).
/// B connects to C (below).
/// A's adjacency says B→below→C so A can multi-hop.
///
/// The server (A) must have sessions with ALL machines for multi-hop to
/// work, since it sends Enter directly to the target.
#[tokio::test]
async fn test_multi_hop_a_to_b_to_c() {
    let mut cluster = setup_cluster(3, |addrs| {
        vec![
            DaemonSpec {
                name: "A".into(),
                screens: vec![
                    ScreenConfig {
                        name: "B".into(),
                        address: Some(addrs[1].to_string()),
                        position: Position::Right,
                        fingerprint: None,
                    },
                    ScreenConfig {
                        name: "C".into(),
                        address: Some(addrs[2].to_string()),
                        position: Position::Below,
                        fingerprint: None,
                    },
                ],
                // A knows that below B is C (for multi-hop routing).
                screen_adjacency: vec![ScreenAdjacency {
                    screen: "B".into(),
                    neighbor: "C".into(),
                    position: Position::Below,
                }],
            },
            DaemonSpec {
                name: "B".into(),
                screens: vec![
                    ScreenConfig {
                        name: "A".into(),
                        address: None,
                        position: Position::Left,
                        fingerprint: None,
                    },
                    ScreenConfig {
                        name: "C".into(),
                        address: Some(addrs[2].to_string()),
                        position: Position::Below,
                        fingerprint: None,
                    },
                ],
                screen_adjacency: vec![],
            },
            DaemonSpec {
                name: "C".into(),
                screens: vec![
                    ScreenConfig {
                        name: "B".into(),
                        address: None,
                        position: Position::Above,
                        fingerprint: None,
                    },
                    ScreenConfig {
                        name: "A".into(),
                        address: None,
                        position: Position::Left,
                        fingerprint: None,
                    },
                ],
                screen_adjacency: vec![],
            },
        ]
    })
    .await;

    // Wait for A to have 2 sessions (B + C), B to have 2 (A + C).
    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.session_count >= 2
    })
    .await
    .expect("A should have sessions with B and C");

    wait_for_status(&mut cluster.statuses[1], Duration::from_secs(5), |s| {
        s.session_count >= 2
    })
    .await
    .expect("B should have sessions with A and C");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Step 1: Push A's cursor right into B.
    cluster.push_cursor_to_edge(0, 500, 0).await;

    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("A should be controlling B");

    wait_for_status(&mut cluster.statuses[1], Duration::from_secs(5), |s| {
        s.controlled_by.is_some()
    })
    .await
    .expect("B should be controlled by A");

    // Step 2: Push cursor down — B's bottom edge. B sends Leave with
    // edge=Bottom. A's adjacency map says (B, Bottom) → C.
    // A should multi-hop: release B, initiate control of C.
    for _ in 0..10 {
        let event = CapturedEvent {
            device_id: DeviceId(2),
            timestamp_us: 3000,
            event: InputEvent::MouseMove { dx: 0, dy: 500 },
        };
        cluster.feeds[0].send(event).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // B should send Leave, A processes it, multi-hops to C.
    // A should now be controlling C (not B).
    wait_for_status(&mut cluster.statuses[0], Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await
    .expect("A should be controlling C after multi-hop");

    // C should be controlled.
    wait_for_status(&mut cluster.statuses[2], Duration::from_secs(5), |s| {
        s.controlled_by.is_some()
    })
    .await
    .expect("C should be controlled by A after multi-hop");

    // B should no longer be controlled.
    wait_for_status(&mut cluster.statuses[1], Duration::from_secs(5), |s| {
        s.controlled_by.is_none()
    })
    .await
    .expect("B should be released after multi-hop");

    cluster.shutdown().await;
}
