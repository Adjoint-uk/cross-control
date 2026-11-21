//! `cross-controlw`: run the daemon in the background, without a console
//! window — what a Windows logon task starts (like `pythonw` / `javaw`).
//!
//! ```text
//! cross-controlw [--env NAME=VALUE]... [cross-control start options]
//! ```
//!
//! It runs `cross-control start` (from the same directory) with its output
//! appended to `cross-control.log` in the runtime directory, and restarts
//! it if it fails, like systemd's `Restart=on-failure`:
//!
//! - a clean exit (`cross-control stop`) ends `cross-controlw` too;
//! - a configuration or usage error is not retried — fix it and restart;
//! - anything else is retried after 5 seconds, giving up after 5 failures
//!   in a row that each lasted under a minute.
//!
//! `--env` sets a variable for the daemon (Task Scheduler actions cannot),
//! e.g. `--env CROSS_CONTROL_CONFIG_DIR=D:\cc`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// Exit codes of `cross-control` that restarting cannot fix.
const USAGE: i32 = 2;
const CONFIG: i32 = 4;

/// `--env` assignments, and the remaining arguments for `start`.
type Parsed = (Vec<(String, String)>, Vec<String>);

const RESTART_DELAY: Duration = Duration::from_secs(5);
/// A run shorter than this counts towards giving up.
const QUICK_FAILURE: Duration = Duration::from_secs(60);
const MAX_QUICK_FAILURES: u32 = 5;
/// The log is started afresh at a launch when it has grown past this.
const MAX_LOG: u64 = 10 * 1024 * 1024;

fn main() -> ExitCode {
    let (env, args) = match parse_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("cross-controlw: {message}");
            return ExitCode::from(2);
        }
    };
    // Apply overrides here too, so the log lands in the right directory.
    let runtime_dir = env
        .iter()
        .find(|(k, _)| k == cross_control_daemon::setup::RUNTIME_DIR_ENV)
        .map_or_else(cross_control_daemon::setup::runtime_dir, |(_, v)| {
            PathBuf::from(v)
        });
    let log_path = runtime_dir.join("cross-control.log");
    let exe = daemon_exe();

    let mut quick_failures = 0;
    loop {
        let log = match open_log(&log_path) {
            Ok(log) => log,
            Err(e) => {
                eprintln!("cross-controlw: cannot open {}: {e}", log_path.display());
                return ExitCode::FAILURE;
            }
        };
        let mut command = Command::new(&exe);
        command
            .arg("start")
            .args(&args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_or_else(|_| Stdio::null(), Stdio::from))
            .stderr(log.try_clone().map_or_else(|_| Stdio::null(), Stdio::from));
        hide_window(&mut command);

        let started = Instant::now();
        let status = match command.status() {
            Ok(status) => status,
            Err(e) => {
                note(&log, &format!("cannot start {}: {e}", exe.display()));
                return ExitCode::FAILURE;
            }
        };
        match status.code() {
            Some(0) => return ExitCode::SUCCESS,
            Some(code @ (USAGE | CONFIG)) => {
                note(
                    &log,
                    &format!("cross-control exited with {code}; not restarting"),
                );
                return ExitCode::from(u8::try_from(code).unwrap_or(1));
            }
            _ => {}
        }
        if started.elapsed() < QUICK_FAILURE {
            quick_failures += 1;
        } else {
            quick_failures = 1;
        }
        if quick_failures >= MAX_QUICK_FAILURES {
            note(
                &log,
                &format!(
                    "cross-control failed {quick_failures} times in a row ({status}); giving up"
                ),
            );
            return ExitCode::FAILURE;
        }
        note(
            &log,
            &format!(
                "cross-control exited ({status}); restarting in {}s",
                RESTART_DELAY.as_secs()
            ),
        );
        std::thread::sleep(RESTART_DELAY);
    }
}

/// Split `--env NAME=VALUE` options from the arguments for `start`.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut env = Vec::new();
    let mut rest = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let assignment = if arg == "--env" {
            Some(args.next().ok_or("--env needs NAME=VALUE")?)
        } else {
            arg.strip_prefix("--env=").map(str::to_string)
        };
        match assignment {
            Some(assignment) => {
                let (name, value) = assignment
                    .split_once('=')
                    .filter(|(name, _)| !name.is_empty())
                    .ok_or_else(|| format!("--env needs NAME=VALUE, got {assignment:?}"))?;
                env.push((name.to_string(), value.to_string()));
            }
            None => rest.push(arg),
        }
    }
    Ok((env, rest))
}

/// `cross-control` from the directory this program is in.
fn daemon_exe() -> PathBuf {
    let name = format!("cross-control{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|exe| exe.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    if let Some(dir) = path.parent() {
        cross_control_daemon::setup::create_private_dir(dir)?;
    }
    let too_big = std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG);
    let mut options = OpenOptions::new();
    options.create(true);
    if too_big {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    options.open(path)
}

/// Append a line of our own to the log.
fn note(mut log: &std::fs::File, message: &str) {
    let _ = writeln!(log, "cross-controlw: {message}");
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`: run a console program without a console.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Parsed, String> {
        parse_args(args.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn env_options_are_split_from_start_arguments() {
        let (env, rest) = parse(&[
            "--env",
            "CROSS_CONTROL_CONFIG_DIR=C:\\cc dir",
            "--config",
            "C:\\cc dir\\config.toml",
            "--env=RUST_LOG=debug,quinn=warn",
        ])
        .unwrap();
        assert_eq!(
            env,
            vec![
                (
                    "CROSS_CONTROL_CONFIG_DIR".to_string(),
                    "C:\\cc dir".to_string()
                ),
                ("RUST_LOG".to_string(), "debug,quinn=warn".to_string()),
            ]
        );
        assert_eq!(rest, vec!["--config", "C:\\cc dir\\config.toml"]);
    }

    #[test]
    fn malformed_env_is_rejected() {
        assert!(parse(&["--env"]).is_err());
        assert!(parse(&["--env", "NOVALUE"]).is_err());
        assert!(parse(&["--env", "=x"]).is_err());
    }
}
