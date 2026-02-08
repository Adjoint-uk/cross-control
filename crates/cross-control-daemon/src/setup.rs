//! Config loading, cert generation, and machine identity.

use std::path::{Path, PathBuf};

use cross_control_certgen::GeneratedCert;
use cross_control_types::MachineId;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::error::DaemonError;

/// Load configuration from the given path, or the default location.
pub fn load_config(path: Option<&str>) -> Result<Config, DaemonError> {
    let config_path = match path {
        Some(p) => PathBuf::from(p),
        None => default_config_path(),
    };

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| DaemonError::Config(format!("failed to read config: {e}")))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| DaemonError::Config(format!("failed to parse config: {e}")))?;
        info!(path = %config_path.display(), "loaded config");
        Ok(config)
    } else {
        info!("no config file found, using defaults");
        Ok(Config::default())
    }
}

/// Load TLS cert and key from the config directory, or generate if missing.
pub fn load_or_generate_certs(config_dir: &Path) -> Result<(String, String), DaemonError> {
    let cert_path = config_dir.join("cross-control.crt");
    let key_path = config_dir.join("cross-control.key");

    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path)
            .map_err(|e| DaemonError::Config(format!("failed to read cert: {e}")))?;
        let key_pem = std::fs::read_to_string(&key_path)
            .map_err(|e| DaemonError::Config(format!("failed to read key: {e}")))?;
        info!(path = %cert_path.display(), "loaded existing TLS cert");
        Ok((cert_pem, key_pem))
    } else {
        std::fs::create_dir_all(config_dir)
            .map_err(|e| DaemonError::Config(format!("failed to create config dir: {e}")))?;

        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "cross-control".to_string());

        let GeneratedCert {
            cert_pem,
            key_pem,
            fingerprint,
        } = cross_control_certgen::generate_certificate(&hostname)
            .map_err(|e| DaemonError::Config(format!("failed to generate cert: {e}")))?;

        std::fs::write(&cert_path, &cert_pem)
            .map_err(|e| DaemonError::Config(format!("failed to write cert: {e}")))?;
        std::fs::write(&key_path, &key_pem)
            .map_err(|e| DaemonError::Config(format!("failed to write key: {e}")))?;

        info!(fingerprint = %fingerprint, "generated new TLS cert");
        Ok((cert_pem, key_pem))
    }
}

/// Load or create a persistent machine ID.
pub fn load_or_create_machine_id(config_dir: &Path) -> Result<MachineId, DaemonError> {
    let id_path = config_dir.join("machine-id");

    if id_path.exists() {
        let content = std::fs::read_to_string(&id_path)
            .map_err(|e| DaemonError::Config(format!("failed to read machine-id: {e}")))?;
        let uuid: Uuid = content
            .trim()
            .parse()
            .map_err(|e| DaemonError::Config(format!("invalid machine-id: {e}")))?;
        info!(id = %uuid, "loaded machine ID");
        Ok(MachineId::from_uuid(uuid))
    } else {
        std::fs::create_dir_all(config_dir)
            .map_err(|e| DaemonError::Config(format!("failed to create config dir: {e}")))?;

        let id = MachineId::new();
        std::fs::write(&id_path, id.as_uuid().to_string())
            .map_err(|e| DaemonError::Config(format!("failed to write machine-id: {e}")))?;

        info!(id = %id, "created new machine ID");
        Ok(id)
    }
}

/// Get the default config directory path.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("cross-control")
}

/// Get the default config file path.
fn default_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Get the PID file path.
pub fn pid_file_path() -> PathBuf {
    dirs::runtime_dir()
        .or_else(dirs::state_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("cross-control.pid")
}
