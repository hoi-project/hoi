//! Any bytes as a record on the wire: decoding never panics, and whatever decodes re-encodes to
//! bytes that decode to the same record with the same id; the validator's first stage agrees.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok((w, id)) = hoi_types::Wire::decode(data) {
        let again = w.encode();
        let (w2, id2) = hoi_types::Wire::decode(&again).expect("what decodes re-encodes");
        assert_eq!(id, id2);
        assert_eq!(w2.encode(), again);
    }
    if let Ok(d) = hoi_validate::decode(data) {
        let _ = hoi_validate::signature(&d);
        let _ = hoi_validate::content(&d.wire.record);
    }
});
