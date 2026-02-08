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

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start { config } => {
            tracing::info!(config = ?config, "starting cross-control daemon");
            // TODO: Phase 1 — load config, start daemon
            eprintln!("cross-control daemon not yet implemented (Phase 1)");
        }
        Commands::Stop => {
            tracing::info!("stopping cross-control daemon");
            // TODO: Phase 1 — send stop signal via IPC
            eprintln!("cross-control daemon not yet implemented (Phase 1)");
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
