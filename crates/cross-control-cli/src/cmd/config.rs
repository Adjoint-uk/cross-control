//! `config check`.

use std::path::PathBuf;

use cross_control_daemon::setup;
use serde_json::json;

use crate::exit;
use crate::Ctx;

/// Validate a configuration file. An invalid or missing file is an error
/// with exit code 4, so it can gate a deployment (e.g. Ansible's
/// `validate:`) before the daemon is restarted with it.
pub fn check(ctx: &Ctx, path: Option<PathBuf>) -> anyhow::Result<u8> {
    let path = path.unwrap_or_else(|| ctx.config_file());
    let config = setup::load_config(Some(&path.to_string_lossy()))?;
    if ctx.json {
        Ctx::print_json(&json!({
            "valid": true,
            "path": path,
            "name": config.identity.name,
            "screens": config.screens.len(),
        }));
    } else {
        println!("{}: OK", path.display());
    }
    Ok(exit::OK)
}
