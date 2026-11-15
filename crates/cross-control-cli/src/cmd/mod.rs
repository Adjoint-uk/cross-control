//! One module per command group.

pub mod config;
pub mod daemon;
pub mod doctor;
pub mod identity;
pub mod pair;
pub mod service;
pub mod trust;

use std::path::PathBuf;

use cross_control_daemon::{setup, Config, TrustStore};

use crate::Ctx;

impl Ctx {
    /// The configuration, from `--config` or the default location.
    pub fn config(&self) -> anyhow::Result<Config> {
        let path = self.config_path.as_ref().map(|p| p.to_string_lossy());
        Ok(setup::load_config(path.as_deref())?)
    }

    /// The configuration file in use (which may not exist yet).
    pub fn config_file(&self) -> PathBuf {
        self.config_path
            .clone()
            .unwrap_or_else(setup::default_config_path)
    }

    /// Print a JSON document on stdout.
    pub fn print_json(value: &serde_json::Value) {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    }
}

/// This machine's trust store, as configured.
pub fn trust_store(config: &Config) -> TrustStore {
    TrustStore::from_config(config, setup::config_dir().join("known_peers"))
}

/// This machine's name and certificate fingerprint (display form), creating
/// the certificate on first use.
pub fn local_identity(config: &Config) -> anyhow::Result<(String, String)> {
    let (cert_pem, _) = setup::load_identity_certs(config, &setup::config_dir())?;
    let fingerprint = cross_control_certgen::fingerprint_from_pem(&cert_pem)?;
    Ok((config.identity.name.clone(), fingerprint))
}
