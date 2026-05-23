//! mDNS/DNS-SD discovery backend built on `mdns-sd`.
//!
//! Service type: `_cross-control._udp.local.` — UDP because cross-control's
//! only transport is QUIC (see ADR 0001).
//!
//! TXT records carry the cryptographic identity needed for TOFU verification:
//!
//! | key     | value                                              |
//! |---------|----------------------------------------------------|
//! | `id`    | `MachineId` as a UUID string                       |
//! | `fp`    | TLS cert SHA-256 fingerprint (lowercase hex)       |
//! | `name`  | human-readable host name                           |
//! | `proto` | protocol version, e.g. `0.1`                       |
//!
//! `id` + `fp` are what the daemon needs to decide whether to trust a
//! discovered peer before opening a QUIC connection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cross_control_types::MachineId;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;
use tracing::debug;
use uuid::Uuid;

use crate::{Discovery, DiscoveryError, DiscoveryEvent, Peer};

/// mDNS service type used by cross-control. UDP because QUIC.
pub const SERVICE_TYPE: &str = "_cross-control._udp.local.";

const TXT_KEY_ID: &str = "id";
const TXT_KEY_FP: &str = "fp";
const TXT_KEY_NAME: &str = "name";
const TXT_KEY_PROTO: &str = "proto";

/// mDNS/DNS-SD backend.
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    /// Full service name of our own advertisement, if registered. Needed for
    /// unregister.
    registered_fullname: Option<String>,
    /// Whether a browse is currently active. Used by `stop_browsing`.
    browsing: bool,
    /// Our own machine id, recorded at advertise time so the browse loop can
    /// filter ourselves out.
    self_id: Option<MachineId>,
    /// SHA-256 cert fingerprint to advertise (lowercase hex).
    fingerprint: Option<String>,
    /// Map of mDNS service fullname → resolved `MachineId`. Needed because
    /// `ServiceEvent::ServiceRemoved` arrives without TXT records, so we
    /// can't recover the id from the event itself.
    resolved: Arc<Mutex<HashMap<String, MachineId>>>,
}

impl MdnsDiscovery {
    /// Construct a new backend with no fingerprint advertised.
    ///
    /// Peers without a fingerprint TXT cannot be TOFU-verified before
    /// connection — prefer [`MdnsDiscovery::with_fingerprint`].
    pub fn new() -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| DiscoveryError::Registration(format!("ServiceDaemon::new: {e}")))?;
        Ok(Self {
            daemon,
            registered_fullname: None,
            browsing: false,
            self_id: None,
            fingerprint: None,
            resolved: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Construct a new backend that advertises the given cert fingerprint
    /// (lowercase hex, no `SHA256:` prefix) so peers can pin before connecting.
    pub fn with_fingerprint(fingerprint: impl Into<String>) -> Result<Self, DiscoveryError> {
        let mut this = Self::new()?;
        this.fingerprint = Some(fingerprint.into());
        Ok(this)
    }
}

#[async_trait]
impl Discovery for MdnsDiscovery {
    async fn advertise(
        &mut self,
        machine_id: MachineId,
        name: &str,
        port: u16,
    ) -> Result<(), DiscoveryError> {
        let hostname = format!("{}.local.", sanitize_label(name));
        let instance = format!("{} ({})", name, short_id(&machine_id));

        let mut props: HashMap<String, String> = HashMap::new();
        props.insert(TXT_KEY_ID.into(), machine_id.to_string());
        props.insert(TXT_KEY_NAME.into(), name.into());
        props.insert(TXT_KEY_PROTO.into(), "0.1".into());
        if let Some(fp) = &self.fingerprint {
            props.insert(TXT_KEY_FP.into(), fp.clone());
        }

        // Empty `my_addrs` tells mdns-sd to auto-detect interface IPs and
        // broadcast on each one. Avoids the daemon publishing a literal
        // 0.0.0.0 or a stale hostname-resolved IP.
        let info = ServiceInfo::new(SERVICE_TYPE, &instance, &hostname, "", port, Some(props))
            .map_err(|e| DiscoveryError::Registration(format!("ServiceInfo: {e}")))?
            .enable_addr_auto();

        let fullname = info.get_fullname().to_string();
        self.daemon
            .register(info)
            .map_err(|e| DiscoveryError::Registration(format!("register: {e}")))?;
        self.registered_fullname = Some(fullname);
        self.self_id = Some(machine_id);
        Ok(())
    }

    async fn stop_advertising(&mut self) -> Result<(), DiscoveryError> {
        if let Some(fullname) = self.registered_fullname.take() {
            self.daemon
                .unregister(&fullname)
                .map_err(|e| DiscoveryError::Registration(format!("unregister: {e}")))?;
        }
        Ok(())
    }

    async fn browse(&mut self) -> Result<mpsc::Receiver<DiscoveryEvent>, DiscoveryError> {
        let receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| DiscoveryError::Browse(format!("browse: {e}")))?;

        let (tx, rx) = mpsc::channel::<DiscoveryEvent>(64);
        let self_id = self.self_id;
        let resolved = Arc::clone(&self.resolved);

        tokio::spawn(async move {
            loop {
                match receiver.recv_async().await {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        let Some(peer) = peer_from_info(&info) else {
                            debug!(fullname = %info.get_fullname(), "ignoring service without id TXT");
                            continue;
                        };
                        if Some(peer.machine_id) == self_id {
                            continue;
                        }
                        if let Ok(mut map) = resolved.lock() {
                            map.insert(info.get_fullname().to_string(), peer.machine_id);
                        }
                        if tx.send(DiscoveryEvent::PeerFound(peer)).await.is_err() {
                            break;
                        }
                    }
                    Ok(ServiceEvent::ServiceRemoved(_ty, fullname)) => {
                        let id = resolved
                            .lock()
                            .ok()
                            .and_then(|mut map| map.remove(&fullname));
                        if let Some(id) = id {
                            if Some(id) == self_id {
                                continue;
                            }
                            if tx.send(DiscoveryEvent::PeerLost(id)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        debug!(error = %e, "mDNS receiver closed");
                        break;
                    }
                }
            }
        });

        self.browsing = true;
        Ok(rx)
    }

    async fn stop_browsing(&mut self) -> Result<(), DiscoveryError> {
        if self.browsing {
            self.daemon
                .stop_browse(SERVICE_TYPE)
                .map_err(|e| DiscoveryError::Browse(format!("stop_browse: {e}")))?;
            self.browsing = false;
        }
        Ok(())
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        // Best-effort cleanup; ignore errors since drop can't propagate.
        if let Some(fullname) = self.registered_fullname.take() {
            let _ = self.daemon.unregister(&fullname);
        }
        let _ = self.daemon.shutdown();
    }
}

/// Extract a [`Peer`] from a resolved mDNS [`ServiceInfo`].
fn peer_from_info(info: &ServiceInfo) -> Option<Peer> {
    let props = info.get_properties();
    let id_str = props.get_property_val_str(TXT_KEY_ID)?;
    let uuid = Uuid::from_str(id_str).ok()?;
    let machine_id = MachineId::from_uuid(uuid);

    let name = props
        .get_property_val_str(TXT_KEY_NAME)
        .map_or_else(|| info.get_fullname().to_string(), str::to_string);
    let fingerprint = props.get_property_val_str(TXT_KEY_FP).map(str::to_string);

    let port = info.get_port();
    let addr = info
        .get_addresses()
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| info.get_addresses().iter().next())
        .copied()?;

    Some(Peer {
        machine_id,
        name,
        address: SocketAddr::new(addr, port),
        fingerprint,
    })
}

/// First 8 hex chars of a `MachineId`. Enough to disambiguate in mDNS
/// service-instance names without being noisy.
fn short_id(id: &MachineId) -> String {
    let s = id.to_string();
    s.chars().take(8).collect()
}

/// Sanitize a string for use as a DNS label. Replaces anything outside
/// `[A-Za-z0-9]` with `-`, collapses runs of `-`, and trims leading/trailing
/// dashes.
fn sanitize_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for c in s.chars() {
        let ch = if c.is_ascii_alphanumeric() { c } else { '-' };
        if ch == '-' {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else {
            out.push(ch);
            last_dash = false;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_unicode_and_collapses_dashes() {
        assert_eq!(sanitize_label("Alice's Laptop"), "Alice-s-Laptop");
        assert_eq!(sanitize_label("--weird--name--"), "weird-name");
        assert_eq!(sanitize_label("plain"), "plain");
    }

    #[test]
    fn short_id_is_eight_chars() {
        let id = MachineId::new();
        assert_eq!(short_id(&id).len(), 8);
    }

    #[tokio::test]
    async fn daemon_construction_does_not_panic() {
        let _ = MdnsDiscovery::new().expect("mDNS daemon should construct");
    }

    #[tokio::test]
    async fn with_fingerprint_records_it() {
        let backend = MdnsDiscovery::with_fingerprint("abc123").expect("ok");
        assert_eq!(backend.fingerprint.as_deref(), Some("abc123"));
    }
}
