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
    tauri_build::build()
}
