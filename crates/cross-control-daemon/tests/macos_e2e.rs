//! End-to-end on a real macOS desktop (CI's macOS runner): a real Quartz
//! event tap captures input on daemon A, which crosses to daemon B over QUIC
//! with mutual TLS, and B's real `CGEventPost` emulation acts on the same
//! desktop.
//!
//! Synthetic "hardware" input comes from `test_support::hardware()`, whose
//! events carry no marker, so A's tap captures them; B's injections carry
//! the daemon's marker, so A ignores them — the same echo prevention as in
//! production. Skips (passes with a message) without the Input Monitoring
//! and Accessibility permissions, unless `CROSS_CONTROL_REQUIRE_DESKTOP`.

#![cfg(target_os = "macos")]
// One scenario, step by step: splitting it would obscure the sequence.
#![allow(clippy::too_many_lines)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use cross_control_daemon::config::{
    Config, DaemonConfig, IdentityConfig, InputConfig, ScreenConfig, SecurityConfig,
};
use cross_control_daemon::{Daemon, DaemonEvent, DaemonStatus};
use cross_control_input::macos::capture::MacCapture;
use cross_control_input::macos::devices;
use cross_control_input::macos::emulation::MacEmulation;
use cross_control_input::macos::test_support::{
    cursor_position, hardware, has_permissions, is_key_down, main_display_size,
    set_cursor_position, skip_without_desktop,
};
use cross_control_input::null::{NullCapture, NullEmulation};
use cross_control_input::InputEmulation;
use cross_control_types::{ButtonState, InputEvent, KeyCode, MachineId, Position, VirtualDeviceId};
use tokio::sync::watch;

/// F20 (`kVK_F20`): on no Mac keyboard, does nothing anywhere.
const F20: u16 = 0x5A;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cc-mac-e2e-{}", MachineId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("known_peers")
}

fn key(code: KeyCode, state: ButtonState) -> InputEvent {
    InputEvent::Key { code, state }
}

async fn wait_for(
    rx: &mut watch::Receiver<DaemonStatus>,
    what: &str,
    pred: impl Fn(&DaemonStatus) -> bool,
) {
    let ok = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if pred(&rx.borrow_and_update()) {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    })
    .await;
    assert!(
        ok.is_ok() && pred(&rx.borrow()),
        "timed out waiting for {what}: {:?}",
        *rx.borrow()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_capture_crosses_to_real_emulation() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    if !has_permissions() {
        skip_without_desktop("no Input Monitoring / Accessibility permission");
        return;
    }
    let (width, height) = main_display_size();
    if width == 0 || !set_cursor_position(f64::from(width / 2), f64::from(height / 2)) {
        skip_without_desktop("cannot move the cursor");
        return;
    }
    let mut hw = hardware();
    let dev = VirtualDeviceId(0);

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

    let base = |name: &str, peer: &str, position, address: Option<String>, fp: &str| Config {
        daemon: DaemonConfig {
            // A's virtual screen must match the real one, so the virtual
            // cursor reaches the edge when the real cursor does.
            screen_width: width,
            screen_height: height,
            discovery: false,
            ..DaemonConfig::default()
        },
        identity: IdentityConfig {
            name: name.to_string(),
            ..IdentityConfig::default()
        },
        input: InputConfig {
            release_hotkey: vec!["F12".to_string()],
            ..InputConfig::default()
        },
        screens: vec![ScreenConfig {
            name: peer.to_string(),
            address,
            position,
            fingerprint: Some(fp.to_string()),
        }],
        security: SecurityConfig {
            trust_on_first_use: false,
            known_peers: Some(scratch()),
        },
        ..Config::default()
    };

    // A: real event-tap capture, shares nothing back.
    let mut daemon_a = Daemon::new(
        base(
            "a",
            "b",
            Position::Right,
            Some(addr_b.to_string()),
            &cert_b.fingerprint,
        ),
        MachineId::new(),
        transport_a,
        Box::new(MacCapture::new()),
        Box::new(NullEmulation::default()),
    );
    daemon_a.set_local_devices(devices());
    // B: real CGEventPost emulation, captures nothing.
    let mut daemon_b = Daemon::new(
        base("b", "a", Position::Left, None, &cert_a.fingerprint),
        MachineId::new(),
        transport_b,
        Box::new(NullCapture::default()),
        Box::new(MacEmulation::new()),
    );
    let mut status_a = daemon_a.status_receiver();
    let mut status_b = daemon_b.status_receiver();
    let (stop_a, stop_b) = (daemon_a.event_sender(), daemon_b.event_sender());
    let handle_b = tokio::spawn(async move { daemon_b.run().await });
    let handle_a = tokio::spawn(async move { daemon_a.run().await });

    wait_for(&mut status_a, "A connected", |s| s.session_count == 1).await;
    wait_for(&mut status_b, "B connected", |s| s.session_count == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. Push the real cursor to the right edge through A's real tap.
    for _ in 0..40 {
        hw.inject(dev, InputEvent::MouseMove { dx: 150, dy: 0 })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(15)).await;
        if status_a.borrow().controlling.is_some() {
            break;
        }
    }
    wait_for(&mut status_a, "A controlling B", |s| {
        s.controlling.is_some()
    })
    .await;
    wait_for(&mut status_b, "B controlled by A", |s| {
        s.controlled_by.is_some()
    })
    .await;

    // 2. Motion captured by A (and swallowed locally: the cursor is parked
    //    and detached) is replayed by B's CGEventPost, so the cursor moves.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let parked = cursor_position().unwrap();
    for _ in 0..5 {
        hw.inject(dev, InputEvent::MouseMove { dx: 0, dy: 40 })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let moved = cursor_position().unwrap();
    assert!(
        moved.1 > parked.1,
        "B's emulation should have moved the cursor down: parked {parked:?}, now {moved:?}"
    );

    // 3. A key captured by A is pressed system-wide by B — and only by B,
    //    since A swallows the original.
    assert!(!is_key_down(F20), "F20 unexpectedly down before the test");
    hw.inject(
        dev,
        key(KeyCode::Unknown(u32::from(F20)), ButtonState::Pressed),
    )
    .await
    .unwrap();
    let down = tokio::time::timeout(Duration::from_secs(5), async {
        while !is_key_down(F20) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    hw.inject(
        dev,
        key(KeyCode::Unknown(u32::from(F20)), ButtonState::Released),
    )
    .await
    .unwrap();
    assert!(down.is_ok(), "B never pressed the key A captured");

    // 4. The release hotkey, captured by A's tap, takes control back.
    hw.inject(dev, key(KeyCode::F12, ButtonState::Pressed))
        .await
        .unwrap();
    hw.inject(dev, key(KeyCode::F12, ButtonState::Released))
        .await
        .unwrap();
    wait_for(&mut status_a, "A released control", |s| {
        s.controlling.is_none()
    })
    .await;
    wait_for(&mut status_b, "B released", |s| s.controlled_by.is_none()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!is_key_down(F20), "a key was left pressed after release");

    let _ = stop_a.send(DaemonEvent::Shutdown).await;
    let _ = stop_b.send(DaemonEvent::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), handle_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), handle_b).await;
}
