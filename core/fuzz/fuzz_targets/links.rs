//! Any text as an invite, a pairing link, a node link or a recovery key: parsing never panics,
//! and an invite that parses shows the same community when written out and read again.
#![no_main]

use hoi_types::uri::{Invite, NodeLink, Pair};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };
    if let Ok(i) = Invite::parse(s) {
        let again = Invite::parse(&i.to_uri()).expect("an invite written out reads back");
        assert_eq!(again.community, i.community);
    }
    let _ = Pair::parse(s);
    let _ = NodeLink::parse(s);
    let _ = hoi_crypto::recovery::decode(s);
});
