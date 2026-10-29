//! End-to-end test of the `cross-control pair` command: run the real binary
//! against a loopback responder and check it prints the peer's fingerprint.

use std::net::SocketAddr;
use std::process::Command;
use std::time::Duration;

use cross_control_daemon::session::PeerSession;
use cross_control_types::{DisplayLayout, MachineId};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_prints_peer_name_and_fingerprint() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // A responder standing in for the machine being paired with.
    let peer_cert = cross_control_certgen::generate_certificate("peer-box").unwrap();
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let transport =
        cross_control_protocol::QuicTransport::bind(bind, &peer_cert.cert_pem, &peer_cert.key_pem)
            .unwrap();
    let addr = transport.local_addr().unwrap();

    let responder = tokio::spawn(async move {
        if let Ok(conn) = transport.accept().await {
            if let Ok((tx, rx)) = conn.accept_control_stream().await {
                let mut session = PeerSession::new(conn, tx, rx);
                let _ = session
                    .handshake_responder(
                        MachineId::new(),
                        "peer-box",
                        &DisplayLayout::single(1920, 1080),
                    )
                    .await;
                // Hold the connection open briefly so the client can finish.
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Run the real binary with an isolated config dir so it mints its own cert.
    let tmp = std::env::temp_dir().join(format!("cc-pair-test-{}", std::process::id()));
    let addr_str = addr.to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_cross-control"))
            .args(["pair", &addr_str])
            .env("XDG_CONFIG_HOME", &tmp)
            .output()
            .expect("run cross-control pair")
    })
    .await
    .unwrap();

    let _ = responder.await;

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "pair failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(&peer_cert.fingerprint),
        "pair output should show the peer fingerprint; got:\n{stdout}"
    );
    assert!(
        stdout.contains("peer-box"),
        "pair output should name the peer; got:\n{stdout}"
    );
    assert!(
        stdout.contains("[[screens]]"),
        "pair output should include a config snippet; got:\n{stdout}"
    );
}
