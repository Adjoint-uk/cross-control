//! Checking on and stopping the daemon process, per platform.

/// Whether a process with this PID is running.
#[cfg(unix)]
pub fn is_alive(pid: u32) -> bool {
    // A zombie has exited; only its parent has not collected it yet.
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // The state follows the parenthesised command name.
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.trim_start().chars().next());
        return state.is_some_and(|s| s != 'Z' && s != 'X');
    }
    alive_via_ps(pid)
}

/// Without procfs (e.g. macOS): ask ps for the process state. It fails when
/// there is no such process; a "Z" state is a zombie.
#[cfg(unix)]
fn alive_via_ps(pid: u32) -> bool {
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|out| {
            out.status.success() && {
                let state = String::from_utf8_lossy(&out.stdout);
                let state = state.trim();
                !state.is_empty() && !state.starts_with('Z')
            }
        })
}

#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\"")))
}

/// Stop the daemon process directly: the fallback for when it does not
/// answer on the control channel.
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

    #[cfg(unix)]
    #[test]
    fn ps_fallback_sees_live_processes_and_not_zombies() {
        assert!(alive_via_ps(std::process::id()));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        // Exited but not yet reaped: a zombie.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!alive_via_ps(pid), "a zombie counted as alive");
        child.wait().unwrap();
        assert!(!alive_via_ps(pid));
    }
}
