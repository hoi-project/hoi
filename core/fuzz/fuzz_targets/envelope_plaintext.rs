//! Any bytes as the plaintext of any request or answer: no panic, and whatever parses writes
//! back to the same bytes.
#![no_main]

use hoi_crypto::envelope::{Kind, Reply, Request};
use libfuzzer_sys::fuzz_target;

const KINDS: [Kind; 8] = [Kind::Put, Kind::Get, Kind::Has, Kind::Drop, Kind::Stored, Kind::Got, Kind::Had, Kind::Dropped];

fuzz_target!(|data: &[u8]| {
    let [kind, w0, w1, rest @ ..] = data else { return };
    let kind = KINDS[*kind as usize % KINDS.len()];
    let want = u16::from_be_bytes([*w0, *w1]) as usize;
    if let Ok(r) = Request::from_plaintext(kind, rest) {
        assert_eq!(r.to_plaintext().unwrap(), rest);
    }
    if let Ok(r) = Reply::from_plaintext(kind, want, rest) {
        assert_eq!(r.to_plaintext(want).unwrap(), rest);
    }
});
