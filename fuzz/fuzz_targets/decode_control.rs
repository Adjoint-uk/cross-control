//! The control stream decodes `ControlMessage` directly (the pre-auth
//! handshake path). Re-encoding whatever decodes must round-trip.
#![no_main]

use cross_control_types::ControlMessage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(msg) = cross_control_protocol::wire::decode_message::<ControlMessage>(data) {
        let encoded = cross_control_protocol::wire::encode_message(&msg)
            .expect("a decoded message must re-encode");
        let again: ControlMessage = cross_control_protocol::wire::decode_message(&encoded[4..])
            .expect("a re-encoded message must decode");
        assert_eq!(format!("{msg:?}"), format!("{again:?}"));
    }
});
