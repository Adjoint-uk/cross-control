//! Daemon configuration loaded from TOML.

use std::collections::{HashMap, HashSet};

use cross_control_types::screen::{DisplayLayout, Position, ScreenGeometry};
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
    /// This machine's monitors. When empty, the daemon falls back to a
    /// single monitor sized by `daemon.screen_width`/`screen_height`.
    #[serde(default)]
    pub monitors: Vec<MonitorConfig>,
}

impl Config {
    /// This machine's display layout. Uses the explicit `[[monitors]]` list
    /// when present, otherwise a single monitor sized by the legacy
    /// `daemon.screen_width`/`screen_height` fields.
    #[must_use]
    pub fn display_layout(&self) -> DisplayLayout {
        if self.monitors.is_empty() {
            DisplayLayout::single(self.daemon.screen_width, self.daemon.screen_height)
        } else {
            DisplayLayout::new(
                self.monitors
                    .iter()
                    .map(|m| ScreenGeometry {
                        width: m.width,
                        height: m.height,
                        x: m.x,
                        y: m.y,
                    })
                    .collect(),
            )
        }
    }

    /// Validate the screen layout before the daemon builds its adjacency map.
    ///
    /// The daemon derives cursor routing from `[[screens]]` and
    /// `[[screen_adjacency]]`; a typo (a dangling neighbor name, two screens
    /// on the same edge) would otherwise silently misroute the cursor. This
    /// turns those into a clear startup error instead.
    pub fn validate(&self) -> Result<(), String> {
        let me = self.identity.name.as_str();

        // Every declared monitor must have a non-zero size, or edge detection
        // against the combined desktop breaks.
        for (i, m) in self.monitors.iter().enumerate() {
            if m.width == 0 || m.height == 0 {
                return Err(format!(
                    "monitor #{i} has a zero dimension ({}x{}); width and height must be > 0",
                    m.width, m.height
                ));
            }
        }

        // Direct neighbors: names must be present, unique, and distinct from
        // this machine.
        let mut names = HashSet::new();
        for sc in &self.screens {
            if sc.name.trim().is_empty() {
                return Err("a [[screens]] entry has an empty name".to_string());
            }
            if sc.name == me {
                return Err(format!(
                    "screen \"{}\" shares this machine's identity.name; remote screens need distinct names",
                    sc.name
                ));
            }
            if !names.insert(sc.name.as_str()) {
                return Err(format!("duplicate screen name \"{}\"", sc.name));
            }
        }

        // Each local edge can hold at most one direct neighbor.
        let mut local_edges: HashMap<_, &str> = HashMap::new();
        for sc in &self.screens {
            let edge = sc.position.local_edge();
            if let Some(other) = local_edges.insert(edge, sc.name.as_str()) {
                return Err(format!(
                    "screens \"{other}\" and \"{}\" are both on this machine's {edge:?} edge; \
                     each edge holds at most one neighbor",
                    sc.name
                ));
            }
        }

        // Adjacency edges must not be self-loops, and must not give one
        // screen two different neighbors on the same edge.
        //
        // Names in `[[screen_adjacency]]` need NOT appear in `[[screens]]`:
        // that block exists precisely to introduce screens more than one hop
        // away, which are not this machine's direct neighbors.
        let mut adjacency_edges = HashSet::new();
        for adj in &self.screen_adjacency {
            if adj.screen == adj.neighbor {
                return Err(format!(
                    "[[screen_adjacency]] links screen \"{}\" to itself",
                    adj.screen
                ));
            }
            let edge = adj.position.local_edge();
            if !adjacency_edges.insert((adj.screen.as_str(), edge)) {
                return Err(format!(
                    "[[screen_adjacency]] gives screen \"{}\" two neighbors on its {edge:?} edge",
                    adj.screen
                ));
            }
        }

        // Every screen named in the adjacency graph must connect back to this
        // machine through some chain of edges — otherwise the block is dead
        // (a typo, or an island the cursor can never reach). Inverse edges are
        // implicit, so reachability is undirected. Start from this machine and
        // its direct neighbors, then expand to a fixpoint.
        let mut reachable: HashSet<&str> = HashSet::new();
        reachable.insert(me);
        for sc in &self.screens {
            reachable.insert(sc.name.as_str());
        }
        loop {
            let mut changed = false;
            for adj in &self.screen_adjacency {
                let s = reachable.contains(adj.screen.as_str());
                let n = reachable.contains(adj.neighbor.as_str());
                if s ^ n {
                    reachable.insert(adj.screen.as_str());
                    reachable.insert(adj.neighbor.as_str());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for adj in &self.screen_adjacency {
            for name in [&adj.screen, &adj.neighbor] {
                if !reachable.contains(name.as_str()) {
                    return Err(format!(
                        "[[screen_adjacency]] screen \"{name}\" is not connected to this \
                         machine \"{me}\" through any chain of screens"
                    ));
                }
            }
        }

        Ok(())
    }
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

/// One monitor attached to this machine. Position is given by the top-left
/// `x`/`y` offset within the machine's combined desktop; both default to 0
/// for a single-monitor setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
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

    /// Build a screen with a name and position, no address.
    fn screen(name: &str, position: Position) -> ScreenConfig {
        ScreenConfig {
            name: name.to_string(),
            address: None,
            position,
            fingerprint: None,
        }
    }

    #[test]
    fn valid_multi_hop_layout_passes() {
        // desk: [center] -- right --> [right] -- right --> [far-right]
        let toml_str = r#"
[identity]
name = "center"

[[screens]]
name = "right"
position = "Right"

[[screen_adjacency]]
screen = "right"
neighbor = "far-right"
position = "Right"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn default_config_validates() {
        assert!(Config::default().validate().is_ok());
    }

    #[test]
    fn duplicate_screen_name_rejected() {
        let mut config = Config::default();
        config.identity.name = "center".to_string();
        config.screens = vec![
            screen("laptop", Position::Left),
            screen("laptop", Position::Right),
        ];
        let err = config.validate().unwrap_err();
        assert!(err.contains("duplicate screen name"), "{err}");
    }

    #[test]
    fn two_screens_on_same_edge_rejected() {
        let mut config = Config::default();
        config.identity.name = "center".to_string();
        config.screens = vec![screen("a", Position::Right), screen("b", Position::Right)];
        let err = config.validate().unwrap_err();
        assert!(err.contains("Right edge"), "{err}");
    }

    #[test]
    fn screen_named_like_self_rejected() {
        let mut config = Config::default();
        config.identity.name = "center".to_string();
        config.screens = vec![screen("center", Position::Left)];
        let err = config.validate().unwrap_err();
        assert!(err.contains("identity.name"), "{err}");
    }

    #[test]
    fn adjacency_reachable_through_chain_passes() {
        // center -[right]- right -[right]- far  (far is 2 hops away and is
        // introduced only by the adjacency entry, not by [[screens]]).
        let mut config = Config::default();
        config.identity.name = "center".to_string();
        config.screens = vec![screen("right", Position::Right)];
        config.screen_adjacency = vec![ScreenAdjacency {
            screen: "right".to_string(),
            neighbor: "far".to_string(),
            position: Position::Right,
        }];
        assert!(config.validate().is_ok(), "{:?}", config.validate());
    }

    #[test]
    fn orphaned_adjacency_block_rejected() {
        // An adjacency edge between two screens that never connect back to
        // this machine — a dead block, usually a typo.
        let mut config = Config::default();
        config.identity.name = "center".to_string();
        config.screens = vec![screen("right", Position::Right)];
        config.screen_adjacency = vec![ScreenAdjacency {
            screen: "island-a".to_string(),
            neighbor: "island-b".to_string(),
            position: Position::Right,
        }];
        let err = config.validate().unwrap_err();
        assert!(err.contains("not connected"), "{err}");
    }

    #[test]
    fn self_loop_adjacency_rejected() {
        let mut config = Config::default();
        config.identity.name = "center".to_string();
        config.screens = vec![screen("right", Position::Right)];
        config.screen_adjacency = vec![ScreenAdjacency {
            screen: "right".to_string(),
            neighbor: "right".to_string(),
            position: Position::Right,
        }];
        let err = config.validate().unwrap_err();
        assert!(err.contains("itself"), "{err}");
    }

    #[test]
    fn display_layout_falls_back_to_screen_dimensions() {
        let mut config = Config::default();
        config.daemon.screen_width = 2560;
        config.daemon.screen_height = 1440;
        let bb = config.display_layout().bounding_box();
        assert_eq!((bb.width, bb.height), (2560, 1440));
    }

    #[test]
    fn display_layout_uses_monitors_when_present() {
        let config = Config {
            monitors: vec![
                MonitorConfig {
                    width: 1920,
                    height: 1080,
                    x: 0,
                    y: 0,
                },
                MonitorConfig {
                    width: 1920,
                    height: 1080,
                    x: 1920,
                    y: 0,
                },
            ],
            ..Config::default()
        };
        let layout = config.display_layout();
        assert_eq!(layout.monitors.len(), 2);
        // Combined desktop is 3840 wide.
        assert_eq!(layout.bounding_box().width, 3840);
    }

    #[test]
    fn zero_size_monitor_rejected() {
        let config = Config {
            monitors: vec![MonitorConfig {
                width: 1920,
                height: 0,
                x: 0,
                y: 0,
            }],
            ..Config::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("zero dimension"), "{err}");
    }

    #[test]
    fn conflicting_adjacency_edges_rejected() {
        // right's Right edge is given two different neighbors.
        let mut config = Config::default();
        config.identity.name = "center".to_string();
        config.screens = vec![screen("right", Position::Right)];
        config.screen_adjacency = vec![
            ScreenAdjacency {
                screen: "right".to_string(),
                neighbor: "far-a".to_string(),
                position: Position::Right,
            },
            ScreenAdjacency {
                screen: "right".to_string(),
                neighbor: "far-b".to_string(),
                position: Position::Right,
            },
        ];
        let err = config.validate().unwrap_err();
        assert!(err.contains("two neighbors on its Right edge"), "{err}");
    }
}
