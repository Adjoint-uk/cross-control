//! `doctor`: check that this machine is ready to run cross-control, and say
//! how to fix what is not. Meant for first-time setup and bring-up on real
//! hardware, where device permissions and layout/trust mismatches are the
//! usual problems.

use std::net::UdpSocket;
use std::path::Path;

use cross_control_daemon::config::{Config, InputBackend};
use cross_control_daemon::setup;
use serde::Serialize;
use serde_json::json;

use crate::exit;
use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Ok,
            detail: detail.into(),
            fix: None,
        }
    }
    fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Warn,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Fail,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

/// Run every check. Exit code 1 if any check failed (warnings pass).
pub fn run(ctx: &Ctx) -> u8 {
    let mut checks = Vec::new();

    let config = match ctx.config() {
        Ok(config) => {
            checks.push(config_check(ctx, &config));
            Some(config)
        }
        Err(e) => {
            checks.push(Check::fail(
                "config",
                format!("{e:#}"),
                "fix the file, then re-check with `cross-control config check`",
            ));
            None
        }
    };

    if let Some(config) = &config {
        checks.push(identity_check(config));
        checks.extend(trust_checks(config));
        checks.extend(input_checks(config.input.backend));
        checks.push(port_check(config));
    }
    checks.push(clipboard_check());
    checks.push(session_check());

    let failed = checks.iter().any(|c| c.status == Status::Fail);
    if ctx.json {
        Ctx::print_json(&json!({ "ok": !failed, "checks": checks }));
    } else {
        for check in &checks {
            let mark = match check.status {
                Status::Ok => "ok  ",
                Status::Warn => "warn",
                Status::Fail => "FAIL",
            };
            println!("[{mark}] {:<10} {}", check.name, check.detail);
            if let Some(fix) = &check.fix {
                for line in fix.lines() {
                    println!("{:18}{line}", "");
                }
            }
        }
        println!();
        if failed {
            println!("Some checks failed; fix those before starting the daemon.");
        } else {
            println!("Ready.");
        }
    }
    if failed {
        exit::ERROR
    } else {
        exit::OK
    }
}

fn config_check(ctx: &Ctx, config: &Config) -> Check {
    let path = ctx.config_file();
    if !path.exists() {
        return Check::warn(
            "config",
            format!("no config file at {} (using defaults)", path.display()),
            "create it with your [identity] name and [[screens]] layout; see examples/config.toml",
        );
    }
    if config.screens.is_empty() {
        return Check::warn(
            "config",
            format!(
                "{}: no [[screens]] — the cursor has nowhere to go",
                path.display()
            ),
            "add a [[screens]] entry for each neighbouring machine",
        );
    }
    Check::ok(
        "config",
        format!(
            "{} — \"{}\", {} neighbour(s)",
            path.display(),
            config.identity.name,
            config.screens.len()
        ),
    )
}

fn identity_check(config: &Config) -> Check {
    let cert = setup::identity_cert_path(config, &setup::config_dir());
    match std::fs::read_to_string(&cert)
        .ok()
        .and_then(|pem| cross_control_certgen::fingerprint_from_pem(&pem).ok())
    {
        Some(fp) => Check::ok("identity", fp),
        None if config.identity.cert.is_some() => Check::fail(
            "identity",
            format!(
                "configured certificate {} is missing or invalid",
                cert.display()
            ),
            "check identity.cert / identity.key in the config",
        ),
        None => Check::warn(
            "identity",
            "not created yet",
            "run `cross-control identity` (also created on first start)",
        ),
    }
}

fn trust_checks(config: &Config) -> Vec<Check> {
    let store = super::trust_store(config);
    let entries = match store.entries() {
        Ok(entries) => entries,
        Err(e) => {
            return vec![Check::fail(
                "trust",
                format!("{e}"),
                format!("check {}", store.known_peers_path().display()),
            )]
        }
    };
    let mut checks = Vec::new();
    if config.security.trust_on_first_use {
        checks.push(Check::warn(
            "trust",
            "trust_on_first_use is on: unknown machines are trusted automatically",
            "turn it off once machines are paired",
        ));
    } else if entries.is_empty() {
        checks.push(Check::warn(
            "trust",
            "no trusted machines: every peer will be refused",
            "`cross-control pair <address>` or `cross-control trust add <name> <fingerprint>`",
        ));
    } else {
        checks.push(Check::ok(
            "trust",
            format!("{} trusted machine(s)", entries.len()),
        ));
    }
    if !config.security.trust_on_first_use {
        for screen in &config.screens {
            if !entries.iter().any(|e| e.name == screen.name) {
                checks.push(Check::warn(
                    "trust",
                    format!(
                        "neighbour \"{}\" is in the layout but not trusted",
                        screen.name
                    ),
                    format!(
                        "on \"{}\" run `cross-control identity`, then here: \
                         `cross-control trust add \"{}\" <fingerprint>`",
                        screen.name, screen.name
                    ),
                ));
            }
        }
    }
    checks
}

fn input_checks(backend: InputBackend) -> Vec<Check> {
    if backend == InputBackend::None {
        return vec![Check::warn(
            "input",
            "backend is \"none\": keyboard and mouse are neither shared nor received",
            "set [input] backend = \"auto\" on machines with a keyboard and mouse",
        )];
    }
    native_input_checks()
}

#[cfg(all(feature = "linux", target_os = "linux"))]
fn native_input_checks() -> Vec<Check> {
    use cross_control_input::linux::capture::EvdevCapture;
    use cross_control_types::DeviceCapability;

    let mut checks = Vec::new();
    let group_fix = "sudo usermod -aG input $USER   (then log out and back in)";

    // Readable event devices.
    let events: Vec<_> = std::fs::read_dir("/dev/input")
        .map(|dir| {
            dir.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("event"))
                })
                .collect()
        })
        .unwrap_or_default();
    let readable = events
        .iter()
        .filter(|p| std::fs::File::open(p).is_ok())
        .count();
    if events.is_empty() {
        checks.push(Check::fail(
            "devices",
            "no /dev/input/event* devices",
            "this machine has no input devices visible (container or VM without passthrough?)",
        ));
    } else if readable == 0 {
        checks.push(Check::fail(
            "devices",
            format!("{} input devices, none readable by this user", events.len()),
            group_fix,
        ));
    } else {
        let found = EvdevCapture::enumerate_devices();
        let keyboards = found
            .iter()
            .filter(|(_, d)| d.capabilities.contains(&DeviceCapability::Keyboard))
            .count();
        let mice = found
            .iter()
            .filter(|(_, d)| d.capabilities.contains(&DeviceCapability::RelativeMouse))
            .count();
        let names: Vec<_> = found.iter().map(|(_, d)| d.name.clone()).collect();
        let detail = format!(
            "{keyboards} keyboard(s), {mice} mouse/mice ({readable}/{} devices readable): {}",
            events.len(),
            names.join(", ")
        );
        if keyboards == 0 || mice == 0 {
            checks.push(Check::warn(
                "devices",
                detail,
                if readable < events.len() {
                    group_fix
                } else {
                    "connect a keyboard and mouse"
                },
            ));
        } else {
            checks.push(Check::ok("devices", detail));
        }
    }

    // uinput, for receiving input from peers.
    let uinput = Path::new("/dev/uinput");
    let uinput_fix = "sudo modprobe uinput\n\
         echo 'KERNEL==\"uinput\", MODE=\"0660\", GROUP=\"input\"' | sudo tee /etc/udev/rules.d/99-uinput.rules\n\
         sudo udevadm control --reload-rules && sudo udevadm trigger";
    if !uinput.exists() {
        checks.push(Check::fail("uinput", "/dev/uinput not found", uinput_fix));
    } else if std::fs::OpenOptions::new()
        .write(true)
        .open(uinput)
        .is_err()
    {
        checks.push(Check::fail(
            "uinput",
            "/dev/uinput is not writable by this user",
            uinput_fix,
        ));
    } else {
        checks.push(Check::ok("uinput", "/dev/uinput writable"));
    }
    checks
}

#[cfg(not(all(feature = "linux", target_os = "linux")))]
fn native_input_checks() -> Vec<Check> {
    vec![Check::fail(
        "input",
        "no native input backend for this platform yet",
        "set [input] backend = \"none\" to run headless",
    )]
}

fn port_check(config: &Config) -> Check {
    let addr = format!("{}:{}", config.daemon.bind, config.daemon.port);
    match UdpSocket::bind(&addr) {
        Ok(_) => Check::ok("port", format!("UDP {addr} is free")),
        Err(_) if daemon_running() => Check::ok(
            "port",
            format!("UDP {addr} is in use by the running daemon"),
        ),
        Err(e) => Check::fail(
            "port",
            format!("cannot bind UDP {addr}: {e}"),
            "another program uses the port; set [daemon] port to another value",
        ),
    }
}

fn daemon_running() -> bool {
    std::fs::read_to_string(setup::pid_file_path())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

fn clipboard_check() -> Check {
    match cross_control_clipboard::ArboardClipboard::new() {
        Ok(_) => Check::ok("clipboard", "available"),
        Err(e) => Check::warn(
            "clipboard",
            format!("unavailable ({e}): clipboard will not sync"),
            "run inside a graphical session (DISPLAY / WAYLAND_DISPLAY set)",
        ),
    }
}

fn session_check() -> Check {
    let kind = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    match kind.as_str() {
        "wayland" | "x11" => Check::ok("session", kind),
        "" => Check::ok("session", "unknown (XDG_SESSION_TYPE unset)"),
        other => Check::ok("session", other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_backend_is_a_warning_not_a_failure() {
        let checks = input_checks(InputBackend::None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Warn);
    }

    #[test]
    fn untrusted_neighbour_is_flagged() {
        let config: Config = toml::from_str(
            "[identity]\nname = \"a\"\n[security]\nknown_peers = \"/nonexistent/kp\"\n\
             [[screens]]\nname = \"b\"\nposition = \"Right\"\n",
        )
        .unwrap();
        let checks = trust_checks(&config);
        assert!(checks
            .iter()
            .any(|c| c.status == Status::Warn && c.detail.contains("\"b\"")));
    }
}
