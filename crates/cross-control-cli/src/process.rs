//! Checking on and stopping the daemon process, per platform.

/// Whether a process with this PID is running.
#[cfg(unix)]
pub fn is_alive(pid: u32) -> bool {
    // Signal 0 checks existence (and permission) without sending anything.
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\"")))
}

/// Ask the daemon to stop.
///
/// On Unix this sends SIGTERM, which the daemon handles gracefully. Windows
/// cannot deliver Ctrl+C to another console, so there the process is
/// terminated; peers notice the dropped connection and the OS removes its
/// input hooks.
#[cfg(unix)]
pub fn terminate(pid: u32) -> anyhow::Result<()> {
    let status = std::process::Command::new("kill")
        .args(["-s", "TERM", &pid.to_string()])
        .status()?;
    anyhow::ensure!(status.success(), "failed to send SIGTERM to PID {pid}");
    Ok(())
}

#[cfg(windows)]
pub fn terminate(pid: u32) -> anyhow::Result<()> {
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(std::process::Stdio::null())
        .status()?;
    anyhow::ensure!(status.success(), "failed to stop PID {pid}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive() {
        assert!(is_alive(std::process::id()));
    }

    #[test]
    fn an_exited_process_is_not() {
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                &["/C", "exit"][..]
            } else {
                &[][..]
            })
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!is_alive(pid));
    }
}
