//! The API every shell calls (mirrors `HoiCore` in packages/core-api). Methods don't persist on
//! their own; hosts call [`Node::commit`] after each call (the dispatcher and FFI wrapper do).

use std::collections::{BTreeMap, BTreeSet};

use hoi_crypto::{Keypair, keystore};
use hoi_moderation::Community;
use hoi_types::cbor::{self, Value};
use hoi_types::uri::{b64, unb64};
use hoi_types::{
    Admission, Body, Cosig, Genesis, Id, Invite as InviteUri, Kind, LedgerAction, LedgerBody, Limits, MediaRef, Record, Verdict, Wire, did_key, parse_key,
};
use hoi_validate::KEEP;
use hoi_validate::media as hmedia;
use zeroize::Zeroize;

use crate::dto::*;
use crate::error::ApiError;
use crate::node::{Blobs, Draft, Node, PendingIdentity, Source, keypair_from_hex};
use crate::state::{Identity, Joined};
use crate::views::limits_dto;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DELEGATION_LIFETIME: u64 = 180 * 86_400;
pub(crate) const PROPOSAL_MAX_AGE: u64 = 7 * 86_400;
/// An established member who read at least this many of a newcomer's records…
const AUTO_VOUCH_READS: usize = 2;
/// …and didn't hide any of them within this long, vouches for them (if they let the app).
const AUTO_VOUCH_DELAY: u64 = 600;
/// Randomness from gestures a new key may mix in (the web app sends a 32-byte digest).
pub const MAX_STIR: usize = 64;

pub type R<T> = Result<T, ApiError>;

pub(crate) fn parse_id(s: &str) -> R<Id> {
    Id::from_hex(s).map_err(|_| ApiError::invalid("That doesn't look like an id."))
}

pub(crate) fn parse_person_key(s: &str) -> R<Id> {
    parse_key(s).map_err(|_| ApiError::invalid("Paste a key: 64 hex characters or a did:key."))
}

fn limits_from(base: &Limits, p: &LimitsPatch) -> R<Limits> {
    let mut l = base.clone();
    if let Some(x) = p.pow_bits {
        l.pow_bits = x;
    }
    if let Some(x) = p.max_retention {
        l.max_retention = x;
    }
    if let Some(x) = p.newcomer_posts_per_day {
        l.newcomer_posts_per_day = x;
    }
    if let Some(x) = p.newcomer_age {
        l.newcomer_age = x;
    }
    if let Some(x) = &p.admission {
        l.admission = Admission::parse(x)?;
    }
    if let Some(x) = p.autoban_threshold {
        l.autoban_threshold = x;
    }
    if let Some(x) = &p.labelers {
        l.labelers = x.iter().map(|k| parse_person_key(k)).collect::<R<_>>()?;
    }
    if let Some(d) = &p.ladder {
        let t = &mut l.ladder;
        if let Some(x) = d.identity_bits {
            t.identity_bits = x;
        }
        if let Some(x) = d.admit_vouches {
            t.admit_vouches = x;
        }
        if let Some(x) = d.established_age {
            t.established_age = x;
        }
        if let Some(x) = d.trusted_posts {
            t.trusted_posts = x;
        }
        if let Some(x) = d.media_score {
            t.media_score = x;
        }
        if let Some(x) = d.hide_quorum {
            t.hide_quorum = x;
        }
        if let Some(x) = d.block_quorum {
            t.block_quorum = x;
        }
        if let Some(x) = d.veto_quorum {
            t.veto_quorum = x;
        }
        if let Some(x) = d.contest_window {
            t.contest_window = x;
        }
    }
    if let Some(x) = p.scatter_resist {
        l.scatter.resist = x;
    }
    if let Some(x) = p.scatter_open {
        l.scatter.open = x;
    }
    if let Some(x) = &p.age_issuers {
        let keys: Vec<Vec<u8>> = x
            .iter()
            .map(|k| k.trim())
            .filter(|k| !k.is_empty())
            .map(|k| hoi_types::hex_decode(k).ok().filter(|b| hoi_types::age_issuer_ok(b)))
            .collect::<Option<_>>()
            .ok_or_else(|| ApiError::invalid("An issuer's key is hex: 64 characters (Ed25519) or 66 or 130 (P-256)."))?;
        if keys.len() > hoi_types::MAX_AGE_ISSUERS {
            return Err(ApiError::invalid("A community trusts at most eight issuers."));
        }
        l.age_issuers = keys;
    }
    if let Some(x) = &p.trackers {
        let t: Vec<String> = x.iter().map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()).collect();
        if t.len() > hoi_types::uri::MAX_TRACKERS {
            return Err(ApiError::invalid("A community names at most two trackers."));
        }
        if let Some(bad) = t.iter().find(|s| !hoi_types::uri::tracker_ok(s)) {
            return Err(ApiError::invalid(format!("{bad} isn't a tracker address. Use https://, with no password or query.")));
        }
        l.trackers = t;
    }
    hoi_validate::limits(&l)?;
    Ok(l)
}

pub(crate) fn patch_of(d: &LimitsDto) -> LimitsPatch {
    LimitsPatch {
        pow_bits: Some(d.pow_bits),
        max_retention: Some(d.max_retention),
        newcomer_posts_per_day: Some(d.newcomer_posts_per_day),
        newcomer_age: Some(d.newcomer_age),
        admission: Some(d.admission.clone()),
        autoban_threshold: Some(d.autoban_threshold),
        labelers: Some(d.labelers.clone()),
        ladder: Some(LadderPatch {
            identity_bits: Some(d.ladder.identity_bits),
            admit_vouches: Some(d.ladder.admit_vouches),
            established_age: Some(d.ladder.established_age),
            trusted_posts: Some(d.ladder.trusted_posts),
            media_score: Some(d.ladder.media_score),
            hide_quorum: Some(d.ladder.hide_quorum),
            block_quorum: Some(d.ladder.block_quorum),
            veto_quorum: Some(d.ladder.veto_quorum),
            contest_window: Some(d.ladder.contest_window),
        }),
        scatter_resist: Some(d.scatter_resist),
        scatter_open: Some(d.scatter_open),
        trackers: Some(d.trackers.clone()),
        age_issuers: Some(d.age_issuers.clone()),
    }
}

pub(crate) fn clean_name(name: &str) -> R<String> {
    let n = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() || n.chars().count() > 32 {
        return Err(ApiError::invalid("A name is 1 to 32 characters."));
    }
    hoi_validate::text::name(&n).map_err(|e| ApiError::invalid(format!("That name won't do: {e}.")))?;
    Ok(n)
}

/// A list of community IDs for the person's own ordering of them: each once, at most 1024.
fn community_list(ids: &[String]) -> R<Vec<String>> {
    if ids.len() > 1024 {
        return Err(ApiError::invalid("That list is too long."));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for id in ids {
        let c = parse_id(id)?.hex();
        if seen.insert(c.clone()) {
            out.push(c);
        }
    }
    Ok(out)
}

/// What `publish_draft` hands back, by kind.
pub enum Published {
    Post(Box<PostSummary>),
    Comment(Box<CommentView>),
    Nothing,
    /// Sent to the person's computer to be signed (crate::remote); a `signed` event follows.
    Signing(String),
}

impl Node {
    pub(crate) fn need_me(&self) -> R<Identity> {
        self.state.me.clone().ok_or_else(ApiError::no_identity)
    }

    pub(crate) fn device_key(&self) -> R<Id> {
        match (&self.device_kp, &self.state.me) {
            (Some(k), _) => Ok(k.public()),
            // the computer keeps this browser's key and signs for it (crate::remote)
            (None, Some(me)) if me.remote => Ok(me.device),
            _ => Err(ApiError::no_identity()),
        }
    }

    pub(crate) fn community_of(&self, id: &Id) -> R<&Community> {
        self.communities.get(id).ok_or_else(|| ApiError::not_found("community"))
    }

    // ------------------------------------------------------------------ boot & you

    pub fn boot(&self) -> Boot {
        Boot {
            platform: self.platform.clone(),
            version: VERSION.into(),
            protocol: hoi_types::PROTOCOL_VERSION,
            has_identity: self.state.me.is_some(),
            blocked_by_other_tab: None,
            cleaned: 0,
            preserved_records: self.preserved_records,
        }
    }

    pub fn me(&self) -> Option<Me> {
        let me = self.state.me.as_ref()?;
        let now = self.now();
        // the devices the person's current root blessed
        let current = self.store.current_root(&me.root);
        let signing = self.root_kp.as_ref().map(|k| k.public()).or_else(|| self.store.delegation(&me.device).map(|d| d.info.root));
        let mut devices: Vec<DeviceInfo> = self
            .store
            .delegations_of(&current)
            .into_iter()
            .map(|(k, d)| DeviceInfo {
                key: k.hex(),
                short: k.short_key(),
                label: self.state.device_labels.get(&k).cloned().unwrap_or_else(|| if k == me.device { "This device".into() } else { "Another device".into() }),
                this_device: k == me.device,
                expires_at: d.info.expires_at,
                revoked: d.info.revoked_at.is_some_and(|r| r <= now),
                signed_here: self.remote_label(&k).is_some(),
            })
            .collect();
        devices.sort_by(|a, b| b.this_device.cmp(&a.this_device).then(a.label.cmp(&b.label)));
        Some(Me {
            root: did_key(&me.root),
            root_key: me.root.hex(),
            work_key: signing.unwrap_or(me.root).hex(),
            fingerprint: hoi_crypto::fingerprint(&me.root),
            short: me.root.short_key(),
            name: me.name.clone(),
            avatar: self.avatar_for(&me.root, None),
            avatar_color: self.avatar_color_for(&me.root, None),
            created_at: me.created_at,
            has_root: self.root_kp.is_some(),
            device: me.device.hex(),
            devices,
            identity_bits: self.store.best_identity(&me.root).map_or(0, |i| i.bits),
            burned: me.burned,
            holds_line: self.master.is_some(),
            restoring: me.restoring,
            replaced: !me.restoring && signing.is_some_and(|s| s != current),
            remote: me.remote,
        })
    }

    pub(crate) fn this_device_label(&self) -> &'static str {
        match self.platform.as_str() {
            "web" => "This browser",
            "desktop" => "This computer",
            "mobile" => "This phone",
            _ => "This device",
        }
    }

    pub fn create_identity(&mut self, name: &str) -> R<Me> {
        self.create_identity_stirred(name, &[])
    }

    /// Makes this device's key and its identity work in one go (native hosts mine on every
    /// core). `stir`: randomness from the person's gestures, already hashed, mixed into the
    /// system's (never instead of it).
    pub fn create_identity_stirred(&mut self, name: &str, stir: &[u8]) -> R<Me> {
        let p = self.new_keys(name, stir)?;
        // the work that makes this key an identity: done once, it stays with the key for life
        let work = hoi_crypto::identity::mine(&p.root.public(), p.bits);
        self.adopt_identity(p, work)
    }

    /// Makes this device's key, like [`Node::create_identity_stirred`], but leaves the identity
    /// work to the host (draft → mine → publish, as for stamps), so it can mine in parallel and
    /// show every try. Nothing is kept until [`Node::publish_identity`]; drafting again
    /// replaces the key in the making.
    pub fn draft_identity(&mut self, name: &str, stir: &[u8]) -> R<IdentityDraft> {
        let p = self.new_keys(name, stir)?;
        let pk = p.root.public();
        let d = IdentityDraft { root: did_key(&pk), root_key: pk.hex(), short: pk.short_key(), fingerprint: hoi_crypto::fingerprint(&pk), bits: p.bits };
        self.pending_identity = Some(p);
        Ok(d)
    }

    /// Finishes the key [`Node::draft_identity`] made, once the host found its work.
    pub fn publish_identity(&mut self, root_key: &str, work: u64) -> R<Me> {
        if self.state.me.is_some() {
            return Err(ApiError::not_allowed("This device already has a key."));
        }
        let root = parse_id(root_key)?;
        let p = self.pending_identity.take_if(|p| p.root.public() == root).ok_or_else(|| ApiError::not_found("key in the making"))?;
        if hoi_crypto::identity::work_bits(&root, work) < p.bits {
            self.pending_identity = Some(p);
            return Err(ApiError::invalid("That identity work doesn't hold enough bits."));
        }
        self.adopt_identity(p, work)
    }

    fn new_keys(&self, name: &str, stir: &[u8]) -> R<PendingIdentity> {
        if self.state.me.is_some() {
            return Err(ApiError::not_allowed("This device already has a key."));
        }
        if stir.len() > MAX_STIR {
            return Err(ApiError::invalid(format!("Stir in at most {MAX_STIR} bytes.")));
        }
        let name = clean_name(name)?;
        // the recovery line's secret: the person's first root and every later one derive from it
        let master = zeroize::Zeroizing::new(Keypair::generate_stirred(stir)?.secret());
        let root = hoi_crypto::recovery::root_at(&master, 0);
        let device = Keypair::generate_stirred(stir)?;
        Ok(PendingIdentity { name, master, root, device, bits: self.identity_bits })
    }

    fn adopt_identity(&mut self, p: PendingIdentity, work: u64) -> R<Me> {
        let PendingIdentity { name, master, root, device, .. } = p;
        // a key drafted earlier and never finished goes
        self.pending_identity = None;
        let now = self.now();
        self.state.me = Some(Identity {
            root: root.public(),
            device: device.public(),
            name,
            created_at: now,
            has_root: true,
            work: Some(work),
            burned: false,
            avatar: None,
            avatar_revision: 0,
            avatar_color: None,
            avatar_color_revision: 0,
            generation: 0,
            restoring: false,
            rc: None,
            of: None,
            remote: false,
        });
        self.state.first_seen.insert(root.public(), now);
        self.state.device_labels.insert(device.public(), self.this_device_label().into());
        self.root_kp = Some(root);
        self.device_kp = Some(device);
        self.master = Some(master);
        self.mark_state();
        self.mark_keys();
        self.ensure_identity();
        self.renew_delegation();
        self.me().ok_or_else(ApiError::no_identity)
    }

    /// Mines more identity work for this key (communities may ask for more than it carries)
    /// and publishes the stronger identity record. Needs the root key.
    pub fn strengthen_identity(&mut self, bits: u32) -> R<Me> {
        self.need_me()?;
        let root = self.root_kp.clone().ok_or_else(|| ApiError::not_allowed("Strengthen your key on the device that holds your root key."))?;
        if !(hoi_crypto::identity::MIN_BITS..=hoi_crypto::identity::MAX_BITS).contains(&bits) {
            return Err(ApiError::invalid(format!("Identity work is {} to {} bits.", hoi_crypto::identity::MIN_BITS, hoi_crypto::identity::MAX_BITS)));
        }
        if self.carries_bits(bits) {
            return self.me().ok_or_else(ApiError::no_identity);
        }
        let work = hoi_crypto::identity::mine(&root.public(), bits);
        self.adopt_more_work(root, work)
    }

    /// Like [`Node::strengthen_identity`], with work the host mined (draft → mine → publish, as
    /// for a new key's), so the node isn't held while it mines and the person can watch it and
    /// stop it.
    pub fn strengthen_identity_with(&mut self, bits: u32, work: u64) -> R<Me> {
        self.need_me()?;
        let root = self.root_kp.clone().ok_or_else(|| ApiError::not_allowed("Strengthen your key on the device that holds your root key."))?;
        if !(hoi_crypto::identity::MIN_BITS..=hoi_crypto::identity::MAX_BITS).contains(&bits) {
            return Err(ApiError::invalid(format!("Identity work is {} to {} bits.", hoi_crypto::identity::MIN_BITS, hoi_crypto::identity::MAX_BITS)));
        }
        if hoi_crypto::identity::work_bits(&root.public(), work) < bits {
            return Err(ApiError::invalid("That identity work doesn't hold enough bits."));
        }
        if self.carries_bits(bits) {
            return self.me().ok_or_else(ApiError::no_identity);
        }
        self.adopt_more_work(root, work)
    }

    /// Whether this person already carries `bits` of work: the best identity along their chain
    /// of roots, which is what communities count, and what `Me::identity_bits` shows.
    fn carries_bits(&self, bits: u32) -> bool {
        self.state.me.as_ref().and_then(|me| self.store.best_identity(&me.root)).is_some_and(|i| i.bits >= bits)
    }

    /// What `root`'s identity records commit to: its recovery key and the person it continues.
    /// The record here first (what others hold), then the line for this generation, then what
    /// this device remembers.
    fn commitments_of(&self, root: &Id, work: u64) -> (Option<Id>, Option<Id>) {
        if let Some(i) = self.store.identity(root) {
            return (i.rc, i.of);
        }
        let Some(me) = self.state.me.as_ref() else { return (None, None) };
        match self.master.as_ref() {
            Some(m) => match Self::identity_body(m, me.generation, work) {
                Body::Identity { rc, of, .. } => (rc, of),
                _ => (me.rc, me.of),
            },
            None => (me.rc, me.of),
        }
    }

    /// Publishes the stronger identity record for work that was checked.
    fn adopt_more_work(&mut self, root: Keypair, work: u64) -> R<Me> {
        // A root commits to one recovery key and one person for life: a second identity record
        // that says otherwise burns it. So the stronger record repeats what the root's
        // record here says, or what the line says for this generation.
        let (rc, of) = self.commitments_of(&root.public(), work);
        if let Some(m) = self.state.me.as_mut() {
            m.work = Some(work);
            m.rc = rc;
            m.of = of;
        }
        self.mark_state();
        let now = self.now();
        let r = Record {
            v: 1,
            community: None,
            author: root.public(),
            parent: None,
            created_at: now,
            expires_at: now + KEEP,
            body: Body::Identity { work, rc, of },
            nonce: 0,
        };
        self.publish(r)?;
        self.announce();
        self.me().ok_or_else(ApiError::no_identity)
    }

    /// Burns this identity for good: a signed record every device honours by
    /// dropping everything the key ever signed and refusing anything it signs later. For a key
    /// that leaked. There is no undo, and no way to carry standing over to a new key.
    /// `confirm` must be the key's short form (`a7f2·91c0`).
    pub fn burn_identity(&mut self, confirm: &str) -> R<()> {
        let me = self.need_me()?;
        if confirm.trim() != me.root.short_key() {
            return Err(ApiError::invalid("Type your key's short form to confirm. Burning can't be undone."));
        }
        let root = self.root_kp.clone().ok_or_else(|| ApiError::not_allowed("Burn your key on the device that holds your root key."))?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: None,
            author: root.public(),
            parent: None,
            created_at: now,
            expires_at: now + KEEP,
            body: Body::Revoke { device: root.public() },
            nonce: 0,
        };
        self.publish(r)?;
        self.changed_everywhere();
        Ok(())
    }

    pub fn set_name(&mut self, name: &str) -> R<Me> {
        let name = clean_name(name)?;
        let me = self.state.me.as_mut().ok_or_else(ApiError::no_identity)?;
        me.name = name.clone();
        let root = me.root;
        self.state.names.insert(root, (name, self.now()));
        self.mark_state();
        self.me().ok_or_else(ApiError::no_identity)
    }

    pub fn export_identity(&self, passphrase: &str) -> R<Vec<u8>> {
        let me = self.need_me()?;
        // The file leaves this device, so its passphrase is held to the local vault's standard.
        if passphrase.chars().count() < 15 {
            return Err(ApiError::invalid("Use a backup passphrase of at least 15 characters. Several unrelated words work well."));
        }
        if passphrase.len() > 1024 {
            return Err(ApiError::invalid("That passphrase is too long."));
        }
        let geneses: Vec<String> =
            self.state.joined.keys().filter(|c| !self.is_private_community(c)).filter_map(|c| self.store.genesis_of(c)).map(|s| b64(&s.wire)).collect();
        let delegation = self.store.delegation(&me.device).and_then(|d| self.store.get(&d.record)).map(|s| b64(&s.wire));
        let identity = self.store.identity(&me.root).and_then(|i| self.store.get(&i.record)).map(|s| b64(&s.wire));
        // a person whose root was replaced: each later root's identity and each rotation
        let chain: Vec<String> = self.store.chain_records(&me.root).iter().filter_map(|id| self.store.get(id)).map(|s| b64(&s.wire)).collect();
        let (registrations, publications) = self.backup_proofs();
        // Version 2 adds the name proofs; files of either version import.
        let doc = serde_json::json!({
            "identity": identity,
            "chain": chain,
            "generation": me.generation,
            "rc": me.rc.map(|k| k.hex()),
            "of": me.of.map(|k| k.hex()),
            "work": me.work,
            "v": 2,
            "name": me.name,
            "avatar": me.avatar.as_ref().map(|image| b64(image)),
            "avatarRevision": me.avatar_revision,
            "avatarColor": me.avatar_color,
            "avatarColorRevision": me.avatar_color_revision,
            "createdAt": me.created_at,
            "rootKey": me.root.hex(),
            "root": self.root_kp.as_ref().map(|k| hoi_types::hex_encode(&k.secret())),
            "device": self.device_kp.as_ref().map(|k| hoi_types::hex_encode(&k.secret())),
            "delegation": delegation,
            "communities": self.community_kps.iter().map(|(c, k)| (c.hex(), hoi_types::hex_encode(&k.secret()))).collect::<BTreeMap<_, _>>(),
            "joined": geneses,
            "friends": self.state.friends.keys().map(|k| k.hex()).collect::<Vec<_>>(),
            "petnames": self.state.petnames.iter().map(|(k, v)| (k.hex(), v.clone())).collect::<BTreeMap<_, _>>(),
            "registrations": registrations,
            "publications": publications,
        });
        let mut plain = serde_json::to_vec(&doc).map_err(|e| ApiError::new("internal", e.to_string()))?;
        let sealed = keystore::seal(&plain, passphrase, keystore::Cost::default());
        plain.zeroize();
        Ok(sealed?)
    }

    pub fn import_identity(&mut self, data: &[u8], passphrase: &str) -> R<Me> {
        if self.state.me.is_some() {
            return Err(ApiError::not_allowed("This device already has a key. Wipe it first to bring another one."));
        }
        let mut plain = keystore::open(data, passphrase)?;
        let parsed = serde_json::from_slice::<serde_json::Value>(&plain);
        plain.zeroize();
        let doc = parsed.map_err(|_| ApiError::invalid("That file isn't a hoi key."))?;
        let s = |k: &str| doc.get(k).and_then(|v| v.as_str()).map(String::from);
        let name = s("name").unwrap_or_else(|| "me".into());
        let root_pub = parse_id(&s("rootKey").ok_or_else(|| ApiError::invalid("That file isn't a hoi key."))?)?;
        let avatar_revision = doc.get("avatarRevision").and_then(|value| value.as_u64()).unwrap_or(0);
        let avatar_color_revision = doc.get("avatarColorRevision").and_then(|value| value.as_u64()).unwrap_or(0);
        let avatar_color = s("avatarColor");
        if avatar_color.as_deref().is_some_and(|color| !hoi_types::is_avatar_color(color)) {
            return Err(ApiError::invalid("Invalid identicon color in key backup."));
        }
        let avatar = s("avatar").map(|encoded| unb64(&encoded).map_err(|_| ApiError::invalid("Invalid profile picture in key backup."))).transpose()?;
        if let Some(image) = &avatar {
            hoi_validate::avatar(image)?;
        }
        let now = self.now();
        let created_at = doc.get("createdAt").and_then(|v| v.as_u64()).unwrap_or(now);
        // the person's identity and, after a rotation, each later root's and each rotation
        let chain: Vec<Vec<u8>> = s("identity")
            .into_iter()
            .chain(doc.get("chain").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)))
            .filter_map(|x| unb64(&x).ok())
            .collect();
        if let Some(root) = s("root").as_deref().and_then(keypair_from_hex) {
            if root.public() != root_pub {
                for w in &chain {
                    let _ = self.ingest(w, &Source::Import);
                }
                if self.store.current_root(&root_pub) != root.public() {
                    return Err(ApiError::invalid("The key file is inconsistent."));
                }
            }
            // Community keys are normally forgotten at creation (below); a file from a build that
            // still held one is honoured only once the root checked out.
            for (c, h) in doc.get("communities").and_then(|v| v.as_object()).into_iter().flatten() {
                if let (Ok(c), Some(kp)) = (parse_id(c), h.as_str().and_then(keypair_from_hex)) {
                    self.community_kps.insert(c, kp);
                }
            }
            // A fresh device key: two devices never share one.
            let device = Keypair::generate()?;
            let work = doc.get("work").and_then(|v| v.as_u64());
            self.state.me = Some(Identity {
                root: root_pub,
                device: device.public(),
                name,
                created_at,
                has_root: true,
                work,
                burned: false,
                avatar: avatar.clone(),
                avatar_revision,
                avatar_color: avatar_color.clone(),
                avatar_color_revision,
                generation: doc.get("generation").and_then(|v| v.as_u64()).map_or(0, |g| g as u32),
                restoring: false,
                rc: s("rc").and_then(|k| Id::from_hex(&k).ok()),
                of: s("of").and_then(|k| Id::from_hex(&k).ok()),
                remote: false,
            });
            self.state.device_labels.insert(device.public(), self.this_device_label().into());
            self.root_kp = Some(root);
            self.device_kp = Some(device);
            for w in &chain {
                let _ = self.ingest(w, &Source::Import);
            }
            self.ensure_identity();
            self.renew_delegation();
        } else {
            let device = s("device").as_deref().and_then(keypair_from_hex).ok_or_else(|| ApiError::invalid("The key file holds no usable key."))?;
            let wire = unb64(&s("delegation").ok_or_else(|| ApiError::invalid("The key file has no device blessing."))?)?;
            if s("identity").and_then(|x| unb64(&x).ok()).is_none() {
                return Err(ApiError::invalid("The key file has no identity record."));
            }
            self.state.me = Some(Identity {
                root: root_pub,
                device: device.public(),
                name,
                created_at,
                has_root: false,
                work: None,
                burned: false,
                avatar,
                avatar_revision,
                avatar_color,
                avatar_color_revision,
                generation: 0,
                restoring: false,
                rc: None,
                of: None,
                remote: false,
            });
            self.device_kp = Some(device);
            for w in chain.iter().cloned().chain(std::iter::once(wire)) {
                match self.ingest(&w, &Source::Import) {
                    Ok(_) | Err(hoi_validate::Reject::Duplicate) => {}
                    Err(e) => {
                        self.state.me = None;
                        self.device_kp = None;
                        return Err(e.into());
                    }
                }
            }
        }
        self.state.first_seen.entry(root_pub).or_insert(created_at.min(now));
        for f in doc.get("friends").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(k) = f.as_str().and_then(|k| Id::from_hex(k).ok()) {
                self.state.friends.insert(k, now);
            }
        }
        for (k, v) in doc.get("petnames").and_then(|v| v.as_object()).into_iter().flatten() {
            if let (Ok(k), Some(v)) = (Id::from_hex(k), v.as_str()) {
                self.state.petnames.insert(k, v.to_string());
            }
        }
        for g in doc.get("joined").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(w) = g.as_str().and_then(|x| unb64(x).ok()) {
                let _ = self.join_genesis(&w);
            }
        }
        self.restore_proofs(doc.get("registrations"), doc.get("publications"));
        self.mark_state();
        self.mark_keys();
        self.me().ok_or_else(ApiError::no_identity)
    }

    pub fn link_request(&mut self, label: &str) -> R<String> {
        self.link_request_as(label, false)
    }

    /// `remote`: this device will keep no signing key; the approving device makes and keeps it
    /// (crate::remote). The key made here then only ties the request to this device.
    pub(crate) fn link_request_as(&mut self, label: &str, remote: bool) -> R<String> {
        if self.state.me.is_some() {
            return Err(ApiError::not_allowed("This device already has a key."));
        }
        let kp = Keypair::generate()?;
        let mut fields = vec![("t", Value::T("link-req".into())), ("device", Value::B(kp.public().0.to_vec())), ("label", Value::T(clean_name(label)?))];
        if remote {
            fields.push(("remote", Value::U(1)));
        }
        let v = Value::map(fields);
        self.pending_kp = Some(kp);
        self.mark_keys();
        Ok(format!("hoi:link:{}", b64(&v.encode())))
    }

    pub(crate) fn decode_code(code: &str, prefix: &str) -> R<std::collections::BTreeMap<String, Value>> {
        let body = code.trim().strip_prefix(prefix).ok_or_else(|| ApiError::invalid("That code isn't the right kind."))?;
        match cbor::decode(&unb64(body)?)? {
            Value::M(e) => Ok(e.into_iter().collect()),
            _ => Err(ApiError::invalid("That code is damaged.")),
        }
    }

    /// On the root device: bless the device that showed `request`. Returns the approval code
    /// for it. Nothing is published yet: the new device signs its own delegation (its consent)
    /// with the blessing inside, so nobody can claim a device that didn't agree.
    pub fn link_approve(&mut self, request: &str) -> R<String> {
        // Codes travel by hand or QR here: few geneses keep them short; the rest syncs.
        self.link_approve_with(request, 8, 64 * 1024)
    }

    /// `geneses`, `max_bytes`: how many of your communities' geneses the approval carries, and
    /// how many bytes of them at most.
    pub(crate) fn link_approve_with(&mut self, request: &str, geneses: usize, max_bytes: usize) -> R<String> {
        let me = self.need_me()?;
        let root = self.root_kp.clone().ok_or_else(|| ApiError::not_allowed("Approve on the device that holds your root key."))?;
        let m = Self::decode_code(request, "hoi:link:")?;
        if m.get("t") != Some(&Value::T("link-req".into())) {
            return Err(ApiError::invalid("That's not a link request."));
        }
        let device = match m.get("device") {
            Some(Value::B(b)) => Id::from_slice(b)?,
            _ => return Err(ApiError::invalid("That code is damaged.")),
        };
        if device == root.public() || self.store.identity(&device).is_some() {
            return Err(ApiError::invalid("That code is damaged."));
        }
        let label = match m.get("label") {
            Some(Value::T(t)) => clean_name(t)?,
            _ => "Another device".into(),
        };
        let now = self.now();
        let draft = Node::delegation_record(&root, device, now);
        let Body::Delegation { scope, expires_at, root_sig, .. } = &draft.body else { unreachable!("built above") };
        // a browser that keeps no key (crate::remote): this device makes it, blesses it, signs its
        // consent and keeps it; the approval carries the finished delegation
        let remote = if matches!(m.get("remote"), Some(Value::U(1))) {
            let rd = Keypair::generate()?;
            let mut r = Node::delegation_record(&root, rd.public(), now);
            r.nonce = crate::node::mine(&r, self.bits_to_stamp(&r));
            let wire = rd.sign_record(r).0.encode();
            self.ingest(&wire, &Source::Local)?;
            self.state.device_labels.insert(rd.public(), label.clone());
            self.remote.keys.insert(rd.public(), crate::remote::RemoteKey { kp: rd, label: label.clone() });
            self.mark_keys();
            Some(wire)
        } else {
            None
        };
        self.state.device_labels.insert(device, label);
        self.mark_state();
        let identity = self
            .store
            .identity(&me.root)
            .and_then(|i| self.store.get(&i.record))
            .map(|s| s.wire.clone())
            .ok_or_else(|| ApiError::new("internal", "this identity has no identity record yet"))?;
        let mut room = max_bytes;
        let geneses: Vec<Value> = self
            .state
            .joined
            .keys()
            .filter(|c| !self.is_private_community(c))
            .filter_map(|c| self.store.genesis_of(c))
            .filter(|s| {
                let fits = s.wire.len() <= room;
                if fits {
                    room -= s.wire.len();
                }
                fits
            })
            .take(geneses)
            .map(|s| Value::B(s.wire.clone()))
            .collect();
        // a person whose root was replaced: each later root's identity and each rotation
        let chain: Vec<Value> = self.store.chain_records(&me.root).iter().filter_map(|id| self.store.get(id)).map(|s| Value::B(s.wire.clone())).collect();
        let v = Value::map([
            ("t", Value::T("link-ok".into())),
            ("root", Value::B(root.public().0.to_vec())),
            ("person", Value::B(me.root.0.to_vec())),
            ("chain", Value::A(chain)),
            ("remote", remote.map_or(Value::Null, Value::B)),
            ("device", Value::B(device.0.to_vec())),
            ("scope", Value::A(scope.iter().map(|s| Value::T(s.clone())).collect())),
            ("created", Value::U(now)),
            ("expires", Value::U(*expires_at)),
            ("root_sig", Value::B(root_sig.0.to_vec())),
            ("identity", Value::B(identity)),
            ("name", Value::T(me.name)),
            ("genesis", Value::A(geneses)),
        ]);
        Ok(format!("hoi:link:{}", b64(&v.encode())))
    }

    /// On the new device: check the blessing, sign the delegation as this device, publish.
    pub fn link_complete(&mut self, approval: &str) -> R<Me> {
        let kp = self.pending_kp.clone().ok_or_else(|| ApiError::not_allowed("Start with a link request on this device."))?;
        let m = Self::decode_code(approval, "hoi:link:")?;
        if m.get("t") != Some(&Value::T("link-ok".into())) {
            return Err(ApiError::invalid("That's not an approval code."));
        }
        let bytes = |k: &str| match m.get(k) {
            Some(Value::B(b)) => Ok(b.clone()),
            _ => Err(ApiError::invalid("That approval code is damaged.")),
        };
        let uint = |k: &str| match m.get(k) {
            Some(Value::U(n)) => Ok(*n),
            _ => Err(ApiError::invalid("That approval code is damaged.")),
        };
        let root = Id::from_slice(&bytes("root")?)?;
        if Id::from_slice(&bytes("device")?)? != kp.public() {
            return Err(ApiError::invalid("That approval is for another device."));
        }
        let scope: Vec<String> = match m.get("scope") {
            Some(Value::A(items)) => items.iter().filter_map(|v| if let Value::T(t) = v { Some(t.clone()) } else { None }).collect(),
            _ => return Err(ApiError::invalid("That approval code is damaged.")),
        };
        let (created, expires) = (uint("created")?, uint("expires")?);
        let root_sig = hoi_types::Sig::from_slice(&bytes("root_sig")?)?;
        let name = match m.get("name") {
            Some(Value::T(t)) => clean_name(t).unwrap_or_else(|_| "me".into()),
            _ => "me".into(),
        };
        let identity = bytes("identity")?;
        let person = Id::from_slice(&bytes("person")?)?;
        let (iw, _) = Wire::decode(&identity)?;
        if iw.record.author != person || iw.record.kind() != Kind::Identity {
            return Err(ApiError::invalid("That approval code is damaged."));
        }
        let chain: Vec<Vec<u8>> = match m.get("chain") {
            Some(Value::A(items)) => items.iter().filter_map(|v| if let Value::B(b) = v { Some(b.clone()) } else { None }).collect(),
            _ => Vec::new(),
        };
        // A browser that keeps no key (crate::remote): the approving device made its key and
        // sends the finished delegation; this device holds only its public half.
        let remote = match m.get("remote") {
            Some(Value::B(w)) => {
                let (dw, _) = Wire::decode(w)?;
                if !matches!(&dw.record.body, Body::Delegation { root: r, .. } if *r == root) {
                    return Err(ApiError::invalid("That approval code is damaged."));
                }
                Some((dw.record.author, w.clone()))
            }
            _ => None,
        };
        let device = remote.as_ref().map_or(kp.public(), |(d, _)| *d);
        self.state.me = Some(Identity {
            root: person,
            device,
            name,
            created_at: created,
            has_root: false,
            work: None,
            burned: false,
            avatar: None,
            avatar_revision: 0,
            avatar_color: None,
            avatar_color_revision: 0,
            generation: 0,
            restoring: false,
            rc: None,
            of: None,
            remote: remote.is_some(),
        });
        self.state.device_labels.insert(device, self.this_device_label().into());
        let rollback = |n: &mut Node| {
            n.state.me = None;
        };
        for w in std::iter::once(&identity).chain(chain.iter()) {
            if let Err(e) = self.ingest(w, &Source::Import)
                && e != hoi_validate::Reject::Duplicate
            {
                rollback(self);
                return Err(e.into());
            }
        }
        if self.store.current_root(&person) != root {
            rollback(self);
            return Err(ApiError::invalid("That approval comes from a root that no longer speaks for this person."));
        }
        if let Some((_, wire)) = &remote {
            if let Err(e) = self.ingest(wire, &Source::Import)
                && e != hoi_validate::Reject::Duplicate
            {
                rollback(self);
                return Err(e.into());
            }
        } else {
            let r = Record {
                v: 1,
                community: None,
                author: kp.public(),
                parent: None,
                created_at: created,
                expires_at: expires,
                body: Body::Delegation { root, scope, expires_at: expires, root_sig, seal: crate::private::seal_public(&kp) },
                nonce: 0,
            };
            if let Err(e) = self.publish(r) {
                rollback(self);
                return Err(e);
            }
            self.device_kp = Some(kp);
        }
        self.pending_kp = None;
        self.state.first_seen.entry(root).or_insert(created);
        if let Some(Value::A(gs)) = m.get("genesis") {
            for g in gs {
                if let Value::B(b) = g {
                    let _ = self.join_genesis(b);
                }
            }
        }
        self.mark_state();
        self.mark_keys();
        self.announce();
        self.me().ok_or_else(ApiError::no_identity)
    }

    /// Names one of your devices, on this device only: a label to tell them apart by. Never
    /// sent anywhere.
    pub fn set_device_label(&mut self, key: &str, label: &str) -> R<Me> {
        let me = self.need_me()?;
        let device = parse_id(key)?;
        if device != me.device && !self.store.delegations_of(&me.root).iter().any(|(k, _)| *k == device) {
            return Err(ApiError::not_found("device"));
        }
        let label = clean_name(label)?;
        self.state.device_labels.insert(device, label);
        self.mark_state();
        self.me().ok_or_else(ApiError::no_identity)
    }

    pub fn revoke_device(&mut self, key: &str) -> R<Me> {
        let me = self.need_me()?;
        let device = parse_id(key)?;
        if device == me.device {
            return Err(ApiError::not_allowed("You can't revoke the device you're using."));
        }
        let root = self.root_kp.clone().ok_or_else(|| ApiError::not_allowed("Revoke from the device that holds your root key."))?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: None,
            author: root.public(),
            parent: None,
            created_at: now,
            expires_at: now + KEEP,
            body: Body::Revoke { device },
            nonce: 0,
        };
        self.publish(r)?;
        // a browser this computer signed for: its key goes too (crate::remote)
        if self.remote.keys.remove(&device).is_some() {
            self.mark_keys();
        }
        self.me().ok_or_else(ApiError::no_identity)
    }

    // ------------------------------------------------------------------ communities

    pub fn communities(&self) -> Vec<CommunitySummary> {
        let mut v: Vec<CommunitySummary> = self.state.joined.keys().filter_map(|c| self.communities.get(c)).map(|c| self.community_summary(c)).collect();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }

    pub fn community(&self, id: &str) -> R<CommunityView> {
        let c = self.community_of(&parse_id(id)?)?;
        Ok(self.community_view(c))
    }

    pub fn create_community(&mut self, d: CommunityDraft) -> R<CommunityView> {
        self.create_community_with_privacy(d, false)
    }

    pub(crate) fn create_community_with_privacy(&mut self, d: CommunityDraft, private: bool) -> R<CommunityView> {
        let me = self.need_me()?;
        // A new open community keeps its posts in pieces unless its draft says otherwise
        // (SCATTER.md S5); a private one's posts go in pieces through its group anyway.
        let mut patch = d.limits.clone().unwrap_or_default();
        if !private && patch.scatter_open.is_none() {
            patch.scatter_open = Some(true);
        }
        let limits = limits_from(&Limits::default(), &patch)?;
        let ck = Keypair::generate()?;
        let c = ck.public();
        if private {
            self.state.private_communities.insert(c);
        }
        let now = self.now();
        let mut owners = vec![me.root];
        for k in d.co_owners.iter().flatten().filter(|k| !k.trim().is_empty()) {
            let k = parse_person_key(k)?;
            if !owners.contains(&k) {
                owners.push(k);
            }
        }
        let threshold = d.threshold.unwrap_or(1);
        if threshold == 0 || threshold as usize > owners.len() {
            return Err(ApiError::invalid(format!("With {} owner(s), the threshold is 1 to {}.", owners.len(), owners.len())));
        }
        let genesis = Genesis {
            name: d.name.trim().to_string(),
            description: d.description.trim().to_string(),
            color: d.color.clone(),
            owners,
            threshold,
            rules_text_cid: None,
            limits,
        };
        let r = Record { v: 1, community: Some(c), author: c, parent: None, created_at: now, expires_at: now + KEEP, body: Body::Community(genesis), nonce: 0 };
        hoi_validate::content(&r)?;
        let registration = crate::community_registration_name(&d.name)
            .map(|name| hoi_types::hex_encode(&ck.sign(crate::community_registration_statement(&name, &c).as_bytes()).0));
        let publication = crate::community_registration_name(&d.name).map(|name| {
            let (protocol_version, index_version) = crate::registration::publication_version();
            let signature = hoi_types::hex_encode(&ck.sign(crate::community_publication_statement(&name, &c, &protocol_version, index_version).as_bytes()).0);
            CommunityPublication { name, community: c.hex(), protocol_version, index_version, signature }
        });
        self.community_kps.insert(c, ck);
        self.state.joined.insert(c, Joined { at: now, pinning: true });
        self.mark_state();
        let published = self.publish(r);
        // The community key signs its genesis and, for registry-compatible names, one fixed
        // name-claim statement. The owners govern through the ledger. Forgetting the key means
        // nobody, not even a thief of this device, can sign a rival genesis or a different claim.
        self.community_kps.remove(&c);
        if let Err(e) = published {
            self.state.joined.remove(&c);
            return Err(e);
        }
        if let Some(proof) = registration {
            self.state.registration_proofs.insert(c, proof);
            self.mark_state();
        }
        if let Some(proof) = publication {
            self.state.publication_proofs.insert(c, proof);
            self.mark_state();
        }
        if let Some(rules) = d.rules.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            // With a threshold above 1 the first rules need cosigning too; the draft is dropped
            // and the owners set them together from the Manage tab.
            if threshold <= 1 {
                self.owner_action(&c, LedgerAction::Rules, None, Some(rules.to_string()), None)?;
            }
        }
        self.announce();
        self.community(&c.hex())
    }

    /// Checks a genesis record handed over outside of sync (invite, link code, key file).
    fn check_genesis(&self, wire: &[u8], want: Option<&Id>) -> R<(Wire, Id)> {
        let d = hoi_validate::decode(wire).map_err(|_| ApiError::new("bad-invite", "The invite's community record is damaged."))?;
        hoi_validate::signature(&d).map_err(|_| ApiError::new("bad-invite", "The invite's community record isn't signed by its key."))?;
        let r = &d.wire.record;
        if r.kind() != Kind::Community || want.is_some_and(|c| r.community.as_ref() != Some(c)) {
            return Err(ApiError::new("bad-invite", "The invite's details don't match its community key."));
        }
        hoi_validate::content(r)?;
        Ok((d.wire, d.id))
    }

    pub(crate) fn join_genesis(&mut self, wire: &[u8]) -> R<Id> {
        let (w, _) = self.check_genesis(wire, None)?;
        let c = w.record.author;
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Use an encrypted invitation for this community."));
        }
        if self.suppressed_community(&c) {
            return Err(ApiError::not_allowed("This node's operator suppressed that community; it is not held here."));
        }
        let fresh = !self.state.joined.contains_key(&c);
        if fresh {
            self.state.joined.insert(c, Joined { at: self.now(), pinning: false });
            self.mark_state();
        }
        match self.ingest(wire, &Source::Import) {
            Ok(_) | Err(hoi_validate::Reject::Duplicate) => Ok(c),
            Err(e) => {
                if fresh {
                    self.state.joined.remove(&c);
                }
                Err(e.into())
            }
        }
    }

    pub fn preview_invite(&self, link: &str) -> R<InvitePreview> {
        let inv = InviteUri::parse(link).map_err(|_| ApiError::new("bad-invite", "That isn't a hoi invite link."))?;
        if self.is_private_community(&inv.community) {
            return Err(ApiError::not_allowed("Use an encrypted invitation for this community."));
        }
        let (genesis_rec, verified) = if let Some(g) = &inv.genesis {
            let (w, _) = self.check_genesis(g, Some(&inv.community))?;
            (w.record, true)
        } else if let Some(s) = self.store.genesis_of(&inv.community) {
            (s.record.clone(), true)
        } else if let Some(g) = self.directory_genesis(&inv.community) {
            // A listing from the directory: checked when it arrived, and again here.
            let (w, _) = self.check_genesis(&g, Some(&inv.community))?;
            (w.record, true)
        } else {
            return Err(ApiError::new("bad-invite", "This invite doesn't carry the community's details, and no peer has sent them yet. Ask for a fresh link."));
        };
        let Body::Community(g) = &genesis_rec.body else { unreachable!("checked") };
        let st = self.communities.get(&inv.community);
        Ok(InvitePreview {
            community: InviteCommunity {
                id: inv.community.hex(),
                name: g.name.clone(),
                color: g.color.clone(),
                description: g.description.clone(),
                limits: limits_dto(st.map_or(&g.limits, |s| &s.limits)),
                owners: g.owners.iter().map(|k| self.person(k, st)).collect(),
            },
            relays: inv.relays.clone(),
            peers: inv.peers.clone(),
            trackers: inv.trackers.clone(),
            already_joined: self.state.joined.contains_key(&inv.community),
            verified,
        })
    }

    pub fn join(&mut self, link: &str) -> R<CommunityView> {
        // A person joins with their key. A native node (`hoi-node --join`) may hold and serve
        // a community without one: it reads and relays, signs nothing.
        if !self.can_relay() {
            self.need_me()?;
        }
        let inv = InviteUri::parse(link).map_err(|_| ApiError::new("bad-invite", "That isn't a hoi invite link."))?;
        self.preview_invite(link)?;
        let known = self.store.genesis_of(&inv.community).map(|s| s.wire.clone()).or_else(|| self.directory_genesis(&inv.community));
        match (&inv.genesis, known) {
            (Some(g), _) => {
                self.join_genesis(g)?;
            }
            (None, Some(g)) => {
                self.join_genesis(&g)?;
            }
            (None, None) => return Err(ApiError::new("bad-invite", "Ask for a fresh invite link.")),
        }
        self.remember_hints(inv.community, &inv.peers, &inv.relays);
        self.remember_trackers(inv.community, &inv.trackers);
        self.announce();
        self.community(&inv.community.hex())
    }

    pub fn leave(&mut self, id: &str) -> R<()> {
        let c = parse_id(id)?;
        if self.is_private_community(&c) {
            self.private_forget(c)?;
        }
        let pending = self.state.public_joins.remove(&c).is_some();
        if self.state.joined.remove(&c).is_none() && !pending {
            return Err(ApiError::not_found("community you're in"));
        }
        self.state.bootstrap.remove(&c);
        self.trial_routes.remove(&c);
        self.state.sync_completed.remove(&c);
        self.forget_carried(&c);
        self.clear_public_join_sync(c);
        self.mark_state();
        self.mark_changed(c);
        self.announce();
        Ok(())
    }

    pub fn set_pinning(&mut self, id: &str, on: bool) -> R<CommunityView> {
        let c = parse_id(id)?;
        let j = self.state.joined.get_mut(&c).ok_or_else(|| ApiError::not_found("community you're in"))?;
        j.pinning = on;
        self.mark_state();
        self.community(id)
    }

    pub fn invite(&self, id: &str) -> R<Invite> {
        let c = parse_id(id)?;
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Ask the recipient for an encrypted join request."));
        }
        let g = self.store.genesis_of(&c).ok_or_else(|| ApiError::not_found("community"))?;
        let (peers, relays) = self.invite_hints(c);
        let addresses = self.invite_addresses(&peers, Some(c));
        let outside = addresses.iter().any(|a| a.scope == "internet");
        let mut trackers = self.trackers_of(&c);
        trackers.truncate(hoi_types::uri::MAX_TRACKERS);
        let inv = InviteUri { community: c, relays, peers, invite: None, genesis: Some(g.wire.clone()), trackers };
        Ok(Invite { uri: inv.to_uri(), web: inv.to_web(&self.web_base), addresses, outside })
    }

    pub fn export_bundle(&self, id: &str) -> R<Vec<u8>> {
        let c = parse_id(id)?;
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Private communities synchronize through their encrypted relay."));
        }
        self.community_of(&c)?;
        let mut wires: Vec<Value> = Vec::new();
        let mut sent = BTreeSet::new();
        for s in self.store.in_community(&c) {
            if !self.forwardable(s) {
                continue;
            }
            for x in self.carriers(s) {
                if sent.insert(x.id) {
                    wires.push(Value::B(x.wire.clone()));
                }
            }
            if sent.insert(s.id) {
                wires.push(Value::B(s.wire.clone()));
            }
        }
        let v = Value::map([("t", Value::T("hoi-bundle".into())), ("v", Value::U(1)), ("c", Value::B(c.0.to_vec())), ("recs", Value::A(wires))]);
        Ok(v.encode())
    }

    pub fn import_bundle(&mut self, data: &[u8]) -> R<ImportReport> {
        let m = match cbor::decode(data).map_err(|_| ApiError::invalid("That file isn't a hoi bundle."))? {
            Value::M(e) => e.into_iter().collect::<BTreeMap<String, Value>>(),
            _ => return Err(ApiError::invalid("That file isn't a hoi bundle.")),
        };
        if m.get("t") != Some(&Value::T("hoi-bundle".into())) {
            return Err(ApiError::invalid("That file isn't a hoi bundle."));
        }
        let Some(Value::A(items)) = m.get("recs") else { return Err(ApiError::invalid("The bundle is empty.")) };
        let mut geneses = Vec::new();
        let mut rest = Vec::new();
        for v in items {
            if let Value::B(b) = v
                && let Ok((w, _)) = Wire::decode(b)
            {
                if w.record.kind() == Kind::Community { geneses.push(b.clone()) } else { rest.push(b.clone()) }
            }
        }
        let mut rep = ImportReport { rejected: (items.len() - geneses.len() - rest.len()) as u64, ..Default::default() };
        if rep.rejected > 0 {
            rep.reasons.insert("decode".into(), rep.rejected);
        }
        fn tally(rep: &mut ImportReport, res: Result<(), String>) {
            match res {
                Ok(()) => rep.accepted += 1,
                Err(code) if code == "duplicate" => rep.duplicates += 1,
                Err(code) => {
                    rep.rejected += 1;
                    *rep.reasons.entry(code).or_default() += 1;
                }
            }
        }
        for g in geneses {
            let res = self.join_genesis(&g).map(|_| ()).map_err(|e| e.code);
            tally(&mut rep, res);
        }
        self.ingest_batch(rest, &|_| Source::Import, &mut |_, _, res| tally(&mut rep, res.map(|_| ()).map_err(|e| e.code().to_string())));
        self.announce();
        Ok(rep)
    }

    // ------------------------------------------------------------------ the ledger

    pub(crate) fn ledger_record(
        &self,
        c: &Id,
        action: LedgerAction,
        target: Option<Id>,
        reason: Option<String>,
        text: Option<String>,
        limits: Option<Limits>,
    ) -> R<Record> {
        let st = self.community_of(c)?;
        let now = self.now();
        let expires_at = if action.kept_until_superseded() { now + KEEP } else { now + st.limits.max_retention };
        Ok(Record {
            v: 1,
            community: Some(*c),
            author: self.device_key()?,
            parent: None,
            created_at: now,
            expires_at,
            body: Body::Ledger(LedgerBody {
                seq: st.max_seq + 1,
                action,
                target,
                reason,
                evidence: None,
                cosign: None,
                text,
                limits,
                drop_key: None,
                patch: None,
            }),
            nonce: 0,
        })
    }

    fn published_entry(&self, c: &Id, id: &Id) -> R<LedgerEntryView> {
        let st = self.community_of(c)?;
        if let Some((_, why)) = st.refused.iter().find(|(e, _)| e == id) {
            return Err(ApiError::not_allowed(format!("Signed, but it didn't take effect: {why}.")));
        }
        self.ledger_view(id, st).ok_or_else(|| ApiError::new("internal", "entry vanished"))
    }

    fn cosigner(&self) -> R<&Keypair> {
        self.root_kp.as_ref().or(self.device_kp.as_ref()).ok_or_else(ApiError::no_identity)
    }

    fn owner_action(&mut self, c: &Id, action: LedgerAction, target: Option<Id>, text: Option<String>, limits: Option<Limits>) -> R<LedgerResult> {
        let me = self.need_me()?;
        let st = self.community_of(c)?;
        if !st.is_owner(&me.root) {
            return Err(ApiError::not_allowed("Only the community's owners can change this."));
        }
        let threshold = st.genesis.threshold;
        // a change to the rules carries itself as a Loro update to the rules document
        let patch = match (action, &text) {
            (LedgerAction::Rules, Some(t)) => Some(self.rules_patch(c, t)?),
            _ => None,
        };
        let mut r = self.ledger_record(c, action, target, None, text, limits)?;
        if let Body::Ledger(l) = &mut r.body {
            l.patch = patch;
        }
        if threshold <= 1 {
            let id = self.publish(r)?;
            return Ok(LedgerResult::Published { entry: self.published_entry(c, &id)? });
        }
        let pid = r.proposal_id();
        let k = self.cosigner()?;
        if let Body::Ledger(l) = &mut r.body {
            l.cosign = Some(vec![Cosig { key: k.public(), sig: k.sign(pid.as_bytes()) }]);
        }
        Ok(LedgerResult::NeedsCosign { proposal: format!("hoi:cosign:{}", b64(&r.encode())), have: 1, need: threshold })
    }

    pub fn cosign(&mut self, proposal: &str) -> R<LedgerResult> {
        let me = self.need_me()?;
        let body = proposal.trim().strip_prefix("hoi:cosign:").ok_or_else(|| ApiError::invalid("That isn't a proposal code."))?;
        let mut r = Record::from_value(cbor::decode(&unb64(body)?)?)?;
        let c = r.community.ok_or_else(|| ApiError::invalid("That proposal is damaged."))?;
        let now = self.now();
        if r.created_at + PROPOSAL_MAX_AGE < now {
            return Err(ApiError::new("expired", "That proposal is more than a week old. Start a new one."));
        }
        let pid = r.proposal_id();
        let st = self.community_of(&c)?;
        if !st.is_owner(&me.root) {
            return Err(ApiError::not_allowed("Only owners cosign."));
        }
        let threshold = st.genesis.threshold;
        let Body::Ledger(l) = &mut r.body else { return Err(ApiError::invalid("That proposal is damaged.")) };
        if !l.action.needs_threshold() {
            return Err(ApiError::invalid("That action doesn't need cosigning."));
        }
        let mut sigs: Vec<Cosig> = l.cosign.take().unwrap_or_default().into_iter().filter(|s| hoi_crypto::verify(&s.key, pid.as_bytes(), &s.sig)).collect();
        let k = self.root_kp.as_ref().or(self.device_kp.as_ref()).ok_or_else(ApiError::no_identity)?;
        let root_of = |key: &Id| self.store.root_of(key);
        if !sigs.iter().any(|s| root_of(&s.key) == me.root) {
            sigs.push(Cosig { key: k.public(), sig: k.sign(pid.as_bytes()) });
        }
        let mut owners: Vec<Id> = sigs.iter().map(|s| root_of(&s.key)).filter(|r| st.is_owner(r)).collect();
        owners.sort();
        owners.dedup();
        let have = owners.len() as u32;
        l.cosign = Some(sigs);
        if have < threshold {
            r.nonce = 0;
            return Ok(LedgerResult::NeedsCosign { proposal: format!("hoi:cosign:{}", b64(&r.encode())), have, need: threshold });
        }
        r.author = self.device_key()?;
        r.nonce = 0;
        let id = self.publish(r)?;
        Ok(LedgerResult::Published { entry: self.published_entry(&c, &id)? })
    }

    pub fn set_limits(&mut self, id: &str, limits: &LimitsDto) -> R<LedgerResult> {
        let c = parse_id(id)?;
        let current = self.community_of(&c)?.limits.clone();
        let l = limits_from(&current, &patch_of(limits))?;
        let mine = self.need_me().ok().and_then(|me| self.store.best_identity(&me.root)).map_or(0, |i| i.bits);
        if l.ladder.identity_bits > mine {
            return Err(ApiError::not_allowed(format!(
                "That would ask every author for {} bits of identity work, and your own key carries {mine}. Strengthen your key first, or you lock yourself out.",
                l.ladder.identity_bits
            )));
        }
        self.owner_action(&c, LedgerAction::Limits, None, None, Some(l))
    }

    pub fn set_rules(&mut self, id: &str, text: &str) -> R<LedgerResult> {
        let c = parse_id(id)?;
        self.owner_action(&c, LedgerAction::Rules, None, Some(text.trim().to_string()), None)
    }

    pub fn add_mod(&mut self, id: &str, key: &str) -> R<LedgerResult> {
        let c = parse_id(id)?;
        let k = parse_person_key(key)?;
        self.owner_action(&c, LedgerAction::ModAdd, Some(k), None, None)
    }

    pub fn remove_mod(&mut self, id: &str, key: &str) -> R<LedgerResult> {
        let c = parse_id(id)?;
        let k = parse_person_key(key)?;
        self.owner_action(&c, LedgerAction::ModRemove, Some(k), None, None)
    }

    /// Says an elector left for good: after a week's notice their silence stops
    /// holding up decisions. Void as soon as they sign anything here again. Only for someone
    /// who has signed nothing here for a month; owners can't be retired.
    pub fn retire(&mut self, id: &str, key: &str) -> R<LedgerResult> {
        let c = parse_id(id)?;
        let k = parse_person_key(key)?;
        let st = self.community_of(&c)?;
        if st.is_owner(&k) {
            return Err(ApiError::not_allowed("Owners can't be retired."));
        }
        if !self.retirable(st, &k) {
            return Err(ApiError::not_allowed("They signed something here, or joined, in the last 30 days: they haven't left."));
        }
        self.owner_action(&c, LedgerAction::Retire, Some(k), None, None)
    }

    /// Every entry that took effect, and every one the community overturned (marked so),
    /// newest first.
    pub fn ledger(&self, id: &str) -> R<Vec<LedgerEntryView>> {
        let c = self.community_of(&parse_id(id)?)?;
        let mut v: Vec<LedgerEntryView> =
            c.applied.iter().map(|a| a.entry).chain(c.standing.overturned.iter().copied()).filter_map(|e| self.ledger_view(&e, c)).collect();
        v.sort_by_key(|e| std::cmp::Reverse((e.at, e.seq)));
        Ok(v)
    }

    pub(crate) fn mod_action(&mut self, c: &Id, action: LedgerAction, target: Id, reason: Option<String>) -> R<LedgerEntryView> {
        let me = self.need_me()?;
        let st = self.community_of(c)?;
        if !(st.is_owner(&me.root) || st.is_mod(&me.root)) {
            return Err(ApiError::not_allowed("Only this community's owners and moderators can do that."));
        }
        let r = self.ledger_record(c, action, Some(target), reason, None, None)?;
        let id = self.publish(r)?;
        self.published_entry(c, &id)
    }

    fn community_of_record(&self, target: &Id) -> R<Id> {
        let s = self.store.get(target).ok_or_else(|| ApiError::not_found("post or comment"))?;
        if !matches!(s.record.kind(), Kind::Post | Kind::Comment) {
            return Err(ApiError::not_allowed("Only posts and comments can be moderated that way."));
        }
        s.record.community.ok_or_else(|| ApiError::not_found("community"))
    }

    pub fn remove(&mut self, target: &str, reason: &str) -> R<LedgerEntryView> {
        let t = parse_id(target)?;
        let c = self.community_of_record(&t)?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(ApiError::invalid("Say why. The reason goes in the public ledger."));
        }
        self.mod_action(&c, LedgerAction::Remove, t, Some(reason.into()))
    }

    pub fn pin(&mut self, target: &str, on: bool) -> R<LedgerEntryView> {
        let t = parse_id(target)?;
        let c = self.community_of_record(&t)?;
        self.mod_action(&c, if on { LedgerAction::Pin } else { LedgerAction::Unpin }, t, None)
    }

    pub fn label(&mut self, target: &str, label: &str, reason: Option<&str>) -> R<LedgerEntryView> {
        let t = parse_id(target)?;
        let c = self.community_of_record(&t)?;
        let label = label.trim();
        if label.is_empty() {
            return Err(ApiError::invalid("A label needs a word or two."));
        }
        let _ = reason;
        self.mod_action(&c, LedgerAction::Label, t, Some(label.into()))
    }

    pub fn ban(&mut self, community: &str, key: &str, reason: &str) -> R<LedgerEntryView> {
        let c = parse_id(community)?;
        let k = parse_person_key(key)?;
        if self.community_of(&c)?.is_owner(&k) {
            return Err(ApiError::not_allowed("Owners can't be banned."));
        }
        if Some(k) == self.me_root() {
            return Err(ApiError::not_allowed("You can't ban yourself."));
        }
        self.mod_action(&c, LedgerAction::Ban, k, Some(reason.trim().to_string()).filter(|s| !s.is_empty()))
    }

    pub fn unban(&mut self, community: &str, key: &str, reason: &str) -> R<LedgerEntryView> {
        let c = parse_id(community)?;
        let k = parse_person_key(key)?;
        let local = self.bounced(&k).is_some_and(|b| b.community == c);
        if local {
            self.state.bounced.remove(&k);
            self.mark_state();
            self.mark_changed(c);
        }
        if !self.community_of(&c)?.is_banned(&k) {
            if local {
                // Only bounced on this device: nothing to sign.
                let me = self.need_me()?;
                return Ok(LedgerEntryView {
                    id: Id::ZERO.hex(),
                    seq: 0,
                    at: self.now(),
                    action: "unban".into(),
                    by: vec![self.person(&me.root, self.communities.get(&c))],
                    auto: true,
                    target: Some(LedgerTarget::Key { person: self.person(&k, self.communities.get(&c)) }),
                    reason: Some("let back in on this device".into()),
                    text: None,
                    limits: None,
                    evidence: 0,
                    expires_at: None,
                    vetoes: 0,
                    vetoes_voice: 0.0,
                    vetoes_needed: 0.0,
                    support: 0,
                    overturned: false,
                    vetoable: false,
                    my_review: None,
                    pending_until: None,
                    proposal: None,
                });
            }
            return Err(ApiError::not_found("ban for that key"));
        }
        self.mod_action(&c, LedgerAction::Unban, k, Some(reason.trim().to_string()).filter(|s| !s.is_empty()))
    }

    pub fn admit(&mut self, community: &str, key: &str) -> R<LedgerEntryView> {
        let c = parse_id(community)?;
        let k = parse_person_key(key)?;
        self.mod_action(&c, LedgerAction::Admit, k, None)
    }

    // ------------------------------------------------------------------ posts

    pub fn feed(&self, community: Option<&str>, sort: &str, limit: Option<usize>) -> R<Vec<PostSummary>> {
        let now = self.now();
        let ids: Vec<Id> = match community {
            Some(c) => vec![parse_id(c)?],
            None => self.state.joined.keys().copied().collect(),
        };
        let mut rows: Vec<(f64, PostSummary)> = Vec::new();
        for c in ids {
            let Some(st) = self.communities.get(&c) else { continue };
            for s in self.store.of_kind(&c, Kind::Post) {
                if !self.shown(s, st) {
                    continue;
                }
                let Some(p) = self.post_summary(s) else { continue };
                let t = self.tally(&s.id);
                let age = now.saturating_sub(s.record.created_at);
                let key = match sort {
                    "fresh" | "friends" => s.record.created_at as f64,
                    "loudest" => hoi_trust::loudest(t.up_w, t.down_w),
                    // the first aggregator this person follows that counts it; the post's own
                    // time breaks ties
                    "counted" => p.tallies.first().map_or(f64::MIN, |x| x.up as f64 - x.down as f64),
                    _ => hoi_trust::hot(t.up_w, t.down_w, age),
                };
                if sort == "friends" && !matches!(p.author.trust.as_str(), "me" | "friend") {
                    continue;
                }
                rows.push((key, p));
            }
        }
        let pinned_first = community.is_some();
        rows.sort_by(|a, b| {
            let pa = pinned_first && a.1.pinned;
            let pb = pinned_first && b.1.pinned;
            pb.cmp(&pa).then(b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)).then(b.1.created_at.cmp(&a.1.created_at))
        });
        Ok(rows.into_iter().take(limit.unwrap_or(200)).map(|(_, p)| p).collect())
    }

    /// Opens a post. Reading counts: an established member who reads a newcomer's words and
    /// doesn't hide them vouches for them after a while (PROTOCOL.md §15.2, if `autoVouch`).
    pub fn post(&mut self, id: &str) -> R<PostView> {
        let view = self.post_view(id)?;
        self.note_reads(&view);
        Ok(view)
    }

    pub(crate) fn post_view(&self, id: &str) -> R<PostView> {
        let pid = parse_id(id)?;
        let Some(s) = self.store.get(&pid) else {
            if let Some(rm) = self.removal_of(&pid) {
                let who = rm.by.as_ref().map_or("its author".to_string(), |p| p.name.clone());
                return Err(ApiError::not_found_msg(format!("Poof. Removed by {who}: {}.", rm.reason)));
            }
            return Err(ApiError::not_found("post"));
        };
        let c = self.community_of(&s.record.community.expect("posts have communities"))?;
        if !self.shown(s, c) {
            return Err(ApiError::not_found("post"));
        }
        let summary = self.post_summary(s).ok_or_else(|| ApiError::not_found("post"))?;
        let text = match (self.latest_edit(s), &s.record.body) {
            (Some((t, _)), _) => t,
            (None, Body::Post { text, .. }) => text.clone(),
            _ => String::new(),
        };
        Ok(PostView { summary, text, comments: self.comment_tree(&pid, c, 0), checks: self.checks(s, c), removed: None })
    }

    /// What sending refuses: records beyond the author's own standing never leave the device
    /// (PROTOCOL.md §15.3). Receivers hold them back too; this just says so up front.
    fn check_standing(&self, c: &Id, r: &Record) -> R<()> {
        let me = self.need_me()?;
        let st = self.community_of(c)?;
        if st.is_banned(&me.root) {
            return Err(ApiError::new("banned", "You're banned from this community."));
        }
        if st.standing.blocked(&me.root) {
            return Err(ApiError::new(
                "banned",
                "The community blocked you: enough established members hid your posts or objected. It lifts when their reasons fade.",
            ));
        }
        let level = st.level(&me.root);
        let need = st.limits.ladder.identity_bits;
        if self.store.best_identity(&me.root).is_none_or(|i| i.bits < need) {
            return Err(ApiError::not_allowed(format!("This community asks for {need} bits of identity work behind every key. Strengthen your key first.")));
        }
        if hoi_validate::has_links(r) && level < hoi_moderation::Level::Member {
            return Err(ApiError::not_allowed("Newcomers share text only. Links unlock once members here vouch for you."));
        }
        if hoi_validate::has_media(r) && level < hoi_moderation::Level::Trusted {
            return Err(ApiError::not_allowed("Images and other media unlock for trusted members: established, active, and well received."));
        }
        Ok(())
    }

    /// Sanitizes files someone attaches: sniffs the type, strips metadata, checks. Returns the
    /// references for the record and the bytes to keep.
    fn prepare_media(&self, drafts: &[MediaDraft]) -> R<(Vec<MediaRef>, Blobs)> {
        if drafts.len() > hoi_types::MAX_MEDIA {
            return Err(ApiError::invalid(format!("A post carries at most {} media items.", hoi_types::MAX_MEDIA)));
        }
        let mut refs = Vec::new();
        let mut blobs = Vec::new();
        for d in drafts {
            let raw = unb64(&d.data).map_err(|_| ApiError::invalid("That file arrived damaged."))?;
            let (clean, info) = hmedia::sanitize(&raw).map_err(|e| ApiError::invalid(format!("That file won't do: {e}.")))?;
            let hash = Id::hash(&clean);
            if crate::look::wants_look(info.mime) {
                use crate::look::Look;
                // a host that decodes elsewhere looked at it before the draft (crate::look)
                let look = if self.look_elsewhere {
                    match self.looks.known.get(&hash) {
                        Some(l) => l.clone(),
                        None if hmedia::is_image(info.mime) => {
                            return Err(ApiError::invalid("That picture wasn't checked on this device yet. Attach it again."));
                        }
                        None => Look::Nothing,
                    }
                } else {
                    crate::look::look_here(info.mime, &clean)
                };
                if look == Look::Unreadable {
                    return Err(ApiError::invalid("That picture can't be read here, so nobody else could check it either. Try saving it as JPEG or PNG."));
                }
                if self.blocked_look(&look) {
                    return Err(ApiError::not_allowed("That matches media blocked on this device."));
                }
            }
            let alt = d.alt.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(String::from);
            refs.push(MediaRef { hash, mime: info.mime.to_string(), size: clean.len() as u64, w: info.w, h: info.h, alt });
            blobs.push((hash, clean));
        }
        Ok((refs, blobs))
    }

    fn prepare_post(&self, d: &PostDraft) -> R<(Record, Blobs)> {
        let me = self.need_me()?;
        let c = parse_id(&d.community)?;
        if !self.state.joined.contains_key(&c) {
            return Err(ApiError::not_allowed("Join the community first."));
        }
        let st = self.community_of(&c)?;
        let (media, blobs) = self.prepare_media(d.media.as_deref().unwrap_or(&[]))?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: None,
            created_at: now,
            expires_at: now + st.limits.max_retention,
            body: Body::Post {
                title: d.title.trim().into(),
                text: d.text.trim().into(),
                link: d.link.clone().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()),
                name: Some(me.name),
                media,
            },
            nonce: 0,
        };
        hoi_validate::content(&r)?;
        self.check_standing(&c, &r)?;
        Ok((r, blobs))
    }

    fn prepare_reply(&self, parent: &str, text: &str) -> R<Record> {
        let me = self.need_me()?;
        let pid = parse_id(parent)?;
        let p = self.store.get(&pid).ok_or_else(|| ApiError::not_found("post or comment"))?;
        if !matches!(p.record.kind(), Kind::Post | Kind::Comment) {
            return Err(ApiError::invalid("Replies go under posts and comments."));
        }
        let c = p.record.community.ok_or_else(|| ApiError::not_found("community"))?;
        if !self.state.joined.contains_key(&c) {
            return Err(ApiError::not_allowed("Join the community first."));
        }
        let st = self.community_of(&c)?;
        let now = self.now();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: Some(pid),
            created_at: now,
            expires_at: (now + st.limits.max_retention).min(p.record.expires_at).max(now + 60),
            body: Body::Comment { text: text.trim().into(), name: Some(me.name) },
            nonce: 0,
        };
        hoi_validate::content(&r)?;
        self.check_standing(&c, &r)?;
        Ok(r)
    }

    fn prepare_edit(&self, id: &str, text: &str, title: Option<&str>) -> R<Record> {
        let me = self.need_me()?;
        let pid = parse_id(id)?;
        let p = self.store.get(&pid).ok_or_else(|| ApiError::not_found("post or comment"))?;
        if p.author_root != me.root {
            return Err(ApiError::not_allowed("Only the author can edit."));
        }
        let now = self.now();
        let r = Record {
            v: 1,
            community: p.record.community,
            author: self.device_key()?,
            parent: Some(pid),
            created_at: now,
            expires_at: p.record.expires_at.max(now + 60),
            body: Body::Edit { text: text.trim().into(), title: title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) },
            nonce: 0,
        };
        hoi_validate::content(&r)?;
        if let Some(c) = r.community {
            self.check_standing(&c, &r)?;
        }
        Ok(r)
    }

    fn make_draft(&mut self, r: Record, media: Blobs) -> DraftDto {
        let bits = self.bits_to_stamp(&r);
        let (prefix, suffix) = r.split_for_nonce();
        let token = self.next_draft;
        self.next_draft += 1;
        let made = self.now();
        self.drafts.insert(token, Draft { record: r, bits, made, media });
        DraftDto { token, prefix: b64(&prefix), suffix: b64(&suffix), bits }
    }

    pub fn draft_post(&mut self, d: &PostDraft) -> R<DraftDto> {
        let (r, media) = self.prepare_post(d)?;
        Ok(self.make_draft(r, media))
    }

    pub fn draft_reply(&mut self, parent: &str, text: &str) -> R<DraftDto> {
        let r = self.prepare_reply(parent, text)?;
        Ok(self.make_draft(r, Vec::new()))
    }

    pub fn draft_edit(&mut self, id: &str, text: &str, title: Option<&str>) -> R<DraftDto> {
        let r = self.prepare_edit(id, text, title)?;
        Ok(self.make_draft(r, Vec::new()))
    }

    /// Finishes a draft once the host found a nonce.
    pub fn publish_draft(&mut self, token: u64, nonce: u64) -> R<Published> {
        let d = self.drafts.remove(&token).ok_or_else(|| ApiError::not_found("draft"))?;
        let mut r = d.record;
        r.nonce = nonce;
        if hoi_crypto::pow::stamp_bits(&r.id(), nonce) < d.bits {
            return Err(ApiError::invalid("That stamp doesn't hold enough work."));
        }
        let kind = r.kind();
        self.keep_media(d.media);
        let id = self.publish_mined(r)?;
        self.published(kind, &id)
    }

    pub(crate) fn published(&self, kind: Kind, id: &Id) -> R<Published> {
        if self.signing(id) {
            return Ok(Published::Signing(id.hex()));
        }
        let s = self.store.get(id).ok_or_else(|| ApiError::new("internal", "record vanished"))?;
        Ok(match kind {
            Kind::Post => Published::Post(Box::new(self.post_summary(s).ok_or_else(|| ApiError::new("internal", "post vanished"))?)),
            Kind::Comment => {
                let c = self.community_of(&s.record.community.expect("comments have communities"))?;
                let parent = s.record.parent.expect("comments have parents");
                let tree = self.comment_tree(&parent, c, 0);
                Published::Comment(Box::new(tree.into_iter().find(|x| x.id == id.hex()).ok_or_else(|| ApiError::new("internal", "comment vanished"))?))
            }
            _ => Published::Nothing,
        })
    }

    pub fn create_post(&mut self, d: &PostDraft) -> R<PostSummary> {
        let (r, media) = self.prepare_post(d)?;
        self.keep_media(media);
        let id = self.publish(r)?;
        match self.published(Kind::Post, &id)? {
            Published::Post(p) => Ok(*p),
            Published::Signing(_) => Err(ApiError::new("signing", "Sent to your computer to sign. It shows up once signed.")),
            _ => unreachable!(),
        }
    }

    pub fn reply(&mut self, parent: &str, text: &str) -> R<CommentView> {
        let r = self.prepare_reply(parent, text)?;
        let id = self.publish(r)?;
        match self.published(Kind::Comment, &id)? {
            Published::Comment(c) => Ok(*c),
            Published::Signing(_) => Err(ApiError::new("signing", "Sent to your computer to sign. It shows up once signed.")),
            _ => unreachable!(),
        }
    }

    pub fn edit(&mut self, id: &str, text: &str, title: Option<&str>) -> R<()> {
        let r = self.prepare_edit(id, text, title)?;
        self.publish(r)?;
        Ok(())
    }

    pub fn delete_own(&mut self, id: &str) -> R<()> {
        let me = self.need_me()?;
        let pid = parse_id(id)?;
        if self.deletion_for_target(pid).is_some_and(|job| self.deletions.saved.jobs[&job].author == me.root) {
            return Ok(());
        }
        if self.deletion_home_placement_pending(pid) {
            return Err(ApiError::new("deletion-pending", "A placement still needs its deletion authority. Retry when the placement finishes."));
        }
        let p = self.store.get(&pid).ok_or_else(|| ApiError::not_found("post or comment"))?;
        if p.author_root != me.root {
            return Err(ApiError::not_allowed("Only the author can delete."));
        }
        let now = self.now();
        let r = Record {
            v: 1,
            community: p.record.community,
            author: self.device_key()?,
            parent: Some(pid),
            created_at: now,
            expires_at: p.record.expires_at.max(now + 60),
            body: Body::Delete,
            nonce: 0,
        };
        self.deletion_admission(&r)?;
        self.publish(r)?;
        Ok(())
    }

    pub fn vote(&mut self, target: &str, value: i64) -> R<i64> {
        self.need_me()?;
        if !(-1..=1).contains(&value) {
            return Err(ApiError::invalid("A vote is up, down or nothing."));
        }
        let t = parse_id(target)?;
        let p = self.store.get(&t).ok_or_else(|| ApiError::not_found("post or comment"))?;
        let now = self.now();
        let seq = (now * 1000).max(self.state.vote_seq + 1);
        let r = Record {
            v: 1,
            community: p.record.community,
            author: self.device_key()?,
            parent: Some(t),
            created_at: now,
            expires_at: p.record.expires_at.max(now + 60),
            body: Body::Vote { value, seq },
            nonce: 0,
        };
        self.state.vote_seq = seq;
        self.mark_state();
        self.publish(r)?;
        Ok(self.tally(&t).score)
    }

    pub fn flag(&mut self, target: &str, reason: &str) -> R<()> {
        let t = parse_id(target)?;
        let s = self.store.get(&t).ok_or_else(|| ApiError::not_found("post or comment"))?;
        let c = s.record.community.ok_or_else(|| ApiError::not_found("community"))?;
        let author = s.author_root;
        self.publish_report(c, author, reason, vec![t])?;
        Ok(())
    }

    pub fn save(&mut self, id: &str, on: bool) -> R<()> {
        let t = parse_id(id)?;
        if on {
            if !self.store.contains(&t) {
                return Err(ApiError::not_found("post"));
            }
            self.state.saved.insert(t);
        } else {
            self.state.saved.remove(&t);
        }
        self.mark_state();
        Ok(())
    }

    pub fn saved(&self) -> Vec<PostSummary> {
        let mut v: Vec<PostSummary> = self.state.saved.iter().filter_map(|id| self.store.get(id)).filter_map(|s| self.post_summary(s)).collect();
        v.sort_by_key(|p| std::cmp::Reverse(p.created_at));
        v
    }

    /// The post a search hit counts for: itself, the post a comment or an edit belongs to, or
    /// nothing for a hidden comment.
    fn post_of_hit(&self, id: &Id) -> Option<Id> {
        let s = self.store.get(id)?;
        if s.record.kind() == Kind::Comment && s.record.community.and_then(|c| self.communities.get(&c)).is_none_or(|st| !self.shown(s, st)) {
            return None;
        }
        let mut cur = Some(s);
        for _ in 0..64 {
            match cur {
                Some(s) if s.record.kind() == Kind::Post => return Some(s.id),
                Some(s) => cur = s.record.parent.and_then(|p| self.store.get(&p)),
                None => return None,
            }
        }
        None
    }

    /// Posts whose title, text, edits or comments hold every word of `q` (as a word or the start
    /// of one), newest first, at most 100. From the in-memory index the store keeps,
    /// not a scan; a post counts only if it is shown here.
    pub fn search(&self, q: &str, community: Option<&str>) -> R<Vec<PostSummary>> {
        if hoi_store::words(q).is_empty() {
            return Ok(Vec::new());
        }
        let only = community.map(parse_id).transpose()?;
        // every word somewhere in the post: its title, text, edits or shown comments
        let mut posts: Option<BTreeSet<Id>> = None;
        for word in hoi_store::words(q) {
            let mut here = BTreeSet::new();
            for id in self.store.word_hits(&word) {
                if let Some(p) = self.post_of_hit(&id) {
                    here.insert(p);
                }
            }
            posts = Some(match posts {
                None => here,
                Some(prev) => prev.intersection(&here).copied().collect(),
            });
            if posts.as_ref().is_some_and(|p| p.is_empty()) {
                break;
            }
        }
        let posts = posts.unwrap_or_default();
        let mut v: Vec<PostSummary> = Vec::new();
        for id in posts {
            let Some(s) = self.store.get(&id) else { continue };
            let Some(c) = s.record.community else { continue };
            if !self.state.joined.contains_key(&c) || only.is_some_and(|o| o != c) {
                continue;
            }
            let Some(st) = self.communities.get(&c) else { continue };
            if self.shown(s, st)
                && let Some(p) = self.post_summary(s)
            {
                v.push(p);
            }
        }
        v.sort_by_key(|p| std::cmp::Reverse(p.created_at));
        v.truncate(100);
        Ok(v)
    }

    // ------------------------------------------------------------------ people

    pub fn person_view(&self, key: &str, community: Option<&str>) -> R<PersonView> {
        let k = parse_person_key(key)?;
        let ctx = community.map(parse_id).transpose()?.and_then(|c| self.communities.get(&c));
        let mut communities = Vec::new();
        let mut posts = 0;
        for c in self.state.joined.keys() {
            let Some(st) = self.communities.get(c) else { continue };
            let n = self.store.of_kind(c, Kind::Post).filter(|s| s.author_root == k).count() as u64;
            let any = n > 0 || self.store.in_community(c).any(|s| s.author_root == k);
            if any {
                communities.push(self.community_ref(st));
            }
            posts += n;
        }
        Ok(PersonView { person: self.person(&k, ctx), communities, posts, first_seen: self.state.first_seen.get(&k).copied().unwrap_or(0) })
    }

    pub fn set_trust(&mut self, key: &str, friend: bool) -> R<Person> {
        let k = parse_person_key(key)?;
        if Some(k) == self.me_root() {
            return Err(ApiError::invalid("That's you."));
        }
        if friend {
            self.state.friends.insert(k, self.now());
        } else {
            self.state.friends.remove(&k);
        }
        self.mark_state();
        self.refresh_ring();
        self.renew_trust();
        self.changed_everywhere();
        Ok(self.person(&k, None))
    }

    pub fn set_muted(&mut self, key: &str, muted: bool) -> R<Person> {
        let k = parse_person_key(key)?;
        if Some(k) == self.me_root() {
            return Err(ApiError::invalid("That's you."));
        }
        if muted {
            self.state.muted.insert(k);
        } else {
            self.state.muted.remove(&k);
        }
        self.mark_state();
        self.changed_everywhere();
        Ok(self.person(&k, None))
    }

    pub fn set_petname(&mut self, key: &str, petname: Option<&str>) -> R<Person> {
        let k = parse_person_key(key)?;
        match petname.map(str::trim).filter(|s| !s.is_empty()) {
            Some(p) => {
                self.state.petnames.insert(k, clean_name(p)?);
            }
            None => {
                self.state.petnames.remove(&k);
            }
        }
        self.mark_state();
        self.changed_everywhere();
        Ok(self.person(&k, None))
    }

    pub(crate) fn changed_everywhere(&mut self) {
        let cs: Vec<Id> = self.state.joined.keys().copied().collect();
        for c in cs {
            self.mark_changed(c);
        }
    }

    // ------------------------------------------------------------------ node

    pub fn stats(&self) -> NodeStats {
        let now = self.now();
        NodeStats {
            records: self.store.len() as u64,
            stored_bytes: self.store.bytes(),
            communities: self.state.joined.len() as u64,
            bounced_keys: self.state.bounced.values().filter(|b| b.active(now)).count() as u64,
            bounced_records: self.state.bounced_records,
            peers: self.peers.values().filter(|p| p.said_hello).count() as u64,
            passed_along_bytes: self.state.passed_along,
            media_bytes: self.store.blob_bytes(),
            saved: self.state.saved.len() as u64,
            reading_notes: self.state.reads.len() as u64,
            blocked_pictures: self.state.similar_blocklist.len() as u64,
            fades_by: self.store.all().filter(|s| matches!(s.record.kind(), Kind::Post | Kind::Comment)).map(|s| s.record.expires_at).max().unwrap_or(0),
            held_pieces: self.piece_stats().map_or(0, |p| p.count),
            held_bytes: self.piece_stats().map_or(0, |p| p.bytes),
            held_budget: self.piece_stats().map_or(0, |p| p.budget),
        }
    }

    pub fn settings(&self) -> Settings {
        self.state.settings.clone()
    }

    pub fn set_settings(&mut self, p: &SettingsPatch) -> R<Settings> {
        const MIN_HOLD_GB: f64 = 0.5;
        const MAX_HOLD_GB: f64 = 100.0;
        let native = matches!(self.platform.as_str(), "desktop" | "node");
        let was_public = self.accepts_carry();
        let s = &mut self.state.settings;
        if let Some(x) = p.relay {
            s.relay = x && native;
            // A public node serves strangers: without relaying it serves nobody.
            s.public_node &= s.relay;
        }
        if let Some(x) = p.public_node {
            s.public_node = x && native;
            s.relay |= s.public_node;
        }
        if let Some(x) = p.hold_pieces_gb {
            if x != 0.0 && !(MIN_HOLD_GB..=MAX_HOLD_GB).contains(&x) {
                return Err(ApiError::invalid("Mailboxes for others take from 0.5 to 100 GB."));
            }
            if x > 0.0 && self.platform != "desktop" {
                return Err(ApiError::new("not-allowed", "Only the desktop app keeps mailboxes for others; on a server, hoi-node --hold-pieces does."));
            }
            s.hold_pieces_gb = x;
        }
        // Nobody could reach a holder that isn't a public node: off with it.
        if !s.public_node {
            s.hold_pieces_gb = 0.0;
        }
        if let Some(x) = &p.vpn {
            // An interface name, as the system lists it; nothing else is accepted.
            let name = x.trim();
            if name.len() > 64 || name.chars().any(|c| c.is_control()) {
                return Err(ApiError::invalid("That isn't a network interface name."));
            }
            s.vpn = if native { name.to_string() } else { String::new() };
        }
        if let Some(x) = p.home_network {
            s.home_network = x;
        }
        // desktops and nodes rebuild for themselves: the setting is for browsers and phones
        if let Some(x) = p.less_data {
            s.less_data = x && !native;
        }
        // rosters come from durable devices: those that see holders for days
        if let Some(x) = p.share_holders {
            s.share_holders = x && native;
        }
        if let Some(x) = p.relay_notice_seen {
            s.relay_notice_seen = x;
        }
        if let Some(x) = p.posting_notice_seen {
            s.posting_notice_seen = x;
        }
        if let Some(x) = p.companion {
            s.companion = x && self.platform == "desktop";
        }
        if let Some(x) = p.daily_cap_gb {
            s.daily_cap_gb = x.clamp(0.1, 1000.0);
        }
        if let Some(x) = p.keep_moderated {
            s.keep_moderated = x;
        }
        if let Some(x) = p.link_previews {
            s.link_previews = x;
        }
        let was_direct = s.direct_links;
        if let Some(x) = p.direct_links {
            s.direct_links = x && !matches!(self.platform.as_str(), "desktop" | "node");
        }
        let direct_changed = was_direct != s.direct_links;
        if let Some(x) = p.tor {
            s.tor = x && self.platform == "desktop";
        }
        if let Some(x) = &p.theme
            && matches!(x.as_str(), "system" | "light" | "dark")
        {
            s.theme = x.clone();
        }
        if let Some(x) = &p.motion
            && matches!(x.as_str(), "system" | "reduced")
        {
            s.motion = x.clone();
        }
        if let Some(x) = p.auto_vouch {
            s.auto_vouch = x;
        }
        if let Some(x) = p.auto_media {
            s.auto_media = x;
        }
        if let Some(x) = &p.favorites {
            s.favorites = community_list(x)?;
        }
        if let Some(x) = &p.community_order {
            s.community_order = community_list(x)?;
        }
        self.mark_state();
        if was_public != self.accepts_carry() || direct_changed {
            // The hello's feature list changed: peers learn whether they may ask this node to
            // carry, or list this browser to others for direct links.
            self.announce();
        }
        if direct_changed && !self.state.settings.direct_links {
            // turned off: the direct links close, and nothing more is offered
            self.signals.here.clear();
            self.signals.tried.clear();
            self.signals.due.clear();
        }
        Ok(self.state.settings.clone())
    }

    pub fn wipe(&mut self) -> R<()> {
        self.wipe_all()
    }

    /// Loads perceptual fingerprints of pictures to refuse (a PDQ hash list: one 64-hex hash at
    /// the start of each line, anything after it ignored, `#` comments). Kept on this device
    /// only. Returns how many were new.
    pub fn import_blocklist(&mut self, text: &str) -> R<u64> {
        let mut added = 0;
        let mut seen = 0;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let token = line.split(|c: char| c == ',' || c.is_whitespace()).next().unwrap_or("");
            let Some(h) = hoi_validate::similar::from_hex(&token.to_ascii_lowercase()) else { continue };
            seen += 1;
            if self.state.similar_blocklist.insert(h) {
                added += 1;
            }
        }
        if seen == 0 {
            return Err(ApiError::invalid("No picture fingerprints in that file. Expected one 64-character PDQ hash per line."));
        }
        // drop what's here already that matches (on hosts that decode elsewhere, once looked at)
        let pictures: Vec<(Id, String)> =
            self.store.blobs().map(|(h, _)| *h).filter_map(|h| self.mime_of_blob(&h).filter(|m| hmedia::is_image(m)).map(|m| (h, m))).collect();
        for (h, mime) in pictures {
            if let Some(look) = self.look_at(h, &mime, None, crate::look::Why::Recheck)
                && self.blocked_look(&look)
            {
                self.refuse_stored_media(h);
            }
        }
        self.purge_blocked_profiles();
        self.mark_state();
        self.changed_everywhere();
        Ok(added)
    }

    // ------------------------------------------------------------------ the ladder

    fn next_seq(&mut self) -> u64 {
        let seq = (self.now() * 1000).max(self.state.vote_seq + 1);
        self.state.vote_seq = seq;
        self.mark_state();
        seq
    }

    /// Where someone (you, by default) stands in a community, and what's missing.
    pub fn standing(&self, community: &str, key: Option<&str>) -> R<StandingView> {
        let c = self.community_of(&parse_id(community)?)?;
        let k = match key {
            Some(k) => parse_person_key(k)?,
            None => self.need_me()?.root,
        };
        Ok(self.standing_view(c, &k))
    }

    /// Vouch for a person (1), object to them with a reason (-1), or take it back (0). Counts
    /// once you are established in the community (PROTOCOL.md §15.2).
    pub fn vouch(&mut self, community: &str, key: &str, value: i64, reason: Option<&str>) -> R<StandingView> {
        let me = self.need_me()?;
        let c = parse_id(community)?;
        let k = parse_person_key(key)?;
        if k == me.root {
            return Err(ApiError::invalid("Nobody vouches for themselves."));
        }
        if !(-1..=1).contains(&value) {
            return Err(ApiError::invalid("A vouch is 1, -1 or 0."));
        }
        if !self.state.joined.contains_key(&c) {
            return Err(ApiError::not_allowed("Join the community first."));
        }
        self.community_of(&c)?;
        let reason = if value == -1 {
            let r = reason.map(str::trim).unwrap_or("");
            if !hoi_types::is_hide_reason(r) {
                return Err(ApiError::invalid("Say why: spam, abuse, illegal, off-topic or rule:<n>."));
            }
            Some(r.to_string())
        } else {
            None
        };
        let now = self.now();
        let seq = self.next_seq();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: Some(k),
            created_at: now,
            expires_at: now + KEEP,
            body: Body::Vouch { value, seq, reason },
            nonce: 0,
        };
        hoi_validate::content(&r)?;
        self.publish(r)?;
        self.state.reads.retain(|x| !(x.community == c && x.author == k));
        self.standing(community, Some(key))
    }

    /// Judge a post or comment ("ok" or "hide" with a reason), or a moderator's or owner's
    /// ledger entry ("veto" with a reason, or "ok" to support it). "none" takes it back.
    pub fn review(&mut self, target: &str, verdict: &str, reason: Option<&str>) -> R<()> {
        let me = self.need_me()?;
        let t = parse_id(target)?;
        let verdict = Verdict::parse(verdict).map_err(|_| ApiError::invalid("A verdict is ok, hide, veto or none."))?;
        let s = self.store.get(&t).ok_or_else(|| ApiError::not_found("post, comment or ledger entry"))?;
        let c = s.record.community.ok_or_else(|| ApiError::not_found("community"))?;
        let author = s.author_root;
        let target_expires = s.record.expires_at;
        let is_ledger = s.record.kind() == Kind::Ledger;
        let proposal = self.communities.get(&c).and_then(|st| st.proposals.get(&t)).map(|p| (p.deadline, p.signers.contains(&me.root)));
        if author == me.root || proposal.is_some_and(|p| p.1) {
            return Err(ApiError::not_allowed(if proposal.is_some() {
                "You proposed it: your signature already says yes."
            } else if is_ledger {
                "Nobody vetoes their own action."
            } else {
                "Nobody reviews their own words."
            }));
        }
        if proposal.is_some_and(|p| self.now() >= p.0) {
            return Err(ApiError::not_allowed("Answers to that proposal closed: it was decided."));
        }
        let reason = match verdict {
            Verdict::Hide => {
                let r = reason.map(str::trim).unwrap_or("");
                if !hoi_types::is_hide_reason(r) {
                    return Err(ApiError::invalid("Say why: spam, abuse, illegal, off-topic or rule:<n>. Disagreeing is what downvotes are for."));
                }
                Some(r.to_string())
            }
            Verdict::Veto => {
                let r = reason.map(str::trim).unwrap_or("");
                if r.is_empty() {
                    return Err(ApiError::invalid(if proposal.is_some() {
                        "Say why you object. The reason is public."
                    } else {
                        "Say why this action was wrong. The reason is public."
                    }));
                }
                Some(r.to_string())
            }
            _ => None,
        };
        let st = self.community_of(&c)?;
        let retention = st.limits.max_retention;
        let now = self.now();
        let seq = self.next_seq();
        let r = Record {
            v: 1,
            community: Some(c),
            author: self.device_key()?,
            parent: Some(t),
            created_at: now,
            // answers to a proposal last as long as it does: they decide it
            expires_at: if proposal.is_some() { target_expires } else { target_expires.min(now + retention).max(now + 60) },
            body: Body::Review { verdict, seq, reason: reason.clone() },
            nonce: 0,
        };
        hoi_validate::content(&r)?;
        self.publish(r)?;
        if verdict == Verdict::Hide {
            self.state.reads.retain(|x| !(x.community == c && x.author == author));
            if reason.as_deref() == Some("illegal") {
                // never keep or pass on media you call illegal
                self.forget_media_of(&t);
            }
        }
        self.mark_changed(c);
        Ok(())
    }

    /// Newcomers' posts waiting for you (established members): read them; what you don't hide
    /// earns them your vouch.
    pub fn review_queue(&self, community: &str) -> R<Vec<PostSummary>> {
        let c = self.community_of(&parse_id(community)?)?;
        Ok(self.review_queue_of(c).iter().filter_map(|p| self.store.get(p)).filter_map(|s| self.post_summary(s)).collect())
    }

    /// Checked media bytes for display.
    pub fn media(&self, hash: &str) -> R<MediaBlob> {
        let h = parse_id(hash)?;
        if self.media_refused(&h) || self.state.bad_media.contains(&h) {
            return Err(ApiError::not_allowed("This device refuses that media."));
        }
        let data = self.store.blob(&h).ok_or_else(|| ApiError::not_found("media"))?;
        let mime = self
            .store
            .media_users(&h)
            .filter(|s| !self.deletion_restore_quarantined(s.record.community))
            .find_map(|s| match &s.record.body {
                Body::Post { media, .. } => media.iter().find(|m| m.hash == h).map(|m| m.mime.clone()),
                _ => None,
            })
            .ok_or_else(|| ApiError::not_found("media"))?;
        Ok(MediaBlob { hash: h.hex(), mime, data: b64(data) })
    }

    /// Remembers whose records this person read in an opened post: newcomers', and members'
    /// whose voice is still thin (vouched in with many others). Kept on this device only.
    fn note_reads(&mut self, v: &PostView) {
        let Some(me) = self.me_root() else { return };
        let Ok(c) = parse_id(&v.summary.community.id) else { return };
        let Some(st) = self.communities.get(&c) else { return };
        if !self.state.settings.auto_vouch || st.level(&me) < hoi_moderation::Level::Established {
            return;
        }
        let now = self.now();
        let mut seen: Vec<(Id, Id)> = Vec::new();
        if v.summary.napping.is_none() && !v.summary.mine {
            seen.push((parse_id(&v.summary.author.key).unwrap_or(Id::ZERO), parse_id(&v.summary.id).unwrap_or(Id::ZERO)));
        }
        let mut stack: Vec<&CommentView> = v.comments.iter().collect();
        while let Some(cm) = stack.pop() {
            if cm.napping.is_none() && cm.removed.is_none() && !cm.mine {
                seen.push((parse_id(&cm.author.key).unwrap_or(Id::ZERO), parse_id(&cm.id).unwrap_or(Id::ZERO)));
            }
            stack.extend(cm.children.iter());
        }
        seen.retain(|(author, _)| self.wants_vouch(st, &me, author));
        if seen.is_empty() {
            return;
        }
        for (author, id) in seen {
            match self.state.reads.iter_mut().find(|r| r.community == c && r.author == author) {
                Some(r) => {
                    r.ids.insert(id);
                }
                None => self.state.reads.push(crate::state::PendingRead { community: c, author, ids: [id].into(), first: now }),
            }
        }
        self.mark_state();
        self.auto_vouch();
    }

    /// Whether `me` reading `author` would earn them a vouch: they're a newcomer, or a member
    /// whose voice is still under a full one, and `me` hasn't vouched for or objected to them.
    fn wants_vouch(&self, st: &Community, me: &Id, author: &Id) -> bool {
        *author != Id::ZERO
            && author != me
            && !st.is_owner(author)
            && !st.is_mod(author)
            && !st.standing.blocked(author)
            && (!st.is_admitted(author) || st.standing.voice(author) < hoi_moderation::ladder::VOICE)
            && self.my_vouch(st, me, author).is_none()
    }

    /// Vouches for people this person read and didn't hide (PROTOCOL.md §15.6): newcomers get
    /// in, members vouched in with many others get a fuller voice.
    pub(crate) fn auto_vouch(&mut self) {
        let Some(me) = self.me_root() else { return };
        if !self.state.settings.auto_vouch || self.device_kp.is_none() {
            return;
        }
        let now = self.now();
        let due: Vec<(Id, Id)> =
            self.state.reads.iter().filter(|r| r.ids.len() >= AUTO_VOUCH_READS && r.first + AUTO_VOUCH_DELAY <= now).map(|r| (r.community, r.author)).collect();
        for (c, author) in due {
            self.state.reads.retain(|r| !(r.community == c && r.author == author));
            self.mark_state();
            let Some(st) = self.communities.get(&c) else { continue };
            if st.level(&me) < hoi_moderation::Level::Established || !self.wants_vouch(st, &me, &author) || self.state.muted.contains(&author) {
                continue;
            }
            let newcomer = !st.is_admitted(&author);
            let hid_any = self.store.by_author(&c, &author).any(|s| self.my_review(&s.id) == Some(Verdict::Hide));
            let reported = self.store.of_kind(&c, Kind::Report).any(|s| s.author_root == me && s.record.parent == Some(author));
            if hid_any || reported {
                continue;
            }
            let name = self.display_name(&author);
            if self.vouch(&c.hex(), &author.hex(), 1, None).is_ok() {
                self.notice(if newcomer {
                    format!("You vouched for {name}: you read their posts and didn't hide them.")
                } else {
                    format!("You vouched for {name}: you read their posts and didn't hide them, so their voice here grows.")
                });
            }
        }
    }
}

impl ApiError {
    pub fn not_found_msg(msg: String) -> ApiError {
        ApiError::new("not-found", msg)
    }
}
