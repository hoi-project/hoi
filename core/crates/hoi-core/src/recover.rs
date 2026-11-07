//! Recovery without losing standing.
//!
//! A person's recovery line holds a master secret. Every root they will ever sign with, and
//! every recovery key that may replace one, is derived from it (`hoi_crypto::recovery`): the
//! root of generation 0 is the person's id for good, and each root's identity record commits to
//! the recovery key of the next generation. This device keeps the master only until the person
//! says they saved the line (or while it restores), so a stolen device gives a thief the root of
//! the day but never the power to replace it.
//!
//! **Restoring** from the line always replaces the root: the device derives the next root,
//! publishes its identity (which names the person in `of`), a `rotate` signed by the committed
//! recovery key, and a blessing for itself. From the rotation's time on, the old root and its
//! devices sign nothing that counts. If the device learns later that the person's chain had gone
//! further than it knew, it rotates again from the current root; every root and key is derived,
//! so nothing else is needed.
//!
//! **Friends** hold the master in shares: three of three to seven people, each share sealed to
//! every device of theirs (`share` records in a community both are in). A new device shows a
//! request with check digits; each friend's device opens its share and seals it to the
//! request's one-time key; three answers rebuild the line.

use hoi_crypto::envelope::{XKey, open_sealed, seal_to};
use hoi_crypto::recovery::{commitment, recovery_at, root_at};
use hoi_crypto::shamir;
use hoi_types::uri::{b64, unb64};
use hoi_types::{Body, Id, Kind, Record};
use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

use crate::api::R;
use crate::error::ApiError;
use crate::node::Node;

/// Shares a person may give: at least the three it takes, at most seven.
pub const MIN_FRIENDS: usize = 3;
pub const MAX_FRIENDS: usize = 7;
/// Shares it takes to bring the line back.
pub const THRESHOLD: u8 = 3;
/// Roots one chain may have; generations beyond this are refused.
const MAX_GENERATIONS: u32 = 64;
const SHARE_INFO: &[u8] = b"hoi recovery share v1";
const ANSWER_INFO: &[u8] = b"hoi recovery answer v1";
const ASK_PREFIX: &str = "hoi-ask-1-";
const ANSWER_PREFIX: &str = "hoi-share-1-";

/// Shares sealed to each of a friend's devices: (device, sealed).
type Sealed = Vec<(Id, Vec<u8>)>;

/// What a friend's device answers a recovery request with, before sealing: the person and the
/// share (`x ‖ y`).
const ANSWER_LEN: usize = 32 + 33;

/// A recovery request this device made: its one-time key and the shares that came back.
#[derive(Default)]
pub(crate) struct Asking {
    key: Option<XKey>,
    /// (person, share) per answer.
    answers: Vec<(Id, shamir::Share)>,
}

/// What [`Node::ask_friends`] shows: the code to send and the digits to read out.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryAsk {
    pub code: String,
    pub digits: String,
}

/// What a friend's device shows for a request before answering it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryAnswer {
    pub digits: String,
    pub code: String,
}

/// One person whose share this device keeps.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldShare {
    pub person: String,
    pub name: String,
    pub short: String,
    pub since: u64,
}

/// How this device's own recovery stands.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    /// The device still keeps the recovery line (until the person says it is saved).
    pub holds_line: bool,
    /// The device is bringing a key back and has not taken over its root yet.
    pub restoring: bool,
    /// How many times this person's root was replaced.
    pub rotations: usize,
    /// The friends who hold a share, by person id.
    pub friends: Vec<String>,
}

fn digits(key: &[u8; 32]) -> String {
    let h = blake3::derive_key("hoi recovery request digits v1", key);
    let n = u32::from_le_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

fn share_aad(person: &Id, friend: &Id, device: &Id) -> Vec<u8> {
    [b"hoi/recovery-share/1".as_slice(), &person.0, &friend.0, &device.0].concat()
}

impl Node {
    // ------------------------------------------------------------------ the line

    /// The master secret's root of generation 0 is the person; `generation` is that of the
    /// root this device signs with. The identity record it publishes commits to the next
    /// recovery key, and names the person when it continues one.
    pub(crate) fn identity_body(master: &[u8; 32], generation: u32, work: u64) -> Body {
        let person = root_at(master, 0).public();
        Body::Identity { work, rc: Some(commitment(&recovery_at(master, generation + 1).public())), of: (generation > 0).then_some(person) }
    }

    /// The recovery line while this device keeps it (`hoi-key-1-…`, `hoi_crypto::recovery`):
    /// whoever holds it can always replace this person's root, so it belongs in a password
    /// manager or with friends, and this device forgets it once told it was saved.
    pub fn recovery_key(&self) -> R<String> {
        let me = self.need_me()?;
        if me.burned {
            return Err(ApiError::not_allowed("This key is burned. There is nothing left to back up."));
        }
        let master = self.master.as_ref().ok_or_else(|| {
            ApiError::not_allowed("This device no longer keeps your recovery key: it is in your password manager, on paper, or with your friends.")
        })?;
        Ok(hoi_crypto::recovery::encode(master))
    }

    /// The person saved the line (or gave it to friends): this device forgets it. From then
    /// on a stolen device can't replace the root it holds.
    pub fn forget_recovery_key(&mut self) -> R<RecoveryStatus> {
        self.need_me()?;
        if self.state.me.as_ref().is_some_and(|m| m.restoring) {
            return Err(ApiError::not_allowed("Wait until this device has found your communities again, then forget the line."));
        }
        self.master = None;
        self.mark_keys();
        self.recovery_status()
    }

    pub fn recovery_status(&self) -> R<RecoveryStatus> {
        let me = self.need_me()?;
        let friends = self.own_shares().into_iter().filter(|(_, sealed)| !sealed.is_empty()).map(|(friend, _)| friend.hex()).collect();
        Ok(RecoveryStatus { holds_line: self.master.is_some(), restoring: me.restoring, rotations: self.store.rotations_of(&me.root).len(), friends })
    }

    /// Brings a person back from their recovery line on a device without a key: a fresh device
    /// key, and a new root that takes over from the current one. The rotation happens
    /// at once; if this device later learns the person's chain had gone further, it rotates
    /// again from there ([`Node::catch_up_rotation`]).
    pub fn restore_identity(&mut self, recovery_key: &str, name: &str) -> R<crate::dto::Me> {
        if self.state.me.is_some() {
            return Err(ApiError::not_allowed("This device already has a key. Wipe it first to bring another one."));
        }
        let name = crate::api::clean_name(name)?;
        let master = Zeroizing::new(hoi_crypto::recovery::decode(recovery_key).map_err(|e| match e {
            hoi_crypto::CryptoError::Format => ApiError::invalid("That isn't a hoi recovery key. It starts with hoi-key-1-."),
            _ => ApiError::new("bad-passphrase", "That recovery key has a typo or is damaged. Check it against your password manager and try again."),
        })?);
        self.restore_from_master(master, name)
    }

    fn restore_from_master(&mut self, master: Zeroizing<[u8; 32]>, name: String) -> R<crate::dto::Me> {
        let person = root_at(&master, 0).public();
        if !hoi_crypto::key_is_strong(&person) {
            return Err(ApiError::invalid("That recovery key doesn't hold a usable key."));
        }
        let device = hoi_crypto::Keypair::generate()?;
        let now = self.now();
        self.state.me = Some(crate::state::Identity {
            root: person,
            device: device.public(),
            name,
            created_at: now,
            has_root: false,
            work: None,
            burned: false,
            avatar: None,
            avatar_revision: 0,
            avatar_color: None,
            avatar_color_revision: 0,
            generation: 0,
            restoring: true,
            rc: None,
            of: None,
            remote: false,
        });
        self.state.first_seen.entry(person).or_insert(now);
        self.state.device_labels.insert(device.public(), self.this_device_label().into());
        self.device_kp = Some(device);
        self.master = Some(master);
        self.mark_state();
        self.mark_keys();
        // The first root's identity, made again from the line when this device has never seen
        // it: the same commitment, so it agrees with the original wherever both are.
        if self.store.identity(&person).is_none() {
            let first = root_at(self.master.as_ref().expect("just set"), 0);
            let work = hoi_crypto::identity::mine(&person, hoi_crypto::identity::MIN_BITS);
            let body = Self::identity_body(self.master.as_ref().expect("just set"), 0, work);
            let r = Record { v: 1, community: None, author: person, parent: None, created_at: now, expires_at: now + hoi_validate::KEEP, body, nonce: 0 };
            self.publish_as(r, &first)?;
        }
        self.catch_up_rotation()?;
        self.me().ok_or_else(ApiError::no_identity)
    }

    /// While this device keeps the line: if the person's current root (as far as this device
    /// knows) isn't the one it signs with, take over from it with the next generation. Runs after
    /// a restore and whenever records arrive during one.
    pub(crate) fn catch_up_rotation(&mut self) -> R<()> {
        let (Some(me), Some(master)) = (self.state.me.clone(), self.master.clone()) else { return Ok(()) };
        // Only while restoring: two devices that both keep the line would otherwise take over
        // from each other for ever.
        if me.burned || !me.restoring {
            return Ok(());
        }
        let current = self.store.current_root(&me.root);
        if self.root_kp.as_ref().is_some_and(|k| k.public() == current) {
            // done once the network has spoken for the person: their identity or a record of
            // theirs came from a peer, so the chain here is the one others hold
            if self.heard_of_person(&me.root)
                && let Some(m) = self.state.me.as_mut()
            {
                m.restoring = false;
                self.mark_state();
                self.changed_everywhere();
            }
            return Ok(());
        }
        let Some(generation) = (0..MAX_GENERATIONS).find(|g| root_at(&master, *g).public() == current) else {
            return Err(ApiError::not_allowed("Someone else's key took over this identity. This recovery key can't bring it back."));
        };
        let next = generation + 1;
        let root = root_at(&master, next);
        let now = self.now();
        let work = hoi_crypto::identity::mine(&root.public(), hoi_crypto::identity::MIN_BITS);
        let identity = Record {
            v: 1,
            community: None,
            author: root.public(),
            parent: None,
            created_at: now,
            expires_at: now + hoi_validate::KEEP,
            body: Self::identity_body(&master, next, work),
            nonce: 0,
        };
        self.publish_as(identity, &root)?;
        let recovery = recovery_at(&master, next);
        let rotate = Record {
            v: 1,
            community: None,
            author: recovery.public(),
            parent: None,
            created_at: now,
            expires_at: now + hoi_validate::KEEP,
            body: Body::Rotate { person: me.root, to: root.public() },
            nonce: 0,
        };
        self.publish_as(rotate, &recovery)?;
        let (rc, of) = match Self::identity_body(&master, next, work) {
            Body::Identity { rc, of, .. } => (rc, of),
            _ => (None, None),
        };
        if let Some(m) = self.state.me.as_mut() {
            m.work = Some(work);
            m.generation = next;
            m.has_root = true;
            // what the new root committed to, for its later identity records (more key work)
            m.rc = rc;
            m.of = of;
        }
        self.root_kp = Some(root);
        self.mark_state();
        self.mark_keys();
        self.renew_delegation();
        self.changed_everywhere();
        Ok(())
    }

    /// Whether a peer told this device about `person`: their identity, or a record of theirs.
    fn heard_of_person(&self, person: &Id) -> bool {
        self.store.identity(person).and_then(|i| self.store.get(&i.record)).is_some_and(|s| s.from_peers > 0)
            || self.state.joined.keys().any(|c| self.store.by_author(c, person).any(|s| s.from_peers > 0))
    }

    // ------------------------------------------------------------------ friends

    /// The `share` records this person published, latest per friend: friend → sealed shares.
    fn own_shares(&self) -> Vec<(Id, Sealed)> {
        let Some(me) = self.state.me.as_ref() else { return Vec::new() };
        let mut latest: std::collections::BTreeMap<Id, (u64, Sealed)> = std::collections::BTreeMap::new();
        for c in self.state.joined.keys() {
            for s in self.store.by_author(c, &me.root).filter(|s| s.record.kind() == Kind::Share) {
                if let (Some(friend), Body::Share { sealed, seq }) = (s.record.parent, &s.record.body)
                    && latest.get(&friend).is_none_or(|(q, _)| q < seq)
                {
                    latest.insert(friend, (*seq, sealed.clone()));
                }
            }
        }
        latest.into_iter().map(|(f, (_, s))| (f, s)).collect()
    }

    /// The seal keys of `person`'s live devices, from their blessings.
    fn seal_keys_of(&self, person: &Id) -> Vec<(Id, [u8; 32])> {
        let now = self.now();
        let root = self.store.current_root(person);
        self.store
            .delegations_of(&root)
            .into_iter()
            .filter(|(_, d)| d.info.expires_at >= now && d.info.revoked_at.is_none_or(|rv| rv > now))
            .filter_map(|(device, d)| match &self.store.get(&d.record)?.record.body {
                Body::Delegation { seal: Some(k), .. } => Some((device, k.0)),
                _ => None,
            })
            .take(16)
            .collect()
    }

    /// Splits the recovery line among `friends` (three to seven people this person shares
    /// `community` with), any three of whom bring it back: one `share` record each, the share
    /// sealed to every device of theirs. Needs the line on this device.
    pub fn give_shares(&mut self, community: &str, friends: &[String]) -> R<RecoveryStatus> {
        let me = self.need_me()?;
        let c = crate::api::parse_id(community)?;
        if !self.state.joined.contains_key(&c) || self.is_private_community(&c) {
            return Err(ApiError::invalid("Give shares in an open or invite-only community you're in."));
        }
        let friends: Vec<Id> = friends.iter().map(|f| crate::api::parse_id(f)).collect::<R<_>>()?;
        let mut unique = friends.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != friends.len() || friends.contains(&me.root) || !(MIN_FRIENDS..=MAX_FRIENDS).contains(&friends.len()) {
            return Err(ApiError::invalid(format!("Pick {MIN_FRIENDS} to {MAX_FRIENDS} different people other than you.")));
        }
        let master = self
            .master
            .clone()
            .ok_or_else(|| ApiError::not_allowed("This device no longer keeps your recovery key. Give shares from the line: bring it back here first."))?;
        let mut keys = Vec::new();
        for f in &friends {
            let k = self.seal_keys_of(f);
            if k.is_empty() {
                return Err(ApiError::not_allowed(format!("{} has no device that can keep a share yet. Ask them to open hoi once.", self.display_name(f))));
            }
            keys.push(k);
        }
        let shares = shamir::split(master.as_slice(), friends.len() as u8, THRESHOLD)?;
        let now = self.now();
        for ((friend, devices), share) in friends.iter().zip(keys).zip(shares) {
            let plain = share.to_bytes();
            let sealed = devices
                .iter()
                .map(|(device, key)| seal_to(key, SHARE_INFO, &share_aad(&me.root, friend, device), &plain).map(|s| (*device, s)))
                .collect::<Result<Vec<_>, _>>()?;
            let r = Record {
                v: 1,
                community: Some(c),
                author: me.device,
                parent: Some(*friend),
                created_at: now,
                expires_at: now + hoi_validate::KEEP,
                body: Body::Share { sealed, seq: now },
                nonce: 0,
            };
            self.publish(r)?;
        }
        // friends no longer named have theirs taken back
        for (old, sealed) in self.own_shares() {
            if !friends.contains(&old) && !sealed.is_empty() {
                let r = Record {
                    v: 1,
                    community: Some(c),
                    author: me.device,
                    parent: Some(old),
                    created_at: now,
                    expires_at: now + hoi_validate::KEEP,
                    body: Body::Share { sealed: Vec::new(), seq: now },
                    nonce: 0,
                };
                self.publish(r)?;
            }
        }
        self.recovery_status()
    }

    /// People in `community` who could keep a share: they have a device that can open one.
    pub fn share_candidates(&self, community: &str) -> R<Vec<crate::dto::Person>> {
        let me = self.need_me()?;
        let c = crate::api::parse_id(community)?;
        let st = self.communities.get(&c).ok_or_else(|| ApiError::not_found("community"))?;
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for s in self.store.in_community(&c).rev() {
            let p = s.author_root;
            if p != me.root && seen.insert(p) && !self.seal_keys_of(&p).is_empty() {
                out.push(self.person(&p, Some(st)));
            }
            if out.len() >= 64 {
                break;
            }
        }
        Ok(out)
    }

    /// The people whose share this device keeps (sealed to it).
    pub fn held_shares(&self) -> R<Vec<HeldShare>> {
        let me = self.need_me()?;
        let mut out = Vec::new();
        for (person, (at, _)) in self.shares_for_me(&me.root, &me.device) {
            out.push(HeldShare { person: person.hex(), name: self.display_name(&person), short: person.short_key(), since: at });
        }
        Ok(out)
    }

    /// person → (when, this device's sealed share), from the latest `share` each person gave us.
    fn shares_for_me(&self, me: &Id, device: &Id) -> std::collections::BTreeMap<Id, (u64, Vec<u8>)> {
        let mut latest: std::collections::BTreeMap<Id, (u64, u64, Option<Vec<u8>>)> = std::collections::BTreeMap::new();
        for c in self.state.joined.keys() {
            for s in self.store.children(me).filter(|s| s.record.community == Some(*c) && s.record.kind() == Kind::Share) {
                if let Body::Share { sealed, seq } = &s.record.body
                    && latest.get(&s.author_root).is_none_or(|(q, _, _)| q < seq)
                {
                    let mine = sealed.iter().find(|(d, _)| d == device).map(|(_, x)| x.clone());
                    latest.insert(s.author_root, (*seq, s.record.created_at, mine));
                }
            }
        }
        latest.into_iter().filter_map(|(p, (_, at, x))| x.map(|x| (p, (at, x)))).collect()
    }

    /// On a device without a key: a one-time key for friends to seal their shares to, as a code
    /// to send them and digits to read out so they know the request is really from you.
    pub fn ask_friends(&mut self) -> R<RecoveryAsk> {
        if self.state.me.is_some() {
            return Err(ApiError::not_allowed("This device already has a key."));
        }
        let key = XKey::generate()?;
        let public = key.public();
        self.asking = Asking { key: Some(key), answers: Vec::new() };
        Ok(RecoveryAsk { code: format!("{ASK_PREFIX}{}", b64(&public)), digits: digits(&public) })
    }

    fn ask_key(code: &str) -> R<[u8; 32]> {
        let body = code.trim().strip_prefix(ASK_PREFIX).ok_or_else(|| ApiError::invalid("That isn't a recovery request. It starts with hoi-ask-1-."))?;
        unb64(body).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()).ok_or_else(|| ApiError::invalid("That recovery request is damaged."))
    }

    /// The digits a request shows, for a friend to compare before answering it.
    pub fn request_digits(&self, code: &str) -> R<String> {
        Ok(digits(&Self::ask_key(code)?))
    }

    /// Answers `person`'s recovery request with the share this device keeps for them, sealed to
    /// the request's one-time key. Only after the person read out the same digits.
    pub fn answer_request(&self, person: &str, code: &str) -> R<RecoveryAnswer> {
        let me = self.need_me()?;
        let person = crate::api::parse_id(person)?;
        let to = Self::ask_key(code)?;
        let (_, sealed) =
            self.shares_for_me(&me.root, &me.device).remove(&person).ok_or_else(|| ApiError::not_found("share for this person on this device"))?;
        let key = self.seal_key().ok_or_else(|| ApiError::not_allowed("This device can't open shares."))?;
        let share = open_sealed(&key, SHARE_INFO, &share_aad(&person, &me.root, &me.device), &sealed)
            .map_err(|_| ApiError::invalid("That share can't be opened on this device."))?;
        let mut plain = Zeroizing::new(Vec::with_capacity(ANSWER_LEN));
        plain.extend_from_slice(&person.0);
        plain.extend_from_slice(&share);
        let answer = seal_to(&to, ANSWER_INFO, &to, &plain)?;
        Ok(RecoveryAnswer { digits: digits(&to), code: format!("{ANSWER_PREFIX}{}", b64(&answer)) })
    }

    /// Takes one friend's answer. Once three agree on the person, the line is rebuilt and the
    /// person restored as from the line itself. Returns how many answers are in, and the person
    /// once restored.
    pub fn take_answer(&mut self, code: &str, name: &str) -> R<(usize, Option<crate::dto::Me>)> {
        let body = code.trim().strip_prefix(ANSWER_PREFIX).ok_or_else(|| ApiError::invalid("That isn't a friend's answer. It starts with hoi-share-1-."))?;
        let sealed = unb64(body).map_err(|_| ApiError::invalid("That answer is damaged."))?;
        let key = self.asking.key.as_ref().ok_or_else(|| ApiError::not_allowed("Ask your friends from this device first."))?;
        let plain = open_sealed(key, ANSWER_INFO, &key.public(), &sealed).map_err(|_| ApiError::invalid("That answer isn't for this request."))?;
        if plain.len() != ANSWER_LEN {
            return Err(ApiError::invalid("That answer is damaged."));
        }
        let person = Id(plain[..32].try_into().expect("32 bytes"));
        let share = shamir::Share::from_bytes(&plain[32..])?;
        if !self.asking.answers.iter().any(|(p, s)| *p == person && s.x == share.x) {
            self.asking.answers.push((person, share));
        }
        let mine: Vec<shamir::Share> = self.asking.answers.iter().filter(|(p, _)| *p == person).map(|(_, s)| s.clone()).collect();
        if mine.len() < THRESHOLD as usize {
            return Ok((mine.len(), None));
        }
        let mut rebuilt = shamir::combine(&mine)?;
        let master: Zeroizing<[u8; 32]> = Zeroizing::new(rebuilt.as_slice().try_into().map_err(|_| ApiError::invalid("Those answers don't fit together."))?);
        rebuilt.zeroize();
        if root_at(&master, 0).public() != person {
            return Err(ApiError::invalid("Those answers don't fit together: one of them is from another request, or a share is wrong."));
        }
        self.asking = Asking::default();
        let name = crate::api::clean_name(name)?;
        let me = self.restore_from_master(master, name)?;
        Ok((mine.len(), Some(me)))
    }
}
