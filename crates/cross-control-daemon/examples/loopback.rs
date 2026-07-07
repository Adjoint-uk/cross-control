//! Two daemons, one machine — the whole networked KVM on loopback.
//!
//! This runs a complete cross-control session between two daemons in a single
//! process, both bound to `127.0.0.1`, with **no second machine, no root, and
//! no display server required**. It exercises everything between the two
//! physical ends: the QUIC handshake, device announce, edge-based cursor
//! crossing, and live input forwarding. The only pieces it does *not* cover
//! are the two ends themselves — real `evdev` capture of a physical mouse and
//! real `uinput` injection into a live compositor — which is exactly the
//! remaining hardware bring-up (issues #1/#3).
//!
//! ## Run it
//!
//! ```console
//! # Headless, deterministic — machine-B uses a mock backend we can observe.
//! cargo run -p cross-control-daemon --example loopback
//!
//! # On a Linux desktop with /dev/uinput access, watch a REAL cursor move:
//! cargo run -p cross-control-daemon --example loopback --features linux -- --real
//! ```
//!
//! In `--real` mode, machine-B injects the forwarded input into an actual
//! uinput virtual device, so after the cursor "crosses" you will see your
//! pointer drift as machine-A streams mouse motion across the link.

use std::net::SocketAddr;
use std::time::Duration;

use cross_control_daemon::config::{Config, DaemonConfig, IdentityConfig, ScreenConfig};
use cross_control_daemon::{Daemon, DaemonEvent, DaemonStatus};
use cross_control_input::mock::{MockCapture, MockEmulation, MockEmulationHandle};
use cross_control_input::InputEmulation;
use cross_control_types::{
    ButtonState, CapturedEvent, DeviceCapability, DeviceId, DeviceInfo, InputEvent, KeyCode,
    MachineId, Position,
};
use tokio::sync::watch;

/// The two devices each daemon advertises to its peer.
fn demo_devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            id: DeviceId(1),
            name: "loopback-keyboard".to_string(),
            capabilities: vec![DeviceCapability::Keyboard],
        },
        DeviceInfo {
            id: DeviceId(2),
            name: "loopback-mouse".to_string(),
            capabilities: vec![DeviceCapability::RelativeMouse, DeviceCapability::Scroll],
        },
    ]
}

/// machine-B's emulation backend, plus a mock handle when we can observe it.
enum EmulationB {
    /// Mock backend — deterministic, inspectable via the handle.
    Mock(MockEmulationHandle),
    /// Real uinput backend — visible on screen but not inspectable here.
    /// Only constructed when built with `--features linux`.
    #[cfg_attr(not(feature = "linux"), allow(dead_code))]
    Real,
}

/// Build machine-B's emulation. `--real` (with `--features linux`) wires a real
/// uinput device so you can watch the cursor move; otherwise a mock backend we
/// can assert against.
fn build_emulation_b(real: bool) -> (Box<dyn InputEmulation>, EmulationB) {
    if real {
        #[cfg(feature = "linux")]
        {
            let emu = cross_control_input::linux::emulation::UinputEmulation::new();
            println!("machine-B: using REAL uinput emulation — watch your cursor.");
            return (Box::new(emu), EmulationB::Real);
        }
        #[cfg(not(feature = "linux"))]
        {
            eprintln!("note: --real needs `--features linux`; falling back to the mock backend.\n");
        }
    }
    let backend = MockEmulation::new();
    let handle = backend.handle();
    (Box::new(backend), EmulationB::Mock(handle))
}

/// Wait until `pred` holds on a status watch channel, or time out.
async fn wait_for(
    rx: &mut watch::Receiver<DaemonStatus>,
    timeout: Duration,
    pred: impl Fn(&DaemonStatus) -> bool,
) -> Result<(), &'static str> {
    tokio::time::timeout(timeout, async {
        loop {
            if pred(&rx.borrow_and_update().clone()) {
                return Ok(());
            }
            if rx.changed().await.is_err() {
                return Err("status channel closed");
            }
        }
    })
    .await
    .map_err(|_| "timed out")?
}

#[tokio::main]
async fn main() {
    let real = std::env::args().any(|a| a == "--real");
    if let Err(e) = run(real).await {
        eprintln!("\n✗ loopback demo failed: {e}");
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_lines)]
async fn run(real: bool) -> Result<(), Box<dyn std::error::Error>> {
    // QUIC needs a process-wide crypto provider installed once.
    let _ = rustls::crypto::ring::default_provider().install_default();

    println!("cross-control loopback demo — two daemons on 127.0.0.1\n");

    // Each daemon gets its own self-signed cert and an ephemeral UDP port.
    let cert_a = cross_control_certgen::generate_certificate("localhost")?;
    let cert_b = cross_control_certgen::generate_certificate("localhost")?;
    let bind: SocketAddr = "127.0.0.1:0".parse()?;
    let transport_a =
        cross_control_protocol::QuicTransport::bind(bind, &cert_a.cert_pem, &cert_a.key_pem)?;
    let transport_b =
        cross_control_protocol::QuicTransport::bind(bind, &cert_b.cert_pem, &cert_b.key_pem)?;
    let addr_b = transport_b.local_addr()?;

    // machine-A sits left of machine-B and dials it directly (static address,
    // so the demo doesn't depend on multicast/mDNS being available).
    let config_a = Config {
        daemon: DaemonConfig {
            screen_width: 1920,
            screen_height: 1080,
            discovery: false,
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
    let config_b = Config {
        daemon: DaemonConfig {
            screen_width: 1920,
            screen_height: 1080,
            discovery: false,
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

    // machine-A: scripted mock capture (we feed it cursor motion) + mock
    // emulation. machine-B: idle capture + the chosen emulation backend.
    let (capture_a, feed_a) = MockCapture::new();
    let (capture_b, _feed_b) = MockCapture::new();
    let (emulation_b, emu_b) = build_emulation_b(real);

    let mut daemon_a = Daemon::new(
        config_a,
        MachineId::new(),
        transport_a,
        Box::new(capture_a),
        Box::new(MockEmulation::new()),
    );
    daemon_a.set_local_devices(demo_devices());
    let mut status_a = daemon_a.status_receiver();
    let shutdown_a = daemon_a.event_sender();

    let mut daemon_b = Daemon::new(
        config_b,
        MachineId::new(),
        transport_b,
        Box::new(capture_b),
        emulation_b,
    );
    daemon_b.set_local_devices(demo_devices());
    let mut status_b = daemon_b.status_receiver();
    let shutdown_b = daemon_b.event_sender();

    // Start B (the server) first, then A dials it.
    let handle_b = tokio::spawn(async move {
        if let Err(e) = daemon_b.run().await {
            eprintln!("machine-B error: {e}");
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let handle_a = tokio::spawn(async move {
        if let Err(e) = daemon_a.run().await {
            eprintln!("machine-A error: {e}");
        }
    });

    // 1) Handshake + pairing.
    wait_for(&mut status_a, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await?;
    wait_for(&mut status_b, Duration::from_secs(5), |s| {
        s.session_count >= 1
    })
    .await?;
    println!("✓ paired: machine-a ⇄ machine-b over QUIC (TLS 1.3)");

    // Give device-announce a moment so B has virtual devices mapped.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 2) Push machine-A's cursor into machine-B's edge.
    println!("→ moving machine-a's cursor right, into the machine-b edge…");
    for _ in 0..5 {
        feed_a
            .send(CapturedEvent {
                device_id: DeviceId(2),
                timestamp_us: 1000,
                event: InputEvent::MouseMove { dx: 500, dy: 0 },
            })
            .await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_for(&mut status_a, Duration::from_secs(5), |s| {
        s.controlling.is_some()
    })
    .await?;
    wait_for(&mut status_b, Duration::from_secs(5), |s| {
        s.controlled_by.is_some()
    })
    .await?;
    println!("✓ crossed: machine-a is now controlling machine-b");
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3) Forward input across the link and confirm it lands on machine-B.
    match emu_b {
        EmulationB::Mock(handle) => {
            println!("→ typing 'A' on machine-a; expecting it on machine-b…");
            for i in 0..5 {
                feed_a
                    .send(CapturedEvent {
                        device_id: DeviceId(1),
                        timestamp_us: 3000 + i,
                        event: InputEvent::Key {
                            code: KeyCode::KeyA,
                            state: ButtonState::Pressed,
                        },
                    })
                    .await?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let landed = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if handle.injected_events().iter().any(|e| {
                        matches!(
                            e.event,
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
            .await;
            if landed.is_err() {
                return Err("machine-b never received the forwarded KeyA".into());
            }
            println!("✓ forwarded: machine-b's virtual keyboard received KeyA");
        }
        EmulationB::Real => {
            println!("→ streaming mouse motion to machine-b — watch the cursor drift…");
            for _ in 0..120 {
                feed_a
                    .send(CapturedEvent {
                        device_id: DeviceId(2),
                        timestamp_us: 4000,
                        event: InputEvent::MouseMove { dx: 3, dy: 3 },
                    })
                    .await?;
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
            println!("✓ streamed 120 motion events into machine-b's real uinput device");
        }
    }

    // 4) Clean shutdown of both daemons.
    let _ = shutdown_a.send(DaemonEvent::Shutdown).await;
    let _ = shutdown_b.send(DaemonEvent::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), handle_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), handle_b).await;

    println!("\n✓ done — the full KVM path works on one machine.");
    println!(
        "  Remaining for real two-box bring-up: physical evdev capture + live uinput/compositor."
    );
    Ok(())
}
