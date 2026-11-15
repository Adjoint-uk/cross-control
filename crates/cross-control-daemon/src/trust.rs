//! Which peers this machine trusts.
//!
//! A peer is identified by the SHA-256 fingerprint of its TLS certificate,
//! bound to the name it announces in the handshake. Trust comes from:
//!
//! - **pins in `config.toml`**: a `fingerprint` on a `[[screens]]` entry;
//! - **the `known_peers` file**: one `<fingerprint> <name>` per line, written
//!   by `cross-control trust add`, `cross-control pair`, or the daemon itself
//!   when `security.trust_on_first_use` is enabled.
//!
//! A peer is authorized when its name is trusted with exactly the
//! fingerprint it presented. A known name with a different fingerprint is
//! always refused (the peer was re-keyed, or someone is impersonating it),
//! even with trust-on-first-use on. The TLS layer has already checked that
//! the peer holds the private key for its certificate.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cross_control_protocol::tls::normalize_fingerprint;
use thiserror::Error;
use tracing::info;

use crate::config::Config;

/// Longest peer name accepted, in bytes.
pub const MAX_NAME_LEN: usize = 64;

/// Why a peer was not authorized.
#[derive(Debug, Error)]
pub enum TrustError {
    #[error(
        "peer \"{name}\" presented fingerprint {presented}, but that name is trusted with a \
         different certificate. If the peer was reinstalled, verify the new fingerprint on it \
         and run `cross-control trust add \"{name}\" {presented}`"
    )]
    FingerprintChanged { name: String, presented: String },

    #[error(
        "peer \"{name}\" presented a certificate that is trusted under the name \"{trusted_as}\". \
         If it was renamed, run `cross-control trust add \"{name}\" {fingerprint}`"
    )]
    NameMismatch {
        name: String,
        trusted_as: String,
        fingerprint: String,
    },

    #[error(
        "peer \"{name}\" ({fingerprint}) is not trusted. Verify the fingerprint on that machine \
         (`cross-control status`), then run `cross-control trust add \"{name}\" {fingerprint}`, \
         or enable `security.trust_on_first_use`"
    )]
    Untrusted { name: String, fingerprint: String },

    #[error("peer presented no certificate")]
    NoCertificate,

    #[error(
        "invalid peer name {0:?}: names must be 1-{MAX_NAME_LEN} bytes with no control characters"
    )]
    InvalidName(String),

    #[error("invalid fingerprint {0:?}: expected a SHA-256 fingerprint (64 hex digits)")]
    InvalidFingerprint(String),

    #[error("known_peers file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// How a peer came to be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorized {
    /// Pinned in `config.toml`.
    Pinned,
    /// Listed in the `known_peers` file.
    Known,
    /// Unknown until now; recorded because trust-on-first-use is on.
    Learned,
}

/// One trusted `(name, fingerprint)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustEntry {
    pub name: String,
    /// Display form: `SHA256:aa:bb:…`.
    pub fingerprint: String,
    /// Where the entry comes from.
    pub source: EntrySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    Config,
    KnownPeers,
}

/// The daemon's view of who it trusts. Cheap to clone; clones share the lock
/// that serializes writes to the `known_peers` file.
#[derive(Debug, Clone)]
pub struct TrustStore {
    /// `(name, canonical fingerprint)` pins from `[[screens]]`.
    pins: Vec<(String, String)>,
    known_peers: PathBuf,
    trust_on_first_use: bool,
    write_lock: Arc<Mutex<()>>,
}

impl TrustStore {
    /// Build the trust store described by `config`. The `known_peers` file
    /// lives at `security.known_peers` if set, else `default_known_peers`.
    #[must_use]
    pub fn from_config(config: &Config, default_known_peers: PathBuf) -> Self {
        let pins = config
            .screens
            .iter()
            .filter_map(|sc| {
                let fp = sc.fingerprint.as_deref()?;
                Some((sc.name.clone(), normalize_fingerprint(fp)))
            })
            .collect();
        Self {
            pins,
            known_peers: config
                .security
                .known_peers
                .clone()
                .unwrap_or(default_known_peers),
            trust_on_first_use: config.security.trust_on_first_use,
            write_lock: Arc::default(),
        }
    }

    /// Path of the `known_peers` file.
    #[must_use]
    pub fn known_peers_path(&self) -> &Path {
        &self.known_peers
    }

    /// Every trusted entry: config pins first, then `known_peers`.
    pub fn entries(&self) -> Result<Vec<TrustEntry>, TrustError> {
        let mut out: Vec<TrustEntry> = self
            .pins
            .iter()
            .map(|(name, fp)| TrustEntry {
                name: name.clone(),
                fingerprint: display_fingerprint(fp),
                source: EntrySource::Config,
            })
            .collect();
        out.extend(
            read_known_peers(&self.known_peers)?
                .into_iter()
                .map(|(name, fp)| TrustEntry {
                    name,
                    fingerprint: display_fingerprint(&fp),
                    source: EntrySource::KnownPeers,
                }),
        );
        Ok(out)
    }

    /// The fingerprint `name` is trusted with, to pin at the TLS layer when
    /// dialing it. `None` when the name is unknown, or trusted with more than
    /// one fingerprint (then authorization after the handshake decides).
    pub fn pin_for(&self, name: &str) -> Option<String> {
        let entries = self.entries().ok()?;
        let mut fps = entries
            .iter()
            .filter(|e| e.name == name)
            .map(|e| normalize_fingerprint(&e.fingerprint));
        let first = fps.next()?;
        fps.all(|fp| fp == first)
            .then(|| display_fingerprint(&first))
    }

    /// Decide whether a peer announcing `name` with certificate
    /// `fingerprint` may connect. With trust-on-first-use on, an unknown
    /// peer is recorded in `known_peers` and accepted.
    pub fn authorize(&self, name: &str, fingerprint: &str) -> Result<Authorized, TrustError> {
        validate_name(name)?;
        let fp = parse_fingerprint(fingerprint)?;

        // Hold the lock across read and (possible) write, so two unknown
        // peers learned at once cannot lose each other's entry.
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let known = read_known_peers(&self.known_peers)?;
        let all = self
            .pins
            .iter()
            .map(|(n, f)| (n, f, Authorized::Pinned))
            .chain(known.iter().map(|(n, f)| (n, f, Authorized::Known)));

        let mut name_known = false;
        let mut trusted_as = None;
        for (entry_name, entry_fp, how) in all {
            match (entry_name == name, *entry_fp == fp) {
                (true, true) => return Ok(how),
                (true, false) => name_known = true,
                (false, true) => trusted_as = Some(entry_name.clone()),
                (false, false) => {}
            }
        }

        if name_known {
            return Err(TrustError::FingerprintChanged {
                name: name.to_string(),
                presented: display_fingerprint(&fp),
            });
        }
        if let Some(trusted_as) = trusted_as {
            return Err(TrustError::NameMismatch {
                name: name.to_string(),
                trusted_as,
                fingerprint: display_fingerprint(&fp),
            });
        }
        if self.trust_on_first_use {
            write_entry(&self.known_peers, name, &fp)?;
            info!(peer = %name, fingerprint = %display_fingerprint(&fp), "trusting new peer on first use");
            return Ok(Authorized::Learned);
        }
        Err(TrustError::Untrusted {
            name: name.to_string(),
            fingerprint: display_fingerprint(&fp),
        })
    }
}

/// Trust `name` with `fingerprint` in the `known_peers` file at `path`,
/// replacing any fingerprint previously recorded for that name.
pub fn add_known_peer(path: &Path, name: &str, fingerprint: &str) -> Result<(), TrustError> {
    validate_name(name)?;
    let fp = parse_fingerprint(fingerprint)?;
    write_entry(path, name, &fp)
}

/// Remove every entry for `name` from the `known_peers` file. Returns
/// whether anything was removed.
pub fn remove_known_peer(path: &Path, name: &str) -> Result<bool, TrustError> {
    let mut entries = read_known_peers(path)?;
    let before = entries.len();
    entries.retain(|(n, _)| n != name);
    if entries.len() == before {
        return Ok(false);
    }
    write_known_peers(path, &entries)?;
    Ok(true)
}

fn write_entry(path: &Path, name: &str, fp: &str) -> Result<(), TrustError> {
    let mut entries = read_known_peers(path)?;
    entries.retain(|(n, _)| n != name);
    entries.push((name.to_string(), fp.to_string()));
    write_known_peers(path, &entries)
}

fn validate_name(name: &str) -> Result<(), TrustError> {
    if name.is_empty()
        || name.len() > MAX_NAME_LEN
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(TrustError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// Canonicalize a fingerprint and check it is a SHA-256 digest.
pub fn parse_fingerprint(fp: &str) -> Result<String, TrustError> {
    let canonical = normalize_fingerprint(fp);
    if canonical.len() == 64 && canonical.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(canonical)
    } else {
        Err(TrustError::InvalidFingerprint(fp.to_string()))
    }
}

/// `SHA256:aa:bb:…` from a canonical (compact lowercase hex) fingerprint.
#[must_use]
pub fn display_fingerprint(canonical: &str) -> String {
    let mut out = String::from("SHA256:");
    for (i, pair) in canonical.as_bytes().chunks(2).enumerate() {
        if i > 0 {
            out.push(':');
        }
        let _ = write!(out, "{}", String::from_utf8_lossy(pair));
    }
    out
}

/// Read `(name, canonical fingerprint)` entries. A missing file is empty;
/// malformed lines are skipped.
fn read_known_peers(path: &Path) -> Result<Vec<(String, String)>, TrustError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(TrustError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (fp, name) = line.split_once(char::is_whitespace)?;
            let fp = parse_fingerprint(fp).ok()?;
            let name = name.trim();
            validate_name(name).ok()?;
            Some((name.to_string(), fp))
        })
        .collect())
}

/// Replace the file atomically (write a temporary file, then rename), owner
/// read/write only.
fn write_known_peers(path: &Path, entries: &[(String, String)]) -> Result<(), TrustError> {
    let io_err = |source| TrustError::Io {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io_err)?;
    }
    let mut text = String::from(
        "# cross-control known peers: <fingerprint> <name>\n\
         # Managed by `cross-control trust`; edit with care.\n",
    );
    for (name, fp) in entries {
        let _ = writeln!(text, "{} {name}", display_fingerprint(fp));
    }

    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(io_err)?;
    file.write_all(text.as_bytes()).map_err(io_err)?;
    file.sync_all().map_err(io_err)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(io_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ScreenConfig, SecurityConfig};
    use cross_control_types::screen::Position;

    const FP_A: &str = "SHA256:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa:aa";
    const FP_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn store(dir: &Path, tofu: bool, pins: &[(&str, &str)]) -> TrustStore {
        let config = Config {
            screens: pins
                .iter()
                .map(|(name, fp)| ScreenConfig {
                    name: (*name).to_string(),
                    address: None,
                    position: Position::Right,
                    fingerprint: Some((*fp).to_string()),
                })
                .collect(),
            security: SecurityConfig {
                trust_on_first_use: tofu,
                known_peers: Some(dir.join("known_peers")),
            },
            ..Config::default()
        };
        TrustStore::from_config(&config, PathBuf::from("/nonexistent"))
    }

    /// A scratch directory removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempdir() -> TempDir {
        let dir = std::env::temp_dir().join(format!("cc-trust-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    #[test]
    fn config_pin_authorizes_matching_name_and_key() {
        let dir = tempdir();
        let s = store(dir.path(), false, &[("bob", FP_A)]);
        assert_eq!(s.authorize("bob", FP_A).unwrap(), Authorized::Pinned);
        // Compact form of the same fingerprint is the same key.
        assert_eq!(
            s.authorize("bob", &FP_A.replace("SHA256:", "").replace(':', ""))
                .unwrap(),
            Authorized::Pinned
        );
    }

    #[test]
    fn unknown_peer_refused_without_tofu() {
        let dir = tempdir();
        let s = store(dir.path(), false, &[]);
        assert!(matches!(
            s.authorize("bob", FP_A),
            Err(TrustError::Untrusted { .. })
        ));
        assert!(!dir.path().join("known_peers").exists(), "nothing recorded");
    }

    #[test]
    fn tofu_records_then_recognises_peer() {
        let dir = tempdir();
        let s = store(dir.path(), true, &[]);
        assert_eq!(s.authorize("bob", FP_A).unwrap(), Authorized::Learned);
        assert_eq!(s.authorize("bob", FP_A).unwrap(), Authorized::Known);
        // A fresh store (daemon restart) reads it back from disk.
        let s2 = store(dir.path(), false, &[]);
        assert_eq!(s2.authorize("bob", FP_A).unwrap(), Authorized::Known);
    }

    #[test]
    fn changed_key_refused_even_with_tofu() {
        let dir = tempdir();
        let s = store(dir.path(), true, &[]);
        s.authorize("bob", FP_A).unwrap();
        assert!(matches!(
            s.authorize("bob", FP_B),
            Err(TrustError::FingerprintChanged { .. })
        ));
        let pinned = store(dir.path(), true, &[("carol", FP_B)]);
        assert!(matches!(
            pinned.authorize("carol", FP_A),
            Err(TrustError::FingerprintChanged { .. })
        ));
    }

    #[test]
    fn trusted_key_cannot_claim_another_name() {
        let dir = tempdir();
        let s = store(dir.path(), true, &[("bob", FP_A)]);
        assert!(matches!(
            s.authorize("mallory", FP_A),
            Err(TrustError::NameMismatch { .. })
        ));
    }

    #[test]
    fn hostile_names_are_rejected_before_touching_the_file() {
        let dir = tempdir();
        let s = store(dir.path(), true, &[]);
        for name in ["", " bob", "bob\nAAAA evil", &"x".repeat(MAX_NAME_LEN + 1)] {
            assert!(
                matches!(s.authorize(name, FP_A), Err(TrustError::InvalidName(_))),
                "{name:?} accepted"
            );
        }
        assert!(!dir.path().join("known_peers").exists());
    }

    #[test]
    fn add_replaces_and_remove_deletes() {
        let dir = tempdir();
        let path = dir.path().join("sub/known_peers");
        add_known_peer(&path, "bob", FP_A).unwrap();
        add_known_peer(&path, "bob", FP_B).unwrap();
        add_known_peer(&path, "carol", FP_A).unwrap();
        let s = store(dir.path().join("sub").as_path(), false, &[]);
        let entries = s.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            s.pin_for("bob"),
            Some(display_fingerprint(&FP_B.to_lowercase()))
        );

        assert!(remove_known_peer(&path, "bob").unwrap());
        assert!(!remove_known_peer(&path, "bob").unwrap());
        assert_eq!(s.pin_for("bob"), None);
        assert!(add_known_peer(&path, "dave", "SHA256:zz").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn known_peers_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
        let path = dir.path().join("known_peers");
        add_known_peer(&path, "bob", FP_A).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn display_round_trips() {
        let canonical = parse_fingerprint(FP_A).unwrap();
        assert_eq!(display_fingerprint(&canonical), FP_A);
    }
}
