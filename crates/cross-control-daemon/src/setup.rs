//! Config loading, cert generation, and machine identity.

use std::path::{Path, PathBuf};

use cross_control_certgen::GeneratedCert;
use cross_control_types::MachineId;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::error::DaemonError;

/// Load configuration from the given path, or the default location.
///
/// A missing file at the default location means "use defaults"; a missing
/// file at an explicitly given path is an error, so a typo in a service
/// definition cannot silently start the daemon with an empty layout.
pub fn load_config(path: Option<&str>) -> Result<Config, DaemonError> {
    let config_path = match path {
        Some(p) => PathBuf::from(p),
        None => default_config_path(),
    };
    if path.is_some() && !config_path.exists() {
        return Err(DaemonError::Config(format!(
            "config file {} not found",
            config_path.display()
        )));
    }

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| DaemonError::Config(format!("failed to read config: {e}")))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| DaemonError::Config(format!("failed to parse config: {e}")))?;
        config
            .validate()
            .map_err(|e| DaemonError::Config(format!("invalid config: {e}")))?;
        info!(path = %config_path.display(), "loaded config");
        Ok(config)
    } else {
        info!("no config file found, using defaults");
        Ok(Config::default())
    }
}

/// This machine's TLS certificate and key (PEM): the files named by
/// `identity.cert` / `identity.key` when set — which must then exist — or
/// else the pair in `config_dir`, generated on first use.
pub fn load_identity_certs(
    config: &Config,
    config_dir: &Path,
) -> Result<(String, String), DaemonError> {
    match (&config.identity.cert, &config.identity.key) {
        (Some(cert_path), Some(key_path)) => {
            restrict_to_owner(key_path);
            let read = |path: &Path, what: &str| {
                std::fs::read_to_string(path).map_err(|e| {
                    DaemonError::Config(format!("failed to read {what} {}: {e}", path.display()))
                })
            };
            let cert_pem = read(cert_path, "identity.cert")?;
            let key_pem = read(key_path, "identity.key")?;
            info!(path = %cert_path.display(), "loaded configured TLS cert");
            Ok((cert_pem, key_pem))
        }
        (None, None) => load_or_generate_certs(config_dir),
        _ => Err(DaemonError::Config(
            "identity.cert and identity.key must be set together".to_string(),
        )),
    }
}

/// Where this machine's certificate lives (see [`load_identity_certs`]).
#[must_use]
pub fn identity_cert_path(config: &Config, config_dir: &Path) -> PathBuf {
    config
        .identity
        .cert
        .clone()
        .unwrap_or_else(|| config_dir.join("cross-control.crt"))
}

/// Load TLS cert and key from the config directory, or generate if missing.
pub fn load_or_generate_certs(config_dir: &Path) -> Result<(String, String), DaemonError> {
    let cert_path = config_dir.join("cross-control.crt");
    let key_path = config_dir.join("cross-control.key");

    if cert_path.exists() && key_path.exists() {
        restrict_to_owner(&key_path);
        let cert_pem = std::fs::read_to_string(&cert_path)
            .map_err(|e| DaemonError::Config(format!("failed to read cert: {e}")))?;
        let key_pem = std::fs::read_to_string(&key_path)
            .map_err(|e| DaemonError::Config(format!("failed to read key: {e}")))?;
        info!(path = %cert_path.display(), "loaded existing TLS cert");
        Ok((cert_pem, key_pem))
    } else {
        create_private_dir(config_dir)
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

        write_private_file(&key_path, key_pem.as_bytes())
            .map_err(|e| DaemonError::Config(format!("failed to write key: {e}")))?;
        write_private_file(&cert_path, cert_pem.as_bytes())
            .map_err(|e| DaemonError::Config(format!("failed to write cert: {e}")))?;

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
        create_private_dir(config_dir)
            .map_err(|e| DaemonError::Config(format!("failed to create config dir: {e}")))?;

        let id = MachineId::new();
        write_private_file(&id_path, id.as_uuid().to_string().as_bytes())
            .map_err(|e| DaemonError::Config(format!("failed to write machine-id: {e}")))?;

        info!(id = %id, "created new machine ID");
        Ok(id)
    }
}

/// Overrides [`config_dir`] when set (any platform).
pub const CONFIG_DIR_ENV: &str = "CROSS_CONTROL_CONFIG_DIR";
/// Overrides [`runtime_dir`] when set (any platform).
pub const RUNTIME_DIR_ENV: &str = "CROSS_CONTROL_RUNTIME_DIR";

fn dir_from_env(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The configuration directory: `$CROSS_CONTROL_CONFIG_DIR` if set, else
/// `cross-control` in the platform's configuration directory
/// (`~/.config` on Linux, `%APPDATA%` on Windows).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = dir_from_env(CONFIG_DIR_ENV) {
        return dir;
    }
    dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cross-control")
}

/// Directory for the PID and status files: `$CROSS_CONTROL_RUNTIME_DIR` if
/// set, else the per-user runtime directory
/// (`$XDG_RUNTIME_DIR`), else the per-user state directory, else the config
/// directory. Never a shared directory such as `/tmp`, where another user
/// could plant or read the files.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = dir_from_env(RUNTIME_DIR_ENV) {
        return dir;
    }
    dirs::runtime_dir()
        .or_else(|| dirs::state_dir().map(|dir| dir.join("cross-control")))
        .unwrap_or_else(config_dir)
}

/// Create `dir` (and parents) if missing; the directory itself is made
/// accessible to its owner only.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Replace `path` with `contents` atomically (temporary file, then rename),
/// readable and writable by the owner only. Creates the parent directory.
pub fn write_private_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        create_private_dir(dir)?;
    }
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

/// Tighten a secret file that is readable by others (e.g. a key written by
/// an older version) to owner-only.
fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.permissions().mode() & 0o077 != 0 {
                tracing::warn!(path = %path.display(), "private key was readable by others; restricting to owner");
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Get the default config file path.
#[must_use]
pub fn default_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Get the PID file path.
pub fn pid_file_path() -> PathBuf {
    runtime_dir().join("cross-control.pid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cc-setup-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn private_file_is_written_atomically_and_owner_only() {
        let dir = scratch();
        let path = dir.join("nested/secret");
        write_private_file(&path, b"one").unwrap();
        write_private_file(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(!dir.join("nested/secret.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn generated_key_is_private_and_loose_key_is_tightened() {
        let dir = scratch();
        let (_, key) = load_or_generate_certs(&dir).unwrap();
        assert!(key.contains("PRIVATE KEY"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_path = dir.join("cross-control.key");
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&key_path), 0o600);
            // A key left world-readable (older versions) is fixed on load.
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).unwrap();
            load_or_generate_certs(&dir).unwrap();
            assert_eq!(mode(&key_path), 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn runtime_files_never_fall_back_to_tmp() {
        assert!(!pid_file_path().starts_with("/tmp"));
        assert!(!crate::status::status_file_path().starts_with("/tmp"));
    }
}
