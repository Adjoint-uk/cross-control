//! `start`, `stop`, `status`.

use std::net::SocketAddr;

use std::time::Duration;

use cross_control_daemon::config::InputBackend;
use cross_control_daemon::control::{self, Endpoint, Request};
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

    // Claim the control channel first: it doubles as the single-instance
    // lock, so a second `start` fails before touching the PID file.
    let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(1);
    let _control = control::serve(Endpoint::current(), control_tx).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            anyhow::anyhow!("cross-control is already running: {e}")
        } else {
            anyhow::anyhow!("cannot open the control channel: {e}")
        }
    })?;

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
            tokio::select! {
                () = wait_for_shutdown_signal() => {}
                Some(_) = control_rx.recv() => {}
            }
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
        InputBackend::Auto => native_backends(),
        InputBackend::Evdev if cfg!(all(feature = "linux", target_os = "linux")) => {
            native_backends()
        }
        InputBackend::Windows if cfg!(all(feature = "windows", windows)) => native_backends(),
        InputBackend::Macos if cfg!(all(feature = "macos", target_os = "macos")) => {
            native_backends()
        }
        other => Err(cross_control_daemon::DaemonError::Config(format!(
            "input backend {other:?} is not available on this platform"
        ))
        .into()),
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

#[cfg(all(feature = "windows", windows))]
#[allow(clippy::unnecessary_wraps)] // same signature as the fallible fallback
fn native_backends() -> anyhow::Result<Backends> {
    use cross_control_input::windows::{
        capture::WindowsCapture, devices, emulation::WindowsEmulation,
    };

    Ok((
        Box::new(WindowsCapture::new()),
        Box::new(WindowsEmulation::new()),
        devices(),
    ))
}

#[cfg(all(feature = "macos", target_os = "macos"))]
#[allow(clippy::unnecessary_wraps)] // same signature as the fallible fallback
fn native_backends() -> anyhow::Result<Backends> {
    use cross_control_input::macos::{capture::MacCapture, devices, emulation::MacEmulation};

    Ok((
        Box::new(MacCapture::new()),
        Box::new(MacEmulation::new()),
        devices(),
    ))
}

#[cfg(not(any(
    all(feature = "linux", target_os = "linux"),
    all(feature = "windows", windows),
    all(feature = "macos", target_os = "macos")
)))]
fn native_backends() -> anyhow::Result<Backends> {
    Err(cross_control_daemon::DaemonError::Config(
        "no native input backend for this platform; \
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
    if crate::process::is_alive(pid) {
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

/// Ask the running daemon to stop, and wait until it has. Exit code 3 if it
/// is not running.
///
/// The daemon is asked over the control channel, so it shuts down cleanly
/// (releasing input and saying goodbye to peers). Only if it does not
/// answer is the process terminated instead.
pub async fn stop(ctx: &Ctx) -> anyhow::Result<u8> {
    let (pid, graceful) = match control::request(&Endpoint::current(), Request::Stop).await {
        Ok(response) if response.ok => (response.pid, true),
        answer => {
            let pid = running_pid().ok_or(exit::NotRunning)?;
            match answer {
                Ok(response) => {
                    tracing::warn!(error = ?response.error, "the daemon refused to stop");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "the daemon did not answer on the control channel");
                }
            }
            tracing::warn!(pid, "terminating the daemon process");
            crate::process::terminate(pid)?;
            (pid, false)
        }
    };
    let exited = wait_for_exit(pid, Duration::from_secs(10)).await;
    if !exited {
        anyhow::bail!("the daemon (PID {pid}) was asked to stop but is still running");
    }
    if ctx.json {
        Ctx::print_json(&json!({ "stopped": pid, "graceful": graceful }));
    } else if graceful {
        println!("Stopped cross-control daemon (PID {pid})");
    } else {
        println!("Terminated cross-control daemon (PID {pid})");
    }
    Ok(exit::OK)
}

/// Wait until the process `pid` has exited, up to `limit`.
async fn wait_for_exit(pid: u32, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    while crate::process::is_alive(pid) {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    true
}
