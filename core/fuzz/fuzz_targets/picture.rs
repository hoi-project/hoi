//! Any bytes as a picture: fingerprinting (decoding, turning, PDQ) and the profile-picture
//! check never panic, and a fingerprint's variants are those of the picture as shown.
#![no_main]

use hoi_validate::{media, similar};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for mime in [media::PNG, media::JPEG, media::GIF, media::WEBP] {
        if let Ok(Some(v)) = similar::examine_variants(mime, data) {
            assert!(v.quality >= similar::MIN_QUALITY);
            assert_eq!(similar::examine(mime, data), Ok(Some(v.shown())));
        }
    }
    let _ = hoi_validate::avatar(data);
    // as raw luma too: any size the bytes allow
    if data.len() >= 2 {
        let w = 5 + data[0] as usize % 60;
        let px: Vec<f32> = data[2..].iter().map(|b| *b as f32).collect();
        let h = px.len() / w;
        if h > 0 {
            let _ = similar::luma_variants(px[..w * h].to_vec(), w, h);
        }
    }
});
