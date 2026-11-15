//! End-to-end tests of the `cross-control pair` command: run the real binary
//! against a loopback responder.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Duration;

use cross_control_certgen::GeneratedCert;
use cross_control_daemon::session::PeerSession;
use cross_control_daemon::DaemonError;
use cross_control_types::{DisplayLayout, MachineId};

/// A responder standing in for the machine being paired with. When
/// `trusts_us` is false it refuses the handshake, as a daemon that does not
/// trust the pairing machine yet would.
fn spawn_responder(
    cert: &GeneratedCert,
    trusts_us: bool,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let transport =
        cross_control_protocol::QuicTransport::bind(bind, &cert.cert_pem, &cert.key_pem).unwrap();
    let addr = transport.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        if let Ok(conn) = transport.accept().await {
            if let Ok((tx, rx)) = conn.accept_control_stream().await {
                let mut session = PeerSession::new(conn.clone(), tx, rx);
                let result = session
                    .handshake_responder(
                        MachineId::new(),
                        "peer-box",
                        &DisplayLayout::single(1920, 1080),
                        |_| {
                            if trusts_us {
                                Ok(())
                            } else {
                                Err(DaemonError::Config("not trusted".into()))
                            }
                        },
                    )
                    .await;
                if result.is_err() {
                    conn.close_with_reason("not authorized");
                }
                // Hold the connection open briefly so the client can finish.
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    });
    (addr, handle)
}

/// Run the real binary with an isolated config dir so it mints its own cert
/// and writes its own `known_peers`.
async fn run_pair(config_home: PathBuf, args: Vec<String>) -> Output {
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_cross-control"))
            .arg("pair")
            .args(args)
            .env(
                "CROSS_CONTROL_CONFIG_DIR",
                config_home.join("cross-control"),
            )
            .env("CROSS_CONTROL_RUNTIME_DIR", config_home.join("run"))
            .output()
            .expect("run cross-control pair")
    })
    .await
    .unwrap()
}

fn scratch_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("cc-pair-{tag}-{}", std::process::id()))
}

fn known_peers(config_home: &std::path::Path) -> String {
    std::fs::read_to_string(config_home.join("cross-control/known_peers")).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_trusts_peer_and_prints_reverse_command() {
    let peer_cert = cross_control_certgen::generate_certificate("peer-box").unwrap();
    let (addr, responder) = spawn_responder(&peer_cert, true);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let home = scratch_dir("ok");
    let output = run_pair(home.clone(), vec![addr.to_string(), "--yes".into()]).await;
    let _ = responder.await;

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "pair failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(&peer_cert.fingerprint),
        "no fingerprint:\n{stdout}"
    );
    assert!(stdout.contains("peer-box"), "no peer name:\n{stdout}");
    assert!(
        stdout.contains("cross-control trust add"),
        "no reverse command:\n{stdout}"
    );
    assert!(
        stdout.contains("[[screens]]"),
        "no config snippet:\n{stdout}"
    );
    // The responder listens on an ephemeral (non-default) port; the snippet
    // must keep it or the daemon would dial the wrong port.
    assert!(
        stdout.contains(&format!("address = \"{addr}\"")),
        "pair output should keep the peer's port; got:\n{stdout}"
    );

    let recorded = known_peers(&home);
    assert!(
        recorded.contains(&format!("{} peer-box", peer_cert.fingerprint)),
        "known_peers should record the peer; got:\n{recorded}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_without_terminal_or_yes_trusts_nothing() {
    let peer_cert = cross_control_certgen::generate_certificate("peer-box").unwrap();
    let (addr, responder) = spawn_responder(&peer_cert, true);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let home = scratch_dir("noyes");
    let output = run_pair(home.clone(), vec![addr.to_string()]).await;
    let _ = responder.await;

    assert!(
        !output.status.success(),
        "must not trust without confirmation"
    );
    assert_eq!(known_peers(&home), "");
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_untrusting_peer_needs_a_name() {
    let peer_cert = cross_control_certgen::generate_certificate("peer-box").unwrap();

    // Without --name: fails, explaining what to run on the other side.
    let (addr, responder) = spawn_responder(&peer_cert, false);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let home = scratch_dir("noname");
    let output = run_pair(home.clone(), vec![addr.to_string(), "--yes".into()]).await;
    let _ = responder.await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("does not trust this one yet"), "{stderr}");
    assert!(stderr.contains("cross-control trust add"), "{stderr}");
    assert_eq!(known_peers(&home), "");

    // With --name: the fingerprint is still learned from TLS and recorded.
    let (addr, responder) = spawn_responder(&peer_cert, false);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let output = run_pair(
        home.clone(),
        vec![
            addr.to_string(),
            "--name".into(),
            "peer-box".into(),
            "--yes".into(),
        ],
    )
    .await;
    let _ = responder.await;
    assert!(
        output.status.success(),
        "pair --name failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(known_peers(&home).contains(&format!("{} peer-box", peer_cert.fingerprint)));
    let _ = std::fs::remove_dir_all(&home);
}
