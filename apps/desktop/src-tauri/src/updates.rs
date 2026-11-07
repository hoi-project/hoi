//! Explicit update checks and installation, using static signed metadata. No local server.
use crate::release_verification as verify;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct Updates(pub tokio::sync::Mutex<Option<Pending>>);
pub struct Pending {
    update: Update,
    size: u64,
    hash: String,
    bytes: Option<Vec<u8>>,
}

fn config() -> Value {
    serde_json::from_str(include_str!("../../../../publication.json")).expect("publication config")
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| "Could not start the update download.".into())
}
async fn fetch(url: &str, max: u64, progress: impl Fn(u64)) -> Result<Vec<u8>, String> {
    let mut response = client()?
        .get(url)
        .send()
        .await
        .map_err(|_| "Could not reach the release host. Check your connection and retry.")?
        .error_for_status()
        .map_err(|_| "The release host did not provide this file.")?;
    if response.content_length().is_some_and(|n| n > max) {
        return Err("Release file is too large.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "The download was interrupted. Retry; the installed app is unchanged.")? {
        if bytes.len() as u64 + chunk.len() as u64 > max {
            return Err("Release file is too large.".into());
        }
        bytes.extend_from_slice(&chunk);
        progress(bytes.len() as u64);
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn hoi_update_check(app: AppHandle, state: State<'_, Updates>) -> Result<Value, String> {
    let mut slot = state.0.lock().await;
    *slot = None;
    let cfg = config();
    let base = cfg["downloads"].as_str().unwrap_or("");
    let key = cfg["manifestKey"].as_str().unwrap_or("");
    let updater_key = cfg["updaterKey"].as_str().unwrap_or("");
    if base.is_empty() || key.is_empty() || updater_key.is_empty() {
        return Ok(json!({"state":"unavailable", "message":"Updates are unavailable in this development build."}));
    }
    let installed = app.package_info().version.to_string();
    let path = app.path().app_data_dir().map_err(|_| "Local update settings are unavailable.")?.join("release-seen.json");
    let seen: Value = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or(Value::Null);
    let build: Value = serde_json::from_str(include_str!("../../../../release.json")).expect("release config");
    let mut minimum = installed.clone();
    if let Some(v) = seen["version"].as_str() {
        if let Ok(v) = semver::Version::parse(v) {
            if v > app.package_info().version {
                minimum = v.to_string();
            }
        }
    }
    let revision = seen["revision"].as_u64().unwrap_or(0).max(build["revision"].as_u64().unwrap_or(1));
    let url = format!("{}/manifest.json", base.trim_end_matches('/'));
    let bytes = fetch(&url, verify::MAX_MANIFEST as u64, |_| {}).await?;
    let sig = fetch(&format!("{url}.minisig"), 4096, |_| {}).await?;
    let manifest = verify::verify(&bytes, std::str::from_utf8(&sig).map_err(|_| "Invalid release signature.")?, key, &minimum, revision)?;
    if build["channel"] == "stable" && manifest["release"]["channel"] != "stable" {
        return Err("A preview update was rejected by this stable build.".into());
    }
    let updater = app
        .updater_builder()
        .pubkey(STANDARD.encode(format!("untrusted comment: hoi updater public key\n{updater_key}\n")))
        .build()
        .map_err(|_| "Updates are not supported by this installation. Use the verified download page.")?;
    let update = updater.restore_update(manifest["updater"].clone()).map_err(|_| "This release has no compatible update for this installation.")?;
    if let Some(update) = update {
        let (size, hash) = verify::artifact(&manifest, update.download_url.as_str(), std::env::consts::OS, std::env::consts::ARCH)?;
        *slot = Some(Pending { update, size, hash, bytes: None });
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "Could not save update verification state.")?;
    }
    std::fs::write(path, json!({"version":manifest["latest"],"revision":manifest["release"]["revision"]}).to_string())
        .map_err(|_| "Could not save update verification state.")?;
    Ok(json!({"state":if slot.is_some() {"available"} else {"current"}, "version":manifest["latest"], "message":manifest["release"]["notice"]["message"]}))
}

#[tauri::command]
pub async fn hoi_update_download(app: AppHandle, state: State<'_, Updates>) -> Result<(), String> {
    let mut slot = state.0.lock().await;
    let pending = slot.as_mut().ok_or("Check for updates first.")?;
    pending.bytes = None;
    let bytes = fetch(pending.update.download_url.as_str(), pending.size, |downloaded| {
        let _ = app.emit("hoi:update-progress", json!({"downloaded":downloaded,"total":pending.size}));
    })
    .await?;
    verify::verify_artifact(&bytes, pending.size, &pending.hash)?;
    pending.update.verify(&bytes).map_err(|_| "The update package signature or signed version did not verify. Nothing was installed.")?;
    pending.bytes = Some(bytes);
    Ok(())
}

#[tauri::command]
pub async fn hoi_update_install(app: AppHandle, state: State<'_, Updates>, startup: State<'_, crate::VaultStartup>) -> Result<(), String> {
    let _startup = startup.0.lock().await;
    let mut slot = state.0.lock().await;
    let pending = slot.as_ref().ok_or("Download and verify the update first.")?;
    let bytes = pending.bytes.as_ref().ok_or("Download and verify the update first.")?;
    verify::verify_artifact(bytes, pending.size, &pending.hash)?;
    // Commit while holding the same mutex all transports need. No new pending writes may
    // appear between the flush and installer exit / restart. Errors leave this process usable.
    let install = || -> Result<(), String> {
        pending.update.install(bytes).map_err(|_| "The update could not be installed. Keep hoi open and retry, or use the verified installer.")?;
        app.restart();
    };
    if let Some(hoi) = app.try_state::<crate::Hoi>() {
        hoi.shared.with(|node| {
            node.commit().map_err(|_| "Changes could not be saved. Restore storage access before updating.")?;
            install()
        })?;
    } else {
        install()?;
    }
    *slot = None;
    Ok(())
}
