//! The release manifest a registry serves: the compiled `release.json`, or a replacement file
//! the operator points at, read on every request so a notice can change without a rebuild.
//! Validated exactly like `scripts/release-schema.mjs`; an invalid manifest fails closed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A replacement manifest larger than this is refused (`loadRelease` in release-config.mjs).
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub schema_version: u32,
    pub revision: u64,
    pub channel: String,
    pub protocol_version: String,
    pub index_version: u32,
    pub supported_protocols: Vec<String>,
    pub notice: Notice,
}

const INVALID: &str = "Invalid release configuration. Expected schema 1 with a channel, revision, protocol/index versions, supported protocols and notice.";

fn is_version(value: &str) -> bool {
    let ok = |b: u8| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-';
    !value.is_empty() && value.len() <= 64 && value.as_bytes()[0].is_ascii_alphanumeric() && value.bytes().all(ok)
}

fn is_text(value: Option<&Value>, max: usize) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= max)
}

impl Release {
    /// Keeps exactly the fields the schema names; anything else in the file is dropped.
    pub fn validate(value: &Value) -> Result<Release, String> {
        let invalid = || INVALID.to_string();
        let object = value.as_object().ok_or_else(invalid)?;
        if object.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
            return Err(invalid());
        }
        let revision = object.get("revision").and_then(Value::as_u64).filter(|r| *r > 0).ok_or_else(invalid)?;
        let channel = object.get("channel").and_then(Value::as_str).filter(|c| matches!(*c, "preview" | "stable")).ok_or_else(invalid)?;
        let protocol_version = object.get("protocolVersion").and_then(Value::as_str).filter(|v| is_version(v)).ok_or_else(invalid)?;
        let index_version = object.get("indexVersion").and_then(Value::as_u64).filter(|v| *v > 0 && *v <= i32::MAX as u64).ok_or_else(invalid)?;
        let supported: Vec<String> = object
            .get("supportedProtocols")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty() && a.len() <= 16)
            .ok_or_else(invalid)?
            .iter()
            .map(|v| v.as_str().filter(|s| is_version(s)).map(String::from).ok_or_else(invalid))
            .collect::<Result<_, _>>()?;
        let distinct: std::collections::BTreeSet<&String> = supported.iter().collect();
        if distinct.len() != supported.len() || !supported.iter().any(|s| s == protocol_version) {
            return Err(invalid());
        }
        let notice = object.get("notice").and_then(Value::as_object).ok_or_else(invalid)?;
        if !is_text(notice.get("title"), 100) || !is_text(notice.get("message"), 1200) {
            return Err(invalid());
        }
        Ok(Release {
            schema_version: 1,
            revision,
            channel: channel.to_string(),
            protocol_version: protocol_version.to_string(),
            index_version: index_version as u32,
            supported_protocols: supported,
            notice: Notice {
                title: notice["title"].as_str().unwrap_or_default().to_string(),
                message: notice["message"].as_str().unwrap_or_default().to_string(),
            },
        })
    }

    /// The manifest compiled into this build.
    pub fn compiled() -> Result<Release, String> {
        let value: Value = serde_json::from_str(hoi_core::release_manifest()).map_err(|e| e.to_string())?;
        Release::validate(&value)
    }

    fn from_file(path: &Path) -> Result<Release, String> {
        let size = std::fs::metadata(path).map_err(|e| format!("release manifest: {e}"))?.len();
        if size > MAX_MANIFEST_BYTES {
            return Err("Release configuration exceeds 16 KiB.".into());
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("release manifest: {e}"))?;
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("release manifest: {e}"))?;
        Release::validate(&value)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Where a registry reads its manifest: compiled in, or a file read on each request.
#[derive(Debug, Clone)]
pub enum ReleaseSource {
    Compiled(Release),
    File(PathBuf),
}

impl ReleaseSource {
    pub fn compiled() -> Result<ReleaseSource, String> {
        Release::compiled().map(ReleaseSource::Compiled)
    }

    /// Fails at startup when the file is unusable, so a bad deployment stops before it listens.
    pub fn file(path: PathBuf) -> Result<ReleaseSource, String> {
        Release::from_file(&path)?;
        Ok(ReleaseSource::File(path))
    }

    pub fn load(&self) -> Result<Release, String> {
        match self {
            ReleaseSource::Compiled(release) => Ok(release.clone()),
            ReleaseSource::File(path) => Release::from_file(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_compiled_manifest_is_valid_and_matches_the_core() {
        let release = Release::compiled().unwrap();
        let (protocol, index) = hoi_core::release_versions();
        assert_eq!((release.protocol_version.as_str(), release.index_version), (protocol.as_str(), index));
        assert!(release.supported_protocols.contains(&protocol));
        assert_eq!(release.to_json()["schemaVersion"], 1);
    }

    #[test]
    fn invalid_manifests_fail_closed() {
        let good = json!({
            "schemaVersion": 1, "revision": 2, "channel": "preview", "protocolVersion": "0.3", "indexVersion": 1,
            "supportedProtocols": ["0.3"], "notice": { "title": "Preview", "message": "Can lose data." }
        });
        assert!(Release::validate(&good).is_ok());
        for (field, value) in [
            ("schemaVersion", json!(2)),
            ("revision", json!(0)),
            ("channel", json!("beta")),
            ("protocolVersion", json!("0.4")),
            ("protocolVersion", json!("-0.3")),
            ("indexVersion", json!(0)),
            ("indexVersion", json!(2_147_483_648u64)),
            ("supportedProtocols", json!([])),
            ("supportedProtocols", json!(["0.3", "0.3"])),
            ("supportedProtocols", json!(["0.2"])),
            ("notice", json!({ "title": " ", "message": "x" })),
            ("notice", json!({ "title": "x", "message": "m".repeat(1201) })),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(Release::validate(&bad).is_err(), "{field}");
        }
        let mut extra = good.clone();
        extra["operator"] = json!("dropped");
        assert!(Release::validate(&extra).unwrap().to_json().get("operator").is_none());
        let dir = std::env::temp_dir();
        let path = dir.join(format!("hoi-release-{}.json", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
        std::fs::write(&path, "x".repeat(17 * 1024)).unwrap();
        assert!(ReleaseSource::file(path.clone()).unwrap_err().contains("16 KiB"));
        std::fs::write(&path, serde_json::to_string(&good).unwrap()).unwrap();
        let source = ReleaseSource::file(path.clone()).unwrap();
        assert_eq!(source.load().unwrap().revision, 2);
        std::fs::write(&path, "{").unwrap();
        assert!(source.load().is_err(), "a broken replacement is refused on the next request, not served");
        std::fs::remove_file(path).unwrap();
    }
}
