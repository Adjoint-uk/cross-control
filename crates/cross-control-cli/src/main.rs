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

    /// Pair with a remote machine.
    Pair {
        /// Address of the remote machine (host:port).
        address: String,
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
            // TODO: Phase 2 — query daemon via IPC
            eprintln!("cross-control status not yet implemented (Phase 2)");
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
        Commands::Pair { address } => {
            tracing::info!(address = %address, "pairing with remote machine");
            // TODO: Phase 2 — connect, exchange fingerprints, pin
            eprintln!("cross-control pairing not yet implemented (Phase 2)");
        }
    }

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
        anyhow::bail!("no input backend available for this platform");
    }

    // Create and run daemon
    let mut daemon = Daemon::new(config, machine_id, transport, capture, emulation);
    daemon.set_local_devices(local_devices);

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
