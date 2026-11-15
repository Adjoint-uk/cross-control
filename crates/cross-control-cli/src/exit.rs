//! Exit codes, so scripts and configuration management can tell outcomes
//! apart without parsing messages. Keep in sync with `EXIT_CODES_HELP`.

use cross_control_daemon::DaemonError;

pub const OK: u8 = 0;
pub const ERROR: u8 = 1;
pub const NOT_RUNNING: u8 = 3;
pub const CONFIG: u8 = 4;
pub const UNAUTHORIZED: u8 = 5;

/// The daemon is not running (for `stop`).
#[derive(Debug)]
pub struct NotRunning;

impl std::fmt::Display for NotRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the daemon is not running")
    }
}

impl std::error::Error for NotRunning {}

/// The exit code for an error, from the first recognised cause.
pub fn code_for(err: &anyhow::Error) -> u8 {
    for cause in err.chain() {
        if cause.is::<NotRunning>() {
            return NOT_RUNNING;
        }
        match cause.downcast_ref::<DaemonError>() {
            Some(DaemonError::Config(_)) => return CONFIG,
            Some(DaemonError::Untrusted(_)) => return UNAUTHORIZED,
            _ => {}
        }
    }
    ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_documented_codes() {
        assert_eq!(code_for(&anyhow::Error::new(NotRunning)), NOT_RUNNING);
        let config = anyhow::Error::new(DaemonError::Config("bad".into()));
        assert_eq!(code_for(&config), CONFIG);
        // Context added on top does not hide the cause.
        assert_eq!(code_for(&config.context("while starting")), CONFIG);
        assert_eq!(code_for(&anyhow::anyhow!("anything else")), ERROR);
    }
}
