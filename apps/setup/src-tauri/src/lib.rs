//! The hoi node setup app: a small window that installs and manages a public node on
//! a server over SSH, or on this computer, step by step. The window (apps/web, built with
//! `--mode setup`) drives the engine through these commands; every line a command prints on
//! the server arrives as a `setup://log` event. Passwords and passphrases live in memory for
//! one session; saved servers keep the host key that was pinned, never a secret. A hoi-node built
//! on this computer (`pnpm build:all`) is found beside the app or picked, read here and uploaded
//! with its SHA-256 checked on the server, so a node installs without a published release.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hoi_setup_engine::{Auth, HostKey, NodeFile, Profile, Profiles, Session, Stream, Target, local_exec, node_files, probe_host_key};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Open sessions by id, handed to the window as opaque strings.
struct Engine {
    sessions: tokio::sync::Mutex<HashMap<String, Arc<Session>>>,
    next: AtomicU64,
}

fn fail(e: impl std::fmt::Display) -> Value {
    json!({ "code": "error", "message": e.to_string() })
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, Value> {
    app.path().app_config_dir().map_err(fail)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One line a command printed, as the window shows it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LogLine {
    run: String,
    stream: Stream,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunResult {
    code: u32,
    text: String,
}

// ---------------------------------------------------------------- saved servers

#[tauri::command]
fn setup_profiles(app: AppHandle) -> Result<Vec<Profile>, Value> {
    Ok(Profiles::load(&config_dir(&app)?).servers)
}

#[tauri::command]
fn setup_save_profile(app: AppHandle, mut profile: Profile) -> Result<Vec<Profile>, Value> {
    let dir = config_dir(&app)?;
    let mut profiles = Profiles::load(&dir);
    if profile.created == 0 {
        profile.created = now();
    }
    profile.last_used = now();
    profiles.upsert(profile);
    profiles.save(&dir).map_err(fail)?;
    Ok(profiles.servers)
}

#[tauri::command]
fn setup_remove_profile(app: AppHandle, id: String) -> Result<Vec<Profile>, Value> {
    let dir = config_dir(&app)?;
    let mut profiles = Profiles::load(&dir);
    profiles.remove(&id);
    profiles.save(&dir).map_err(fail)?;
    Ok(profiles.servers)
}

// ---------------------------------------------------------------- the server

/// A first look at a server: its host key, to compare and pin. Nobody is authenticated.
#[tauri::command]
async fn setup_probe(target: Target) -> Result<HostKey, Value> {
    probe_host_key(&target).await.map_err(fail)
}

/// Connects with the pinned host key and the login given; returns the session's id.
#[tauri::command]
async fn setup_connect(engine: State<'_, Engine>, target: Target, auth: Auth, host_key: String) -> Result<String, Value> {
    let session = Session::connect(target, auth, &host_key).await.map_err(fail)?;
    let id = format!("s{}", engine.next.fetch_add(1, Ordering::SeqCst) + 1);
    engine.sessions.lock().await.insert(id.clone(), Arc::new(session));
    Ok(id)
}

async fn session(engine: &Engine, id: &str) -> Result<Arc<Session>, Value> {
    engine.sessions.lock().await.get(id).cloned().ok_or_else(|| fail("that connection is gone; connect again"))
}

/// Runs a command on the server, every line sent to the window as it comes; with `pty`, a
/// password prompt (sudo) is answered with `answer` once.
#[tauri::command]
async fn setup_run(app: AppHandle, engine: State<'_, Engine>, session: String, run: String, command: String, pty: bool, answer: Option<String>) -> Result<RunResult, Value> {
    let s = session_of(&engine, &session).await?;
    let mut text = String::new();
    let handle = app.clone();
    let run_id = run.clone();
    let code = s
        .exec(&command, pty, None, answer.as_deref(), move |stream, line| {
            text.push_str(&line);
            text.push('\n');
            let _ = handle.emit("setup://log", LogLine { run: run_id.clone(), stream, text: line });
        })
        .await
        .map_err(fail)?;
    Ok(RunResult { code, text: String::new() })
}

async fn session_of(engine: &State<'_, Engine>, id: &str) -> Result<Arc<Session>, Value> {
    session(engine, id).await
}

/// Runs a command and returns what it printed, for reading a file or a status.
#[tauri::command]
async fn setup_read(engine: State<'_, Engine>, session: String, command: String) -> Result<RunResult, Value> {
    let s = session_of(&engine, &session).await?;
    let (code, text) = s.read(&command).await.map_err(fail)?;
    Ok(RunResult { code, text })
}

/// Writes a text file on the server through the session.
#[tauri::command]
async fn setup_upload(engine: State<'_, Engine>, session: String, path: String, text: String, mode: String) -> Result<(), Value> {
    let s = session_of(&engine, &session).await?;
    s.upload(&path, text.as_bytes(), &mode).await.map_err(fail)
}

/// How far an upload is, for the window's progress bar.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UploadProgress {
    run: String,
    sent: u64,
    total: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Uploaded {
    name: String,
    path: String,
    sha256: String,
    bytes: u64,
}

/// Uploads a hoi-node archive from this computer into the wizard's private folder on the server
/// and checks what arrived. The file is read here; the window only names it.
#[tauri::command]
async fn setup_upload_node(app: AppHandle, engine: State<'_, Engine>, session: String, run: String, path: String, dir: String) -> Result<Uploaded, Value> {
    if !node_files::is_upload_dir(&dir) {
        return Err(fail("uploads go into the wizard's own folder on the server"));
    }
    let s = session_of(&engine, &session).await?;
    let local = PathBuf::from(&path);
    let (file, bytes) = tauri::async_runtime::spawn_blocking(move || node_files::read(&local)).await.map_err(fail)?.map_err(fail)?;
    let remote = format!("{dir}/{}", file.name);
    let handle = app.clone();
    let sha256 = s
        .upload_verified(&remote, &bytes, "0644", move |sent, total| {
            let _ = handle.emit("setup://upload", UploadProgress { run: run.clone(), sent, total });
        })
        .await
        .map_err(fail)?;
    Ok(Uploaded { name: file.name, path: remote, sha256, bytes: file.bytes })
}

#[tauri::command]
async fn setup_disconnect(engine: State<'_, Engine>, session: String) -> Result<(), Value> {
    if let Some(s) = engine.sessions.lock().await.remove(&session)
        && let Ok(s) = Arc::try_unwrap(s)
    {
        s.close().await;
    }
    Ok(())
}

// ---------------------------------------------------------------- this computer

/// The same steps on this computer: a program with its arguments, lines streamed as above.
#[tauri::command]
async fn setup_local_run(app: AppHandle, run: String, program: String, args: Vec<String>, stdin: Option<String>) -> Result<RunResult, Value> {
    let handle = app.clone();
    let code = tauri::async_runtime::spawn_blocking(move || {
        local_exec(&program, &args, stdin.as_deref(), move |stream, line| {
            let _ = handle.emit("setup://log", LogLine { run: run.clone(), stream, text: line });
        })
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;
    Ok(RunResult { code: code.max(0) as u32, text: String::new() })
}

/// Writes a file for a local install into the app's own cache folder and returns its path.
#[tauri::command]
fn setup_write_local(app: AppHandle, name: String, text: String) -> Result<String, Value> {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(fail("a plain file name"));
    }
    let dir = app.path().app_cache_dir().map_err(fail)?.join("kit");
    std::fs::create_dir_all(&dir).map_err(fail)?;
    let path = dir.join(name);
    std::fs::write(&path, text).map_err(fail)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(path.to_string_lossy().into_owned())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Platform {
    os: String,
    arch: String,
    user: String,
    home: String,
}

#[tauri::command]
fn setup_platform() -> Platform {
    Platform {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        user: std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default(),
        home: std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------- hoi-node from this computer

/// Where a hoi-node archive is looked for: beside the app, `../server` (the portable app kept in
/// builds/hoi-<version>/node-setup/), and in a development build the checkout's builds/*/server.
fn node_dirs() -> Vec<PathBuf> {
    let exe = std::env::current_exe().ok();
    let builds = cfg!(debug_assertions).then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../builds"));
    node_files::search_dirs(exe.as_deref(), builds.as_deref())
}

/// The hoi-node archives found on this computer, nearest first.
#[tauri::command]
fn setup_node_files() -> Vec<NodeFile> {
    node_files::find(&node_dirs())
}

/// A hoi-node archive picked in a native dialog; refused unless its name is one.
#[tauri::command]
async fn setup_pick_node(app: AppHandle) -> Result<Option<NodeFile>, Value> {
    let picked = app.dialog().file().set_title("hoi-node for the server (hoi-node_…_linux-gnu.tar.gz)").add_filter("hoi-node archive", &["gz"]).blocking_pick_file();
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    node_files::node_file(&path).map(Some).map_err(fail)
}

// ---------------------------------------------------------------- files and links

/// A key file, picked in a native dialog: the path only, read by the engine when connecting.
#[tauri::command]
async fn setup_pick_file(app: AppHandle) -> Result<Option<String>, Value> {
    let picked = app.dialog().file().set_title("Your SSH key file").blocking_pick_file();
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn setup_save_file(app: AppHandle, name: String, text: String) -> Result<bool, Value> {
    let Some(path) = app.dialog().file().set_file_name(&name).blocking_save_file() else {
        return Ok(false);
    };
    let path = path.into_path().map_err(fail)?;
    std::fs::write(&path, text).map_err(fail)?;
    Ok(true)
}

/// Opens an https:// page in the person's browser: the node's link in hoi, hoi.sh. Nothing else.
#[tauri::command]
fn setup_open_url(url: String) -> Result<(), Value> {
    if url.len() > 8192 || !url.starts_with("https://") || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(fail("Only https:// links open from here."));
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(fail)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Engine { sessions: tokio::sync::Mutex::new(HashMap::new()), next: AtomicU64::new(0) })
        .invoke_handler(tauri::generate_handler![
            setup_profiles,
            setup_save_profile,
            setup_remove_profile,
            setup_probe,
            setup_connect,
            setup_run,
            setup_read,
            setup_upload,
            setup_upload_node,
            setup_disconnect,
            setup_local_run,
            setup_write_local,
            setup_platform,
            setup_pick_file,
            setup_node_files,
            setup_pick_node,
            setup_save_file,
            setup_open_url
        ])
        .run(tauri::generate_context!())
        .expect("hoi node setup could not start");
}
