//! Any bytes as media: sanitizing and checking never panic; what sanitizing keeps passes the
//! checks as the type it was sniffed as, and sanitizing it again changes nothing.
#![no_main]

use hoi_validate::media;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for mime in [media::PNG, media::JPEG, media::GIF, media::WEBP, media::OGG, "video/mp4", "video/webm", "audio/mpeg"] {
        let _ = media::check(mime, data);
    }
    if let Ok((clean, info)) = media::sanitize(data) {
        assert!(media::check(info.mime, &clean).is_ok(), "sanitized {} fails its own checks", info.mime);
        let (again, _) = media::sanitize(&clean).expect("sanitized media sanitizes");
        assert_eq!(again, clean, "sanitizing twice changes {}", info.mime);
    }
});
