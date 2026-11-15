//! `start`, `stop`, `status`.

use std::net::SocketAddr;

use cross_control_daemon::config::InputBackend;
use cross_control_daemon::status::{status_file_path, StatusSnapshot};
use cross_control_daemon::{daemon::Daemon, setup};
use serde_json::json;

use crate::exit;
use crate::Ctx;

pub async fn start(ctx: &Ctx) -> anyhow::Result<u8> {
    let config = ctx.config()?;
    let config_dir = setup::config_dir();
    let (cert_pem, key_pem) = setup::load_identity_certs(&config, &config_dir)?;
    let machine_id = setup::load_or_create_machine_id(&config_dir)?;

    let pid_path = setup::pid_file_path();
    setup::write_private_file(&pid_path, std::process::id().to_string().as_bytes())?;
    tracing::info!(pid_file = %pid_path.display(), "wrote PID file");

    let bind_addr: SocketAddr = format!("{}:{}", config.daemon.bind, config.daemon.port)
        .parse()
        .map_err(|e| {
            cross_control_daemon::DaemonError::Config(format!("invalid bind address: {e}"))
        })?;
    let transport = cross_control_protocol::QuicTransport::bind(bind_addr, &cert_pem, &key_pem)?;

    let (capture, emulation, local_devices) = input_backends(config.input.backend)?;

    {
        let mut daemon = Daemon::new(config, machine_id, transport, capture, emulation);
        daemon.set_local_devices(local_devices);
        if let Ok(fingerprint) = cross_control_certgen::fingerprint_from_pem(&cert_pem) {
            // Strip the "SHA256:" prefix for TXT compactness; peers
            // reconstruct it.
            let fp = fingerprint.strip_prefix("SHA256:").unwrap_or(&fingerprint);
            daemon.set_local_fingerprint(fp.to_lowercase());
        }
        // A missing display is normal on headless boxes: run without
        // clipboard sync.
        match cross_control_clipboard::ArboardClipboard::new() {
            Ok(cb) => daemon.set_clipboard(Box::new(cb)),
            Err(e) => {
                tracing::warn!(error = %e, "clipboard backend unavailable; running without sync");
            }
        }

        let shutdown_tx = daemon.event_sender();
        tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            let _ = shutdown_tx
                .send(cross_control_daemon::daemon::DaemonEvent::Shutdown)
                .await;
        });

        tracing::info!(machine_id = %machine_id, bind = %bind_addr, "starting cross-control daemon");
        daemon.run().await?;
        let _ = std::fs::remove_file(&pid_path);
        tracing::info!("daemon stopped");
        Ok(exit::OK)
    }
}

type Backends = (
    Box<dyn cross_control_input::InputCapture>,
    Box<dyn cross_control_input::InputEmulation>,
    Vec<cross_control_types::DeviceInfo>,
);

/// The input capture/emulation pair (and local devices) for `backend`.
fn input_backends(backend: InputBackend) -> anyhow::Result<Backends> {
    use cross_control_input::null::{NullCapture, NullEmulation};

    match backend {
        InputBackend::None => {
            tracing::warn!(
                "input backend is \"none\": this machine neither shares nor receives keyboard and mouse"
            );
            Ok((
                Box::new(NullCapture::default()),
                Box::new(NullEmulation::default()),
                Vec::new(),
            ))
        }
        InputBackend::Auto | InputBackend::Evdev => native_backends(),
    }
}

#[cfg(all(feature = "linux", target_os = "linux"))]
#[allow(clippy::unnecessary_wraps)] // same signature as the fallible non-Linux version
fn native_backends() -> anyhow::Result<Backends> {
    use cross_control_input::linux::capture::EvdevCapture;
    use cross_control_input::linux::emulation::UinputEmulation;

    let devices = EvdevCapture::enumerate_devices()
        .into_iter()
        .map(|(_, info)| info)
        .collect();
    Ok((
        Box::new(EvdevCapture::new()),
        Box::new(UinputEmulation::new()),
        devices,
    ))
}

#[cfg(not(all(feature = "linux", target_os = "linux")))]
fn native_backends() -> anyhow::Result<Backends> {
    Err(cross_control_daemon::DaemonError::Config(
        "no native input backend for this platform yet (Linux only); \
         set [input] backend = \"none\" to run without input"
            .to_string(),
    )
    .into())
}

/// Resolve when the process is asked to stop (SIGTERM/SIGINT on Unix,
/// Ctrl+C elsewhere).
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut int)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) else {
            tracing::warn!("cannot install signal handlers; falling back to Ctrl+C only");
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            _ = term.recv() => tracing::info!("received SIGTERM"),
            _ = int.recv() => tracing::info!("received SIGINT"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("received Ctrl+C");
    }
}

/// The running daemon's PID, if the PID file names a live process.
/// A stale PID file is removed.
fn running_pid() -> Option<u32> {
    let pid_path = setup::pid_file_path();
    let pid = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())?;
    if std::path::Path::new(&format!("/proc/{pid}")).exists() {
        Some(pid)
    } else {
        let _ = std::fs::remove_file(&pid_path);
        None
    }
}

/// Show status. Exit code 3 when the daemon is not running (the LSB
/// `status` convention), so `cross-control status` works as a health check.
pub fn status(ctx: &Ctx) -> u8 {
    let pid = running_pid();
    let config = ctx.config().ok();
    let fingerprint = config.as_ref().and_then(|c| {
        let pem =
            std::fs::read_to_string(setup::identity_cert_path(c, &setup::config_dir())).ok()?;
        cross_control_certgen::fingerprint_from_pem(&pem).ok()
    });
    let snapshot = pid
        .and_then(|_| std::fs::read_to_string(status_file_path()).ok())
        .and_then(|s| StatusSnapshot::from_json(&s).ok());
    let focus = snapshot.as_ref().map_or("local".to_string(), |s| {
        match (&s.controlling, &s.controlled_by) {
            (Some(c), _) => format!("controlling {c}"),
            (None, Some(b)) => format!("controlled by {b}"),
            (None, None) => "local".to_string(),
        }
    });
    let code = if pid.is_some() {
        exit::OK
    } else {
        exit::NOT_RUNNING
    };

    if ctx.json {
        Ctx::print_json(&json!({
            "running": pid.is_some(),
            "pid": pid,
            "name": config.as_ref().map(|c| c.identity.name.clone()),
            "fingerprint": fingerprint,
            "config": ctx.config_file(),
            "focus": pid.map(|_| focus.clone()),
            "controlling": snapshot.as_ref().and_then(|s| s.controlling.clone()),
            "controlled_by": snapshot.as_ref().and_then(|s| s.controlled_by.clone()),
            "peers": snapshot.as_ref().map(|s| s.peers.clone()).unwrap_or_default(),
        }));
        return code;
    }

    match pid {
        Some(pid) => {
            println!("Status:  running");
            println!("PID:     {pid}");
        }
        None => println!("Status:  stopped"),
    }
    println!("Config:  {}", ctx.config_file().display());
    if let Some(config) = &config {
        println!("Name:    {}", config.identity.name);
    }
    if let Some(fp) = &fingerprint {
        println!("Cert:    {fp}");
    }
    if pid.is_some() {
        println!("Focus:   {focus}");
        match snapshot.as_ref().map(|s| &s.peers) {
            Some(peers) if !peers.is_empty() => {
                println!("Peers:   {} connected", peers.len());
                println!();
                print_peer_row("NAME", "STATE", "LATENCY", "ADDRESS");
                for peer in peers {
                    let latency = peer
                        .latency_ms
                        .map_or_else(|| "—".to_string(), |ms| format!("{ms} ms"));
                    print_peer_row(&peer.name, &peer.state, &latency, &peer.address);
                }
            }
            _ => println!("Peers:   none connected"),
        }
    }
    code
}

/// Print one aligned row of the peer table.
fn print_peer_row(name: &str, state: &str, latency: &str, address: &str) {
    println!("  {name:<20} {state:<10} {latency:<10} {address}");
}

/// Ask the running daemon to stop. Exit code 3 if it is not running.
pub fn stop(ctx: &Ctx) -> anyhow::Result<u8> {
    let pid = running_pid().ok_or(exit::NotRunning)?;
    tracing::info!(pid, "sending SIGTERM to daemon");
    let status = std::process::Command::new("kill")
        .args(["-s", "TERM", &pid.to_string()])
        .status()?;
    if !status.success() {
        anyhow::bail!("failed to send SIGTERM to PID {pid}");
    }
    if ctx.json {
        Ctx::print_json(&json!({ "stopped": pid }));
    } else {
        println!("Sent stop signal to cross-control daemon (PID {pid})");
    }
    Ok(exit::OK)
}
