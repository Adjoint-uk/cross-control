//! cross-control CLI — user-facing binary for the cross-control virtual KVM.
//!
//! Every command is scriptable: `--json` switches output to a single JSON
//! document on stdout, logs go to stderr, and the exit code says what
//! happened (see [`exit`]).

mod cmd;
mod exit;
mod process;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Exit codes, also shown in `--help`.
const EXIT_CODES_HELP: &str = "\
Exit codes:
  0  success
  1  error
  2  invalid command line
  3  daemon not running (status, stop)
  4  invalid or missing configuration
  5  peer not authorized";

#[derive(Parser)]
#[command(
    name = "cross-control",
    about = "Share keyboard and mouse across machines",
    version,
    propagate_version = true,
    after_help = EXIT_CODES_HELP
)]
struct Cli {
    /// Configuration file (default: config.toml in the configuration
    /// directory). It must exist when given explicitly.
    #[arg(short, long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print a single JSON document on stdout instead of text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the cross-control daemon.
    Start,

    /// Stop the running daemon cleanly, and wait until it has exited.
    Stop,

    /// Show daemon status and connected machines.
    Status,

    /// Check that this machine is ready (devices, permissions, config,
    /// trust, port) and explain how to fix what is not. Exit 1 on failure.
    Doctor,

    /// Show this machine's name and certificate fingerprint, creating its
    /// identity (certificate, key, machine id) on first use.
    Identity,

    /// Work with the configuration file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

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

    /// Start the daemon at login: a systemd user service on Linux, a logon
    /// task on Windows.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Check that a configuration file parses and is valid, without
    /// starting anything. Exit code 4 if it is not.
    Check {
        /// File to check (default: the `--config` path, else the default).
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum TrustAction {
    /// Show this machine's fingerprint and the machines it trusts (default).
    List,
    /// Trust a machine's certificate. Re-adding an identical entry changes
    /// nothing (reported as `"changed": false` with `--json`).
    Add {
        /// The machine's name (its `identity.name`).
        name: String,
        /// Its certificate fingerprint, as shown by `cross-control identity`
        /// on that machine (`SHA256:aa:bb:…`).
        fingerprint: String,
    },
    /// Stop trusting a machine. Pins in config.toml are not touched.
    Remove {
        /// The machine's name.
        name: String,
    },
}

#[derive(Subcommand, Clone, Copy)]
pub(crate) enum ServiceAction {
    /// Print the definition for this binary (and `--config`): the systemd
    /// user unit, or the Task Scheduler XML on Windows.
    Unit,
    /// Install it (a user unit plus `daemon-reload`, or a registered logon
    /// task). An unchanged definition is left alone.
    Install {
        /// Also start it now (`systemctl --user enable --now`, or running
        /// the task).
        #[arg(long)]
        enable: bool,
    },
    /// Stop the daemon and remove the unit or task.
    Uninstall,
}

/// Options every command can see.
pub(crate) struct Ctx {
    pub config_path: Option<PathBuf>,
    pub json: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Logs go to stderr so stdout stays clean for `--json`. Only the daemon
    // is chatty by default; one-shot commands log warnings and errors.
    let default_level = if matches!(cli.command, Commands::Start) {
        "info"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        // No colour codes in log files (e.g. under `cross-controlw`).
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level)),
        )
        .init();

    let ctx = Ctx {
        config_path: cli.config,
        json: cli.json,
    };
    let result = match cli.command {
        Commands::Start => cmd::daemon::start(&ctx).await,
        Commands::Stop => cmd::daemon::stop(&ctx).await,
        Commands::Status => Ok(cmd::daemon::status(&ctx)),
        Commands::Identity => cmd::identity::show(&ctx),
        Commands::Doctor => Ok(cmd::doctor::run(&ctx)),
        Commands::Config {
            action: ConfigAction::Check { path },
        } => cmd::config::check(&ctx, path),
        Commands::GenerateCert { output } => cmd::identity::generate_cert(&ctx, &output),
        Commands::Pair { address, name, yes } => cmd::pair::run(&ctx, &address, name, yes).await,
        Commands::Trust { action } => cmd::trust::run(&ctx, action.unwrap_or(TrustAction::List)),
        Commands::Service { action } => cmd::service::run(&ctx, action).await,
    };

    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(exit::code_for(&e))
        }
    }
}
