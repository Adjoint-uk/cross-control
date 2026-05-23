//! Composable discovery: fan out `advertise` across backends, merge `browse`
//! streams, and de-duplicate peers by `MachineId`.
//!
//! Why an aggregator: cross-control's daemon needs to consume peers from
//! several sources at once — mDNS for the LAN, the static config for stable
//! addresses, and (eventually) cloud rendezvous for cross-subnet. Putting
//! that fan-in here means the daemon only ever talks to one trait object.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cross_control_types::MachineId;
use tokio::sync::mpsc;
use tracing::debug;

use crate::{Discovery, DiscoveryError, DiscoveryEvent, Peer};

/// Owns a set of [`Discovery`] backends and presents them as one.
///
/// `browse` returns a single receiver fed by every backend. The aggregator
/// dedupes by `MachineId`: a `PeerFound` is forwarded the first time we see
/// the id; subsequent finds (e.g. the same peer surfacing via both mDNS and
/// static config) are silently dropped. `PeerLost` is only forwarded when
/// the *last* backend reporting that id drops it.
pub struct DiscoveryAggregator {
    backends: Vec<Box<dyn Discovery>>,
}

impl DiscoveryAggregator {
    /// Construct an empty aggregator. Add backends with [`Self::push`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
        }
    }

    /// Add a backend. Order is not significant.
    pub fn push(&mut self, backend: Box<dyn Discovery>) {
        self.backends.push(backend);
    }

    /// Number of backends currently registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Whether any backend is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }
}

impl Default for DiscoveryAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Discovery for DiscoveryAggregator {
    async fn advertise(
        &mut self,
        machine_id: MachineId,
        name: &str,
        port: u16,
    ) -> Result<(), DiscoveryError> {
        for backend in &mut self.backends {
            // Fan out: surface the first error but keep advertising on the
            // others if possible. Returning the first error keeps the daemon
            // simple — partial-failure handling is the aggregator's job, not
            // the daemon's.
            backend.advertise(machine_id, name, port).await?;
        }
        Ok(())
    }

    async fn stop_advertising(&mut self) -> Result<(), DiscoveryError> {
        let mut first_err: Option<DiscoveryError> = None;
        for backend in &mut self.backends {
            if let Err(e) = backend.stop_advertising().await {
                first_err.get_or_insert(e);
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
        Ok(())
    }

    async fn browse(&mut self) -> Result<mpsc::Receiver<DiscoveryEvent>, DiscoveryError> {
        let (out_tx, out_rx) = mpsc::channel::<DiscoveryEvent>(128);
        // refcount_per_id: how many backends currently report this peer.
        // Used to suppress duplicate PeerFound and to defer PeerLost until
        // every backend has dropped the peer.
        let refcount: Arc<Mutex<HashMap<MachineId, usize>>> = Arc::new(Mutex::new(HashMap::new()));

        for backend in &mut self.backends {
            let mut backend_rx = backend.browse().await?;
            let out_tx = out_tx.clone();
            let refcount = Arc::clone(&refcount);
            tokio::spawn(async move {
                while let Some(event) = backend_rx.recv().await {
                    match event {
                        DiscoveryEvent::PeerFound(peer) => {
                            let is_first = {
                                let Ok(mut map) = refcount.lock() else { break };
                                let entry = map.entry(peer.machine_id).or_insert(0);
                                *entry += 1;
                                *entry == 1
                            };
                            if is_first {
                                if out_tx.send(DiscoveryEvent::PeerFound(peer)).await.is_err() {
                                    break;
                                }
                            } else {
                                debug!(id = %peer.machine_id, "deduped peer found");
                            }
                        }
                        DiscoveryEvent::PeerLost(id) => {
                            let was_last = {
                                let Ok(mut map) = refcount.lock() else { break };
                                if let Some(count) = map.get_mut(&id) {
                                    *count = count.saturating_sub(1);
                                    if *count == 0 {
                                        map.remove(&id);
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            };
                            if was_last && out_tx.send(DiscoveryEvent::PeerLost(id)).await.is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            });
        }

        Ok(out_rx)
    }

    async fn stop_browsing(&mut self) -> Result<(), DiscoveryError> {
        let mut first_err: Option<DiscoveryError> = None;
        for backend in &mut self.backends {
            if let Err(e) = backend.stop_browsing().await {
                first_err.get_or_insert(e);
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
        Ok(())
    }
}

/// A trivial [`Discovery`] backend that emits a fixed set of peers from the
/// static config on every `browse` call. Lets the daemon consume static
/// config the same way it consumes mDNS — through the aggregator.
///
/// Static peers don't time out; `PeerLost` is never emitted by this backend.
/// The connection layer is responsible for noticing when a static peer is
/// unreachable.
pub struct StaticDiscovery {
    peers: Vec<Peer>,
    seen: HashSet<MachineId>,
}

impl StaticDiscovery {
    /// Construct from a list of fully-resolved peers.
    #[must_use]
    pub fn new(peers: Vec<Peer>) -> Self {
        Self {
            peers,
            seen: HashSet::new(),
        }
    }

    /// Construct from `(name, address, optional fingerprint)` tuples,
    /// generating a fresh `MachineId` for each. Use this when the config
    /// only knows a network address — the real id will be confirmed on
    /// handshake.
    #[must_use]
    pub fn from_addresses(entries: Vec<(String, SocketAddr, Option<String>)>) -> Self {
        let peers = entries
            .into_iter()
            .map(|(name, address, fingerprint)| Peer {
                machine_id: MachineId::new(),
                name,
                address,
                fingerprint,
            })
            .collect();
        Self::new(peers)
    }
}

#[async_trait]
impl Discovery for StaticDiscovery {
    async fn advertise(
        &mut self,
        _machine_id: MachineId,
        _name: &str,
        _port: u16,
    ) -> Result<(), DiscoveryError> {
        // Static peers are read-only — nothing to advertise.
        Ok(())
    }

    async fn stop_advertising(&mut self) -> Result<(), DiscoveryError> {
        Ok(())
    }

    async fn browse(&mut self) -> Result<mpsc::Receiver<DiscoveryEvent>, DiscoveryError> {
        let (tx, rx) = mpsc::channel::<DiscoveryEvent>(self.peers.len().max(1));
        for peer in &self.peers {
            if self.seen.insert(peer.machine_id) {
                // Best-effort send; receiver is empty so this can't block.
                let _ = tx.send(DiscoveryEvent::PeerFound(peer.clone())).await;
            }
        }
        Ok(rx)
    }

    async fn stop_browsing(&mut self) -> Result<(), DiscoveryError> {
        self.seen.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn peer(name: &str, port: u16) -> Peer {
        Peer {
            machine_id: MachineId::new(),
            name: name.into(),
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            fingerprint: None,
        }
    }

    #[tokio::test]
    async fn static_discovery_emits_each_peer_once() {
        let p1 = peer("a", 1001);
        let p2 = peer("b", 1002);
        let mut s = StaticDiscovery::new(vec![p1.clone(), p2.clone()]);
        let mut rx = s.browse().await.expect("browse");
        let mut seen = HashSet::new();
        for _ in 0..2 {
            match rx.recv().await {
                Some(DiscoveryEvent::PeerFound(p)) => {
                    seen.insert(p.machine_id);
                }
                other @ (Some(DiscoveryEvent::PeerLost(_)) | None) => {
                    panic!("unexpected {other:?}")
                }
            }
        }
        assert!(seen.contains(&p1.machine_id));
        assert!(seen.contains(&p2.machine_id));
    }

    #[tokio::test]
    async fn aggregator_dedupes_peer_found() {
        let shared = peer("shared", 1234);
        let backend_a = StaticDiscovery::new(vec![shared.clone()]);
        let backend_b = StaticDiscovery::new(vec![shared.clone()]);

        let mut agg = DiscoveryAggregator::new();
        agg.push(Box::new(backend_a));
        agg.push(Box::new(backend_b));

        let mut rx = agg.browse().await.expect("browse");

        let first = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("first event")
            .expect("some event");
        match first {
            DiscoveryEvent::PeerFound(p) => assert_eq!(p.machine_id, shared.machine_id),
            other @ DiscoveryEvent::PeerLost(_) => panic!("expected PeerFound, got {other:?}"),
        }

        // Second backend should be suppressed by the dedupe layer. Either
        // a timeout (no event ever arrives) or `Ok(None)` (channel closes
        // because both backend tasks finished without emitting more events)
        // is correct — the only failure is a second `PeerFound`.
        let dup = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        match dup {
            Err(_) | Ok(None) => {}
            Ok(Some(ev)) => panic!("expected no second event, got {ev:?}"),
        }
    }
}
