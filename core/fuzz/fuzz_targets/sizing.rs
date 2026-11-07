//! Any level, generation and number of places: sizing never panics, and every size it gives
//! keeps its own rules and meets the level it was made for.
#![no_main]

use hoi_crypto::scatter::size;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [l, g, p0, p1, p2, p3, ..] = data else { return };
    let level = size::LEVELS[*l as usize % size::LEVELS.len()];
    let g = g % 32;
    let places = u32::from_be_bytes([*p0, *p1, *p2, *p3]);
    if let Some(s) = size::size(level, g, places) {
        assert!(s.k >= 3 && s.k + 2 <= s.n && s.n <= s.m && s.m <= size::budget(s.n));
        assert!(s.n <= size::LINKS * (s.k - 1));
        assert!(size::meets(level, g, places, s));
        assert!(size::reading(g, places, s).is_some_and(|r| r >= level));
    }
});
