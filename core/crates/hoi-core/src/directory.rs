//! A directory of open communities: what connected nodes carry, and what this device heard of
//! before, from them and from the nodes they heard of it from (PROTOCOL.md §14.4). A listing
//! carries only the signed community definition and public join routes: no posts, profiles or
//! pairing credentials. It is a discovery aid, not a global name registry or an automatic
//! subscription: nothing here joins anything or dials a route until the person chooses.
//!
//! Every listing is checked when it arrives (signature, stamp, open admission, time) and kept
//! by key, so equal names stay separate communities. A device keeps what it heard between
//! starts, bounded, and forgets a listing no node confirmed holding for three weeks. Nodes that
//! advertise `directory` pass what they heard on, a window at a time, so a community carried by
//! one public node comes to be known by every node a few hops away, and every device that
//! reaches any of them, with nothing configured anywhere.
//!
//! What a listing can make a device do is the question: its routes are passive
//! until the person joins, and then only tried; an invite from a listing keeps a route once it
//! delivered records. A listing that came only from this device's own home node or devices may
//! be its person's own community, so it is never passed on to strangers. A browser or phone
//! passes on only what strangers' public nodes listed, and only to public nodes it dialed: its
//! own communities stay salted hashes (PROTOCOL.md §10).

use std::collections::{BTreeMap, BTreeSet};

use hoi_sync::{CatalogEntry, Msg};
use hoi_types::{Admission, Body, Id, Invite, Kind, Wire};
use serde::{Deserialize, Serialize};

use crate::Node;
use crate::dto::DiscoveredCommunity;
use crate::net::{Role, valid_peer_hint, valid_relay_hint};

/// The storage blob the directory lives in, apart from the rest of the state: it is a cache of
/// what others listed, written less often, and starts over if it can't be read.
pub(crate) const BLOB: &str = "directory";
/// Listings a browser or phone keeps; a native node keeps more, it passes them on.
const KEEP_LIGHT: usize = 512;
const KEEP_NATIVE: usize = 4096;
/// A listing no node confirmed holding for this long is forgotten...
const FORGET_AFTER: u64 = 21 * 86_400;
/// ...and one older than this is no longer passed on: its carriers may be gone.
const PASS_ON_FOR: u64 = 7 * 86_400;
/// Relayed listings go out to every peer this often, a window at a time...
pub(crate) const GOSSIP_EVERY: u64 = 300;
/// ...and to a browser or phone (a peer that doesn't say `keeps`) this often: it passes little
/// on, and a public node serving dozens of them would otherwise spend a gigabyte a day on
/// windows alone. It still gets a window with every hello.
pub(crate) const GOSSIP_LIGHT_EVERY: u64 = 1800;
/// Own communities in a catalog to a peer that takes relayed listings; the rest of the 64 is
/// the window. Own ones that don't fit travel in the window too.
const OWN_ROOM: usize = 48;
/// A listing first heard of this recently goes out ahead of the rotation, in up to this many
/// of a window's places; the rotation fills the rest, so it always moves on.
const FRESH_FOR: u64 = 3600;
const FRESH_ROOM: usize = 16;
/// Routes and relays kept per listing.
const ROUTES_KEPT: usize = 8;
const RELAYS_KEPT: usize = 4;
/// New listings one connection may add in an hour; more are ignored.
const NEW_PER_PEER_HOUR: u32 = 128;
/// The directory is written at most this often, or this soon after something new arrived.
const WRITE_EVERY: u64 = 600;
const WRITE_NEW_AFTER: u64 = 30;

/// One open community this device heard of.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Listing {
    /// The signed genesis, base64url, as checked when it arrived.
    pub g: String,
    /// Where nodes said it is carried, freshest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub p: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r: Vec<String>,
    /// When this device first heard of it, and when a node last listed it as its own (as far as
    /// this device knows: a relayed listing brings the age it was told).
    pub first: u64,
    pub held: u64,
    /// Heard from a node this device dialed, or from its own devices: outlives one that only a
    /// caller brought when the directory is full.
    #[serde(default)]
    pub dialed: bool,
    /// Heard from a stranger's node: a public listing, passed on. One only this device's own
    /// home node or devices listed is never passed on to strangers.
    #[serde(default)]
    pub public: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Directory {
    #[serde(default)]
    pub listings: BTreeMap<Id, Listing>,
    #[serde(skip)]
    dirty: bool,
    #[serde(skip)]
    new: bool,
    #[serde(skip)]
    written: u64,
}

impl Directory {
    /// The bytes to store now, if any: soon after something new arrived, else every ten minutes.
    pub(crate) fn take_save(&mut self, now: u64) -> Option<Vec<u8>> {
        let since = now.saturating_sub(self.written);
        if !self.dirty || since < if self.new { WRITE_NEW_AFTER } else { WRITE_EVERY } {
            return None;
        }
        let bytes = serde_json::to_vec(self).ok()?;
        (self.dirty, self.new, self.written) = (false, false, now);
        Some(bytes)
    }

    fn genesis(&self, id: &Id) -> Option<Vec<u8>> {
        hoi_types::uri::unb64(&self.listings.get(id)?.g).ok()
    }

    pub(crate) fn len(&self) -> usize {
        self.listings.len()
    }
}

/// Each once, in order, at most `max`.
fn merged(first: &[String], then: &[String], max: usize, valid: fn(&str) -> bool) -> Vec<String> {
    let mut seen = BTreeSet::new();
    first.iter().chain(then).filter(|r| valid(r) && seen.insert(r.as_str())).take(max).cloned().collect()
}

fn entry_bytes(e: &CatalogEntry) -> usize {
    (Msg::Catalog { entries: vec![e.clone()] }).encode().len()
}

impl Node {
    /// Open communities on connected nodes, this device's own, and those heard of before. Equal
    /// names remain separate IDs; callers must let the person choose which signed community to
    /// join.
    pub fn discover_communities(&self) -> Vec<DiscoveredCommunity> {
        let now = self.now();
        let mut live = BTreeMap::<Id, CatalogEntry>::new();
        for peer in self.peers.values().filter(|p| p.said_hello) {
            for (id, entry) in &peer.catalog {
                live.entry(*id).or_insert_with(|| entry.clone());
            }
        }
        let mut out = Vec::new();
        let mut push = |id: Id, genesis: &[u8], peers: Vec<String>, relays: Vec<String>, seen: u64| {
            let Ok((wire, _)) = Wire::decode(genesis) else { return };
            if wire.record.expires_at <= now {
                return;
            }
            let Body::Community(g) = wire.record.body else { return };
            let joined = self.state.joined.contains_key(&id);
            // Our known community state wins over what anyone listed, including a community
            // which has since changed its admission policy away from open.
            if self.communities.get(&id).is_some_and(|st| st.limits.admission != Admission::Open) || self.is_private_community(&id) {
                return;
            }
            // The genesis travels with the invite, so a link copied from here works on another
            // device too; `join` also finds it in the directory for an invite that has none.
            // the community's trackers, as its limits (or the genesis's) name them
            let mut trackers = self.communities.get(&id).map_or_else(|| g.limits.trackers.clone(), |st| st.limits.trackers.clone());
            trackers.truncate(hoi_types::uri::MAX_TRACKERS);
            let invite = Invite { community: id, peers, relays, genesis: Some(genesis.to_vec()), invite: None, trackers };
            out.push(DiscoveredCommunity {
                id: id.hex(),
                name: g.name,
                color: g.color,
                description: g.description,
                invite: invite.to_web(&self.web_base),
                joined,
                live: live.contains_key(&id),
                seen,
            });
        };
        let own = self.own_entries();
        let own_ids: BTreeSet<Id> = own.iter().filter_map(|e| Wire::decode(&e.genesis).ok()).map(|(w, _)| w.record.author).collect();
        for entry in own {
            let Ok((wire, _)) = Wire::decode(&entry.genesis) else { continue };
            let id = wire.record.author;
            let seen = self.directory.listings.get(&id).map_or(0, |l| l.held);
            push(id, &entry.genesis, entry.peers, entry.relays, seen);
        }
        for (id, entry) in &live {
            if own_ids.contains(id) || self.state.joined.contains_key(id) {
                continue;
            }
            match self.directory.listings.get(id) {
                // Kept here: the invite needs no genesis, and gets the routes heard elsewhere too.
                Some(l) => push(*id, &entry.genesis, merged(&entry.peers, &l.p, hoi_sync::MAX_HINT_PEERS, valid_peer_hint), entry.relays.clone(), l.held),
                None => push(*id, &entry.genesis, entry.peers.clone(), entry.relays.clone(), now),
            }
        }
        for (id, l) in &self.directory.listings {
            if own_ids.contains(id) || live.contains_key(id) || self.state.joined.contains_key(id) {
                continue;
            }
            let Ok(genesis) = hoi_types::uri::unb64(&l.g) else { continue };
            push(*id, &genesis, l.p.iter().take(hoi_sync::MAX_HINT_PEERS).cloned().collect(), l.r.clone(), l.held);
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// A remembered listing's signed genesis, for `join` and `preview_invite` when the invite
    /// came from the directory and carries none.
    pub(crate) fn directory_genesis(&self, id: &Id) -> Option<Vec<u8>> {
        self.directory.genesis(id)
    }

    /// Every open community this node holds, in a stable order, as catalog entries.
    fn own_entries(&self) -> Vec<CatalogEntry> {
        let mut entries = Vec::new();
        for id in self.state.joined.keys() {
            if self.is_private_community(id) {
                continue;
            }
            let Some(c) = self.communities.get(id) else { continue };
            let Some(s) = self.store.genesis_of(id) else { continue };
            let Body::Community(g) = &s.record.body else { continue };
            if c.limits.admission != Admission::Open || g.limits.admission != Admission::Open || s.record.expires_at <= self.now() {
                continue;
            }
            let (peers, relays) = self.community_hints(*id);
            // How many durable copies besides its own this node saw, quantised (PROTOCOL.md
            // §20): a receiver takes it as a hint for its own reading, never as a count to add
            // up. Read before the catalog is built (`refresh_warmth_hints`).
            let warmth = Some(self.warmth_hints.get(id).map(|(_, h)| *h).unwrap_or(0));
            entries.push(CatalogEntry { genesis: s.wire.clone(), peers, relays, warmth, relayed: false, age: None });
        }
        entries
    }

    /// What fits one catalog: at most `max` entries and the wire's byte limit, in order.
    fn fitting(entries: Vec<CatalogEntry>, max: usize, bytes: &mut usize) -> (Vec<CatalogEntry>, Vec<CatalogEntry>) {
        let mut fit = Vec::new();
        let mut rest = Vec::new();
        for e in entries {
            // Counting each entry with a full wrapper slightly overestimates its size, while
            // keeping catalog construction linear and below the wire's total byte limit.
            let size = entry_bytes(&e);
            if fit.len() < max && *bytes + size <= hoi_sync::MAX_CATALOG_BYTES {
                *bytes += size;
                fit.push(e);
            } else {
                rest.push(e);
            }
        }
        (fit, rest)
    }

    pub(crate) fn send_catalog(&mut self, peer: &str) {
        let Some(p) = self.peers.get(peer) else { return };
        if !p.said_hello || !p.features.contains(hoi_sync::FEATURE_CATALOG) {
            return;
        }
        self.refresh_warmth_hints();
        let own = self.own_entries();
        if self.peers.get(peer).is_some_and(|p| p.gossip_ids.is_empty()) {
            self.next_window(peer, &own);
        }
        self.send_catalog_entries(peer, &own);
    }

    /// Whether `peer` gets our own communities in a catalog, and whether it gets relayed
    /// listings. A catalog lists what we hold: strangers get it only while we serve them as a
    /// public node; a browser's, a phone's or a node's that isn't relaying goes to its own
    /// devices only. Relayed listings go to whoever advertised `directory` and gets our own,
    /// and from a device that isn't a public node to the public nodes it dialed itself.
    fn catalog_rights(&self, peer: &str) -> (bool, bool) {
        let Some(p) = self.peers.get(peer) else { return (false, false) };
        let stranger = p.role() == Role::Peer;
        let own = !stranger || self.accepts_members();
        let takes = p.features.contains(hoi_sync::FEATURE_DIRECTORY);
        let relay = takes && (own || (!p.meta.incoming && p.features.contains(hoi_sync::FEATURE_CARRY)));
        (own, relay)
    }

    /// The catalog `peer` gets now: our own snapshot, then its current window of relayed ones.
    fn catalog_for(&self, peer: &str, own: &[CatalogEntry]) -> Option<Vec<CatalogEntry>> {
        let p = self.peers.get(peer)?;
        let (own_ok, relay_ok) = self.catalog_rights(peer);
        if !own_ok && !relay_ok {
            return None;
        }
        let mut bytes = 32usize;
        let mine = if own_ok { own.to_vec() } else { Vec::new() };
        if !relay_ok {
            return Some(Self::fitting(mine, hoi_sync::MAX_CATALOG_ENTRIES, &mut bytes).0);
        }
        let (mut out, overflow) = Self::fitting(mine, OWN_ROOM, &mut bytes);
        let now = self.now();
        let in_snapshot: BTreeSet<Id> = out.iter().filter_map(|e| Wire::decode(&e.genesis).ok()).map(|(w, _)| w.record.author).collect();
        let overflow: BTreeMap<Id, CatalogEntry> =
            overflow.into_iter().filter_map(|e| Wire::decode(&e.genesis).ok().map(|(w, _)| (w.record.author, e))).collect();
        let mut window = Vec::new();
        for id in &p.gossip_ids {
            if in_snapshot.contains(id) {
                continue;
            }
            let entry = match (overflow.get(id), self.directory.listings.get(id)) {
                (Some(e), _) => CatalogEntry { relayed: true, age: Some(0), warmth: None, ..e.clone() },
                (None, Some(l)) => {
                    let Ok(genesis) = hoi_types::uri::unb64(&l.g) else { continue };
                    let age = now.saturating_sub(l.held);
                    if age > PASS_ON_FOR {
                        continue;
                    }
                    CatalogEntry { genesis, peers: l.p.clone(), relays: l.r.clone(), warmth: None, relayed: true, age: Some(age) }
                }
                (None, None) => continue,
            };
            window.push(entry);
        }
        let room = hoi_sync::MAX_CATALOG_ENTRIES - out.len();
        out.extend(Self::fitting(window, room, &mut bytes).0);
        Some(out)
    }

    /// Picks the next window of relayed listings for `peer`: own communities that didn't fit
    /// its snapshot, then listings first heard of within the hour, newest first, then the rest
    /// in turn from where the last window ended. Nothing it listed itself, nothing private to
    /// this device's own devices, nothing too old to pass on.
    fn next_window(&mut self, peer: &str, own: &[CatalogEntry]) {
        let (own_ok, relay_ok) = self.catalog_rights(peer);
        let Some(p) = self.peers.get(peer) else { return };
        if !relay_ok {
            return;
        }
        let now = self.now();
        let stranger = p.role() == Role::Peer;
        let theirs: BTreeSet<Id> = p.catalog.keys().copied().collect();
        let own_ids: Vec<Id> = own.iter().filter_map(|e| Wire::decode(&e.genesis).ok()).map(|(w, _)| w.record.author).collect();
        let snapshot: BTreeSet<Id> = if own_ok { own_ids.iter().take(OWN_ROOM).copied().collect() } else { BTreeSet::new() };
        let passable = |id: &Id, l: &Listing| {
            !snapshot.contains(id)
                && !theirs.contains(id)
                && (l.public || !stranger)
                && now.saturating_sub(l.held) <= PASS_ON_FOR
                && !self.is_refused(id)
                && !self.is_private_community(id)
        };
        let room = hoi_sync::MAX_CATALOG_ENTRIES - snapshot.len();
        let mut ids: Vec<Id> = Vec::new();
        if own_ok {
            ids.extend(own_ids.iter().skip(OWN_ROOM).filter(|id| !theirs.contains(id)).take(room));
        }
        let mut fresh: Vec<(&Id, &Listing)> =
            self.directory.listings.iter().filter(|(id, l)| now.saturating_sub(l.first) < FRESH_FOR && passable(id, l)).collect();
        fresh.sort_by_key(|(_, l)| std::cmp::Reverse(l.first));
        let fresh_room = (ids.len() + FRESH_ROOM).min(room);
        for (id, _) in fresh {
            if ids.len() >= fresh_room {
                break;
            }
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        let start = p.gossip_cursor;
        let after = self.directory.listings.iter().filter(|(id, _)| start.is_none_or(|s| **id > s));
        let before = self.directory.listings.iter().filter(|(id, _)| start.is_some_and(|s| **id <= s));
        let mut cursor = start;
        for (id, l) in after.chain(before) {
            if ids.len() >= room {
                break;
            }
            if passable(id, l) && !ids.contains(id) {
                ids.push(*id);
                cursor = Some(*id);
            }
        }
        let p = self.peers.get_mut(peer).expect("connected peer");
        p.gossip_ids = ids;
        p.gossip_cursor = cursor;
        p.gossip_at = now;
    }

    fn send_catalog_entries(&mut self, peer: &str, own: &[CatalogEntry]) {
        let Some(p) = self.peers.get(peer) else { return };
        if !p.said_hello || !p.features.contains(hoi_sync::FEATURE_CATALOG) {
            return;
        }
        let Some(entries) = self.catalog_for(peer, own) else { return };
        let public_node = self.accepts_members();
        let p = self.peers.get_mut(peer).expect("connected peer");
        if p.sent_catalog.as_deref() == Some(&entries[..]) {
            return;
        }
        // Someone who gets no catalog of ours never hears an empty one either.
        if entries.is_empty() && p.sent_catalog.is_none() && p.role() == Role::Peer && !public_node {
            return;
        }
        p.sent_catalog = Some(entries.clone());
        self.send_msg(peer, Msg::Catalog { entries });
    }

    pub(crate) fn broadcast_catalogs(&mut self) {
        if self.peers.is_empty() {
            return;
        }
        self.refresh_warmth_hints();
        let own = self.own_entries();
        let peers: Vec<_> = self.peers.keys().cloned().collect();
        for peer in peers {
            self.send_catalog_entries(&peer, &own);
        }
    }

    /// Every [`GOSSIP_EVERY`]: forgets listings nobody confirmed for too long, and gives every
    /// peer that takes relayed listings its next window. Called from `sweep`.
    pub(crate) fn gossip_directory(&mut self) {
        let now = self.now();
        if now.saturating_sub(self.gossiped_at) < GOSSIP_EVERY {
            return;
        }
        self.gossiped_at = now;
        let before = self.directory.listings.len();
        self.directory.listings.retain(|_, l| now.saturating_sub(l.held) < FORGET_AFTER);
        if before != self.directory.listings.len() {
            self.directory.dirty = true;
        }
        if self.peers.is_empty() {
            return;
        }
        self.refresh_warmth_hints();
        let own = self.own_entries();
        let peers: Vec<_> = self.peers.keys().cloned().collect();
        for peer in peers {
            let light = self.peers.get(&peer).is_some_and(|p| !p.features.contains(hoi_sync::FEATURE_KEEPS));
            let last = self.peers.get(&peer).map_or(0, |p| p.gossip_at);
            if !light || now.saturating_sub(last) >= GOSSIP_LIGHT_EVERY {
                self.next_window(&peer, &own);
            }
            self.send_catalog_entries(&peer, &own);
        }
    }

    pub(crate) fn receive_catalog(&mut self, peer: &str, entries: Vec<CatalogEntry>) -> bool {
        let now = self.now();
        let Some(p) = self.peers.get_mut(peer) else { return false };
        if !p.features.contains(hoi_sync::FEATURE_CATALOG) {
            return false;
        }
        if p.catalog_minute != now / 60 {
            p.catalog_minute = now / 60;
            p.catalog_msgs = 0;
        }
        p.catalog_msgs += 1;
        if p.catalog_msgs > 60 {
            self.send_msg(peer, Msg::Bye { reason: "too many community directory updates".into() });
            return false;
        }
        let mut verified = BTreeMap::new();
        let mut relayed = BTreeMap::new();
        for mut entry in entries {
            let Ok(d) = hoi_validate::decode(&entry.genesis) else { continue };
            let r = &d.wire.record;
            let Body::Community(g) = &r.body else { continue };
            if self.is_private_community(&r.author)
                || g.limits.admission != Admission::Open
                || self.store.genesis_of(&r.author).is_some_and(|known| known.id != d.id)
                || hoi_validate::stamp(&d.id, r.nonce, hoi_validate::stamp_floor(Kind::Community)).is_err()
                || hoi_validate::signature(&d).is_err()
                || hoi_validate::content(r).is_err()
                || hoi_validate::time(r, now, false, None).is_err()
            {
                continue;
            }
            // Routes remain passive metadata until the person explicitly accepts the invite.
            entry.peers.retain(|p| valid_peer_hint(p));
            entry.relays.retain(|r| valid_relay_hint(r));
            if entry.relayed {
                relayed.entry(r.author).or_insert(entry);
            } else {
                verified.entry(r.author).or_insert(entry);
            }
        }
        let p = self.peers.get_mut(peer).expect("connected peer");
        // A listing that only changed its warmth hint is the same listing: the hint is read
        // on demand, and must not let a peer make this node redraw its directory at will.
        let same = |a: &CatalogEntry, b: &CatalogEntry| a.genesis == b.genesis && a.peers == b.peers && a.relays == b.relays;
        let changed: Vec<Id> = p
            .catalog
            .keys()
            .chain(verified.keys())
            .filter(|id| match (p.catalog.get(id), verified.get(id)) {
                (Some(a), Some(b)) => !same(a, b),
                _ => true,
            })
            .copied()
            .collect::<BTreeSet<Id>>()
            .into_iter()
            .collect();
        let withdrawn: Vec<Id> = p.catalog.keys().filter(|id| !verified.contains_key(id)).copied().collect();
        let own: Vec<CatalogEntry> = verified.values().cloned().collect();
        p.catalog = verified;
        for id in changed {
            self.mark_changed(id);
        }
        self.withdraw_listings(peer, &withdrawn);
        let fresh = self.remember_listings(peer, own.iter().chain(relayed.values()));
        for id in fresh {
            self.mark_changed(id);
        }
        true
    }

    /// A node stopped listing these as its own: unless another connected node still lists one,
    /// it goes from the directory too (a changed admission policy withdraws it at once). Held
    /// elsewhere, it comes back with the next window from there.
    fn withdraw_listings(&mut self, peer: &str, ids: &[Id]) {
        for id in ids {
            let elsewhere = self.peers.iter().any(|(k, p)| k.as_str() != peer && p.said_hello && p.catalog.contains_key(id));
            if !elsewhere && self.directory.listings.remove(id).is_some() {
                self.directory.dirty = true;
                self.mark_changed(*id);
            }
        }
    }

    /// Verified listings from `peer` go into the directory: new ones within the connection's
    /// hourly allowance, known ones get its routes first and the youngest age anyone told.
    /// Returns the communities heard of for the first time.
    fn remember_listings<'a>(&mut self, peer: &str, entries: impl Iterator<Item = &'a CatalogEntry>) -> Vec<Id> {
        let now = self.now();
        let Some(p) = self.peers.get(peer) else { return Vec::new() };
        let own_device = p.client || p.home;
        let dialed = own_device || !p.meta.incoming;
        let hour = now / 3600;
        let mut allowance = if p.listed_new.0 == hour { NEW_PER_PEER_HOUR.saturating_sub(p.listed_new.1) } else { NEW_PER_PEER_HOUR };
        let mut added = Vec::new();
        for e in entries {
            let Ok((wire, _)) = Wire::decode(&e.genesis) else { continue };
            let id = wire.record.author;
            if self.is_refused(&id) || self.is_private_community(&id) {
                continue;
            }
            let held = if e.relayed { now.saturating_sub(e.age.unwrap_or(hoi_sync::MAX_LISTING_AGE)) } else { now };
            if now.saturating_sub(held) >= FORGET_AFTER {
                continue;
            }
            match self.directory.listings.get_mut(&id) {
                Some(l) => {
                    let routes = merged(&e.peers, &l.p, ROUTES_KEPT, valid_peer_hint);
                    let relays = merged(&e.relays, &l.r, RELAYS_KEPT, valid_relay_hint);
                    // Seen times change on every catalog; written with the next periodic write.
                    if routes != l.p || relays != l.r || (dialed && !l.dialed) || (!own_device && !l.public) {
                        self.directory.dirty = true;
                    }
                    if held > l.held + 3600 {
                        self.directory.dirty = true;
                    }
                    l.p = routes;
                    l.r = relays;
                    l.held = l.held.max(held);
                    l.dialed |= dialed;
                    l.public |= !own_device;
                }
                None => {
                    if allowance == 0 {
                        continue;
                    }
                    allowance -= 1;
                    self.directory.listings.insert(
                        id,
                        Listing {
                            g: hoi_types::uri::b64(&e.genesis),
                            p: merged(&e.peers, &[], ROUTES_KEPT, valid_peer_hint),
                            r: merged(&e.relays, &[], RELAYS_KEPT, valid_relay_hint),
                            first: now,
                            held,
                            dialed,
                            public: !own_device,
                        },
                    );
                    self.directory.dirty = true;
                    self.directory.new = true;
                    added.push(id);
                }
            }
        }
        if let Some(p) = self.peers.get_mut(peer) {
            p.listed_new = (hour, NEW_PER_PEER_HOUR - allowance);
        }
        self.trim_directory();
        added
    }

    /// Over the limit, what only callers brought goes first, then the longest unconfirmed.
    fn trim_directory(&mut self) {
        let keep = if self.can_relay() { KEEP_NATIVE } else { KEEP_LIGHT };
        let over = self.directory.listings.len().saturating_sub(keep);
        if over == 0 {
            return;
        }
        let mut order: Vec<(bool, u64, Id)> = self.directory.listings.iter().map(|(id, l)| (l.dialed, l.held, *id)).collect();
        order.sort();
        for (_, _, id) in order.into_iter().take(over) {
            self.directory.listings.remove(&id);
        }
        self.directory.dirty = true;
    }
}
