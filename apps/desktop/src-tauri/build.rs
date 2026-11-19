fn main() {
    println!("cargo:rerun-if-changed=../../../publication.json");
    println!("cargo:rerun-if-env-changed=HOI_PUBLICATION");
    if std::env::var("HOI_PUBLICATION").as_deref() == Ok("1") {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../../publication.json")).expect("publication.json");
        for key in ["repository", "site", "downloads", "contact", "manifestKey", "updaterKey"] {
            let value = config[key].as_str().unwrap_or("");
            assert!(!value.is_empty() && !value.contains("OWNER") && !value.contains("placeholder"), "Configure publication.json before publication: {key}");
        }
    }
    tauri_build::build();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // tauri-winres writes resource.lib here. The Windows library-test executable
        // links it explicitly because Tauri attaches the resource only to binary targets.
        println!("cargo:rustc-link-search=native={}", std::env::var("OUT_DIR").unwrap());
    }
}
