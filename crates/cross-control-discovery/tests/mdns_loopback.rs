//! Loopback advertise/browse test for the mDNS backend.
//!
//! Marked `#[ignore]` because it requires functional multicast on the test
//! host — some CI environments and containers block UDP 5353. Run locally
//! with `cargo test -p cross-control-discovery -- --ignored`.

use std::time::Duration;

use cross_control_discovery::{Discovery, DiscoveryEvent, MdnsDiscovery};
use cross_control_types::MachineId;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires functional multicast — run with --ignored"]
async fn advertise_and_browse_round_trip() {
    let advertiser_id = MachineId::new();
    let mut advertiser = MdnsDiscovery::with_fingerprint("deadbeef").expect("advertiser construct");
    advertiser
        .advertise(advertiser_id, "test-advertiser", 24800)
        .await
        .expect("advertise");

    // Browser is a separate ServiceDaemon so it really exercises the network
    // path rather than an in-process shortcut.
    let mut browser = MdnsDiscovery::new().expect("browser construct");
    let mut rx = browser.browse().await.expect("browse");

    let event = timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("did not receive a PeerFound within 10s")
        .expect("channel closed before event");

    match event {
        DiscoveryEvent::PeerFound(peer) => {
            assert_eq!(peer.machine_id, advertiser_id, "machine id should match");
            assert_eq!(
                peer.fingerprint.as_deref(),
                Some("deadbeef"),
                "fingerprint TXT should round-trip"
            );
            assert_eq!(peer.address.port(), 24800, "port should match");
        }
        other @ DiscoveryEvent::PeerLost(_) => panic!("expected PeerFound, got {other:?}"),
    }

    advertiser.stop_advertising().await.ok();
    browser.stop_browsing().await.ok();
}
