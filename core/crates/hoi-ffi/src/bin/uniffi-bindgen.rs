//! `cargo run -p hoi-ffi --features cli --bin uniffi-bindgen -- generate --library <libhoi_ffi> --language swift|kotlin --out-dir <dir>`
//!
//! Built from this crate so the generator always matches the UniFFI version the library uses.

fn main() {
    uniffi::uniffi_bindgen_main()
}
