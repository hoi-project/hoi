//! hoi-wasm: the core for browsers. The web app runs [`WasmNode`] in a Web Worker; storage
//! (IndexedDB) and sockets stay in JavaScript, which drains [`WasmNode::take_changes`] and
//! [`WasmNode::take_outbox`] after every call. [`mine_range`] lets a pool of mining workers
//! search for stamps in parallel.

use std::sync::{Arc, Mutex};

use hoi_core::{Node, Role};
use hoi_store::{Change, Loaded, Persist};
use js_sys::{Array, Uint8Array};
use wasm_bindgen::prelude::*;

/// The worker keeps this key only while the local database is unlocked.
#[wasm_bindgen]
pub struct WasmVault(hoi_crypto::vault::LocalVault);

fn vault_error(_: hoi_crypto::CryptoError) -> JsValue {
    JsValue::from_str("{\"code\":\"bad-passphrase\",\"message\":\"The passphrase is incorrect or saved data is damaged.\"}")
}

#[wasm_bindgen]
impl WasmVault {
    #[wasm_bindgen(constructor)]
    pub fn new(passphrase: &str) -> Result<WasmVault, JsValue> {
        hoi_crypto::vault::LocalVault::create(passphrase).map(Self).map_err(vault_error)
    }

    pub fn unlock(header: &[u8], passphrase: &str) -> Result<WasmVault, JsValue> {
        hoi_crypto::vault::LocalVault::unlock(header, passphrase).map(Self).map_err(vault_error)
    }

    pub fn header(&self) -> Vec<u8> {
        self.0.header().to_vec()
    }

    /// Opens with a passkey: `prf` is its authenticator's answer for the slot's salt.
    pub fn unlock_passkey(header: &[u8], credential: &[u8], prf: &[u8]) -> Result<WasmVault, JsValue> {
        let prf: [u8; 32] = prf.try_into().map_err(|_| vault_error(hoi_crypto::CryptoError::Format))?;
        hoi_crypto::vault::LocalVault::unlock_passkey(header, credential, &prf).map(Self).map_err(vault_error)
    }

    /// The passkeys a header lists, as `[{ credential, salt }]` (both bytes), before unlocking.
    pub fn passkeys(header: &[u8]) -> Array {
        hoi_crypto::vault::passkeys(header)
            .into_iter()
            .map(|p| {
                let o = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&o, &"credential".into(), &Uint8Array::from(p.credential.as_slice()));
                let _ = js_sys::Reflect::set(&o, &"salt".into(), &Uint8Array::from(p.salt.as_slice()));
                JsValue::from(o)
            })
            .collect()
    }

    /// Lets a passkey open this vault too; the header changes, so the host saves it.
    pub fn add_passkey(&mut self, credential: &[u8], salt: &[u8], prf: &[u8]) -> Result<(), JsValue> {
        let salt: [u8; 32] = salt.try_into().map_err(|_| vault_error(hoi_crypto::CryptoError::Format))?;
        let prf: [u8; 32] = prf.try_into().map_err(|_| vault_error(hoi_crypto::CryptoError::Format))?;
        self.0
            .add_passkey(credential, &salt, &prf)
            .map_err(|_| JsValue::from_str("{\"code\":\"invalid\",\"message\":\"That passkey can't open this vault: four at most.\"}"))
    }

    pub fn remove_passkeys(&mut self) -> Result<(), JsValue> {
        self.0.remove_passkeys().map_err(vault_error)
    }

    pub fn fingerprint(&self, bytes: &[u8]) -> String {
        hoi_crypto::vault::LocalVault::fingerprint(bytes)
    }

    pub fn seal(&self, context: &str, plaintext: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.0.seal(context, plaintext).map_err(vault_error)
    }

    pub fn unseal(&self, context: &str, ciphertext: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.0.unseal(context, ciphertext).map_err(vault_error)
    }
}

#[derive(Clone, Default)]
struct Pending(Arc<Mutex<Vec<Change>>>, Arc<Mutex<Option<Loaded>>>);

impl Persist for Pending {
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.extend_from_slice(changes);
        Ok(())
    }
    fn load(&mut self) -> Result<Loaded, String> {
        Ok(self.1.lock().map_err(|e| e.to_string())?.take().unwrap_or_default())
    }
}

fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn err(e: hoi_core::ApiError) -> JsValue {
    JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| "{\"code\":\"internal\",\"message\":\"?\"}".into()))
}

#[wasm_bindgen]
pub struct WasmNode {
    node: Node,
    pending: Pending,
}

#[wasm_bindgen]
impl WasmNode {
    /// `records`: stored wire records (Uint8Array[]); `blobs`: [key, Uint8Array][] from IndexedDB.
    #[wasm_bindgen(constructor)]
    pub fn new(platform: &str, web_base: &str, records: Array, blobs: Array) -> Result<WasmNode, JsValue> {
        let mut loaded = Loaded::default();
        for r in records.iter() {
            loaded.records.push(Uint8Array::new(&r).to_vec());
        }
        for b in blobs.iter() {
            let pair = Array::from(&b);
            if let Some(k) = pair.get(0).as_string() {
                loaded.blobs.push((k, Uint8Array::new(&pair.get(1)).to_vec()));
            }
        }
        let pending = Pending(Arc::default(), Arc::new(Mutex::new(Some(loaded))));
        let mut node = Node::open(Box::new(pending.clone()), platform, web_base, Box::new(now)).map_err(err)?;
        // A person's browser: operators' notices are read before it says anything.
        node.set_consent_gate(true);
        // Pictures are decoded in a worker of their own, video by the page's player:
        // under panic = "abort" a decoder bug here would take the whole node down.
        node.set_look_elsewhere(true);
        Ok(WasmNode { node, pending })
    }

    /// Calls an API method with a JSON array of arguments; returns JSON. Errors are JSON
    /// `{code, message}` strings.
    pub fn call(&mut self, method: &str, args_json: &str) -> Result<String, JsValue> {
        let args: serde_json::Value = serde_json::from_str(args_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let out = self.node.call(method, args).map_err(err)?;
        serde_json::to_string(&out).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// JSON array of events since the last call.
    pub fn take_events(&mut self) -> String {
        serde_json::to_string(&self.node.take_events()).unwrap_or_else(|_| "[]".into())
    }

    /// Storage changes to apply to IndexedDB, in order:
    /// `{op:"put", id, community, expiresAt, wire}`, `{op:"delete", id}`, `{op:"blob", key, value}`, `{op:"wipe"}`.
    pub fn take_changes(&mut self) -> Array {
        let changes: Vec<Change> = self.pending.0.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
        let out = Array::new();
        for c in changes {
            let o = js_sys::Object::new();
            let set = |k: &str, v: &JsValue| {
                let _ = js_sys::Reflect::set(&o, &JsValue::from_str(k), v);
            };
            match c {
                Change::Put { id, community, expires_at, wire } => {
                    set("op", &"put".into());
                    set("id", &id.hex().into());
                    set("community", &community.map(|c| c.hex()).map_or(JsValue::NULL, JsValue::from));
                    set("expiresAt", &JsValue::from_f64(expires_at as f64));
                    set("wire", &Uint8Array::from(wire.as_slice()).into());
                }
                Change::Delete { id } => {
                    set("op", &"delete".into());
                    set("id", &id.hex().into());
                }
                Change::Blob { key, value } => {
                    set("op", &"blob".into());
                    set("key", &key.into());
                    set("value", &value.map_or(JsValue::NULL, |v| Uint8Array::from(v.as_slice()).into()));
                }
                Change::Wipe => set("op", &"wipe".into()),
            }
            out.push(&o);
        }
        out
    }

    /// `role`: "home" (we dialed our home node), "client" (a device dialed us), "peer".
    pub fn net_connect(&mut self, peer: &str, role: &str) {
        let r = match role {
            "home" => Role::Home,
            "client" => Role::Client,
            _ => Role::Peer,
        };
        self.node.net_connect(peer, r);
    }

    /// How a connection travels: `transport` "websocket" or "webrtc"; `local`: the bytes never
    /// leave this machine (a loopback home node); `tls`: a `wss://` address; `dialed`: a JSON
    /// array of the `rtc:` routes this WebRTC link was opened with (one node's, one
    /// certificate), so the core knows which communities' routes led there.
    /// `rtc`, for a WebRTC link: JSON `{node, browser, ufrag}`, the node's and this browser's
    /// certificate fingerprints (base64url SHA-256) and the ICE username fragment, which the node
    /// binds to its key before anything else is taken from the link (hoi_core::bind).
    pub fn set_peer_meta(&mut self, peer: &str, transport: &str, local: bool, tls: bool, dialed: Option<String>, rtc: Option<String>) {
        let dialed: Vec<String> = dialed.and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default();
        let dialed = dialed.into_iter().take(hoi_types::uri::MAX_PAIR_ROUTES * 2).collect();
        #[derive(serde::Deserialize)]
        struct Channel {
            node: String,
            browser: String,
            ufrag: String,
        }
        let rtc = rtc.and_then(|r| serde_json::from_str::<Channel>(&r).ok()).and_then(|c| {
            let node = hoi_types::uri::unb64(&c.node).ok().filter(|f| f.len() == 32)?;
            let browser = hoi_types::uri::unb64(&c.browser).ok().filter(|f| f.len() == 32)?;
            (c.ufrag.len() <= 256).then_some(hoi_core::bind::RtcChannel { node, browser, ufrag: c.ufrag })
        });
        self.node.set_peer_meta(peer, hoi_core::PeerMeta { transport: transport.into(), local, tls, dialed, rtc, ..Default::default() });
    }

    /// JSON array of `rtc:` routes of members' nodes to try when there is no home node.
    pub fn rtc_candidates(&self) -> String {
        serde_json::to_string(&self.node.rtc_candidates()).unwrap_or_else(|_| "[]".into())
    }

    /// How many more links to keep for the mailboxes of private groups (crate::mailbox): one
    /// per mailbox in use, or a few while a group this device owns is still choosing them.
    pub fn mailbox_links(&self) -> u32 {
        self.node.mailbox_links() as u32
    }

    /// JSON array of `rtc:` routes of public nodes to try for finding communities and other
    /// nodes: links opened here, nodes reached before, then what other nodes passed on.
    pub fn discovery_candidates(&self) -> String {
        serde_json::to_string(&self.node.discovery_candidates()).unwrap_or_else(|_| "[]".into())
    }

    /// A dial of this route got nowhere: routes passed on for that node go, and a node kept
    /// here is left alone for a while.
    pub fn dial_failed(&mut self, route: &str) {
        self.node.dial_failed(route);
    }

    /// Returns false when the connection should be dropped.
    pub fn net_receive(&mut self, peer: &str, bytes: &[u8]) -> bool {
        let keep = self.node.net_receive(peer, bytes);
        let _ = self.node.commit();
        keep
    }

    /// Whether an active catch-up stopped making progress and needs a fresh connection.
    pub fn sync_stalled(&self, peer: &str) -> bool {
        self.node.sync_stalled(peer)
    }

    /// Whether the core ended `peer` for an operator's notice the person has not read yet
    /// (crate::notice): no refusal to count, no backoff; it is dialed again once answered.
    pub fn consent_closed(&self, peer: &str) -> bool {
        self.node.consent_closed(peer)
    }

    pub fn net_disconnect(&mut self, peer: &str) {
        self.node.net_disconnect(peer);
    }

    pub fn set_net_error(&mut self, msg: Option<String>) {
        self.node.set_net_error(msg);
    }

    /// `[[peer, Uint8Array], …]` messages to send.
    pub fn take_outbox(&mut self) -> Array {
        let out = Array::new();
        for (peer, bytes) in self.node.take_outbox() {
            let pair = Array::new();
            pair.push(&JsValue::from_str(&peer));
            pair.push(&Uint8Array::from(bytes.as_slice()));
            out.push(&pair);
        }
        out
    }

    /// The paired home node as JSON `{url, token, name, rtc}` or null.
    pub fn home(&self) -> String {
        serde_json::to_string(&self.node.home()).unwrap_or_else(|_| "null".into())
    }

    pub fn sweep(&mut self) -> f64 {
        let n = self.node.sweep();
        let _ = self.node.commit();
        n as f64
    }

    /// Media waiting for a look (hoi_core::look): `[{id, mime, data: Uint8Array}, …]`, each
    /// handed out once. Answer each with [`WasmNode::looked`].
    pub fn looks_wanted(&mut self) -> Array {
        let out = Array::new();
        for (id, mime, data) in self.node.looks_wanted() {
            let o = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&o, &"id".into(), &id.hex().into());
            let _ = js_sys::Reflect::set(&o, &"mime".into(), &mime.into());
            let _ = js_sys::Reflect::set(&o, &"data".into(), &Uint8Array::from(data.as_slice()).into());
            out.push(&o);
        }
        out
    }

    /// Domains to check as handles (hoi_core::handle), each handed out once: `["mira.example.org", …]`.
    /// Answer each with [`WasmNode::handle_checked`].
    #[wasm_bindgen(js_name = handleChecks)]
    pub fn handle_checks(&mut self) -> Array {
        self.node.take_handle_checks().into_iter().map(|c| JsValue::from(c.domain)).collect()
    }

    /// What `https://<domain>/.well-known/hoi` answered, line by line; none when it didn't.
    #[wasm_bindgen(js_name = handleChecked)]
    pub fn handle_checked(&mut self, domain: &str, lines: Vec<String>) {
        self.node.handle_checked(domain, lines);
        let _ = self.node.commit();
    }

    /// What a look worker saw, as JSON (hoi_core::look::LookDto). A malformed look reads as an
    /// unreadable picture.
    pub fn looked(&mut self, id: &str, look_json: &str) {
        let Ok(id) = hoi_types::Id::from_hex(id) else { return };
        let look = serde_json::from_str::<hoi_core::look::LookDto>(look_json).map_or(hoi_core::look::Look::Unreadable, |l| l.parse());
        self.node.looked(id, look);
        let _ = self.node.commit();
    }

    /// Direct links with other members' browsers (hoi_core::signal): links to start and offers
    /// or answers to hand on, as a JSON array of `{t: "offer", via, to, stun}` and
    /// `{t: "signal", via, from, stun, data}`.
    #[wasm_bindgen(js_name = takeDirect)]
    pub fn take_direct(&mut self) -> String {
        serde_json::to_string(&self.node.take_direct()).unwrap_or_else(|_| "[]".into())
    }

    /// Passes an offer or an answer to `to` (a handle) through the node at `via`.
    pub fn signal(&mut self, via: &str, to: &str, data: &[u8]) -> Result<(), JsValue> {
        self.node.signal(via, to, data).map_err(err)
    }

    /// A direct link opened as `peer`: through the node at `via`, with the other side's handle
    /// there; `incoming` when this side answered. Call right after `net_connect`.
    #[wasm_bindgen(js_name = setDirectMeta)]
    pub fn set_direct_meta(&mut self, peer: &str, via: &str, handle: &str, incoming: bool) {
        let Some(h) = hoi_types::hex_decode(handle).ok().and_then(|b| <[u8; 16]>::try_from(b).ok()) else { return };
        self.node.set_peer_meta(peer, hoi_core::PeerMeta { transport: "webrtc".into(), incoming, direct: Some((via.to_string(), h)), ..Default::default() });
    }

    /// The scattered-delivery round, every few seconds: answers that never came, holder
    /// sightings, and the mailboxes of private groups (SCATTER.md §5.13).
    #[wasm_bindgen(js_name = scatterTick)]
    pub fn scatter_tick(&mut self) {
        self.node.scatter_tick();
        let _ = self.node.commit();
    }
}

/// Searches nonces `start, start+step, …` for `tries` tries. Returns the nonce or -1.
/// Nonces stay below 2^53 so they survive the trip through a JS number.
#[wasm_bindgen]
pub fn mine_range(prefix: &[u8], suffix: &[u8], bits: u32, start: f64, step: f64, tries: f64) -> f64 {
    let p = hoi_crypto::pow::Puzzle { prefix: prefix.to_vec(), suffix: suffix.to_vec(), bits };
    match p.search(start as u64, step as u64, tries as u64) {
        Some(n) => n as f64,
        None => -1.0,
    }
}

/// Identity work one try at a time (PROTOCOL.md §4.1), so the page can mine a new key's work
/// across workers and show every attempt. One 8 MiB buffer per miner.
#[wasm_bindgen]
pub struct IdentityMiner(hoi_crypto::identity::Miner);

#[wasm_bindgen]
impl IdentityMiner {
    /// `root`: the 32-byte public root key.
    #[wasm_bindgen(constructor)]
    pub fn new(root: &[u8]) -> Result<IdentityMiner, JsValue> {
        let root: [u8; 32] = root.try_into().map_err(|_| JsValue::from_str("a root key is 32 bytes"))?;
        Ok(IdentityMiner(hoi_crypto::identity::Miner::new(hoi_types::Id(root))))
    }

    /// The Argon2id output for `work` (32 bytes); its leading zero bits are the work it carries.
    /// Nonces stay below 2^53 so they survive the trip through a JS number.
    pub fn attempt(&mut self, work: f64) -> Vec<u8> {
        self.0.hash(work as u64).to_vec()
    }
}

/// For the look worker (hoi_core::look): the look of media of type `mime`, checked first, as
/// JSON. Runs in a worker of its own, so a decoder bug there costs one picture, not the node.
#[wasm_bindgen]
pub fn look_media(mime: &str, bytes: &[u8]) -> String {
    let look = if hoi_validate::media::check(mime, bytes).is_ok() { hoi_core::look::look_here(mime, bytes) } else { hoi_core::look::Look::Unreadable };
    serde_json::to_string(&hoi_core::look::LookDto::from(&look)).unwrap_or_else(|_| "\"unreadable\"".into())
}

/// For the look worker: a file someone attaches, sanitized as the core will, and its look:
/// JSON `{hash, mime, look}`, or an error string when the file won't do.
#[wasm_bindgen]
pub fn sanitize_and_look(raw: &[u8]) -> Result<String, JsValue> {
    let (hash, mime, look) = hoi_core::look::sanitize_and_look(raw).map_err(|e| JsValue::from_str(&e))?;
    Ok(serde_json::json!({ "hash": hash.hex(), "mime": mime, "look": hoi_core::look::LookDto::from(&look) }).to_string())
}

/// For the look worker: one video frame's luma (w×h, row-major, 0–255), fingerprinted: JSON
/// `{hashes, quality}`, or `null` when the frame is too small or too flat to say anything.
#[wasm_bindgen]
pub fn look_luma(luma: &[f32], w: u32, h: u32) -> String {
    match hoi_validate::similar::luma_variants(luma.to_vec(), w as usize, h as usize) {
        Some(v) => serde_json::to_string(&hoi_core::look::VariantsDto::from(&v)).unwrap_or_else(|_| "null".into()),
        None => "null".into(),
    }
}

#[wasm_bindgen]
pub fn core_version() -> String {
    hoi_core::VERSION.to_string()
}
