//! Any bytes as a sync message: decoding never panics, and every message that decodes encodes
//! to bytes that decode to the same message.
#![no_main]

use hoi_sync::Msg;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(m) = Msg::decode(data) {
        let again = Msg::decode(&m.encode()).expect("what decodes re-encodes");
        assert_eq!(again, m);
    }
});
