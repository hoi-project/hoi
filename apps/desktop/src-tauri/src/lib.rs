//! The hoi desktop app: the same UI as the web app (apps/web, built with HOI_TARGET=desktop) over
//! the native core. Two faces: the full app, or the companion, a small window and a tray icon with
//! status, traffic and pairing, no forum and no key needed (Settings → Open as companion). Either
//! way it is a full node: it keeps your communities in SQLite, mines stamps on every core, talks
//! to members' nodes over iroh using routes from community invites (no default relays or
//! discovery services), serves your browser on this computer over a loopback WebSocket and your
//! phone over WebRTC on your network, and keeps your communities reachable while it runs. With
//! *Be a public node* on it serves strangers too: it carries open communities for whoever asks,
//! tells whoever connects about the other public nodes it reached, and asks the router to forward
//! its ports so other networks can reach it. Off, it publishes no address of its own.
//! A public node may also keep mailboxes for others' private communities, encrypted, within a
//! budget the person picks (SCATTER.md §5.7, `hoi_net::sync_piece_store`).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hoi_core::Node;
use hoi_net::Shared;
use serde_json::{Value, json};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;

mod os_unlock;
mod release_verification;
mod updates;

/// Paired devices dial this port on the same machine.
const HOME_PORT: u16 = 7842;
/// Window sizes of the two faces: the companion (the default) and the full app.
const COMPANION_SIZE: (f64, f64) = (560.0, 860.0);
const FULL_SIZE: (f64, f64) = (1240.0, 860.0);
/// Started by the system at login: stay in the tray, no window.
const HIDDEN: &str = "--hidden";
/// Asked by an installer (deploy/install): start hoi at login from now on, as the companion's
/// switch does, with the path the autostart plugin knows for this installation.
const START_AT_LOGIN: &str = "--start-at-login";
/// The tray icon is up: closing the window hides it instead of quitting.
static IN_TRAY: AtomicBool = AtomicBool::new(false);

type Slot<T> = Arc<tokio::sync::Mutex<Option<T>>>;

struct Hoi {
    shared: Arc<Shared>,
    p2p: Slot<hoi_net::P2pHandle>,
    rtc: Slot<hoi_net::RtcHandle>,
    /// Stop flag of the link to *our* home node, if this computer is paired with another one.
    home_link: Mutex<Option<Arc<AtomicBool>>>,
    /// The router forwarding this computer's ports while it is a public node.
    forwarding: Slot<Forwarding>,
    /// The DHT among public nodes, run while this computer is one.
    dht: Slot<hoi_net::dht::DhtHandle>,
    /// LAN beacons: while not "Only through my VPN".
    lan: Slot<hoi_net::lan::LanHandle>,
    /// The networks the transports were started for, and the VPN as it was then.
    applied: tokio::sync::Mutex<(hoi_net::vpn::NetPolicy, hoi_net::vpn::VpnState)>,
}

/// A running port mapper and the task that puts what the router forwards into the routes.
struct Forwarding {
    mapper: hoi_net::portmap::PortMapper,
    advertiser: tauri::async_runtime::JoinHandle<()>,
    /// Whether what the router forwards goes into the routes: not while the person only looks
    /// at which address the router would forward from, before turning on *Be a public node*.
    publish: tokio::sync::watch::Sender<bool>,
    previewing: bool,
}

struct VaultStartup(tokio::sync::Mutex<()>);

#[tauri::command]
async fn hoi_vault_status(app: AppHandle, startup: State<'_, VaultStartup>) -> Result<Value, Value> {
    let _guard = startup.0.lock().await;
    if app.try_state::<Hoi>().is_some() {
        return Ok(json!({ "state": "ready" }));
    }
    let db = storage(&app).map_err(internal)?;
    let state = if hoi_store::vault::EncryptedSqlite::header(&db).map_err(internal)?.is_some() {
        "locked"
    } else if db.snapshot().map_err(internal)?.is_empty() {
        "create"
    } else {
        "migrate"
    };
    Ok(json!({ "state": state }))
}

#[tauri::command]
async fn hoi_vault_unlock(app: AppHandle, startup: State<'_, VaultStartup>, mut passphrase: String) -> Result<(), Value> {
    let _guard = startup.0.lock().await;
    if app.try_state::<Hoi>().is_some() {
        hoi_crypto::vault::clear_passphrase(&mut passphrase);
        return Ok(());
    }
    let handle = app.clone();
    let node = tauri::async_runtime::spawn_blocking(move || {
        let result = open_node(&handle, &passphrase);
        hoi_crypto::vault::clear_passphrase(&mut passphrase);
        result
    })
    .await
    .map_err(internal)??;
    start_unlocked(&app, node);
    Ok(())
}

#[tauri::command]
async fn hoi_os_unlock_status(app: AppHandle, startup: State<'_, VaultStartup>) -> Result<Value, Value> {
    let _guard = startup.0.lock().await;
    let directory = app.path().app_data_dir().map_err(internal)?;
    let header = hoi_store::vault::EncryptedSqlite::header(&storage(&app).map_err(internal)?).map_err(internal)?;
    Ok(json!({
        "enabled": header.is_some_and(|header| os_unlock::enabled(&directory, &header)),
        "configured": os_unlock::configured(&directory)
    }))
}

#[tauri::command]
async fn hoi_os_unlock_enable(app: AppHandle, startup: State<'_, VaultStartup>, passphrase: String) -> Result<(), Value> {
    // Zeroize even if checking the runtime or acquiring storage fails.
    let passphrase = zeroize::Zeroizing::new(passphrase);
    let _guard = startup.0.lock().await;
    let shared = app.try_state::<Hoi>().ok_or_else(|| internal("Unlock hoi first."))?.shared.clone();
    tauri::async_runtime::spawn_blocking(move || {
        shared.with(|node| {
            node.commit().map_err(err)?;
            let directory = app.path().app_data_dir().map_err(internal)?;
            let db = storage(&app).map_err(saved_data_error)?;
            let header = hoi_store::vault::EncryptedSqlite::header(&db).map_err(saved_data_error)?.ok_or_else(|| internal("Protect this device first."))?;
            let vault = hoi_crypto::vault::LocalVault::unlock(&header, &passphrase)
                .map_err(|_| json!({ "code": "bad-passphrase", "message": "The vault password did not match. OS unlock was not enabled." }))?;
            // Authenticate all stored cells too, before exporting its data key to the OS.
            let secret = vault.os_unlock_secret().map_err(internal)?;
            let check = hoi_crypto::vault::LocalVault::unlock_os_secret(&header, &secret).map_err(internal)?;
            hoi_store::vault::EncryptedSqlite::open(db, check, &[]).map_err(saved_data_error)?;
            os_unlock::enable(&directory, &os_unlock::entry(&directory).map_err(internal)?, &vault).map_err(internal)
        })
    })
    .await
    .map_err(internal)?
}

#[tauri::command]
async fn hoi_os_unlock_disable(app: AppHandle, startup: State<'_, VaultStartup>) -> Result<(), Value> {
    let _guard = startup.0.lock().await;
    if app.try_state::<Hoi>().is_none() {
        return Err(internal("Unlock hoi first."));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let directory = app.path().app_data_dir().map_err(internal)?;
        os_unlock::disable(&directory, &os_unlock::entry(&directory).map_err(internal)?).map_err(internal)
    })
    .await
    .map_err(internal)?
}

#[tauri::command]
async fn hoi_os_unlock(app: AppHandle, startup: State<'_, VaultStartup>) -> Result<(), Value> {
    let _guard = startup.0.lock().await;
    if app.try_state::<Hoi>().is_some() {
        return Ok(());
    }
    let handle = app.clone();
    let node = tauri::async_runtime::spawn_blocking(move || {
        let directory = handle.path().app_data_dir().map_err(internal)?;
        let db = storage(&handle).map_err(saved_data_error)?;
        let header = hoi_store::vault::EncryptedSqlite::header(&db).map_err(saved_data_error)?.ok_or_else(|| internal("Use your vault password."))?;
        if !os_unlock::enabled(&directory, &header) {
            return Err(internal("OS unlock is not enabled for this vault. Use your vault password."));
        }
        let secret = os_unlock::read(&os_unlock::entry(&directory).map_err(internal)?).map_err(internal)?;
        let mut node = open_os_vault_node(db, &secret)?;
        node.set_node_name(hostname());
        Ok(node)
    })
    .await
    .map_err(internal)??;
    start_unlocked(&app, node);
    Ok(())
}

/// Recovery is available without opening the encrypted node. It never bypasses unlock.
#[tauri::command]
async fn hoi_vault_reset(app: AppHandle, startup: State<'_, VaultStartup>, confirmation: String) -> Result<(), Value> {
    let _guard = startup.0.lock().await;
    let directory = app.path().app_data_dir().map_err(internal)?;
    if confirmation != "RESET" || app.try_state::<Hoi>().is_some() {
        return Err(internal("Lock hoi and type RESET to confirm clearing local data."));
    }
    // Cleanup must succeed before deleting the vault. Keep cancellation retryable.
    tauri::async_runtime::spawn_blocking(move || {
        if os_unlock::configured(&directory) {
            os_unlock::disable(&directory, &os_unlock::entry(&directory).map_err(internal)?).map_err(internal)?;
        }
        reset_locked_storage(&directory, &confirmation, false).map_err(internal)
    })
    .await
    .map_err(internal)?
}

fn reset_locked_storage(directory: &std::path::Path, confirmation: &str, unlocked: bool) -> Result<(), String> {
    if confirmation != "RESET" {
        return Err("Type RESET to confirm clearing local data.".into());
    }
    if unlocked {
        return Err("Lock hoi before using recovery.".into());
    }
    // Only the database and its exact SQLite sidecars belong to recovery. A malformed
    // database must also be recoverable, so do not require opening or parsing its schema.
    for name in ["hoi.db", "hoi.db-wal", "hoi.db-shm", "hoi.db-journal"] {
        match std::fs::remove_file(directory.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Could not clear local storage. Close other hoi processes and check storage access: {error}"));
            }
        }
    }
    Ok(())
}

/// Restarting the process terminates every task that owns an unlocked node. Hiding/reloading
/// the webview alone would leave keys reachable by background transports and the tray.
#[tauri::command]
fn hoi_vault_lock(app: AppHandle, hoi: State<'_, Hoi>) -> Result<(), Value> {
    tauri::async_runtime::block_on(stop_forwarding(&hoi));
    flush_before_lock(&hoi.shared, || app.restart()).map_err(|_| {
        json!({
            "code": "internal", "message": "Changes could not be saved. Keep hoi open and restore storage access before locking."
        })
    })
}

fn flush_before_lock(shared: &Shared, locked: impl FnOnce()) -> Result<(), hoi_core::ApiError> {
    shared.with(|node| {
        node.commit()?;
        // Keep the node mutex held through process restart. No background transport can
        // create another pending write between the successful flush and termination.
        locked();
        Ok(())
    })
}

#[cfg(test)]
mod vault_lock_tests {
    use super::*;
    use hoi_store::{Change, Loaded, Persist};

    struct Failing(Arc<AtomicBool>);
    impl Persist for Failing {
        fn apply(&mut self, _: &[Change]) -> Result<(), String> {
            if self.0.load(Ordering::Relaxed) { Err("disk unavailable".into()) } else { Ok(()) }
        }
        fn load(&mut self) -> Result<Loaded, String> {
            Ok(Loaded::default())
        }
    }

    #[test]
    fn failed_flush_keeps_the_process_alive_until_storage_recovers() {
        let fail = Arc::new(AtomicBool::new(false));
        let node = Node::open(Box::new(Failing(fail.clone())), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).unwrap();
        let shared = Shared::new(node, Box::new(|_| {}));
        fail.store(true, Ordering::Relaxed);
        shared.with(|n| n.set_settings(&hoi_core::dto::SettingsPatch { relay: Some(false), ..Default::default() }).unwrap());
        let locked = AtomicBool::new(false);
        assert!(flush_before_lock(&shared, || locked.store(true, Ordering::Relaxed)).is_err());
        assert!(!locked.load(Ordering::Relaxed));
        fail.store(false, Ordering::Relaxed);
        flush_before_lock(&shared, || locked.store(true, Ordering::Relaxed)).unwrap();
        assert!(locked.load(Ordering::Relaxed));
    }

    #[test]
    fn only_web_links_open_in_the_browser() {
        for ok in ["https://hoi.sh/app/#/pair?t=x", "http://localhost:5173/", "http://127.0.0.1:7842/app/"] {
            assert!(web_link(ok), "{ok}");
        }
        for bad in
            ["http://localhost:1@evil.example/", "http://localhost/", "http://evil.example/", "file:///etc/passwd", "https://x.example/ a", "hoi://join?c=1"]
        {
            assert!(!web_link(bad), "{bad}");
        }
    }

    #[test]
    fn locked_recovery_requires_confirmation_and_only_removes_hoi_storage() {
        let directory = std::env::temp_dir().join(format!("hoi-recovery-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<16>().unwrap())));
        std::fs::create_dir(&directory).unwrap();
        for name in ["hoi.db", "hoi.db-wal", "hoi.db-shm", "hoi.db-journal", "keep.hoikey"] {
            std::fs::write(directory.join(name), b"test data, including an unreadable database").unwrap();
        }
        assert!(reset_locked_storage(&directory, "", false).is_err());
        assert!(reset_locked_storage(&directory, "RESET", true).is_err());
        assert!(directory.join("hoi.db").exists());
        reset_locked_storage(&directory, "RESET", false).unwrap();
        assert!(!directory.join("hoi.db").exists());
        assert!(!directory.join("hoi.db-wal").exists());
        assert!(!directory.join("hoi.db-shm").exists());
        assert!(!directory.join("hoi.db-journal").exists());
        assert!(directory.join("keep.hoikey").exists());
        reset_locked_storage(&directory, "RESET", false).unwrap();
        std::fs::remove_file(directory.join("keep.hoikey")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn password_reopens_native_identity_and_data_errors_are_not_password_errors() {
        let directory = std::env::temp_dir().join(format!("hoi-unlock-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<16>().unwrap())));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("hoi.db");
        let passphrase = "native unlock regression passphrase";
        let mut node = open_vault_node(hoi_store::sqlite::Sqlite::open(&path).unwrap(), passphrase).unwrap();
        node.set_identity_bits(4);
        let root = node.create_identity("unlock test").unwrap().root_key;
        node.commit().unwrap();
        drop(node);
        let header = hoi_store::vault::EncryptedSqlite::header(&hoi_store::sqlite::Sqlite::open(&path).unwrap()).unwrap().unwrap();
        let vault = hoi_crypto::vault::LocalVault::unlock(&header, passphrase).unwrap();
        let mut secret = vault.os_unlock_secret().unwrap();
        let reopened = open_os_vault_node(hoi_store::sqlite::Sqlite::open(&path).unwrap(), &secret).unwrap();
        assert_eq!(reopened.me().unwrap().root_key, root);
        drop(reopened);
        let before = hoi_store::sqlite::Sqlite::open(&path).unwrap().snapshot().unwrap();
        secret[0] ^= 1;
        assert!(open_os_vault_node(hoi_store::sqlite::Sqlite::open(&path).unwrap(), &secret).is_err());
        assert_eq!(hoi_store::sqlite::Sqlite::open(&path).unwrap().snapshot().unwrap(), before);
        let wrong = open_vault_node(hoi_store::sqlite::Sqlite::open(&path).unwrap(), "incorrect long passphrase").err().unwrap();
        assert_eq!(wrong["code"], "bad-passphrase");
        assert_eq!(hoi_store::sqlite::Sqlite::open(&path).unwrap().snapshot().unwrap(), before);
        let node = open_vault_node(hoi_store::sqlite::Sqlite::open(&path).unwrap(), passphrase).unwrap();
        assert_eq!(node.me().unwrap().root_key, root);
        drop(node);
        let mut raw = hoi_store::sqlite::Sqlite::open(&path).unwrap();
        raw.apply(&[Change::Blob { key: "v:keys".into(), value: Some(b"tampered ciphertext".to_vec()) }]).unwrap();
        drop(raw);
        let corrupt = open_vault_node(hoi_store::sqlite::Sqlite::open(&path).unwrap(), passphrase).err().unwrap();
        assert_eq!(corrupt["code"], "saved-data");
        reset_locked_storage(&directory, "RESET", false).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}

fn err(e: hoi_core::ApiError) -> Value {
    json!({ "code": e.code, "message": e.message })
}

fn internal(e: impl std::fmt::Display) -> Value {
    json!({ "code": "internal", "message": e.to_string() })
}

/// Every API call from the UI. Same JSON surface as the web worker (`Node::call`).
#[tauri::command]
async fn hoi_call(app: AppHandle, hoi: State<'_, Hoi>, method: String, args: Value) -> Result<Value, Value> {
    if method == "vpnInterfaces" {
        // The host's own call: what the person can pick for "Only through my VPN".
        let list: Vec<Value> =
            hoi_net::vpn::interfaces().into_iter().map(|i| json!({ "name": i.name, "label": i.label, "tunnel": i.tunnel, "addresses": i.addresses })).collect();
        return Ok(Value::Array(list));
    }
    if method == "previewPublicNode" {
        // The host's own call: the core doesn't talk to routers (HoiCore.previewPublicNode).
        sync_forwarding(&hoi, Some(args.get(0).and_then(Value::as_bool).unwrap_or(false))).await;
        return serde_json::to_value(hoi.shared.with(|n| n.net_status())).map_err(internal);
    }
    if method == "wipe" {
        // The router's forwards point at ports about to close.
        stop_forwarding(&hoi).await;
    }
    // Stop the transports before erasing their identities. Serialize with startup so an
    // endpoint can never publish its old address back into a freshly wiped node.
    let mut transports = if method == "wipe" { Some((hoi.p2p.lock().await, hoi.rtc.lock().await)) } else { None };
    if let Some((p2p, rtc)) = transports.as_mut() {
        if let Some(handle) = p2p.take() {
            handle.shutdown().await;
        }
        if let Some(handle) = rtc.take() {
            handle.shutdown().await;
        }
    }
    let out = hoi
        .shared
        .with(|n| {
            let serving = if method == "wipe" { n.net_status().serving.map(|s| s.url) } else { None };
            let result = n.call(&method, args)?;
            if let Some(url) = serving {
                n.set_serving(Some(&url))?;
            }
            Ok(result)
        })
        .map_err(err);
    let mut started = None;
    if let Some((mut p2p, mut rtc)) = transports {
        hoi.shared.close_all("client-");
        let policy = net_policy(&hoi.shared);
        let vpn = policy.state();
        *p2p = start_peers(hoi.shared.clone(), app.clone(), &vpn).await;
        *rtc = start_rtc(hoi.shared.clone(), app.clone(), policy.clone()).await;
        started = Some((policy, vpn));
    }
    // After the transports' locks are let go: the VPN watcher takes them in the other order.
    if let Some(applied) = started {
        *hoi.applied.lock().await = applied;
    }
    if method == "setSettings" {
        // "Only through my VPN" may have changed: the transports follow at once.
        follow_vpn(&hoi, &app).await;
    }
    if matches!(method.as_str(), "setSettings" | "wipe") {
        sync_forwarding(&hoi, None).await;
        sync_pieces(&app, &hoi.shared);
    }
    if matches!(method.as_str(), "pair" | "unpair" | "wipe") {
        restart_home_link(&hoi);
    }
    out
}

/// Mines a stamp on every core without holding the node.
#[tauri::command]
async fn hoi_mine(prefix: String, suffix: String, bits: u32) -> Result<u64, Value> {
    let bad = |_| json!({ "code": "invalid", "message": "bad puzzle" });
    let p = hoi_crypto::pow::Puzzle { prefix: hoi_types::uri::unb64(&prefix).map_err(bad)?, suffix: hoi_types::uri::unb64(&suffix).map_err(bad)?, bits };
    tauri::async_runtime::spawn_blocking(move || hoi_core::dispatch::mine_puzzle(&p)).await.map_err(internal)
}

/// More identity work for a key that has some (Settings → Key work), which can take minutes:
/// mined on every core without holding the node, so the rest of the app keeps answering. One
/// at a time; `hoi_identity_progress` reads it and `hoi_identity_stop` calls it off.
#[derive(Default)]
struct IdentityWork(Mutex<Option<Arc<hoi_crypto::identity::Watch>>>);

#[tauri::command]
async fn hoi_mine_identity(work: State<'_, IdentityWork>, root_key: String, bits: u32) -> Result<Option<u64>, Value> {
    let root = hoi_types::Id::from_hex(&root_key).map_err(|_| json!({ "code": "invalid", "message": "bad key" }))?;
    if !(hoi_crypto::identity::MIN_BITS..=hoi_crypto::identity::MAX_BITS).contains(&bits) {
        return Err(json!({ "code": "invalid", "message": "identity work out of range" }));
    }
    let watch = Arc::new(hoi_crypto::identity::Watch::default());
    if let Some(old) = work.0.lock().map_err(internal)?.replace(watch.clone()) {
        old.stop.store(true, Ordering::Relaxed);
    }
    let w = watch.clone();
    let found = tauri::async_runtime::spawn_blocking(move || hoi_crypto::identity::mine_watched(&root, bits, &w)).await.map_err(internal)?;
    let mut slot = work.0.lock().map_err(internal)?;
    if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &watch)) {
        *slot = None;
    }
    Ok(found)
}

#[tauri::command]
fn hoi_identity_progress(work: State<'_, IdentityWork>) -> Value {
    let slot = work.0.lock().ok().and_then(|s| s.clone());
    slot.map_or(Value::Null, |w| json!({ "tries": w.tries.load(Ordering::Relaxed), "best": w.best.load(Ordering::Relaxed) }))
}

#[tauri::command]
fn hoi_identity_stop(work: State<'_, IdentityWork>) {
    if let Some(w) = work.0.lock().ok().and_then(|s| s.clone()) {
        w.stop.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn hoi_benchmark() -> Value {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
    json!({ "hashesPerSecond": hoi_crypto::pow::benchmark(200_000) * threads })
}

/// Saves exported keys and bundles with a native dialog. Returns false if cancelled.
#[tauri::command]
async fn hoi_save_file(app: AppHandle, name: String, data: String) -> Result<bool, Value> {
    let bytes = hoi_types::uri::unb64(&data).map_err(|_| json!({ "code": "invalid", "message": "bad file data" }))?;
    let Some(path) = app.dialog().file().set_file_name(&name).blocking_save_file() else {
        return Ok(false);
    };
    let path = path.into_path().map_err(internal)?;
    std::fs::write(&path, bytes).map_err(internal)?;
    Ok(true)
}

/// Opens a web page in the person's own browser: the app's pairing link, hoi.sh. Nothing else
/// (no files, no other schemes), so the window can't use this to launch programs.
#[tauri::command]
fn hoi_open_url(url: String) -> Result<(), Value> {
    open_url(&url)
}

fn open_url(url: &str) -> Result<(), Value> {
    if !web_link(url) {
        return Err(json!({ "code": "invalid", "message": "Only web links open from here." }));
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(internal)
}

/// `https://` anywhere, or plain `http://` to this machine only: the authority is parsed, not
/// prefix-matched, so `http://localhost:1@evil.example/` is not a local link.
fn web_link(url: &str) -> bool {
    if url.len() > 4096 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p),
        None => return false,
    };
    (host == "localhost" || host == "127.0.0.1") && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
}

/// Whether hoi starts (in the tray, without a window) when you log in. `on` changes it.
#[tauri::command]
fn hoi_autostart(app: AppHandle, on: Option<bool>) -> Result<bool, Value> {
    let launcher = app.autolaunch();
    match on {
        Some(true) => launcher.enable().map_err(internal)?,
        Some(false) => launcher.disable().map_err(internal)?,
        None => {}
    }
    launcher.is_enabled().map_err(internal)
}

fn restart_home_link(hoi: &Hoi) {
    let mut slot = hoi.home_link.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(stop) = slot.take() {
        stop.store(true, Ordering::SeqCst);
    }
    hoi.shared.close_all("home-");
    let Some(home) = hoi.shared.with(|n| n.home()) else {
        return;
    };
    // A native home link speaks WebSocket; a WebRTC-only pairing link is for browsers and phones.
    if home.url.is_empty() {
        return;
    }
    // Only through a VPN: a home node on the internet waits until the VPN carries the traffic.
    let policy = net_policy(&hoi.shared);
    let nearby = url_ip(&home.url).is_some_and(|ip| ip.is_loopback() || (policy.home_network && hoi_net::vpn::home_network(ip)));
    if policy.vpn.is_some() && !nearby && !matches!(policy.state(), hoi_net::vpn::VpnState::Up(_)) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    tauri::async_runtime::spawn(hoi_net::dial(hoi.shared.clone(), home.url, Arc::new(move || !s2.load(Ordering::SeqCst))));
    *slot = Some(stop);
}

/// The address a `ws://` or `wss://` URL names, when it names one rather than a host name.
fn url_ip(url: &str) -> Option<std::net::IpAddr> {
    let host = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once(':').map_or(host, |(h, port)| if port.chars().all(|c| c.is_ascii_digit()) { h } else { host });
    host.trim_matches(['[', ']']).parse().ok()
}

/// Which networks the desktop may use, as the person set it.
fn net_policy(shared: &Shared) -> hoi_net::vpn::NetPolicy {
    shared.with(|n| {
        let s = n.settings();
        hoi_net::vpn::NetPolicy { vpn: (!s.vpn.is_empty()).then(|| s.vpn.clone()), home_network: s.home_network }
    })
}

/// Follows "Only through my VPN": run every few seconds and after each settings change. When
/// the VPN comes, goes or changes its address, the native transport starts again bound to it
/// (or stays stopped: the kill switch), WebRTC looks at its interfaces again, and the router's
/// forwards and the home link follow. A new setting restarts WebRTC with it.
async fn follow_vpn(hoi: &Hoi, app: &AppHandle) {
    let mut applied = hoi.applied.lock().await;
    let policy = net_policy(&hoi.shared);
    let vpn = policy.state();
    if applied.0 == policy && applied.1 == vpn {
        return;
    }
    let new_policy = applied.0 != policy;
    *applied = (policy.clone(), vpn.clone());
    hoi.shared.with(|n| n.set_vpn_status(policy.vpn.as_deref().and_then(|name| vpn.report(name))));
    stop_forwarding(hoi).await;
    {
        let mut p2p = hoi.p2p.lock().await;
        if let Some(h) = p2p.take() {
            h.shutdown().await;
        }
        *p2p = start_peers(hoi.shared.clone(), app.clone(), &vpn).await;
    }
    {
        let mut rtc = hoi.rtc.lock().await;
        if new_policy {
            if let Some(h) = rtc.take() {
                h.shutdown().await;
            }
            *rtc = start_rtc(hoi.shared.clone(), app.clone(), policy).await;
        } else if let Some(h) = rtc.as_ref() {
            h.rescan();
        }
    }
    drop(applied);
    sync_forwarding(hoi, None).await;
    restart_home_link(hoi);
}

/// Brings the window back (from the tray, or minimised), optionally on a screen of the app.
fn show_window(app: &AppHandle, hash: Option<&str>) {
    if let Some(w) = app.get_webview_window("main") {
        if let Some(h) = hash {
            let _ = w.eval(format!("location.hash = {}", serde_json::to_string(h).unwrap_or_default()));
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Shows the companion or the full app, in a window sized for it.
fn show_face(app: &AppHandle, full: bool) {
    if let Some(w) = app.get_webview_window("main") {
        size_for(&w, full);
    }
    show_window(app, Some(if full { "#/" } else { "#/companion" }));
}

/// The full app gets at least its size, the companion at most its own; a window someone already
/// sized to fit stays where and how it is.
fn size_for(w: &tauri::WebviewWindow, full: bool) {
    let (Ok(now), Ok(scale)) = (w.inner_size(), w.scale_factor()) else {
        return;
    };
    let now = now.to_logical::<f64>(scale);
    let (width, height) = if full { FULL_SIZE } else { COMPANION_SIZE };
    if (full && now.width < width) || (!full && now.width > width) {
        let _ = w.set_size(tauri::LogicalSize::new(width, height));
        let _ = w.center();
    }
}

/// The UI switched faces: size the window for it.
#[tauri::command]
fn hoi_face(app: AppHandle, full: bool) {
    if let Some(w) = app.get_webview_window("main") {
        size_for(&w, full);
    }
}

/// `hoi://join?…` opens the join screen; `hoi://pair?…` the pairing screen.
fn open_link(app: &AppHandle, url: &str) {
    let hash = if let Some(q) = url.strip_prefix("hoi://join?").or_else(|| url.strip_prefix("hoi://join/?")) {
        format!("#/join?{q}")
    } else if let Some(q) = url.strip_prefix("hoi://pair?").or_else(|| url.strip_prefix("hoi://pair/?")) {
        format!("#/pair?{q}")
    } else {
        return;
    };
    show_window(app, Some(&hash));
}

/// Mailboxes and pieces kept for others, while the person has *Keep mailboxes for others* on
/// (a public node only): their own file beside the database, deleted when it goes off.
fn sync_pieces(app: &AppHandle, shared: &Shared) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    if let Err(e) = hoi_net::sync_piece_store(shared, &dir.join("pieces.db")) {
        shared.with(|n| n.set_net_error(Some(e)));
    }
}

fn storage(app: &AppHandle) -> Result<hoi_store::sqlite::Sqlite, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    hoi_store::sqlite::Sqlite::open(&dir.join("hoi.db"))
}

fn saved_data_error(error: impl std::fmt::Display) -> Value {
    json!({ "code": "saved-data", "message": format!("Saved data could not be opened. Check free storage and close other hoi processes. No replacement key was created. Details: {error}") })
}

fn open_node(app: &AppHandle, passphrase: &str) -> Result<Node, Value> {
    let mut node = open_vault_node(storage(app).map_err(saved_data_error)?, passphrase)?;
    // Without a key here (the companion), your phone and browser call it by the computer's name.
    node.set_node_name(hostname());
    Ok(node)
}

fn open_vault_node(mut db: hoi_store::sqlite::Sqlite, passphrase: &str) -> Result<Node, Value> {
    use hoi_store::Persist;
    let vault = match hoi_store::vault::EncryptedSqlite::header(&db).map_err(saved_data_error)? {
        Some(header) => hoi_crypto::vault::LocalVault::unlock(&header, passphrase)
            .map_err(|_| json!({ "code": "bad-passphrase", "message": "This device's passphrase did not unlock its vault. Check the saved browser or desktop login; a key-file backup can have a different passphrase. A damaged vault header can also cause this. No data was replaced." })),
        None => {
            // Validate a legacy database using only an in-memory journal. Unsupported or
            // damaged data must not be rewritten merely because someone tried to unlock it.
            let loaded = db.load().map_err(saved_data_error)?;
            Node::open(Box::new(hoi_store::Journal::with_initial(loaded)), "desktop", "https://hoi.sh/app/", hoi_core::system_clock())
                .map_err(saved_data_error)?;
            hoi_crypto::vault::LocalVault::create(passphrase)
                .map_err(|_| json!({ "code": "invalid", "message": "Could not protect this device. Use a passphrase of at least 15 characters and at most 1024 bytes." }))
        }
    }?;
    let db = hoi_store::vault::EncryptedSqlite::open(db, vault, &[]).map_err(saved_data_error)?;
    Node::open(Box::new(db), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).map_err(saved_data_error)
}

fn open_os_vault_node(db: hoi_store::sqlite::Sqlite, secret: &[u8]) -> Result<Node, Value> {
    let header = hoi_store::vault::EncryptedSqlite::header(&db).map_err(saved_data_error)?.ok_or_else(|| internal("Use your vault password."))?;
    let vault = hoi_crypto::vault::LocalVault::unlock_os_secret(&header, secret)
        .map_err(|_| internal("The OS entry does not match this vault. Use your vault password and enable OS unlock again."))?;
    // A fingerprint binds a secret to its header; the authenticated inventory proves its key.
    let db = hoi_store::vault::EncryptedSqlite::open(db, vault, &[]).map_err(saved_data_error)?;
    Node::open(Box::new(db), "desktop", "https://hoi.sh/app/", hoi_core::system_clock()).map_err(saved_data_error)
}

/// The computer's name as people know it ("lena-laptop"), if the system says.
fn hostname() -> Option<String> {
    let raw = if cfg!(windows) {
        std::env::var("COMPUTERNAME").ok()
    } else {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .or_else(|| std::process::Command::new("hostname").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()))
    }?;
    let name = raw.trim().trim_end_matches(".local").trim_end_matches(".lan").to_string();
    (!name.is_empty() && name != "localhost").then_some(name)
}

fn notice(app: &AppHandle, shared: &Shared, text: &str) {
    let _ = app.emit("hoi://event", hoi_core::dto::Event::Notice { text: text.into() });
    shared.with(|n| n.set_net_error(Some(text.into())));
}

async fn start_peers(shared: Arc<Shared>, app: AppHandle, vpn: &hoi_net::vpn::VpnState) -> Option<hoi_net::P2pHandle> {
    // Only through a VPN: bound to its address, and not at all while it is down.
    let bind = match vpn {
        hoi_net::vpn::VpnState::Off => None,
        hoi_net::vpn::VpnState::Up(_) => Some(SocketAddr::new(vpn.ipv4()?.into(), 0)),
        hoi_net::vpn::VpnState::Down(_) => return None,
    };
    match hoi_net::start_p2p(shared.clone(), hoi_net::P2pConfig { bind, ..Default::default() }).await {
        Ok(handle) => Some(handle),
        Err(_) => {
            notice(&app, &shared, "Peer networking could not start. Restart hoi to try again.");
            None
        }
    }
}

/// Phones and browsers on your network reach this computer over WebRTC (UDP 7844, or any free
/// port if another hoi has it). No certificate, no domain: they pin this computer's key.
async fn start_rtc(shared: Arc<Shared>, app: AppHandle, policy: hoi_net::vpn::NetPolicy) -> Option<hoi_net::RtcHandle> {
    match hoi_net::start_rtc(shared.clone(), hoi_net::RtcConfig { policy, ..Default::default() }).await {
        Ok(handle) => Some(handle),
        Err(_) => {
            let _ = app.emit(
                "hoi://event",
                hoi_core::dto::Event::Notice {
                    text: "Your phone can't connect to this computer: its network port didn't open. Restart hoi to try again.".into(),
                },
            );
            None
        }
    }
}

/// Asks the router to forward this computer's WebRTC and native ports while *Be a public node*
/// is on, and stops (removing the forwards) when it is off. What the router forwards from goes
/// into the routes, and so into invites and the node link; what it said goes to the app.
/// `preview` starts or ends asking without publishing, so the person sees the address first.
async fn sync_forwarding(hoi: &Hoi, preview: Option<bool>) {
    sync_dht(hoi).await;
    sync_lan(hoi).await;
    let mut slot = hoi.forwarding.lock().await;
    let public = hoi.shared.with(|n| n.settings().public_node);
    let previewing = !public && preview.unwrap_or(slot.as_ref().is_some_and(|f| f.previewing));
    match slot.as_mut() {
        Some(_) if !public && !previewing => {
            if let Some(f) = slot.take() {
                drop_forwarding(hoi, f).await;
            }
            return;
        }
        Some(f) => {
            f.previewing = previewing;
            let _ = f.publish.send(public);
            return;
        }
        None if !public && !previewing => return,
        None => {}
    }
    let rtc_port = hoi.rtc.lock().await.as_ref().map(|h| h.port());
    let peer_ports = hoi.p2p.lock().await.as_ref().map(|h| h.ipv4_ports()).unwrap_or_default();
    let dht_ports = hoi.dht.lock().await.as_ref().map(|h| [h.ports.dht, h.ports.answer]).into_iter().flatten().collect::<Vec<_>>();
    let ports: Vec<u16> = rtc_port.into_iter().chain(peer_ports.iter().copied()).chain(dht_ports).collect();
    if ports.is_empty() {
        return;
    }
    // Through a VPN, only its own gateway is asked: the home router would publish the home.
    let policy = net_policy(&hoi.shared);
    let via_vpn = policy.vpn.is_some();
    if via_vpn && !matches!(policy.state(), hoi_net::vpn::VpnState::Up(_)) {
        return;
    }
    let via = policy.vpn.map_or(hoi_net::portmap::Via::DefaultRoute, hoi_net::portmap::Via::Interface);
    hoi.shared.with(|n| n.set_port_mapping(Some(hoi_core::dto::PortMapping { state: "asking".into(), via_vpn, ..Default::default() })));
    let (tx, mut forwarded) = tokio::sync::watch::channel(Vec::<hoi_net::portmap::Forward>::new());
    let (publish, mut published) = tokio::sync::watch::channel(public);
    let (rtc, p2p) = (hoi.rtc.clone(), hoi.p2p.clone());
    let advertiser = tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                r = forwarded.changed() => if r.is_err() { break },
                r = published.changed() => if r.is_err() { break },
            }
            let forwards = if *published.borrow_and_update() { forwarded.borrow_and_update().clone() } else { Vec::new() };
            let public = |port: Option<u16>| hoi_net::portmap::PortMapper::external_for(&forwards, port);
            if let Some(h) = rtc.lock().await.as_ref() {
                h.set_advertised(public(rtc_port));
            }
            if let Some(h) = p2p.lock().await.as_ref() {
                h.set_advertised(peer_ports.iter().flat_map(|p| public(Some(*p))).collect());
            }
        }
    });
    let shared = hoi.shared.clone();
    let mapper = hoi_net::portmap::PortMapper::spawn(
        ports,
        via,
        move |f| {
            let _ = tx.send(f.to_vec());
        },
        move |status| shared.with(|n| n.set_port_mapping(status.report(via_vpn))),
    );
    *slot = Some(Forwarding { mapper, advertiser, publish, previewing });
}

/// The DHT among public nodes runs while this computer is a public node, bound to
/// the VPN in "Only through my VPN", and not while that VPN is down.
async fn sync_dht(hoi: &Hoi) {
    let public = hoi.shared.with(|n| n.settings().public_node);
    let policy = net_policy(&hoi.shared);
    let bind = match (&policy.vpn, policy.state()) {
        (None, _) => Some(None),
        (Some(_), v @ hoi_net::vpn::VpnState::Up(_)) => v.ipv4().map(Some),
        _ => None,
    };
    let mut slot = hoi.dht.lock().await;
    match (public, bind, slot.is_some()) {
        (true, Some(bind), false) => match hoi_net::dht::start_dht(hoi.shared.clone(), hoi_net::dht::DhtConfig { bind, ..Default::default() }).await {
            Ok(h) => *slot = Some(h),
            Err(e) => eprintln!("hoi: the DHT among public nodes could not start: {e}"),
        },
        (false, _, true) | (true, None, true) => {
            slot.take();
            hoi.shared.with(|n| n.set_dht(None));
        }
        _ => {}
    }
}

/// Beacons on the local network for members of the same communities: on with the
/// native transport, off in "Only through my VPN" (a beacon goes out on the home network).
async fn sync_lan(hoi: &Hoi) {
    let policy = net_policy(&hoi.shared);
    let port = hoi.p2p.lock().await.as_ref().and_then(|h| h.ipv4_ports().first().copied());
    let mut slot = hoi.lan.lock().await;
    match (policy.vpn.is_none(), port, slot.is_some()) {
        (true, Some(port), false) => match hoi_net::lan::start_lan(hoi.shared.clone(), port, None) {
            Ok(h) => *slot = Some(h),
            Err(e) => eprintln!("hoi: LAN beacons could not start: {e}"),
        },
        (false, _, true) | (_, None, true) => {
            slot.take();
        }
        _ => {}
    }
    hoi.shared.with(|n| n.set_lan(slot.is_some()));
}

/// Removes the router's forwards and the public addresses they gave the routes.
async fn stop_forwarding(hoi: &Hoi) {
    if hoi.dht.lock().await.take().is_some() {
        hoi.shared.with(|n| n.set_dht(None));
    }
    let taken = hoi.forwarding.lock().await.take();
    if let Some(f) = taken {
        drop_forwarding(hoi, f).await;
    }
}

async fn drop_forwarding(hoi: &Hoi, f: Forwarding) {
    f.mapper.shutdown().await;
    f.advertiser.abort();
    let _ = f.advertiser.await;
    if let Some(h) = hoi.rtc.lock().await.as_ref() {
        h.set_advertised(Vec::new());
    }
    if let Some(h) = hoi.p2p.lock().await.as_ref() {
        h.set_advertised(Vec::new());
    }
    hoi.shared.with(|n| n.set_port_mapping(None));
}

/// The tray icon: hoi keeps running there when the window is closed. A click brings the
/// window back as the companion (unless someone chose the full app as what hoi opens to); the
/// menu has both.
fn tray(app: &AppHandle, shared: Option<Arc<Shared>>) -> tauri::Result<()> {
    let companion = MenuItem::with_id(app, "companion", "Show the companion", true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", "Open hoi in my browser", true, None::<&str>)?;
    let full = MenuItem::with_id(app, "full", "Open the full app", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit hoi", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&companion, &browser, &full, &PredefinedMenuItem::separator(app)?, &quit])?;
    let clicked = shared.clone();
    let tooltip = if shared.is_some() { "hoi: keeping your communities reachable" } else { "hoi: unlock to connect" };
    let mut builder = TrayIconBuilder::with_id("hoi")
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, e| match e.id.as_ref() {
            "browser" => {
                if let Some(s) = shared.as_ref().and_then(|s| s.with(|n| n.net_status().serving)) {
                    let _ = open_url(&s.pairing_web);
                } else {
                    show_window(app, None);
                }
            }
            "companion" => show_face(app, false),
            "full" => show_face(app, true),
            "quit" => {
                // The router's forwards go with hoi, not two hours later.
                if let Some(hoi) = app.try_state::<Hoi>() {
                    tauri::async_runtime::block_on(stop_forwarding(&hoi));
                }
                app.exit(0)
            }
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                // An open window just comes to the front; locked, there is only the unlock screen.
                let app = tray.app_handle();
                let open = app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
                match clicked.as_ref().map(|s| s.with(|n| n.settings().companion)) {
                    Some(true) if !open => show_face(app, false),
                    _ => show_window(app, None),
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn start_unlocked(app: &AppHandle, mut node: Node) {
    let handle = app.clone();
    let companion = node.settings().companion;
    // With "Be a public node" on, the desktop carries open communities whoever asks, within the
    // core's bounds, and passes on the public nodes it reached (hoi-core `carry`, `nodes`); the
    // core keeps the policy off until the person turns it on.
    node.set_carry_policy(hoi_core::CarryPolicy::Any, hoi_core::carry::DEFAULT_CARRY_LIMIT);
    // A person's computer: operators' notices are read before it says anything to a stranger
    // it dialed; strangers that dial in hear it as the node it is.
    node.set_consent_gate(true);
    let emitter = handle.clone();
    let shared = Shared::new(
        node,
        Box::new(move |events| {
            for e in events {
                let _ = emitter.emit("hoi://event", e);
            }
        }),
    );
    let p2p: Slot<hoi_net::P2pHandle> = Arc::new(tokio::sync::Mutex::new(None));
    let rtc: Slot<hoi_net::RtcHandle> = Arc::new(tokio::sync::Mutex::new(None));
    // Only through a VPN: the transports start bound to it, or the native one waits for it.
    let policy = net_policy(&shared);
    let vpn = policy.state();
    shared.with(|n| n.set_vpn_status(policy.vpn.as_deref().and_then(|name| vpn.report(name))));
    let hoi = Hoi {
        shared: shared.clone(),
        p2p: p2p.clone(),
        rtc: rtc.clone(),
        home_link: Mutex::new(None),
        forwarding: Arc::default(),
        dht: Arc::default(),
        lan: Arc::default(),
        applied: tokio::sync::Mutex::new((policy.clone(), vpn.clone())),
    };
    let (peer_node, peer_app) = (shared.clone(), handle.clone());
    let peers_up = tauri::async_runtime::spawn(async move {
        let mut slot = p2p.lock().await;
        *slot = start_peers(peer_node, peer_app, &vpn).await;
    });

    // Serve this machine's browser as a home node; phones come in over WebRTC.
    let s = shared.clone();
    let rtc_app = handle.clone();
    let rtc_up = tauri::async_runtime::spawn(async move {
        let preferred: SocketAddr = ([127, 0, 0, 1], HOME_PORT).into();
        if hoi_net::serve(s.clone(), preferred, &hoi_net::origins_from_env()).await.is_err() {
            // another hoi already holds the port: take any free one
            let _ = hoi_net::serve(s.clone(), ([127, 0, 0, 1], 0).into(), &hoi_net::origins_from_env()).await;
        }
        // After serving, so the pairing link carries both the WebSocket and the routes.
        let mut slot = rtc.lock().await;
        *slot = start_rtc(s, rtc_app, policy).await;
    });
    sync_pieces(app, &shared);
    // link previews for this app and its paired browsers, under the person's network
    // policy as it is at each fetch
    let pv = shared.clone();
    tauri::async_runtime::spawn(async move {
        let (from, again) = (pv.clone(), pv.clone());
        let _previews = hoi_net::preview::start_previews(pv.clone(), Arc::new(move || net_policy(&from)));
        // names that are domains, checked against the keys they claim
        let _handles = hoi_net::handle::start_handle_checks(pv, Arc::new(move || net_policy(&again)));
        std::future::pending::<()>().await;
    });
    let sweeper = shared.clone();
    tauri::async_runtime::spawn(async move {
        let _sweeper = hoi_net::spawn_sweeper(sweeper);
        std::future::pending::<()>().await;
    });
    restart_home_link(&hoi);
    app.manage(hoi);
    // A public node asks the router once both transports know their ports.
    let forwarding_app = handle.clone();
    tauri::async_runtime::spawn(async move {
        let _ = peers_up.await;
        let _ = rtc_up.await;
        sync_forwarding(&forwarding_app.state::<Hoi>(), None).await;
        // Then the VPN is watched, every few seconds, for as long as hoi runs.
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            follow_vpn(&forwarding_app.state::<Hoi>(), &forwarding_app).await;
        }
    });
    app.remove_tray_by_id("hoi");
    match tray(&handle, Some(shared)) {
        Ok(()) => IN_TRAY.store(true, Ordering::SeqCst),
        Err(e) => eprintln!("hoi: no tray icon ({e}); closing the window quits"),
    }

    if let Some(w) = app.get_webview_window("main") {
        size_for(&w, !companion);
        let _ = w.show();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|a| a == START_AT_LOGIN) {
                // An installer, while hoi already runs: register it, show nothing.
                if let Err(e) = app.autolaunch().enable() {
                    eprintln!("hoi: could not start at login: {e}");
                }
            } else if let Some(url) = argv.iter().find(|a| a.starts_with("hoi://")) {
                open_link(app, url);
            } else {
                show_window(app, None);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![HIDDEN])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_window_event(|window, event| {
            // Closing the window keeps hoi running in the tray: your communities stay reachable.
            // Quit from the tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event
                && IN_TRAY.load(Ordering::SeqCst)
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            app.manage(VaultStartup(tokio::sync::Mutex::new(())));
            app.manage(updates::Updates::default());
            app.manage(IdentityWork::default());
            if tray(app.handle(), None).is_ok() {
                IN_TRAY.store(true, Ordering::SeqCst);
            }
            let start_at_login = std::env::args().any(|a| a == START_AT_LOGIN);
            if start_at_login && let Err(e) = app.autolaunch().enable() {
                eprintln!("hoi: could not start at login: {e}");
            }
            // No node, keys, listener or network transport exists before local unlock.
            if let Some(w) = app.get_webview_window("main")
                && !start_at_login
                && !std::env::args().any(|a| a == HIDDEN)
            {
                let _ = w.show();
            }

            #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register_all();
            }
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let h = app.handle().clone();
                app.deep_link().on_open_url(move |e| {
                    for u in e.urls() {
                        open_link(&h, u.as_str());
                    }
                });
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    for u in urls {
                        open_link(app.handle(), u.as_str());
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hoi_call,
            hoi_mine,
            hoi_mine_identity,
            hoi_identity_progress,
            hoi_identity_stop,
            hoi_benchmark,
            hoi_save_file,
            hoi_open_url,
            hoi_autostart,
            hoi_face,
            hoi_vault_status,
            hoi_vault_unlock,
            hoi_os_unlock_status,
            hoi_os_unlock_enable,
            hoi_os_unlock_disable,
            hoi_os_unlock,
            hoi_vault_reset,
            hoi_vault_lock,
            updates::hoi_update_check,
            updates::hoi_update_download,
            updates::hoi_update_install
        ])
        .run(tauri::generate_context!())
        .expect("hoi failed to start");
}
