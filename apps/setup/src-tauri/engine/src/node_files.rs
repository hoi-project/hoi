//! The hoi-node this computer can give a server, so a node installs without a published release:
//! release archives named `hoi-node_<version>_<rust target>.tar.gz`, as `pnpm build:all` puts
//! them in `builds/hoi-<version>/server/` and `scripts/release-local.mjs` in its release folder.
//! The app looks beside itself, in `../server/` (the portable app kept in `builds/…/node-setup/`),
//! and in a development build in the checkout's `builds/*/server/`; any other file is picked.
//! A file is read here, never through the window, and only when its name is such an archive.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ssh::SetupError;

/// A hoi-node archive is a few tens of megabytes; this bounds what is read into memory.
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;

/// The targets hoi-node is released for (core/crates/hoi-node, .github/workflows/release.yml).
pub const TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

/// One archive found or picked on this computer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NodeFile {
    pub path: String,
    pub name: String,
    pub version: String,
    pub target: String,
    pub bytes: u64,
}

/// The version and target in `hoi-node_<version>_<target>.tar.gz`, or None for any other name.
pub fn parse_name(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("hoi-node_")?.strip_suffix(".tar.gz")?;
    // The target has underscores of its own (x86_64), so it is matched whole from the end.
    let (version, target) = TARGETS.iter().find_map(|t| rest.strip_suffix(t)?.strip_suffix('_').map(|v| (v, *t)))?;
    let version_ok = !version.is_empty() && version.len() <= 64 && version.starts_with(|c: char| c.is_ascii_digit()) && version.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c));
    version_ok.then(|| (version.to_string(), target.to_string()))
}

/// The archive at `path`, when its name is a hoi-node archive's and it is a file of a sensible size.
pub fn node_file(path: &Path) -> Result<NodeFile, SetupError> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let Some((version, target)) = parse_name(name) else {
        return Err(SetupError::Upload(format!("{name} is not a hoi-node archive: the name is hoi-node_<version>_<target>.tar.gz, as pnpm build:all makes it")));
    };
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(SetupError::Upload(format!("{name} is not a file")));
    }
    if meta.len() == 0 || meta.len() > MAX_BYTES {
        return Err(SetupError::Upload(format!("{name} is {} bytes, not a hoi-node archive", meta.len())));
    }
    Ok(NodeFile { path: path.to_string_lossy().into_owned(), name: name.to_string(), version, target, bytes: meta.len() })
}

/// The archive's bytes, read once and checked against what was found: what goes to the server.
pub fn read(path: &Path) -> Result<(NodeFile, Vec<u8>), SetupError> {
    let file = node_file(path)?;
    let bytes = std::fs::read(path)?;
    if bytes.len() as u64 != file.bytes {
        return Err(SetupError::Upload(format!("{} changed while it was read; try again", file.name)));
    }
    Ok((file, bytes))
}

/// Where to look, in order: beside the app, `../server` and `server` next to it (the builds/
/// layout), then each `builds/*/server`, newest first, when a checkout's builds folder is given.
pub fn search_dirs(exe: Option<&Path>, builds: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(here) = exe.and_then(Path::parent) {
        dirs.push(here.to_path_buf());
        if let Some(up) = here.parent() {
            dirs.push(up.join("server"));
        }
        dirs.push(here.join("server"));
    }
    if let Some(builds) = builds
        && let Ok(entries) = std::fs::read_dir(builds)
    {
        let mut versions: Vec<(std::time::SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .map(|e| e.path().join("server"))
            .filter(|p| p.is_dir())
            .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
            .collect();
        versions.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        dirs.extend(versions.into_iter().map(|(_, p)| p));
    }
    dirs
}

/// Every hoi-node archive in `dirs` (not their subfolders), in the order of `dirs`, each once.
pub fn find(dirs: &[PathBuf]) -> Vec<NodeFile> {
    let mut found: Vec<NodeFile> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut names: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        names.sort();
        for path in names {
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if let Ok(file) = node_file(&path)
                && seen.insert(key)
            {
                found.push(file);
            }
        }
    }
    found
}

/// SHA-256 as `sha256sum` prints it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `dir` is a private folder the wizard made on the server (`mktemp -d /tmp/hoi-setup.XXXXXXXX`).
pub fn is_upload_dir(dir: &str) -> bool {
    dir.strip_prefix("/tmp/hoi-setup.").is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoi-node-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn archive_names_give_version_and_target() {
        assert_eq!(parse_name("hoi-node_1.0.0-preview.1_x86_64-unknown-linux-gnu.tar.gz"), Some(("1.0.0-preview.1".into(), "x86_64-unknown-linux-gnu".into())));
        assert_eq!(parse_name("hoi-node_0.2.0_aarch64-unknown-linux-gnu.tar.gz"), Some(("0.2.0".into(), "aarch64-unknown-linux-gnu".into())));
        for name in [
            "hoi-node_1.0.0_x86_64-unknown-linux-musl.tar.gz",
            "hoi-node_1.0.0_x86_64-unknown-linux-gnu.zip",
            "hoi-server_1.0.0_linux-x64.tar.gz",
            "hoi-node__x86_64-unknown-linux-gnu.tar.gz",
            "hoi-node_v1_x86_64-unknown-linux-gnu.tar.gz",
            "hoi-node_1.0.0;rm -rf_x86_64-unknown-linux-gnu.tar.gz",
            "hoi-node",
        ] {
            assert_eq!(parse_name(name), None, "{name}");
        }
    }

    #[test]
    fn only_archives_of_a_sensible_size_are_offered() {
        let dir = temp("size");
        let good = dir.join("hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz");
        std::fs::write(&good, b"archive").unwrap();
        let file = node_file(&good).unwrap();
        assert_eq!((file.version.as_str(), file.target.as_str(), file.bytes), ("1.0.0", "x86_64-unknown-linux-gnu", 7));
        let (_, bytes) = read(&good).unwrap();
        assert_eq!(bytes, b"archive");
        let empty = dir.join("hoi-node_1.0.0_aarch64-unknown-linux-gnu.tar.gz");
        std::fs::write(&empty, b"").unwrap();
        assert!(node_file(&empty).is_err());
        let other = dir.join("id_ed25519");
        std::fs::write(&other, b"a key").unwrap();
        assert!(matches!(node_file(&other), Err(SetupError::Upload(_))));
        std::fs::create_dir(dir.join("hoi-node_2.0.0_x86_64-unknown-linux-gnu.tar.gz")).unwrap();
        assert!(node_file(&dir.join("hoi-node_2.0.0_x86_64-unknown-linux-gnu.tar.gz")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_portable_app_finds_the_server_folder_beside_its_own() {
        let root = temp("layout");
        let version = root.join("builds").join("hoi-1.0.0");
        std::fs::create_dir_all(version.join("node-setup")).unwrap();
        std::fs::create_dir_all(version.join("server")).unwrap();
        std::fs::write(version.join("server/hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz"), b"x").unwrap();
        std::fs::write(version.join("server/hoi-node_1.0.0_aarch64-unknown-linux-gnu.tar.gz"), b"a").unwrap();
        std::fs::write(version.join("server/hoi-server_1.0.0_linux-x64.tar.gz"), b"bundle").unwrap();
        let exe = version.join("node-setup/hoi-node-setup_1.0.0_windows-x64-portable.exe");
        let dirs = search_dirs(Some(&exe), Some(&root.join("builds")));
        assert_eq!(dirs[0], version.join("node-setup"));
        assert_eq!(dirs[1], version.join("server"));
        let found = find(&dirs);
        // Each once, though builds/*/server names the same folder again.
        assert_eq!(found.iter().map(|f| f.target.as_str()).collect::<Vec<_>>(), ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"]);
        assert!(find(&[root.join("missing")]).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hashes_match_sha256sum() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn uploads_go_only_into_the_wizards_private_folder() {
        assert!(is_upload_dir("/tmp/hoi-setup.Ab12cD34"));
        for dir in ["/tmp/hoi-setup.", "/tmp/hoi-setup.a/../../etc", "/tmp/other", "/tmp/hoi-setup.a b", "/etc"] {
            assert!(!is_upload_dir(dir), "{dir}");
        }
    }
}
