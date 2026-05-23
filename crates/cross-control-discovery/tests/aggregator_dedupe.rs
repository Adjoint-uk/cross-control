//! Integration-level dedupe test for `DiscoveryAggregator`.
//!
//! Built on `StaticDiscovery` because we want to exercise the public surface
//! end-to-end (`browse().recv()`), not just the internal refcount.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use cross_control_discovery::{
    Discovery, DiscoveryAggregator, DiscoveryEvent, Peer, StaticDiscovery,
};
use cross_control_types::MachineId;
use tokio::time::timeout;

fn peer(name: &str, port: u16) -> Peer {
    Peer {
        machine_id: MachineId::new(),
        name: name.into(),
        address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        fingerprint: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_peers_across_backends_surface_once() {
    let shared = peer("shared-peer", 9001);
    let unique_a = peer("only-in-a", 9002);
    let unique_b = peer("only-in-b", 9003);

    let backend_a = StaticDiscovery::new(vec![shared.clone(), unique_a.clone()]);
    let backend_b = StaticDiscovery::new(vec![shared.clone(), unique_b.clone()]);

    let mut agg = DiscoveryAggregator::new();
    agg.push(Box::new(backend_a));
    agg.push(Box::new(backend_b));

    let mut rx = agg.browse().await.expect("browse");

    let mut seen_ids = std::collections::HashSet::new();
    let mut shared_count = 0;
    // Expect 3 PeerFound events: shared (once), unique_a, unique_b.
    for _ in 0..3 {
        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("event should arrive")
            .expect("channel should still be open");
        match event {
            DiscoveryEvent::PeerFound(p) => {
                if p.machine_id == shared.machine_id {
                    shared_count += 1;
                }
                seen_ids.insert(p.machine_id);
            }
            other @ DiscoveryEvent::PeerLost(_) => panic!("unexpected {other:?}"),
        }
    }

    assert_eq!(shared_count, 1, "shared peer should surface exactly once");
    assert!(seen_ids.contains(&shared.machine_id));
    assert!(seen_ids.contains(&unique_a.machine_id));
    assert!(seen_ids.contains(&unique_b.machine_id));
    assert_eq!(seen_ids.len(), 3);

    // No further events should arrive.
    let extra = timeout(Duration::from_millis(200), rx.recv()).await;
    match extra {
        Err(_) | Ok(None) => {}
        Ok(Some(ev)) => panic!("unexpected extra event: {ev:?}"),
    }
}
