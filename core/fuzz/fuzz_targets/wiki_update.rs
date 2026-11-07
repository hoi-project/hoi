//! Any bytes as a wiki edit or a rules change: the check at ingest never panics, and
//! neither does importing what passed it, or reading the text after.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if loro::LoroDoc::decode_import_blob_meta(data, true).is_ok() {
        let doc = loro::LoroDoc::new();
        let _ = doc.import(data);
        let _ = doc.get_text("body").to_string();
        let _ = doc.get_text("rules").to_string();
    }
});
