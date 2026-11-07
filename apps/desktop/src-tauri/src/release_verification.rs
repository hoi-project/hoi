//! Authenticate metadata before interpreting its version, notice or artifact addresses.
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MAX_MANIFEST: usize = 1024 * 1024;
pub const MAX_ARTIFACT: u64 = 1024 * 1024 * 1024;

pub fn verify(bytes: &[u8], signature: &str, key: &str, minimum: &str, revision: u64) -> Result<Value, String> {
    if bytes.len() > MAX_MANIFEST || signature.len() > 4096 {
        return Err("Release metadata is too large.".into());
    }
    let key = PublicKey::from_base64(key).map_err(|_| "Release signing key is not configured.")?;
    let signature = Signature::decode(signature).map_err(|_| "Invalid release signature.")?;
    key.verify(bytes, &signature, false).map_err(|_| "Release signature did not verify.")?;
    let manifest: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid release list.")?;
    let latest = manifest["latest"].as_str().ok_or("Release version is missing.")?;
    let version = Version::parse(latest).map_err(|_| "Invalid release version.")?;
    if version < Version::parse(minimum).map_err(|_| "Invalid installed release version.")? {
        return Err("An older release list was rejected.".into());
    }
    let release = &manifest["release"];
    if manifest["schema"] != 1
        || release["schemaVersion"] != 1
        || release["revision"].as_u64().is_none_or(|n| n < revision)
        || !matches!(release["channel"].as_str(), Some("stable" | "preview"))
        || release["notice"]["message"].as_str().is_none_or(|s| s.len() > 4800)
        || manifest["releases"].as_array().is_none_or(|r| r.len() != 1 || r[0]["version"] != latest)
        || manifest["updater"]["version"] != latest
    {
        return Err("Unsupported release metadata.".into());
    }
    Ok(manifest)
}

/// The updater's choice must also be a hashed desktop artifact for this platform.
pub fn artifact(manifest: &Value, url: &str, platform: &str, arch: &str) -> Result<(u64, String), String> {
    let files = manifest["releases"][0]["files"].as_array().ok_or("Release files are missing.")?;
    let matching: Vec<_> = files.iter().filter(|f| f["url"] == url).collect();
    if matching.len() != 1 {
        return Err("Updater artifact is not uniquely listed in the signed manifest.".into());
    }
    let file = matching[0];
    let size = file["size"].as_u64().ok_or("Invalid artifact size.")?;
    let hash = file["sha256"].as_str().ok_or("Artifact checksum is missing.")?;
    let url = reqwest::Url::parse(url).map_err(|_| "Invalid artifact address.")?;
    let kind_ok = match platform {
        "macos" => file["kind"] == "app.tar.gz",
        "windows" => matches!(file["kind"].as_str(), Some("exe" | "msi")),
        "linux" => matches!(file["kind"].as_str(), Some("AppImage" | "AppImage.tar.gz")),
        _ => false,
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !file["product"].is_null()
        || file["platform"] != platform
        || !kind_ok
        || (file["arch"] != arch && !(platform == "macos" && file["arch"] == "universal"))
        || size == 0
        || size > MAX_ARTIFACT
        || hash.len() != 64
        || !hash.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("Release artifact has the wrong product, platform, size or checksum.".into());
    }
    Ok((size, hash.into()))
}

pub fn verify_artifact(bytes: &[u8], size: u64, hash: &str) -> Result<(), String> {
    if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(bytes)) != hash {
        return Err("The update file does not match its signed checksum. Nothing was installed; retry the download.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_metadata_and_exact_artifact_fail_closed() {
        let fixture: Value = serde_json::from_str(include_str!("../../../../scripts/fixtures/release-signed.json")).unwrap();
        let bytes = fixture["manifest"].as_str().unwrap().as_bytes();
        let signature = fixture["signature"].as_str().unwrap();
        let key = fixture["key"].as_str().unwrap();
        let manifest = verify(bytes, signature, key, "1.0.0", 1).unwrap();
        assert!(verify(bytes, signature, key, "3.0.0", 1).is_err());
        assert!(verify(bytes, signature, key, "1.0.0", 2).is_err());
        let mut changed = bytes.to_vec();
        changed.push(b' ');
        assert!(verify(&changed, signature, key, "1.0.0", 1).is_err());
        assert!(verify(bytes, "", key, "1.0.0", 1).is_err());
        assert!(verify(bytes, signature, "", "1.0.0", 1).is_err());
        let url = manifest["updater"]["platforms"]["linux-x86_64"]["url"].as_str().unwrap();
        let (size, hash) = artifact(&manifest, url, "linux", "x86_64").unwrap();
        let payload = fixture["artifact"].as_str().unwrap().as_bytes();
        verify_artifact(payload, size, &hash).unwrap();
        assert!(verify_artifact(&payload[1..], size, &hash).is_err());
        assert!(artifact(&manifest, url, "windows", "x86_64").is_err());
        assert!(artifact(&manifest, url, "linux", "aarch64").is_err());
        let mut wrong_product = manifest.clone();
        wrong_product["releases"][0]["files"][0]["product"] = "setup".into();
        assert!(artifact(&wrong_product, url, "linux", "x86_64").is_err());
    }
}
