//! Any bytes a peer sends as one frame must decode to `Ok` or `Err` —
//! never panic, abort, or allocate past the frame limit.
#![no_main]

use cross_control_types::Message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = cross_control_protocol::wire::decode_message::<Message>(data);
});
