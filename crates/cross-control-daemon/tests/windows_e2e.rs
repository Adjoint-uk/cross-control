//! End-to-end on a real Windows desktop (CI's Windows runner): real
//! low-level hooks capture input on daemon A, which crosses to daemon B over
//! QUIC with mutual TLS, and B's real `SendInput` emulation acts on the same
//! desktop.
//!
//! Synthetic "hardware" input comes from `test_support::inject_as_hardware`,
//! which tags events so A's hooks accept them; B's own injections are not
//! tagged, so A ignores them — the same echo prevention as in production.
//! Skips (passes with a message) when there is no interactive desktop.

#![cfg(windows)]
// One scenario, step by step: splitting it would obscure the sequence.
#![allow(clippy::too_many_lines)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use cross_control_daemon::config::{
    Config, DaemonConfig, IdentityConfig, InputConfig, ScreenConfig, SecurityConfig,
};
use cross_control_daemon::{Daemon, DaemonEvent, DaemonStatus};
use cross_control_input::null::{NullCapture, NullEmulation};
use cross_control_input::windows::capture::WindowsCapture;
use cross_control_input::windows::devices;
use cross_control_input::windows::emulation::WindowsEmulation;
use cross_control_input::windows::test_support::{
    cursor_pos, inject_as_hardware, is_scan_code_down, primary_screen_size, set_cursor_pos,
    skip_without_desktop,
};
use cross_control_types::{ButtonState, InputEvent, KeyCode, MachineId, Position};
use tokio::sync::watch;

/// F24: exists on no physical keyboard, does nothing anywhere.
const F24_SCAN: u32 = 0x76;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cc-win-e2e-{}", MachineId::new()));
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

    let (width, height) = primary_screen_size();
    if width == 0 || !set_cursor_pos((width / 2).cast_signed(), (height / 2).cast_signed()) {
        skip_without_desktop("cannot move the cursor");
        return;
    }

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
            // Not Ctrl+Shift+Esc: that opens Task Manager on Windows.
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

    // A: real hook capture, shares nothing back.
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
        Box::new(WindowsCapture::new()),
        Box::new(NullEmulation::default()),
    );
    daemon_a.set_local_devices(devices());
    // B: real SendInput emulation, captures nothing.
    let mut daemon_b = Daemon::new(
        base("b", "a", Position::Left, None, &cert_a.fingerprint),
        MachineId::new(),
        transport_b,
        Box::new(NullCapture::default()),
        Box::new(WindowsEmulation::new()),
    );
    let mut status_a = daemon_a.status_receiver();
    let mut status_b = daemon_b.status_receiver();
    let (stop_a, stop_b) = (daemon_a.event_sender(), daemon_b.event_sender());
    let handle_b = tokio::spawn(async move { daemon_b.run().await });
    let handle_a = tokio::spawn(async move { daemon_a.run().await });

    wait_for(&mut status_a, "A connected", |s| s.session_count == 1).await;
    wait_for(&mut status_b, "B connected", |s| s.session_count == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. Push the real cursor to the right edge through A's real hooks.
    for _ in 0..40 {
        inject_as_hardware(&InputEvent::MouseMove { dx: 150, dy: 0 }).unwrap();
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

    // 2. Motion captured by A (and swallowed locally: the cursor is parked)
    //    is replayed by B's SendInput, so the real cursor moves.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let parked = cursor_pos().unwrap();
    for _ in 0..5 {
        inject_as_hardware(&InputEvent::MouseMove { dx: 0, dy: 40 }).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let moved = cursor_pos().unwrap();
    assert!(
        moved.1 > parked.1,
        "B's emulation should have moved the cursor down: parked {parked:?}, now {moved:?}"
    );

    // 3. A key captured by A is pressed system-wide by B — and only by B,
    //    since A swallows the original.
    assert!(
        !is_scan_code_down(0x76),
        "F24 unexpectedly down before the test"
    );
    inject_as_hardware(&key(KeyCode::Unknown(F24_SCAN), ButtonState::Pressed)).unwrap();
    let down = tokio::time::timeout(Duration::from_secs(5), async {
        while !is_scan_code_down(0x76) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    inject_as_hardware(&key(KeyCode::Unknown(F24_SCAN), ButtonState::Released)).unwrap();
    assert!(down.is_ok(), "B never pressed the key A captured");

    // 4. The release hotkey, captured by A's hooks, takes control back.
    inject_as_hardware(&key(KeyCode::F12, ButtonState::Pressed)).unwrap();
    inject_as_hardware(&key(KeyCode::F12, ButtonState::Released)).unwrap();
    wait_for(&mut status_a, "A released control", |s| {
        s.controlling.is_none()
    })
    .await;
    wait_for(&mut status_b, "B released", |s| s.controlled_by.is_none()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !is_scan_code_down(0x76),
        "a key was left pressed after release"
    );

    let _ = stop_a.send(DaemonEvent::Shutdown).await;
    let _ = stop_b.send(DaemonEvent::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), handle_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), handle_b).await;
}
