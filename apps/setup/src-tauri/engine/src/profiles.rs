//! Servers the person set up, remembered between starts: how to reach them and the host key
//! that was pinned, never a password or a key's passphrase. One JSON file in the app's own
//! configuration folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AuthKind {
    Agent,
    Key { path: String },
    /// Asked for each time; never stored.
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    /// What the person calls it: the node's name by default.
    pub label: String,
    /// `local` for this computer; otherwise the SSH host.
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default = "default_auth")]
    pub auth: AuthKind,
    /// The host key pinned on the first connection (`SHA256:…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key: Option<String>,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice_hash: Option<String>,
    /// The configuration and the notice as last installed, so Manage can reinstall or edit.
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub notice: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub last_used: u64,
}

fn default_port() -> u16 {
    22
}
fn default_auth() -> AuthKind {
    AuthKind::Agent
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Profiles {
    #[serde(default)]
    pub servers: Vec<Profile>,
}

impl Profiles {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("servers.json")
    }

    pub fn load(dir: &Path) -> Profiles {
        std::fs::read_to_string(Self::path(dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join("servers.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, Self::path(dir))
    }

    /// Adds or replaces (by id), newest first.
    pub fn upsert(&mut self, profile: Profile) {
        self.servers.retain(|p| p.id != profile.id);
        self.servers.insert(0, profile);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.servers.len();
        self.servers.retain(|p| p.id != id);
        self.servers.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_round_trip_without_secrets() {
        let dir = std::env::temp_dir().join(format!("hoi-setup-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut profiles = Profiles::load(&dir);
        assert!(profiles.servers.is_empty());
        profiles.upsert(Profile {
            id: "a".into(),
            label: "sleepy-walrus".into(),
            host: "203.0.113.9".into(),
            port: 22,
            user: "root".into(),
            auth: AuthKind::Key { path: "/home/lena/.ssh/id_ed25519".into() },
            host_key: Some("SHA256:abc".into()),
            role: "public".into(),
            name: "sleepy-walrus".into(),
            node_link: None,
            notice_hash: None,
            config: "role=public\n".into(),
            notice: "{}".into(),
            created: 1,
            last_used: 1,
        });
        profiles.save(&dir).unwrap();
        let text = std::fs::read_to_string(Profiles::path(&dir)).unwrap();
        assert!(text.contains("\"hostKey\": \"SHA256:abc\"") && !text.to_lowercase().contains("passphrase"));
        let back = Profiles::load(&dir);
        assert_eq!(back.servers, profiles.servers);
        let mut again = back;
        again.upsert(Profile { label: "renamed".into(), ..again.servers[0].clone() });
        assert_eq!(again.servers.len(), 1);
        assert_eq!(again.servers[0].label, "renamed");
        assert!(again.remove("a") && !again.remove("a"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
