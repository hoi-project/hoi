//! Any pieces, any indexes: rebuilding and undoing the transform never panic, and a sealed object
//! that comes out has its class's length.
#![no_main]

use hoi_crypto::scatter::{Class, Shards, aont};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [class, k, rest @ ..] = data else { return };
    let Some(class) = Class::from_index(class % 2) else { return };
    let k = (*k as usize % 20).max(1);
    let len = class.piece_len(k);
    // indexes from the input, piece bytes cycled from the rest of it
    let (idx, body) = rest.split_at(rest.len().min(k));
    if body.is_empty() {
        let _ = aont::unpackage(rest);
        return;
    }
    let pieces: Vec<Vec<u8>> = (0..idx.len()).map(|p| (0..len).map(|j| body[(p * len + j) % body.len()]).collect()).collect();
    let refs: Vec<(u8, &[u8])> = idx.iter().zip(&pieces).map(|(&i, p)| (i, p.as_slice())).collect();
    if let Ok(sealed) = Shards::rebuild(class, k, &refs).and_then(|s| s.sealed()) {
        assert_eq!(sealed.len(), class.sealed_len());
    }
    let _ = aont::unpackage(body);
});
