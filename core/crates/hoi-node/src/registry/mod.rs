//! The registry role (`--registry ADDR`): the public-name API served by this node, request for
//! request the same as `registry/server.mjs` and the Worker, on the same database shape. A
//! registry is one participant among others: it arbitrates names in its own namespace while it
//! runs, lists the public nodes that announced themselves, and can mirror another registry's
//! names so a second one holds them when the first stops. It serves no HTML, keeps no access
//! log and never prints a client's address.

pub mod log;
pub mod mirror;
pub mod protocol;
pub mod release;
pub mod store;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use protocol::{
    ANNOUNCER_REFUSED, WITHHELD, admit_announcement, loopback_origins, normalize_community_key, normalize_name, registry_origin, select_public_nodes,
    validate_announcers, validate_app_url, validate_origin, validate_public_nodes, verified_claim,
};
use release::ReleaseSource;
use store::Store;

/// A name claim is a few hundred bytes; a node announcement lists up to 64 keys and 8 routes.
const MAX_BODY: usize = 2048;
const MAX_ANNOUNCEMENT_BODY: usize = 8192;
/// Connections served at once; beyond that a client gets a closed socket and tries again.
const MAX_CONNECTIONS: usize = 256;
/// A request's headers and body must each arrive within this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// A whole connection (one request: keep-alive is off) is cut after this.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Operator configuration, validated before the registry listens.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    /// Exact browser origins allowed to call the API (`tauri://localhost` for the desktop app).
    pub origins: Vec<String>,
    /// Routes listed before any announced node.
    pub public_nodes: Vec<String>,
    /// Whether nodes may announce themselves.
    pub announcements: bool,
    pub app_url: String,
    pub release: ReleaseSource,
    /// Per-minute budgets: reads, name claims, node announcements.
    pub limits: (u32, u32, u32),
    /// This registry's public URL (`--registry-url`), whose origin announcements must name.
    /// Without it, only announcements signed for the loopback address a connection arrived at.
    pub url: Option<String>,
    /// Allowlist mode (`--registry-announcer`): the endpoint keys whose announcements are
    /// accepted. Empty: any node's.
    pub announcers: Vec<String>,
}

impl RegistryConfig {
    pub fn new(release: ReleaseSource) -> RegistryConfig {
        RegistryConfig {
            origins: Vec::new(),
            public_nodes: Vec::new(),
            announcements: true,
            app_url: "https://hoi.sh/app/".into(),
            release,
            limits: (600, 30, 120),
            url: None,
            announcers: Vec::new(),
        }
    }
}

/// One fixed-size counter per minute. Global on purpose: no client identifier is kept.
struct Budget {
    capacity: u32,
    started: u64,
    used: u32,
}

impl Budget {
    fn new(capacity: u32) -> Budget {
        Budget { capacity, started: 0, used: 0 }
    }

    /// `0` when the request may proceed, else the seconds to wait (`Retry-After`).
    fn take(&mut self, now_ms: u64) -> u64 {
        if now_ms.saturating_sub(self.started) >= 60_000 {
            self.started = now_ms;
            self.used = 0;
        }
        self.used = self.used.saturating_add(1);
        if self.used <= self.capacity { 0 } else { (60_000u64.saturating_sub(now_ms.saturating_sub(self.started))).div_ceil(1000).max(1) }
    }
}

pub type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

/// The running registry: its database, budgets and configuration, shared by every connection.
pub struct Registry {
    store: Mutex<Store>,
    /// The public log's tree and key, caught up with the store on each read.
    log: Mutex<log::Log>,
    origins: Vec<String>,
    public_nodes: Vec<String>,
    announcements: bool,
    app_url: String,
    release: ReleaseSource,
    budgets: Mutex<(Budget, Budget, Budget)>,
    /// Milliseconds since the Unix epoch.
    clock: Clock,
    /// The origin announcements must name, when configured.
    origin: Option<String>,
    announcers: Option<std::collections::BTreeSet<String>>,
}

fn system_clock() -> Clock {
    Box::new(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0))
}

/// What a handler answers: a status code and an optional JSON body, plus any extra headers.
struct Answer {
    status: u16,
    body: Option<Value>,
    headers: Vec<(&'static str, String)>,
}

fn send(status: u16, body: Value) -> Answer {
    Answer { status, body: Some(body), headers: Vec::new() }
}

fn empty(status: u16) -> Answer {
    Answer { status, body: None, headers: Vec::new() }
}

/// A fault with the request itself (bad JSON, too long, too slow), as the Node service reports it.
struct RequestFault {
    status: u16,
    message: String,
}

impl Registry {
    pub fn open(data_dir: &Path, config: &RegistryConfig) -> Result<Arc<Registry>, String> {
        Self::open_with(data_dir, config, system_clock())
    }

    fn open_with(data_dir: &Path, config: &RegistryConfig, clock: Clock) -> Result<Arc<Registry>, String> {
        let origins = config.origins.iter().map(|o| validate_origin(o)).collect::<Result<Vec<_>, _>>()?;
        let public_nodes = validate_public_nodes(&config.public_nodes)?;
        let app_url = validate_app_url(&config.app_url)?;
        // Invalid deployment configuration fails at startup, before binding a port.
        config.release.load()?;
        let (reads, writes, announcements) = config.limits;
        if reads == 0 || writes == 0 || announcements == 0 {
            return Err("Rate limits must be positive integers.".into());
        }
        let origin = config.url.as_deref().map(registry_origin).transpose()?;
        let announcers = validate_announcers(&config.announcers)?;
        let mut store = Store::open(&data_dir.join("registry.db"))?;
        let log = log::Log::new(store.log_secret()?);
        Ok(Arc::new(Registry {
            store: Mutex::new(store),
            log: Mutex::new(log),
            origins,
            public_nodes,
            announcements: config.announcements,
            app_url,
            release: config.release.clone(),
            budgets: Mutex::new((Budget::new(reads), Budget::new(writes), Budget::new(announcements))),
            clock,
            origin,
            announcers,
        }))
    }

    /// The origins announcements must name for a connection that reached this registry at
    /// `local`: the configured one, or (unconfigured) that loopback address, read from the socket.
    fn registries(&self, local: Option<SocketAddr>) -> Vec<String> {
        match (&self.origin, local) {
            (Some(origin), _) => vec![origin.clone()],
            (None, Some(local)) => loopback_origins(local),
            (None, None) => Vec::new(),
        }
    }

    /// Whether this registry knows the origin announcements must name, for its status line.
    pub fn announcement_origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// How many endpoint keys may announce, in allowlist mode.
    pub fn allowlist(&self) -> Option<usize> {
        self.announcers.as_ref().map(|a| a.len())
    }

    fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    fn seconds(&self) -> u64 {
        self.now_ms() / 1000
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `N names · M announced nodes`, for the status line.
    pub fn summary(&self) -> String {
        match self.store().counts(self.seconds()) {
            Ok((names, nodes)) => format!("{names} name{} · {nodes} announced node{}", if names == 1 { "" } else { "s" }, if nodes == 1 { "" } else { "s" }),
            Err(_) => "database unavailable".into(),
        }
    }

    /// Operator-configured routes first, then nodes that announced themselves recently (only
    /// allowed ones in allowlist mode); nobody is named as a carrier of a withheld community.
    fn public_nodes(&self, store: &Store, community: Option<&str>) -> Result<Vec<String>, rusqlite::Error> {
        let mut live = if self.announcements { store.live_nodes(self.seconds())? } else { Vec::new() };
        if let Some(allowed) = &self.announcers {
            live.retain(|n| allowed.contains(&n.endpoint));
        }
        let community = match community {
            Some(c) if store.is_withheld(c)? => None,
            other => other,
        };
        Ok(select_public_nodes(&self.public_nodes, &live, community))
    }

    fn metadata(&self, store: &Store, release: &release::Release) -> Result<Value, rusqlite::Error> {
        Ok(json!({
            "release": release.to_json(),
            "registry": { "apiVersion": 2, "schemaVersion": 3, "indexVersion": release.index_version },
            "publicNodes": self.public_nodes(store, None)?,
            "appUrl": self.app_url,
        }))
    }

    /// Everything after the method check: the budget, then the API, exactly as the Node service.
    /// `registries` are the origins an announcement on this connection must name.
    #[allow(clippy::too_many_arguments)]
    fn answer(
        &self,
        method: &Method,
        path: &str,
        pairs: &[(String, String)],
        has_query: bool,
        content_type: Option<&str>,
        body: Result<Value, RequestFault>,
        registries: &[String],
    ) -> Answer {
        let version = if path == "/v2/communities" { 2 } else { 1 };
        let is_nodes = path == "/v2/nodes";
        let is_metadata = matches!(path, "/v1/meta" | "/release.json");
        let now_ms = self.now_ms();
        let retry = {
            let mut budgets = self.budgets.lock().unwrap_or_else(|e| e.into_inner());
            if method == Method::PUT { if is_nodes { budgets.2.take(now_ms) } else { budgets.1.take(now_ms) } } else { budgets.0.take(now_ms) }
        };
        if retry > 0 {
            let mut a = send(429, json!({ "error": "rate-limited", "message": "The registry is busy. Retry after the indicated delay." }));
            a.headers.push(("Retry-After", retry.to_string()));
            return a;
        }
        let unavailable = || send(503, json!({ "error": "unavailable", "message": "The registry could not complete this request. Retry later." }));
        if log::LOG_PATHS.contains(&path) {
            let store = self.store();
            let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
            let Ok(size) = store.log_size() else { return unavailable() };
            if log.size() < size {
                let Ok(rows) = store.log_entries(log.size(), size) else { return unavailable() };
                log.extend(rows.into_iter().map(|(entry, _)| entry));
            }
            let (status, body) = log.answer(path, pairs, self.seconds(), |start, end| store.log_entries(start, end).unwrap_or_default());
            return send(status, body);
        }
        let Ok(release) = self.release.load() else { return unavailable() };
        let mut store = self.store();
        if is_metadata {
            if has_query {
                return send(400, json!({ "error": "invalid", "message": "Metadata requests do not use query parameters." }));
            }
            return match self.metadata(&store, &release) {
                Ok(meta) => send(200, if path == "/release.json" { meta["release"].clone() } else { meta }),
                Err(_) => unavailable(),
            };
        }
        let json_body = content_type.is_some_and(|ct| {
            let head = ct.split(';').next().unwrap_or("").trim();
            head.eq_ignore_ascii_case("application/json")
        });
        if is_nodes {
            if method == Method::GET {
                let community = (pairs.len() == 1 && pairs[0].0 == "community").then(|| normalize_community_key(&pairs[0].1)).flatten();
                let Some(community) = community else {
                    return send(400, json!({ "error": "invalid", "message": "Ask for the public nodes carrying one community key." }));
                };
                return match self.public_nodes(&store, Some(&community)) {
                    Ok(nodes) => send(200, json!({ "publicNodes": nodes })),
                    Err(_) => unavailable(),
                };
            }
            if !self.announcements {
                return send(403, json!({ "error": "announcements-disabled", "message": "This registry lists only operator-configured public nodes." }));
            }
            if has_query {
                return send(400, json!({ "error": "invalid", "message": "Announcements do not use query parameters." }));
            }
            if !json_body {
                return send(415, json!({ "error": "content-type", "message": "Send application/json." }));
            }
            let at = self.seconds();
            let body = match body {
                Ok(body) => body,
                // Without a known origin the body was never read; the refusal says why.
                Err(_) if registries.is_empty() => Value::Null,
                Err(fault) => {
                    return send(
                        fault.status,
                        json!({ "error": if fault.status == 408 { "request-timeout" } else { "invalid-request" }, "message": fault.message }),
                    );
                }
            };
            let announcement = match admit_announcement(&body, at, registries, self.announcers.as_ref()) {
                Ok(announcement) => announcement,
                Err((status, error, message)) => return send(status, json!({ "error": error, "message": message })),
            };
            return match store.announce(&announcement, at) {
                Ok(Some(kept)) => send(200, kept),
                Ok(None) => send(ANNOUNCER_REFUSED.0, json!({ "error": ANNOUNCER_REFUSED.1, "message": ANNOUNCER_REFUSED.2 })),
                Err(_) => unavailable(),
            };
        }
        if method == Method::GET {
            let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
            let paginated = version == 2 && keys.len() == 2 && keys.contains(&"prefix") && keys.contains(&"after");
            let by_key = version == 2 && keys.len() == 1 && keys[0] == "community";
            if !paginated && !by_key && (keys.len() != 1 || !matches!(keys[0], "name" | "prefix")) {
                return send(400, json!({ "error": "invalid", "message": "Use one name, community or prefix query; v2 prefix may also include after." }));
            }
            let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
            if by_key {
                let Some(community) = get("community").and_then(normalize_community_key) else {
                    return send(400, json!({ "error": "invalid", "message": "Invalid community key." }));
                };
                return match store.find_publication_by_key(&community, &release) {
                    Ok(Some(found)) if found.get("withheld").is_some() => send(404, json!({ "error": WITHHELD.0, "message": WITHHELD.1 })),
                    Ok(Some(found)) => send(200, found),
                    Ok(None) => send(404, json!({ "error": "not-found", "message": "This community key has no public name." })),
                    Err(_) => unavailable(),
                };
            }
            if let Some(raw) = get("name") {
                let Some(name) = normalize_name(raw) else {
                    return send(
                        400,
                        json!({ "error": "invalid", "message": "Use a public name of 1–32 ASCII letters, digits, dots, underscores or hyphens, starting with a letter or digit." }),
                    );
                };
                let found = if version == 2 { store.find_publication(&name, &release) } else { store.find(&name) };
                return match found {
                    Ok(Some(found)) if found.get("withheld").is_some() => send(404, json!({ "error": WITHHELD.0, "message": WITHHELD.1 })),
                    Ok(Some(found)) => send(200, found),
                    Ok(None) => send(404, json!({ "error": "not-found", "message": "This public name has not been reserved." })),
                    Err(_) => unavailable(),
                };
            }
            let raw_prefix = get("prefix").unwrap_or("").trim();
            let prefix = if raw_prefix.is_empty() { Some(String::new()) } else { normalize_name(raw_prefix) };
            let Some(prefix) = prefix else {
                return send(400, json!({ "error": "invalid", "message": "Invalid public name prefix." }));
            };
            if version == 2 {
                let after = match get("after") {
                    None => Some(String::new()),
                    Some(cursor) => normalize_name(cursor),
                };
                let Some(after) = after else {
                    return send(400, json!({ "error": "invalid", "message": "Invalid pagination cursor." }));
                };
                return match store.list_publications(&prefix, &after, &release) {
                    Ok(list) => send(200, Value::Array(list)),
                    Err(_) => unavailable(),
                };
            }
            return match store.list(&prefix) {
                Ok(list) => send(200, Value::Array(list)),
                Err(_) => unavailable(),
            };
        }
        if has_query {
            return send(400, json!({ "error": "invalid", "message": "Reservation requests do not use query parameters." }));
        }
        if !json_body {
            return send(415, json!({ "error": "content-type", "message": "Send application/json." }));
        }
        let body = match body {
            Ok(body) => body,
            Err(fault) => {
                return send(
                    fault.status,
                    json!({ "error": if fault.status == 408 { "request-timeout" } else { "invalid-request" }, "message": fault.message }),
                );
            }
        };
        let Some(claim) = verified_claim(&body, version) else {
            return send(
                400,
                json!({ "error": "invalid-proof", "message": "The community key must sign the exact public name, community ID and, for v2, protocol and index versions." }),
            );
        };
        let result = if version == 2 { store.publish(&claim, &release) } else { store.reserve(&claim) };
        match result {
            Ok(outcome) => send(outcome.status, outcome.body),
            Err(_) => unavailable(),
        }
    }
}

/// Reads a PUT body within its size and time limits; faults carry the Node service's wording.
async fn read_body(body: Incoming, declared: Option<u64>, max: usize) -> Result<Value, RequestFault> {
    let too_large = || RequestFault { status: 413, message: format!("Request body exceeds {max} bytes.") };
    if declared.is_some_and(|n| n > max as u64) {
        return Err(too_large());
    }
    let collected = tokio::time::timeout(REQUEST_TIMEOUT, Limited::new(body, max).collect())
        .await
        .map_err(|_| RequestFault { status: 408, message: "The request took too long.".into() })?
        .map_err(|_| too_large())?;
    let bytes = collected.to_bytes();
    serde_json::from_slice::<Value>(&bytes).map_err(|_| RequestFault { status: 400, message: "Expected a JSON registration proof.".into() })
}

fn respond(answer: Answer, origin: Option<&str>) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(answer.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Strict-Transport-Security", "max-age=31536000; includeSubDomains")
        .header("Vary", "Origin");
    if let Some(origin) = origin {
        builder = builder.header("Access-Control-Allow-Origin", origin);
    }
    for (name, value) in answer.headers {
        builder = builder.header(name, value);
    }
    let body = answer.body.map(|v| v.to_string()).unwrap_or_default();
    builder.body(Full::new(Bytes::from(body))).unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

/// One request, start to finish: origin, path, method, budget, body, answer. `local` is the
/// address the connection reached, from the socket.
async fn handle(registry: Arc<Registry>, request: Request<Incoming>, local: Option<SocketAddr>) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let origin = request.headers().get("origin").and_then(|v| v.to_str().ok()).map(String::from);
    // As a tracker the public nodes carrying a community are public: any app may read
    // them, wherever it is served, and so is the log of names. Everything else keeps the
    // operator's origins.
    let tracker_read = request.method() == Method::GET && (request.uri().path() == "/v2/nodes" || log::LOG_PATHS.contains(&request.uri().path()));
    let known = origin.as_ref().is_none_or(|o| registry.origins.iter().any(|allowed| allowed == o));
    if !known && !tracker_read {
        return Ok(respond(send(403, json!({ "error": "origin-denied", "message": "This app origin is not allowed by the registry operator." })), None));
    }
    let allowed_origin = if known { origin.as_deref() } else { Some("*") };
    let target = request.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let Ok(url) = url::Url::parse("http://registry.local").and_then(|base| base.join(&target)) else {
        return Ok(respond(send(404, json!({ "error": "not-found" })), allowed_origin));
    };
    let path = url.path().to_string();
    let is_communities = matches!(path.as_str(), "/v1/communities" | "/v2/communities");
    let is_nodes = path == "/v2/nodes";
    let is_metadata = matches!(path.as_str(), "/v1/meta" | "/release.json");
    let is_log = log::LOG_PATHS.contains(&path.as_str());
    if !is_communities && !is_metadata && !is_nodes && !is_log {
        return Ok(respond(send(404, json!({ "error": "not-found" })), allowed_origin));
    }
    let methods: &[Method] = if is_metadata || is_log { &[Method::GET] } else { &[Method::GET, Method::PUT] };
    let method_list = methods.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ");
    if request.method() == Method::OPTIONS {
        let requested = request.headers().get("access-control-request-method").and_then(|v| v.to_str().ok());
        if requested.is_some_and(|m| !methods.iter().any(|allowed| allowed.as_str() == m)) {
            return Ok(respond(send(405, json!({ "error": "method-not-allowed" })), allowed_origin));
        }
        let mut answer = empty(204);
        answer.headers.push(("Access-Control-Allow-Methods", method_list));
        answer.headers.push(("Access-Control-Allow-Headers", "Content-Type".into()));
        return Ok(respond(answer, allowed_origin));
    }
    if !methods.contains(request.method()) {
        let mut answer = send(405, json!({ "error": "method-not-allowed" }));
        answer.headers.push(("Allow", format!("{method_list}, OPTIONS")));
        return Ok(respond(answer, allowed_origin));
    }
    let has_query = url.query().is_some_and(|q| !q.is_empty());
    let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let content_type = request.headers().get("content-type").and_then(|v| v.to_str().ok()).map(String::from);
    let method = request.method().clone();
    let registries = registry.registries(local);
    let body = if method == Method::PUT && (is_communities || is_nodes) {
        let declared = request.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
        let max = if is_nodes { MAX_ANNOUNCEMENT_BODY } else { MAX_BODY };
        // The content type is checked before the body is read (415 before 413), as the Node
        // service does; an unreadable body is reported where the API logic expects it.
        let json_body = content_type.as_deref().is_some_and(|ct| ct.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"));
        if has_query || !json_body || (is_nodes && (!registry.announcements || registries.is_empty())) {
            Err(RequestFault { status: 400, message: String::new() })
        } else {
            read_body(request.into_body(), declared, max).await
        }
    } else {
        Err(RequestFault { status: 400, message: String::new() })
    };
    let answer = registry.answer(&method, &path, &pairs, has_query, content_type.as_deref(), body, &registries);
    Ok(respond(answer, allowed_origin))
}

/// Binds `config.listen` and serves until the task is dropped. Returns the bound address.
pub async fn serve(registry: Arc<Registry>, listen: SocketAddr) -> std::io::Result<(JoinHandle<()>, SocketAddr)> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let local = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
            let registry = registry.clone();
            let reached = stream.local_addr().ok();
            tokio::spawn(async move {
                let _permit = permit;
                let io = TokioIo::new(stream);
                let service = service_fn(move |request| handle(registry.clone(), request, reached));
                let connection = http1::Builder::new()
                    .timer(TokioTimer::new())
                    .header_read_timeout(REQUEST_TIMEOUT)
                    .keep_alive(false)
                    .max_buf_size(16 * 1024)
                    .serve_connection(io, service);
                let _ = tokio::time::timeout(CONNECTION_TIMEOUT, connection).await;
            });
        }
    });
    Ok((task, local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn registry(dir: &Path, clock: Arc<AtomicU64>) -> Arc<Registry> {
        let mut config = RegistryConfig::new(ReleaseSource::compiled().unwrap());
        config.origins = vec!["https://hoi.example".into(), "tauri://localhost".into()];
        config.limits = (10_000, 2, 10_000);
        Registry::open_with(dir, &config, Box::new(move || clock.load(Ordering::Relaxed))).unwrap()
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hoi-registry-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        for name in ["registry.db", "registry.db-wal", "registry.db-shm"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
        let _ = std::fs::remove_dir(dir);
    }

    async fn get(addr: SocketAddr, path: &str) -> (u16, Value) {
        let response = http::client().unwrap().get(format!("http://{addr}{path}")).send().await.unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_api_answers_like_the_node_service() {
        let dir = temp_dir();
        let clock = Arc::new(AtomicU64::new(1_000_000_000_000));
        let registry = registry(&dir, clock.clone());
        let (_task, addr) = serve(registry.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let client = http::client().unwrap();

        let (status, meta) = get(addr, "/v1/meta").await;
        assert_eq!(status, 200);
        assert_eq!(meta["registry"], json!({ "apiVersion": 2, "schemaVersion": 3, "indexVersion": hoi_core::release_versions().1 }));
        assert_eq!(meta["publicNodes"], json!([]));
        assert_eq!(meta["appUrl"], "https://hoi.sh/app/");
        assert_eq!(get(addr, "/release.json").await.1, meta["release"]);
        assert_eq!(get(addr, "/v1/meta?x=1").await.0, 400);
        for path in ["/", "/index.html", "/registry.js", "/app/", "/hoi-registry.db", "/relay/v1/meta", "/v1/meta/"] {
            let response = client.get(format!("http://{addr}{path}")).send().await.unwrap();
            assert_eq!(response.status().as_u16(), 404, "{path}");
            assert!(response.headers().get("content-type").unwrap().to_str().unwrap().starts_with("application/json"));
            assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
            let head = client.head(format!("http://{addr}{path}")).send().await.unwrap();
            assert_eq!(head.status().as_u16(), 404, "HEAD {path}");
        }
        assert_eq!(client.delete(format!("http://{addr}/v1/communities")).send().await.unwrap().status().as_u16(), 405);
        let head = client.head(format!("http://{addr}/v1/meta")).send().await.unwrap();
        assert_eq!(head.status().as_u16(), 405);
        assert_eq!(head.headers().get("allow").unwrap(), "GET, OPTIONS");

        // CORS: only configured exact origins, never the opaque origin.
        let kp = hoi_crypto::Keypair::from_secret(&[1; 32]);
        let community = kp.public().hex();
        let statement = hoi_core::community_registration_statement("music", &kp.public());
        let claim = json!({ "name": "music", "community": community, "signature": hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0) });
        let put = |origin: Option<&str>, body: Value| {
            let mut request = client.put(format!("http://{addr}/v1/communities")).header("Content-Type", "application/json").body(body.to_string());
            if let Some(o) = origin {
                request = request.header("Origin", o);
            }
            request.send()
        };
        assert_eq!(put(Some("https://untrusted.example"), claim.clone()).await.unwrap().status().as_u16(), 403);
        assert_eq!(put(Some("null"), claim.clone()).await.unwrap().status().as_u16(), 403);
        let options = client
            .request(Method::OPTIONS, format!("http://{addr}/v1/communities"))
            .header("Origin", "https://hoi.example")
            .header("Access-Control-Request-Method", "PUT")
            .send()
            .await
            .unwrap();
        assert_eq!(options.status().as_u16(), 204);
        assert_eq!(options.headers().get("access-control-allow-origin").unwrap(), "https://hoi.example");
        assert_eq!(options.headers().get("access-control-allow-methods").unwrap(), "GET, PUT");
        let refused = client
            .request(Method::OPTIONS, format!("http://{addr}/v1/meta"))
            .header("Origin", "https://hoi.example")
            .header("Access-Control-Request-Method", "PUT")
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status().as_u16(), 405);
        let first = put(Some("https://hoi.example"), claim.clone()).await.unwrap();
        assert_eq!(first.status().as_u16(), 200);
        assert_eq!(first.headers().get("access-control-allow-origin").unwrap(), "https://hoi.example");
        assert_eq!(first.json::<Value>().await.unwrap()["proof"]["version"], 1);
        assert_eq!(put(Some("tauri://localhost"), claim.clone()).await.unwrap().status().as_u16(), 200);

        // The claim budget is spent (two writes a minute here): the third is told when to retry.
        let limited = put(None, claim.clone()).await.unwrap();
        assert_eq!(limited.status().as_u16(), 429);
        assert_eq!(limited.headers().get("retry-after").unwrap(), "60");
        clock.fetch_add(60_001, Ordering::Relaxed);
        let mut oversized = claim.clone();
        oversized["padding"] = json!("x".repeat(3000));
        assert_eq!(put(None, oversized).await.unwrap().status().as_u16(), 413);
        let unsupported =
            client.put(format!("http://{addr}/v1/communities")).header("Content-Type", "text/plain").body(claim.to_string()).send().await.unwrap();
        assert_eq!(unsupported.status().as_u16(), 415);
        // Every PUT spends the claim budget, answered or refused; a new minute, a new budget.
        clock.fetch_add(60_001, Ordering::Relaxed);
        let broken = client.put(format!("http://{addr}/v1/communities")).header("Content-Type", "application/json").body("{").send().await.unwrap();
        assert_eq!(broken.status().as_u16(), 400);
        assert_eq!(broken.json::<Value>().await.unwrap()["error"], "invalid-request");
        assert_eq!(get(addr, "/v1/communities?name=music").await.1["community"], json!(community));
        assert_eq!(get(addr, "/v1/communities?name=Music").await.0, 200, "lookups are case-insensitive");
        assert_eq!(get(addr, "/v1/communities?name=nobody").await.0, 404);
        for query in ["", "?name=x&prefix=x", "?name=x&name=y", "?prefix=%25", "?prefix=%E2%84%AA", "?community=abc"] {
            assert_eq!(get(addr, &format!("/v1/communities{query}")).await.0, 400, "{query}");
        }
        assert_eq!(get(addr, "/v2/communities?prefix=").await.1, json!([]), "a v1 reservation is not current discovery");
        assert_eq!(get(addr, "/v2/communities?name=music").await.1["indexed"], json!(false));
        assert_eq!(get(addr, &format!("/v2/communities?community={community}")).await.1["name"], "music");
        assert_eq!(get(addr, "/v2/communities?community=music").await.0, 400);
        assert_eq!(get(addr, "/v2/nodes").await.0, 400);
        assert_eq!(get(addr, &format!("/v2/nodes?community={}", "a".repeat(64))).await.1, json!({ "publicNodes": [] }));
        assert_eq!(registry.summary(), "1 name · 0 announced nodes");
        cleanup(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn announcements_follow_the_registry_clock_and_the_public_address_rule() {
        let dir = temp_dir();
        let now_ms = 1_700_000_000_000u64;
        let clock = Arc::new(AtomicU64::new(now_ms));
        let registry = registry(&dir, clock.clone());
        let (_task, addr) = serve(registry.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let client = http::client().unwrap();
        let kp = hoi_crypto::Keypair::from_secret(&[5; 32]);
        let endpoint = kp.public().hex();
        let garden = "a".repeat(64);
        // Unconfigured, the registry answers to the loopback address it was reached at.
        let here = format!("http://{addr}");
        let announce_for = |registry: &str, routes: Vec<String>, communities: Vec<String>, issued_at: u64| {
            let ids: Vec<hoi_types::Id> = communities.iter().map(|c| hoi_types::Id::from_hex(c).unwrap()).collect();
            let statement = hoi_core::node_announcement_statement(registry, &kp.public(), issued_at, &routes, &ids);
            let signature = hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0);
            json!({ "registry": registry, "endpoint": endpoint, "routes": routes, "communities": communities, "issuedAt": issued_at, "signature": signature })
        };
        let announce = |routes: Vec<String>, communities: Vec<String>, issued_at: u64| announce_for(&here, routes, communities, issued_at);
        let put = |body: String| client.put(format!("http://{addr}/v2/nodes")).header("Content-Type", "application/json").body(body).send();
        let route = format!("{endpoint}@203.0.113.9:7843");
        let saved = put(announce(vec![route.clone()], vec![garden.clone()], now_ms / 1000).to_string()).await.unwrap();
        assert_eq!(saved.status().as_u16(), 200);
        assert_eq!(
            saved.json::<Value>().await.unwrap(),
            json!({ "endpoint": endpoint, "routes": [route], "communities": [garden], "issuedAt": now_ms / 1000 })
        );
        assert_eq!(get(addr, "/v1/meta").await.1["publicNodes"], json!([route]));
        assert_eq!(get(addr, &format!("/v2/nodes?community={garden}")).await.1["publicNodes"], json!([route]));
        assert_eq!(put(announce(vec![route.clone()], vec![garden.clone()], now_ms / 1000 - 700).to_string()).await.unwrap().status().as_u16(), 400, "stale");
        assert_eq!(
            put(announce(vec![format!("{endpoint}@10.0.0.9:7843")], vec![], now_ms / 1000).to_string()).await.unwrap().status().as_u16(),
            400,
            "private"
        );
        assert_eq!(put("x".repeat(9000)).await.unwrap().status().as_u16(), 413);
        assert_eq!(
            put(announce(vec![route.clone()], vec![], now_ms / 1000).to_string().replace("\"communities\":[]", "\"communities\":[],\"name\":\"pi\""))
                .await
                .unwrap()
                .status()
                .as_u16(),
            400
        );
        assert_eq!(
            client.put(format!("http://{addr}/v2/nodes?x=1")).header("Content-Type", "application/json").body("{}").send().await.unwrap().status().as_u16(),
            400
        );
        // Signed for another registry: refused, whatever it says about itself.
        let elsewhere = put(announce_for("https://other-registry.example", vec![route.clone()], vec![], now_ms / 1000).to_string()).await.unwrap();
        assert_eq!(elsewhere.status().as_u16(), 400);
        assert_eq!(elsewhere.json::<Value>().await.unwrap()["error"], "wrong-registry");
        let localhost = format!("http://localhost:{}", addr.port());
        assert_eq!(put(announce_for(&localhost, vec![route.clone()], vec![], now_ms / 1000 + 1).to_string()).await.unwrap().status().as_u16(), 200);
        clock.fetch_add(31 * 60 * 1000, Ordering::Relaxed);
        assert_eq!(get(addr, "/v1/meta").await.1["publicNodes"], json!([]), "half an hour of silence drops a node");
        cleanup(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_configured_registry_takes_only_its_own_origin_and_allowlisted_keys() {
        let dir = temp_dir();
        let now_ms = 1_700_000_000_000u64;
        let mut config = RegistryConfig::new(ReleaseSource::compiled().unwrap());
        config.url = Some("https://registry.example/registry".into());
        let allowed = hoi_crypto::Keypair::from_secret(&[6; 32]);
        config.announcers = vec![allowed.public().hex()];
        let registry = Registry::open_with(&dir, &config, Box::new(move || now_ms)).unwrap();
        assert_eq!(registry.announcement_origin(), Some("https://registry.example"));
        assert_eq!(registry.allowlist(), Some(1));
        let (_task, addr) = serve(registry, "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let client = http::client().unwrap();
        let body = |kp: &hoi_crypto::Keypair, registry: &str| {
            let endpoint = kp.public().hex();
            let routes = vec![format!("{endpoint}@203.0.113.9:7843")];
            let statement = hoi_core::node_announcement_statement(registry, &kp.public(), now_ms / 1000, &routes, &[]);
            let signature = hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0);
            json!({ "registry": registry, "endpoint": endpoint, "routes": routes, "communities": [], "issuedAt": now_ms / 1000, "signature": signature })
        };
        let put = |body: Value| client.put(format!("http://{addr}/v2/nodes")).header("Content-Type", "application/json").body(body.to_string()).send();
        assert_eq!(put(body(&allowed, "https://registry.example")).await.unwrap().status().as_u16(), 200);
        let local = put(body(&allowed, &format!("http://{addr}"))).await.unwrap();
        assert_eq!(
            (local.status().as_u16(), local.json::<Value>().await.unwrap()["error"].clone()),
            (400, json!("wrong-registry")),
            "configured, only its origin"
        );
        let stranger = put(body(&hoi_crypto::Keypair::from_secret(&[7; 32]), "https://registry.example")).await.unwrap();
        assert_eq!((stranger.status().as_u16(), stranger.json::<Value>().await.unwrap()["error"].clone()), (403, json!("announcer-not-allowed")));
        assert_eq!(get(addr, "/v1/meta").await.1["publicNodes"].as_array().unwrap().len(), 1);
        for bad in ["http://registry.example", "https://user:pw@registry.example"] {
            let mut wrong = config.clone();
            wrong.url = Some(bad.into());
            assert!(Registry::open(&dir, &wrong).is_err(), "{bad}");
        }
        let mut wrong = config.clone();
        wrong.announcers = vec!["not a key".into()];
        assert!(Registry::open(&dir, &wrong).is_err());
        cleanup(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn withheld_listings_and_dropped_nodes_answer_like_the_node_service() {
        let dir = temp_dir();
        let now_ms = 1_700_000_000_000u64;
        let clock = Arc::new(AtomicU64::new(now_ms));
        let registry = registry(&dir, clock.clone());
        let (_task, addr) = serve(registry.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let kp = hoi_crypto::Keypair::from_secret(&[8; 32]);
        let (protocol, index) = hoi_core::release_versions();
        let statement = hoi_core::community_publication_statement("garden", &kp.public(), &protocol, index);
        let claim = json!({ "name": "garden", "community": kp.public().hex(), "protocolVersion": protocol, "indexVersion": index,
            "signature": hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0) });
        let client = http::client().unwrap();
        let publish = || client.put(format!("http://{addr}/v2/communities")).header("Content-Type", "application/json").body(claim.to_string()).send();
        assert_eq!(publish().await.unwrap().status().as_u16(), 200);
        // The operator's tool writes the same database while the registry runs.
        let mut admin = Store::open(&dir.join("registry.db")).unwrap();
        admin.hide("garden", 1).unwrap();
        for path in
            ["/v2/communities?name=garden".to_string(), format!("/v2/communities?community={}", kp.public().hex()), "/v1/communities?name=garden".into()]
        {
            let (status, body) = get(addr, &path).await;
            assert_eq!((status, body["error"].as_str()), (404, Some("withheld")), "{path}");
        }
        assert_eq!(get(addr, "/v2/communities?prefix=").await.1, json!([]));
        let again = publish().await.unwrap();
        assert_eq!(again.status().as_u16(), 409);
        assert!(!again.text().await.unwrap().contains(&kp.public().hex()), "a withheld answer names no key");
        admin.unhide("garden").unwrap();
        assert_eq!(get(addr, "/v2/communities?name=garden").await.0, 200);
        cleanup(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn announcements_can_be_turned_off_and_configuration_is_validated() {
        let dir = temp_dir();
        let mut config = RegistryConfig::new(ReleaseSource::compiled().unwrap());
        config.announcements = false;
        config.public_nodes = vec![format!("rtc:{}@127.0.0.1:9000", hoi_types::uri::b64(&[7u8; 32]))];
        let registry = Registry::open(&dir, &config).unwrap();
        let (_task, addr) = serve(registry, "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let client = http::client().unwrap();
        let off = client.put(format!("http://{addr}/v2/nodes")).header("Content-Type", "application/json").body("{}").send().await.unwrap();
        assert_eq!(off.status().as_u16(), 403);
        assert_eq!(get(addr, "/v1/meta").await.1["publicNodes"], json!(config.public_nodes));
        for (field, value) in [("origins", vec!["null".to_string()]), ("public_nodes", vec!["https://x".to_string()])] {
            let mut bad = config.clone();
            if field == "origins" {
                bad.origins = value
            } else {
                bad.public_nodes = value
            }
            assert!(Registry::open(&dir, &bad).is_err(), "{field}");
        }
        let mut bad = config.clone();
        bad.app_url = "http://example.com/".into();
        assert!(Registry::open(&dir, &bad).is_err());
        bad = config.clone();
        bad.limits = (0, 1, 1);
        assert!(Registry::open(&dir, &bad).is_err());
        cleanup(&dir);
    }
}
