//! Daemon configuration loaded from TOML.

use cross_control_types::screen::Position;
use serde::{Deserialize, Serialize};

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub identity: IdentityConfig,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default)]
    pub clipboard: ClipboardConfig,
    #[serde(default)]
    pub screens: Vec<ScreenConfig>,
    #[serde(default)]
    pub screen_adjacency: Vec<ScreenAdjacency>,
}

/// An adjacency edge between two screens in the full screen graph.
///
/// Used by the server to know where to route the cursor when it leaves
/// a remote screen (multi-hop navigation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenAdjacency {
    /// The screen the cursor is leaving.
    pub screen: String,
    /// The neighboring screen in the given direction.
    pub neighbor: String,
    /// The position of `neighbor` relative to `screen`.
    pub position: Position,
}

/// Daemon network and runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_true")]
    pub discovery: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_screen_width")]
    pub screen_width: u32,
    #[serde(default = "default_screen_height")]
    pub screen_height: u32,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            bind: default_bind(),
            discovery: true,
            log_level: default_log_level(),
            screen_width: default_screen_width(),
            screen_height: default_screen_height(),
        }
    }
}

/// Machine identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityConfig {
    #[serde(default = "default_name")]
    pub name: String,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
        }
    }
}

/// Input subsystem settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    #[serde(default = "default_release_hotkey")]
    pub release_hotkey: Vec<String>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            release_hotkey: default_release_hotkey(),
        }
    }
}

/// Clipboard subsystem settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_clipboard_size")]
    pub max_size: usize,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_size: default_max_clipboard_size(),
        }
    }
}

/// A remote screen definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenConfig {
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    pub position: Position,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

fn default_port() -> u16 {
    24800
}

fn default_bind() -> String {
    "0.0.0.0".to_string()
}

fn default_true() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "cross-control".to_string())
}

fn default_release_hotkey() -> Vec<String> {
    vec![
        "LeftCtrl".to_string(),
        "LeftShift".to_string(),
        "Escape".to_string(),
    ]
}

fn default_max_clipboard_size() -> usize {
    10 * 1024 * 1024 // 10 MiB
}

fn default_screen_width() -> u32 {
    1920
}

fn default_screen_height() -> u32 {
    1080
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_serializes() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("port = 24800"));
    }

    #[test]
    fn parse_example_config() {
        let toml_str = r#"
[daemon]
port = 24800
bind = "0.0.0.0"
discovery = true
log_level = "info"

[identity]
name = "workstation-left"

[input]
release_hotkey = ["LeftCtrl", "LeftShift", "Escape"]

[clipboard]
enabled = true
max_size = 10485760

[[screens]]
name = "laptop-right"
address = "192.168.1.42"
position = "Right"
fingerprint = "SHA256:abc123"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.daemon.port, 24800);
        assert_eq!(config.identity.name, "workstation-left");
        assert_eq!(config.screens.len(), 1);
        assert_eq!(config.screens[0].name, "laptop-right");
        assert_eq!(config.screens[0].position, Position::Right);
    }
}
