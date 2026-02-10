//! Integration tests exercising the full daemon event loop on loopback.

use std::net::SocketAddr;
use std::time::Duration;

use cross_control_daemon::config::{Config, DaemonConfig, IdentityConfig, ScreenConfig};
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
