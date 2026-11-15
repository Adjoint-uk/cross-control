//! cross-control CLI — user-facing binary for the cross-control virtual KVM.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cross-control",
    about = "Share keyboard and mouse across machines",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the cross-control daemon.
    Start {
        /// Path to configuration file.
        #[arg(short, long)]
        config: Option<String>,
    },

    /// Stop the running daemon.
    Stop,

    /// Show daemon status and connected machines.
    Status,

    /// Generate a TLS certificate for this machine.
    GenerateCert {
        /// Output directory for certificate files.
        #[arg(short, long, default_value = ".")]
        output: String,
    },

    /// Pair with a remote machine: fetch its certificate fingerprint,
    /// confirm it, and trust it.
    Pair {
        /// Address of the remote machine (host or host:port).
        address: String,
        /// The remote machine's name (its `identity.name`). Needed when it
        /// does not trust this machine yet, as it then won't say.
        #[arg(long)]
        name: Option<String>,
        /// Trust the fingerprint without asking.
        #[arg(short, long)]
        yes: bool,
    },

    /// Manage the machines this one trusts.
    Trust {
        #[command(subcommand)]
        action: Option<TrustAction>,
    },
}

#[derive(Subcommand)]
enum TrustAction {
    /// Show this machine's fingerprint and the machines it trusts (default).
    List,
    /// Trust a machine's certificate.
    Add {
        /// The machine's name (its `identity.name`).
        name: String,
        /// Its certificate fingerprint, as shown by `cross-control status`
        /// on that machine (`SHA256:aa:bb:…`).
        fingerprint: String,
    },
    /// Stop trusting a machine. Pins in config.toml are not touched.
    Remove {
        /// The machine's name.
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start { config } => {
            start_daemon(config.as_deref()).await?;
        }
        Commands::Stop => {
            stop_daemon()?;
        }
        Commands::Status => {
            show_status();
        }
        Commands::GenerateCert { output } => {
            let hostname = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "cross-control".to_string());

            tracing::info!(hostname = %hostname, output = %output, "generating TLS certificate");

            let cert = cross_control_certgen::generate_certificate(&hostname)?;

            let cert_path = format!("{output}/cross-control.crt");
            let key_path = format!("{output}/cross-control.key");

            std::fs::write(&cert_path, &cert.cert_pem)?;
            std::fs::write(&key_path, &cert.key_pem)?;

            println!("Certificate: {cert_path}");
            println!("Private key: {key_path}");
            println!("Fingerprint: {}", cert.fingerprint);
        }
        Commands::Pair { address, name, yes } => {
            pair_with(&address, name, yes).await?;
        }
        Commands::Trust { action } => {
            manage_trust(action.unwrap_or(TrustAction::List))?;
        }
    }

    Ok(())
}

/// This machine's trust store, as configured.
fn trust_store(config: &cross_control_daemon::Config) -> cross_control_daemon::TrustStore {
    cross_control_daemon::TrustStore::from_config(
        config,
        cross_control_daemon::setup::config_dir().join("known_peers"),
    )
}

/// This machine's name and certificate fingerprint, for the other side's
/// `trust add` command.
fn local_identity(config: &cross_control_daemon::Config) -> anyhow::Result<(String, String)> {
    let (cert_pem, _) = cross_control_daemon::setup::load_or_generate_certs(
        &cross_control_daemon::setup::config_dir(),
    )?;
    let fingerprint = cross_control_certgen::fingerprint_from_pem(&cert_pem)?;
    Ok((config.identity.name.clone(), fingerprint))
}

fn manage_trust(action: TrustAction) -> anyhow::Result<()> {
    use cross_control_daemon::setup;
    use cross_control_daemon::trust::{add_known_peer, remove_known_peer, EntrySource};

    let config = setup::load_config(None)?;
    let store = trust_store(&config);
    match action {
        TrustAction::List => {
            let (name, fingerprint) = local_identity(&config)?;
            println!("This machine: {name}");
            println!("  {fingerprint}");
            println!();
            let entries = store.entries()?;
            if entries.is_empty() {
                println!("No trusted machines. Use `cross-control pair <address>` or");
                println!("`cross-control trust add <name> <fingerprint>`.");
            } else {
                println!("Trusted machines:");
                for entry in entries {
                    let source = match entry.source {
                        EntrySource::Config => "config.toml",
                        EntrySource::KnownPeers => "known_peers",
                    };
                    println!("  {:<20} {}  ({source})", entry.name, entry.fingerprint);
                }
            }
            if config.security.trust_on_first_use {
                println!();
                println!(
                    "Warning: security.trust_on_first_use is on; unknown machines are trusted"
                );
                println!("automatically on first contact.");
            }
        }
        TrustAction::Add { name, fingerprint } => {
            add_known_peer(store.known_peers_path(), &name, &fingerprint)?;
            println!("Trusted \"{name}\" ({fingerprint}).");
            println!("A running daemon picks this up on the peer's next connection.");
        }
        TrustAction::Remove { name } => {
            if remove_known_peer(store.known_peers_path(), &name)? {
                println!("No longer trusting \"{name}\".");
                println!("Restart the daemon to drop an existing connection.");
            } else {
                println!(
                    "\"{name}\" is not in {}.",
                    store.known_peers_path().display()
                );
                if config
                    .screens
                    .iter()
                    .any(|s| s.name == name && s.fingerprint.is_some())
                {
                    println!("It is pinned in config.toml; remove its `fingerprint` there.");
                }
            }
        }
    }
    Ok(())
}

/// Ask the user to confirm a fingerprint, ssh-style. Without a terminal the
/// answer is no: pass `--yes` to pair non-interactively.
fn confirm(prompt: &str) -> anyhow::Result<bool> {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("not a terminal; re-run with --yes to trust the fingerprint shown above");
    }
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
}

/// Connect to a peer, learn its name and TLS fingerprint, confirm it with
/// the user, record it in `known_peers`, and print what to run on the other
/// machine so it trusts us back.
async fn pair_with(address: &str, name: Option<String>, yes: bool) -> anyhow::Result<()> {
    use cross_control_daemon::session::PeerSession;
    use cross_control_daemon::setup;
    use cross_control_daemon::trust::add_known_peer;
    use std::net::SocketAddr;

    let config = setup::load_config(None)?;
    let config_dir = setup::config_dir();
    let (cert_pem, key_pem) = setup::load_or_generate_certs(&config_dir)?;
    let machine_id = setup::load_or_create_machine_id(&config_dir)?;
    let (our_name, our_fingerprint) = local_identity(&config)?;
    let our_layout = config.display_layout();

    // Accept "host" or "host:port"; default to the standard port.
    let addr: SocketAddr = address
        .parse()
        .or_else(|_| format!("{address}:24800").parse())
        .map_err(|e| anyhow::anyhow!("invalid address '{address}': {e}"))?;

    // Bind an ephemeral endpoint just for this pairing exchange.
    let bind: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let transport = cross_control_protocol::QuicTransport::bind(bind, &cert_pem, &key_pem)?;

    println!("Connecting to {addr} …");
    // No pin yet: accept whatever certificate is presented (the peer still
    // has to prove it holds the key) and show its fingerprint for the user
    // to verify out of band.
    let conn = transport
        .connect_pinned(addr, "cross-control", None)
        .await?;
    let fingerprint = conn
        .peer_fingerprint()
        .ok_or_else(|| anyhow::anyhow!("peer presented no TLS certificate"))?;

    // Handshake to learn the peer's name. A peer that does not trust us yet
    // refuses before saying who it is; then the user must supply the name.
    let (control_tx, control_rx) = conn.open_control_stream().await?;
    let mut session = PeerSession::new(conn, control_tx, control_rx);
    let announced = match session
        .handshake_initiator(machine_id, &our_name, &our_layout, |_| Ok(()))
        .await
    {
        Ok(()) => Some(session.name.clone()),
        Err(e) => {
            tracing::debug!(error = %e, "peer refused the handshake");
            None
        }
    };
    let _ = session.disconnect().await;

    let trust_us = format!("cross-control trust add \"{our_name}\" {our_fingerprint}");
    let peer_name = match (name, announced) {
        (Some(given), Some(announced)) if given != announced => {
            anyhow::bail!("the machine at {addr} calls itself \"{announced}\", not \"{given}\"")
        }
        (Some(given), _) => given,
        (None, Some(announced)) => announced,
        (None, None) => anyhow::bail!(
            "the machine at {addr} ({fingerprint}) does not trust this one yet, so it did not \
             say its name.\nEither run this on it first:\n\n  {trust_us}\n\nor re-run with \
             --name <its identity.name>."
        ),
    };

    println!();
    println!("Machine \"{peer_name}\" at {addr}");
    println!("  Fingerprint: {fingerprint}");
    println!();
    println!("Check that this matches `cross-control status` on \"{peer_name}\".");
    if !yes && !confirm("Trust this machine?")? {
        anyhow::bail!("not trusted");
    }

    let store = trust_store(&config);
    add_known_peer(store.known_peers_path(), &peer_name, &fingerprint)?;
    println!(
        "Trusted \"{peer_name}\" (saved to {}).",
        store.known_peers_path().display()
    );
    println!();
    println!("On \"{peer_name}\", trust this machine back:");
    println!();
    println!("  {trust_us}");
    println!();
    println!(
        "Then place it in your layout in {}:",
        config_dir.join("config.toml").display()
    );
    println!();
    println!("  [[screens]]");
    println!("  name = \"{peer_name}\"");
    println!("  address = \"{addr}\"");
    println!("  position = \"Right\"   # Left / Right / Above / Below");

    Ok(())
}

async fn start_daemon(config_path: Option<&str>) -> anyhow::Result<()> {
    use cross_control_daemon::{daemon::Daemon, setup};
    use std::net::SocketAddr;

    let config = setup::load_config(config_path)?;
    let config_dir = setup::config_dir();
    let (cert_pem, key_pem) = setup::load_or_generate_certs(&config_dir)?;
    let machine_id = setup::load_or_create_machine_id(&config_dir)?;

    // Write PID file
    let pid_path = setup::pid_file_path();
    std::fs::write(&pid_path, std::process::id().to_string())?;
    tracing::info!(pid_file = %pid_path.display(), "wrote PID file");

    // Bind transport
    let bind_addr: SocketAddr = format!("{}:{}", config.daemon.bind, config.daemon.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid bind address: {e}"))?;

    let transport = cross_control_protocol::QuicTransport::bind(bind_addr, &cert_pem, &key_pem)?;

    // Create input backends
    #[cfg(feature = "linux")]
    let (capture, emulation, local_devices) = {
        use cross_control_input::linux::capture::EvdevCapture;
        use cross_control_input::linux::emulation::UinputEmulation;

        let capture = EvdevCapture::new();
        let emulation = UinputEmulation::new();
        let devices: Vec<_> = EvdevCapture::enumerate_devices()
            .into_iter()
            .map(|(_, info)| info)
            .collect();
        (
            Box::new(capture) as Box<dyn cross_control_input::InputCapture>,
            Box::new(emulation) as Box<dyn cross_control_input::InputEmulation>,
            devices,
        )
    };

    #[cfg(not(feature = "linux"))]
    {
        anyhow::bail!(
            "no input backend available for this platform. \
             cross-control currently supports Linux only. \
             Windows support is planned for a future release."
        );
    }

    // Create and run daemon
    let mut daemon = Daemon::new(config, machine_id, transport, capture, emulation);
    daemon.set_local_devices(local_devices);
    if let Ok(fingerprint) = cross_control_certgen::fingerprint_from_pem(&cert_pem) {
        // Strip the "SHA256:" prefix for TXT compactness; peers reconstruct it.
        let fp = fingerprint.strip_prefix("SHA256:").unwrap_or(&fingerprint);
        daemon.set_local_fingerprint(fp.to_lowercase());
    }
    // Install the system clipboard backend if a display is reachable. A
    // missing display is normal on headless boxes — log and run without
    // clipboard sync.
    match cross_control_clipboard::ArboardClipboard::new() {
        Ok(cb) => daemon.set_clipboard(Box::new(cb)),
        Err(e) => tracing::warn!(error = %e, "clipboard backend unavailable; running without sync"),
    }

    let event_tx = daemon.event_sender();

    // Signal handling
    let shutdown_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("failed to register SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM");
            }
            _ = sigint.recv() => {
                tracing::info!("received SIGINT");
            }
        }

        let _ = shutdown_tx
            .send(cross_control_daemon::daemon::DaemonEvent::Shutdown)
            .await;
    });

    tracing::info!(
        machine_id = %machine_id,
        bind = %bind_addr,
        "starting cross-control daemon"
    );

    daemon.run().await?;

    // Clean up PID file
    let _ = std::fs::remove_file(&pid_path);
    tracing::info!("daemon stopped");

    Ok(())
}

fn show_status() {
    use cross_control_daemon::setup;

    let pid_path = setup::pid_file_path();
    let config_dir = setup::config_dir();
    let config_path = config_dir.join("config.toml");

    let pid = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    // Check if process is alive via /proc/{pid}
    let alive = pid.is_some_and(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists());

    match pid {
        Some(pid) if alive => {
            println!("Status:  running");
            println!("PID:     {pid}");
        }
        Some(_) => {
            println!("Status:  stopped (stale PID file)");
            let _ = std::fs::remove_file(&pid_path);
        }
        None => println!("Status:  stopped"),
    }

    println!("Config:  {}", config_path.display());

    // Show machine name from config if available
    if config_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(config) = toml::from_str::<cross_control_daemon::config::Config>(&content) {
                println!("Name:    {}", config.identity.name);
            }
        }
    }

    // Show cert fingerprint if available
    let cert_path = config_dir.join("cross-control.crt");
    if cert_path.exists() {
        if let Ok(cert_pem) = std::fs::read_to_string(&cert_path) {
            if let Ok(fingerprint) = cross_control_certgen::fingerprint_from_pem(&cert_pem) {
                println!("Cert:    {fingerprint}");
            }
        }
    }

    // Show live peers, latency, and focus from the daemon's status snapshot.
    // The daemon writes this file every couple of seconds; a running daemon
    // that hasn't written it yet (just started) simply shows no peers.
    if alive {
        print_live_status();
    }
}

/// Read and render the daemon's status snapshot: which peer holds focus and
/// the table of connected peers with latency. Silent if the file is missing
/// or unreadable — the daemon may not have written it yet.
fn print_live_status() {
    use cross_control_daemon::status::{status_file_path, StatusSnapshot};

    let Ok(content) = std::fs::read_to_string(status_file_path()) else {
        return;
    };
    let Ok(snapshot) = StatusSnapshot::from_json(&content) else {
        return;
    };

    // Focus line: who is driving whom right now.
    let focus = match (&snapshot.controlling, &snapshot.controlled_by) {
        (Some(c), _) => format!("controlling {c}"),
        (None, Some(b)) => format!("controlled by {b}"),
        (None, None) => "local".to_string(),
    };
    println!("Focus:   {focus}");

    if snapshot.peers.is_empty() {
        println!("Peers:   none connected");
        return;
    }

    println!("Peers:   {} connected", snapshot.peers.len());
    println!();
    print_peer_row("NAME", "STATE", "LATENCY", "ADDRESS");
    for peer in &snapshot.peers {
        let latency = peer
            .latency_ms
            .map_or_else(|| "—".to_string(), |ms| format!("{ms} ms"));
        print_peer_row(&peer.name, &peer.state, &latency, &peer.address);
    }
}

/// Print one aligned row of the peer table. Inlining the captured identifiers
/// (rather than passing string literals as args) keeps the header call free of
/// the `print_literal` lint.
fn print_peer_row(name: &str, state: &str, latency: &str, address: &str) {
    println!("  {name:<20} {state:<10} {latency:<10} {address}");
}

fn stop_daemon() -> anyhow::Result<()> {
    use cross_control_daemon::setup;

    let pid_path = setup::pid_file_path();
    if !pid_path.exists() {
        anyhow::bail!("no PID file found — daemon may not be running");
    }

    let pid_str = std::fs::read_to_string(&pid_path)?;
    let pid: u32 = pid_str.trim().parse()?;

    tracing::info!(pid, "sending SIGTERM to daemon");

    // Use the kill command to send SIGTERM
    let status = std::process::Command::new("kill")
        .args(["-s", "TERM", &pid.to_string()])
        .status()?;

    if !status.success() {
        anyhow::bail!("failed to send SIGTERM to PID {pid}");
    }

    println!("Sent stop signal to cross-control daemon (PID {pid})");
    Ok(())
}
