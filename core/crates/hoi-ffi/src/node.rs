//! [`HoiNode`]: the node a phone app holds, and the typed `HoiCore` API on it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hoi_core::dto::*;
use hoi_core::{ApiError, Node, Published};
use hoi_net::Shared;
use hoi_store::sqlite::Sqlite;
use tokio::runtime::Runtime;

use crate::events::{Events, lock};
use crate::keys::Split;
use crate::link::Link;
use crate::{EventListener, HoiError, KeyStore};

/// The node's platform name: device labels say "This phone", the relay pill "only on this phone".
const PLATFORM: &str = "mobile";
/// Where https invite links point (`https://hoi.sh/app/#/join?…`).
const WEB_BASE: &str = "https://hoi.sh/app/";
/// A sync burst is over once nothing changed for this long.
const QUIET: Duration = Duration::from_secs(2);
/// Hashes per second assumed before this phone's own benchmark is in.
const FALLBACK_HASHRATE: f64 = 4.0e6;

type R<T> = Result<T, HoiError>;

/// One hoi node: this phone's keys, records and settings, and its link to a home node.
///
/// Open one per process, keep it for the app's lifetime, and call it from background threads:
/// every method blocks while it reads storage, validates, mines or writes.
///
/// Lifecycle on a phone (no background sockets, TECHNICAL.md §4):
/// - App comes to the foreground: [`HoiNode::start_home_link`] (and [`HoiNode::sweep`]).
/// - App goes to the background: [`HoiNode::stop_home_link`].
/// - Background refresh (iOS `BGAppRefreshTask`, Android `WorkManager`): [`HoiNode::sync_burst`]
///   starts the link, waits until the home node has gone quiet, and stops it again.
/// - Show [`HoiNode::last_synced_at`] when coming back: "last synced 4 min ago".
#[derive(uniffi::Object)]
pub struct HoiNode {
    shared: Arc<Shared>,
    rt: Option<Runtime>,
    events: Arc<Events>,
    link: Mutex<Link>,
    hashrate: Arc<OnceLock<f64>>,
}

impl HoiNode {
    fn api<T>(&self, f: impl FnOnce(&mut Node) -> Result<T, ApiError>) -> R<T> {
        self.shared.with(f).map_err(HoiError::from)
    }

    fn read<T>(&self, f: impl FnOnce(&mut Node) -> T) -> T {
        self.shared.with(f)
    }

    fn rt(&self) -> &tokio::runtime::Handle {
        self.rt.as_ref().expect("runtime lives as long as the node").handle()
    }

    /// Expected milliseconds of work for `bits`, from this phone's benchmark.
    fn estimate_ms(&self, bits: u32) -> u64 {
        let rate = self.hashrate.get().copied().unwrap_or(FALLBACK_HASHRATE).max(1.0);
        (2f64.powi(bits as i32) / rate * 1000.0).round() as u64
    }

    /// Draft under the lock, mine without it (other calls and the link keep going), publish.
    fn mined(&self, draft: impl FnOnce(&mut Node) -> Result<DraftDto, ApiError>) -> R<Published> {
        let d = self.api(draft)?;
        let bad = |_| HoiError::new("internal", "The draft came back damaged.");
        let puzzle = hoi_crypto::pow::Puzzle {
            prefix: hoi_types::uri::unb64(&d.prefix).map_err(bad)?,
            suffix: hoi_types::uri::unb64(&d.suffix).map_err(bad)?,
            bits: d.bits,
        };
        let estimate = self.estimate_ms(d.bits);
        self.events.stamp_start(d.bits, estimate);
        let t = Instant::now();
        let nonce = hoi_core::dispatch::mine_puzzle(&puzzle);
        self.events.stamp_done(d.bits, estimate, t.elapsed().as_millis() as u64);
        self.api(|n| n.publish_draft(d.token, nonce))
    }

    fn restart_link_if_running(&self) {
        let running = lock(&self.link).is_running();
        if running {
            self.start_home_link();
        }
    }
}

impl Drop for HoiNode {
    fn drop(&mut self) {
        if let Some(rt) = self.rt.take() {
            lock(&self.link).stop(rt.handle(), &self.shared);
            if tokio::runtime::Handle::try_current().is_ok() {
                rt.shutdown_background();
            } else {
                rt.shutdown_timeout(Duration::from_millis(500));
            }
        }
        self.events.flush_sync();
    }
}

// ---------------------------------------------------------------- lifecycle and network

#[uniffi::export]
impl HoiNode {
    /// Opens (or creates) the node in `data_dir`: SQLite at `<data_dir>/hoi.db`, secret keys in
    /// `keystore`, events to `listener`. Replays and re-validates everything stored, so call it
    /// off the main thread. iOS: Application Support; Android: `Context.filesDir`.
    #[uniffi::constructor]
    pub fn open(data_dir: String, keystore: Arc<dyn KeyStore>, listener: Arc<dyn EventListener>) -> R<Arc<HoiNode>> {
        let dir = PathBuf::from(&data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| HoiError::new("internal", format!("Can't create {data_dir}: {e}")))?;
        let db = Sqlite::open(&dir.join("hoi.db")).map_err(|e| HoiError::new("internal", format!("Can't open storage: {e}")))?;
        let mut node = Node::open(Box::new(Split::new(db, keystore)), PLATFORM, WEB_BASE, hoi_core::system_clock())?;
        // A person's phone: operators' notices are read before it says anything.
        node.set_consent_gate(true);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("hoi")
            .enable_all()
            .build()
            .map_err(|e| HoiError::new("internal", format!("Can't start the network runtime: {e}")))?;
        let events = Arc::new(Events::new(listener, dir.join("last-sync")));
        let sink = events.clone();
        let shared = Shared::new(node, Box::new(move |evs| sink.core(evs)));
        let _sweeper = {
            let _guard = rt.enter();
            hoi_net::spawn_sweeper(shared.clone())
        };
        let hashrate = Arc::new(OnceLock::new());
        let hr = hashrate.clone();
        rt.spawn_blocking(move || {
            let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16) as f64;
            hr.get_or_init(|| hoi_crypto::pow::benchmark(100_000) * threads);
        });
        Ok(Arc::new(HoiNode { shared, rt: Some(rt), events, link: Mutex::new(Link::default()), hashrate }))
    }

    /// Connects to the paired home node and keeps the link up (retrying with backoff) until
    /// [`HoiNode::stop_home_link`]. Call when the app comes to the foreground. Returns false when
    /// no home node is paired. Progress arrives as `net` events.
    pub fn start_home_link(&self) -> bool {
        let Some(home) = self.read(|n| n.home()) else { return false };
        lock(&self.link).start(self.rt(), &self.shared, &self.events, home.url);
        true
    }

    /// Closes the link. Call when the app goes to the background: phones get no background
    /// sockets, and a socket the OS kills silently is worse than one closed politely.
    pub fn stop_home_link(&self) {
        lock(&self.link).stop(self.rt(), &self.shared);
        self.events.flush_sync();
    }

    /// One background sync: starts the link, waits until the home node has been reached and
    /// nothing changed for a couple of seconds (or `max_seconds` passed), then stops it again
    /// unless the app had it running. Blocks; call it from the background task's thread
    /// (iOS `BGAppRefreshTask`: keep `max_seconds` under ~25; Android `WorkManager`: a worker).
    /// Returns whether the home node was reached.
    pub fn sync_burst(&self, max_seconds: u32) -> bool {
        let was_running = lock(&self.link).is_running();
        if !self.start_home_link() {
            return false;
        }
        let started = Instant::now();
        let max = Duration::from_secs(u64::from(max_seconds.max(1)));
        let mut reached: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let st = self.read(|n| n.net_status());
            if st.home_node.as_ref().is_some_and(|h| h.connected) {
                self.events.net(&st);
                reached.get_or_insert_with(Instant::now);
            }
            if reached.is_some_and(|t| t.elapsed() >= QUIET) && self.events.since_change() >= QUIET {
                break;
            }
            if started.elapsed() >= max {
                break;
            }
        }
        if !was_running {
            self.stop_home_link();
        }
        reached.is_some()
    }

    /// When this phone last reached its home node (Unix seconds), kept across restarts. Nothing
    /// if it never has. "last synced 4 min ago".
    pub fn last_synced_at(&self) -> Option<u64> {
        self.events.last_synced_at()
    }

    /// The relay pill: "via mira's computer", "looking for your home node", "only on this phone".
    pub fn net_status(&self) -> NetStatus {
        self.read(|n| n.net_status())
    }

    /// Pairs with a home node (a desktop app or `hoi-node`): `hoi://pair?u=ws://…&t=…`, usually
    /// scanned from its QR code. A running link moves over to the new home node.
    pub fn pair(&self, link: String) -> R<NetStatus> {
        let st = self.api(|n| n.pair(&link))?;
        self.events.net(&st);
        self.restart_link_if_running();
        Ok(st)
    }

    /// Operators' notices this phone met: pending ones first. `net_status().pending_notices`
    /// holds those waiting for an answer; a `nodeNotice` event announces a new one. Show the
    /// sections as they are and answer with [`HoiNode::accept_notice`] or [`HoiNode::decline_notice`].
    pub fn node_notices(&self) -> Vec<NoticeView> {
        self.read(|n| n.node_notices())
    }

    /// The person read a node's notice and accepts it (by its hash): the node is talked to again.
    pub fn accept_notice(&self, node: String, hash: String) -> R<NoticeView> {
        self.api(|n| n.accept_notice(&node, &hash))
    }

    /// The person declines: the node is left alone until the notice is forgotten.
    pub fn decline_notice(&self, node: String) -> R<NoticeView> {
        self.api(|n| n.decline_notice(&node))
    }

    pub fn forget_notice(&self, node: String) -> R<()> {
        self.api(|n| n.forget_notice(&node))
    }

    /// What a node link (`hoi://node?…`, scanned or pasted) says before anything is dialed: where
    /// it points and the operator's notice in it, verified against the node it names.
    pub fn preview_node_link(&self, link: String) -> R<NodeLinkPreview> {
        self.api(|n| n.preview_node_link(&link))
    }

    /// Forgets the home node and closes the link.
    pub fn unpair(&self) -> NetStatus {
        self.stop_home_link();
        let st = self.read(|n| n.unpair());
        self.events.forget_sync();
        self.events.net(&st);
        st
    }

    /// Deletes everything past its expiry. Runs every few minutes on its own while the app is
    /// alive; call it when the app comes back to the foreground too. Returns how many went.
    pub fn sweep(&self) -> u64 {
        self.read(|n| n.sweep())
    }

    /// Any `HoiCore` method by name with a JSON array of arguments (bytes as base64url), for what
    /// isn't typed here yet: `call("draftPost", "[{…}]")`. Returns JSON. Prefer the typed methods
    /// for pairing and wiping: they also manage the link.
    pub fn call(&self, method: String, args_json: String) -> R<String> {
        let args: serde_json::Value = if args_json.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&args_json).map_err(|e| HoiError::new("invalid", format!("Arguments aren't JSON: {e}.")))?
        };
        let out = self.api(|n| n.call(&method, args))?;
        serde_json::to_string(&out).map_err(|e| HoiError::new("internal", e.to_string()))
    }
}

// ---------------------------------------------------------------- you

#[uniffi::export]
impl HoiNode {
    pub fn boot(&self) -> Boot {
        self.read(|n| n.boot())
    }

    pub fn me(&self) -> Option<Me> {
        self.read(|n| n.me())
    }

    /// Makes this phone's root and device keys. They go to the [`KeyStore`].
    pub fn create_identity(&self, name: String) -> R<Me> {
        self.api(|n| n.create_identity(&name))
    }

    pub fn set_name(&self, name: String) -> R<Me> {
        self.api(|n| n.set_name(&name))
    }

    /// A square JPEG of at most 128 pixels and 4 KiB. Empty restores the identicon.
    pub fn set_avatar(&self, image: Vec<u8>) -> R<Me> {
        self.api(|n| n.set_avatar(&image))
    }

    pub fn set_avatar_color(&self, color: Option<String>) -> R<Me> {
        self.api(|n| n.set_avatar_color(color.as_deref()))
    }

    /// Your keys, encrypted with Argon2id + XChaCha20-Poly1305 under `passphrase`. Slow on
    /// purpose (about a second).
    pub fn export_identity(&self, passphrase: String) -> R<Vec<u8>> {
        self.api(|n| n.export_identity(&passphrase))
    }

    pub fn import_identity(&self, data: Vec<u8>, passphrase: String) -> R<Me> {
        self.api(|n| n.import_identity(&data, &passphrase))
    }

    /// The recovery line (`hoi-key-1-…`) for the person's password manager, while this device
    /// keeps it. Whoever holds it can always replace the person's root.
    pub fn recovery_key(&self) -> R<String> {
        self.api(|n| n.recovery_key())
    }

    /// The line is saved: this device forgets it.
    pub fn forget_recovery_key(&self) -> R<()> {
        self.api(|n| n.forget_recovery_key().map(|_| ()))
    }

    /// Brings a person back from their recovery line on a phone without a key: a new root that
    /// takes over from the old one, a fresh device key; the standing stays.
    pub fn restore_identity(&self, recovery_key: String, name: String) -> R<Me> {
        self.api(|n| n.restore_identity(&recovery_key, &name))
    }

    /// On a new phone: a `hoi:link:…` code to show as QR to a device that holds your root key.
    pub fn link_request(&self, label: String) -> R<String> {
        self.api(|n| n.link_request(&label))
    }

    /// On the root device: blesses the new device; returns a code for it.
    pub fn link_approve(&self, request: String) -> R<String> {
        self.api(|n| n.link_approve(&request))
    }

    /// On the new phone: finishes linking with the approval code.
    pub fn link_complete(&self, approval: String) -> R<Me> {
        self.api(|n| n.link_complete(&approval))
    }

    pub fn revoke_device(&self, key: String) -> R<Me> {
        self.api(|n| n.revoke_device(&key))
    }

    /// Mines more identity work for this key (a community may ask for more). Takes seconds:
    /// call from a background thread.
    pub fn strengthen_identity(&self, bits: u32) -> R<Me> {
        self.api(|n| n.strengthen_identity(bits))
    }

    /// Like `strengthen_identity`, with work the app mined itself (so it can show and stop it).
    pub fn strengthen_identity_with(&self, bits: u32, work: u64) -> R<Me> {
        self.api(|n| n.strengthen_identity_with(bits, work))
    }

    /// Names one of your devices, on this phone only.
    pub fn set_device_label(&self, key: String, label: String) -> R<Me> {
        self.api(|n| n.set_device_label(&key, &label))
    }

    /// Burns this identity for good: every device drops what it signed and refuses what it signs
    /// later. For a leaked key. `confirm` is the key's short form, as shown in `Me.short`.
    pub fn burn_identity(&self, confirm: String) -> R<()> {
        self.api(|n| n.burn_identity(&confirm))
    }

    /// Loads perceptual fingerprints of pictures to refuse (a PDQ hash list, one hash per line).
    /// Returns how many were new.
    pub fn import_blocklist(&self, text: String) -> R<u64> {
        self.api(|n| n.import_blocklist(&text))
    }

    /// Deletes everything on this phone: keys (from the [`KeyStore`] too), records, settings.
    pub fn wipe(&self) -> R<()> {
        self.stop_home_link();
        self.api(|n| n.wipe())?;
        self.events.forget_sync();
        Ok(())
    }
}

// ---------------------------------------------------------------- communities

#[uniffi::export]
impl HoiNode {
    pub fn communities(&self) -> Vec<CommunitySummary> {
        self.read(|n| n.communities())
    }

    pub fn discover_communities(&self) -> Vec<DiscoveredCommunity> {
        self.read(|n| n.discover_communities())
    }

    pub fn community_registration(&self, id: String) -> R<CommunityRegistration> {
        self.api(|n| n.community_registration(&id))
    }

    pub fn community_publication(&self, id: String) -> R<CommunityPublication> {
        self.api(|n| n.community_publication(&id))
    }

    pub fn public_join(&self, id: String, peers: Vec<String>, name: Option<String>) -> R<PublicJoinStatus> {
        self.api(|n| n.public_join(&id, &peers, name.as_deref()))
    }

    pub fn public_join_status(&self, id: String) -> R<PublicJoinStatus> {
        self.api(|n| n.public_join_status(&id))
    }

    /// How safe each public community's records are, read from this phone's own links
    /// (PROTOCOL.md §20): copies seen, holders, completeness. Never a delivery receipt.
    pub fn warmth(&self) -> WarmthView {
        self.read(|n| n.warmth())
    }

    pub fn community_warmth(&self, id: String) -> R<CommunityWarmth> {
        self.api(|n| n.community_warmth(&id))
    }

    /// Asks the connected public nodes that take requests to keep an open community available.
    /// Their answers arrive as notices; Warmth shows what actually arrived.
    pub fn request_carry(&self, id: String) -> R<CarryRequest> {
        self.api(|n| n.request_carry(&id))
    }

    pub fn community(&self, id: String) -> R<CommunityView> {
        self.api(|n| n.community(&id))
    }

    pub fn create_community(&self, draft: CommunityDraft) -> R<CommunityView> {
        self.api(|n| n.create_community(draft))
    }

    /// What an invite link (`hoi://join?…` or `https://hoi.sh/app/#/join?…`) leads to.
    pub fn preview_invite(&self, link: String) -> R<InvitePreview> {
        self.api(|n| n.preview_invite(&link))
    }

    pub fn join(&self, link: String) -> R<CommunityView> {
        self.api(|n| n.join(&link))
    }

    pub fn leave(&self, id: String) -> R<()> {
        self.api(|n| n.leave(&id))
    }

    pub fn set_pinning(&self, id: String, on: bool) -> R<CommunityView> {
        self.api(|n| n.set_pinning(&id, on))
    }

    /// `uri` for QR codes and other hoi apps, `web` for everyone else.
    pub fn invite(&self, id: String) -> R<Invite> {
        // Sharing an invite asks the public nodes in reach to keep the community.
        self.api(|n| {
            n.carry_for_invite(&id)?;
            n.invite(&id)
        })
    }

    pub fn export_bundle(&self, id: String) -> R<Vec<u8>> {
        self.api(|n| n.export_bundle(&id))
    }

    pub fn import_bundle(&self, data: Vec<u8>) -> R<ImportReport> {
        self.api(|n| n.import_bundle(&data))
    }
}

// ---------------------------------------------------------------- owners and mods

#[uniffi::export]
impl HoiNode {
    pub fn set_limits(&self, id: String, limits: LimitsDto) -> R<LedgerResult> {
        self.api(|n| n.set_limits(&id, &limits))
    }

    pub fn set_rules(&self, id: String, text: String) -> R<LedgerResult> {
        self.api(|n| n.set_rules(&id, &text))
    }

    pub fn add_mod(&self, id: String, key: String) -> R<LedgerResult> {
        self.api(|n| n.add_mod(&id, &key))
    }

    pub fn remove_mod(&self, id: String, key: String) -> R<LedgerResult> {
        self.api(|n| n.remove_mod(&id, &key))
    }

    /// Says an elector left for good.
    pub fn retire(&self, id: String, key: String) -> R<LedgerResult> {
        self.api(|n| n.retire(&id, &key))
    }

    /// Adds your signature to a `hoi:cosign:…` proposal from another owner.
    pub fn cosign(&self, proposal: String) -> R<LedgerResult> {
        self.api(|n| n.cosign(&proposal))
    }

    pub fn ledger(&self, id: String) -> R<Vec<LedgerEntryView>> {
        self.api(|n| n.ledger(&id))
    }

    pub fn remove(&self, target: String, reason: String) -> R<LedgerEntryView> {
        self.api(|n| n.remove(&target, &reason))
    }

    pub fn pin(&self, target: String, on: bool) -> R<LedgerEntryView> {
        self.api(|n| n.pin(&target, on))
    }

    #[uniffi::method(default(reason = None))]
    pub fn label(&self, target: String, label: String, reason: Option<String>) -> R<LedgerEntryView> {
        self.api(|n| n.label(&target, &label, reason.as_deref()))
    }

    pub fn ban(&self, community: String, key: String, reason: String) -> R<LedgerEntryView> {
        self.api(|n| n.ban(&community, &key, &reason))
    }

    pub fn unban(&self, community: String, key: String, reason: String) -> R<LedgerEntryView> {
        self.api(|n| n.unban(&community, &key, &reason))
    }

    pub fn admit(&self, community: String, key: String) -> R<LedgerEntryView> {
        self.api(|n| n.admit(&community, &key))
    }
}

// ---------------------------------------------------------------- the ladder

#[uniffi::export]
impl HoiNode {
    /// Where someone (you, without `key`) stands in a community, and what's missing.
    #[uniffi::method(default(key = None))]
    pub fn standing(&self, community: String, key: Option<String>) -> R<StandingView> {
        self.api(|n| n.standing(&community, key.as_deref()))
    }

    /// 1 vouches, -1 objects (`reason`: "spam", "abuse", "illegal", "off-topic" or "rule:<n>"),
    /// 0 takes it back. Counts once you are established in the community.
    #[uniffi::method(default(reason = None))]
    pub fn vouch(&self, community: String, key: String, value: i32, reason: Option<String>) -> R<StandingView> {
        self.api(|n| n.vouch(&community, &key, i64::from(value), reason.as_deref()))
    }

    /// `verdict`: "ok" or "hide" (with a reason) for posts and comments; "veto" (with a reason)
    /// or "ok" for moderators' and owners' ledger entries; "none" takes it back.
    #[uniffi::method(default(reason = None))]
    pub fn review(&self, target: String, verdict: String, reason: Option<String>) -> R<()> {
        self.api(|n| n.review(&target, &verdict, reason.as_deref()))
    }

    /// Newcomers' posts waiting for your review (established members).
    pub fn review_queue(&self, community: String) -> R<Vec<PostSummary>> {
        self.api(|n| n.review_queue(&community))
    }

    /// Checked media bytes for display (base64url in `data`).
    pub fn media(&self, hash: String) -> R<MediaBlob> {
        self.api(|n| n.media(&hash))
    }
}

// ---------------------------------------------------------------- posts

#[uniffi::export]
impl HoiNode {
    /// `sort`: "hot" (default), "fresh", "loudest", "friends". Without `community`, everything
    /// you joined.
    #[uniffi::method(default(community = None, sort = None, limit = None))]
    pub fn feed(&self, community: Option<String>, sort: Option<String>, limit: Option<u32>) -> R<Vec<PostSummary>> {
        self.api(|n| n.feed(community.as_deref(), sort.as_deref().unwrap_or("hot"), limit.map(|l| l as usize)))
    }

    pub fn post(&self, id: String) -> R<PostView> {
        self.api(|n| n.post(&id))
    }

    /// Signs and sends a post. Mines its stamp first (`stamp` events around it, typically well
    /// under a second): call from a background thread or coroutine.
    pub fn create_post(&self, draft: PostDraft) -> R<PostSummary> {
        match self.mined(|n| n.draft_post(&draft))? {
            Published::Post(p) => Ok(*p),
            _ => Err(HoiError::new("internal", "The post vanished.")),
        }
    }

    /// Signs and sends a reply. Mines like [`HoiNode::create_post`].
    pub fn reply(&self, parent: String, text: String) -> R<CommentView> {
        match self.mined(|n| n.draft_reply(&parent, &text))? {
            Published::Comment(c) => Ok(*c),
            _ => Err(HoiError::new("internal", "The reply vanished.")),
        }
    }

    /// Edits your post or comment (`title` for posts). Mines like [`HoiNode::create_post`].
    #[uniffi::method(default(title = None))]
    pub fn edit(&self, id: String, text: String, title: Option<String>) -> R<()> {
        self.mined(|n| n.draft_edit(&id, &text, title.as_deref())).map(|_| ())
    }

    pub fn delete_own(&self, id: String) -> R<()> {
        self.api(|n| n.delete_own(&id))
    }

    /// `value`: 1 up, -1 down, 0 take it back. Returns the new score.
    pub fn vote(&self, target: String, value: i32) -> R<i64> {
        self.api(|n| n.vote(&target, i64::from(value)))
    }

    /// A signed flag for the mods. `reason`: "spam", "rule:<n>" or "illegal".
    pub fn flag(&self, target: String, reason: String) -> R<()> {
        self.api(|n| n.flag(&target, &reason))
    }

    pub fn save(&self, id: String, on: bool) -> R<()> {
        self.api(|n| n.save(&id, on))
    }

    pub fn saved(&self) -> Vec<PostSummary> {
        self.read(|n| n.saved())
    }

    /// Searches what's on this phone. Nothing leaves it.
    #[uniffi::method(default(community = None))]
    pub fn search(&self, q: String, community: Option<String>) -> R<Vec<PostSummary>> {
        self.api(|n| n.search(&q, community.as_deref()))
    }
}

// ---------------------------------------------------------------- people and the node

#[uniffi::export]
impl HoiNode {
    #[uniffi::method(default(community = None))]
    pub fn person(&self, key: String, community: Option<String>) -> R<PersonView> {
        self.api(|n| n.person_view(&key, community.as_deref()))
    }

    pub fn set_trust(&self, key: String, friend: bool) -> R<Person> {
        self.api(|n| n.set_trust(&key, friend))
    }

    pub fn set_muted(&self, key: String, muted: bool) -> R<Person> {
        self.api(|n| n.set_muted(&key, muted))
    }

    pub fn set_petname(&self, key: String, petname: Option<String>) -> R<Person> {
        self.api(|n| n.set_petname(&key, petname.as_deref()))
    }

    pub fn stats(&self) -> NodeStats {
        self.read(|n| n.stats())
    }

    pub fn settings(&self) -> Settings {
        self.read(|n| n.settings())
    }

    /// Relay and Tor stay off on phones whatever the patch says.
    pub fn set_settings(&self, patch: SettingsPatch) -> R<Settings> {
        self.api(|n| n.set_settings(&patch))
    }
}
