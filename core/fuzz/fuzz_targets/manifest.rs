//! Any bytes as a manifest or a pointer: no panic, and whatever decodes keeps the floors and
//! re-encodes to the same bytes; an update to itself is never accepted.
#![no_main]

use hoi_types::scatter::{Manifest, Pointer};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(m) = Manifest::decode(data) {
        assert!(m.check().is_ok());
        assert_eq!(m.encode(), data);
        assert!(m.update_to(&m).is_err());
    }
    if let Ok(p) = Pointer::decode(data) {
        assert!(p.manifest.check().is_ok());
        assert_eq!(p.encode(), data);
    }
});
