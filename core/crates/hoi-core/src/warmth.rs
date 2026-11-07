//! Warmth, read from what this node saw on its own links (PROTOCOL.md §20): which connected
//! peers hold a community and how much of its live window, what others listed that is not
//! here yet, when the last catch-up finished, and the one-hop hint a node may attach to its
//! catalog. The arithmetic is `hoi_sync::warmth`; this file gathers the observation.
//!
//! What a peer holds is counted from what it showed, as far as this node can tell. A peer this
//! node dialed, by a route it already trusted, is believed about what it lists; a peer that
//! dialed in counts only the records it delivered and the ones it was given, so a stranger's
//! inventory alone warms nothing. Whether a copy outlives the connection is the peer's own
//! word (`keeps` in its hello): a browser never says it, a desktop or hoi-node always does.
//!
//! Where a copy is counts as much as whether it exists: holders in one network (the address's
//! IPv4 /24 or IPv6 /48, or this machine) or under one operator (as their notices name them)
//! are one place, and a copy reached only over a link that is unencrypted or whose other end
//! was never checked reads colder. The same keys read the device's own connections.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};

use hoi_sync::warmth::{Holder, Link, Observation, assess, assess_links, hint};
use hoi_types::{Body, Id, Kind};

use crate::api::{R, parse_id};
use crate::dto::{CommunityWarmth, DirectoryWarmth, LinkWarmthView, WarmthView};
use crate::net::{PeerMeta, Role};
use crate::{ApiError, Node};

/// A short key for a text: the first eight bytes of its BLAKE3.
fn key_of(text: &str) -> u64 {
    let id = hoi_types::notice::hash_of(text.as_bytes());
    u64::from_le_bytes(id.0[..8].try_into().expect("eight bytes"))
}

/// Who runs a node, as its notice names them (operator and contact, case and spacing ignored):
/// `None` when the notice names nobody. Nodes with the same key are one place.
pub(crate) fn operator_key(notice: &[u8]) -> Option<u64> {
    let n = hoi_types::NodeNotice::from_canonical(notice).ok()?;
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let (operator, contact) = (norm(&n.operator), norm(&n.contact));
    if operator.is_empty() && contact.is_empty() {
        return None;
    }
    Some(key_of(&format!("operator\n{operator}\n{contact}")))
}

/// The network a connection comes from, as a key: the address's IPv4 /24 or IPv6 /48, or this
/// machine for a loopback link. Nodes in one network are one place: one party or one outage.
pub(crate) fn network_key(m: &PeerMeta) -> Option<u64> {
    if m.local {
        return Some(key_of("network\nthis machine"));
    }
    let addr = m.addr.clone().or_else(|| m.dialed.iter().find_map(|r| r.rsplit_once('@').map(|(_, a)| a.to_string())))?;
    let ip = addr.parse::<SocketAddr>().map(|a| a.ip()).or_else(|_| addr.parse::<IpAddr>()).ok()?;
    let network = match ip {
        IpAddr::V4(v4) if v4.is_loopback() => "this machine".to_string(),
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("v4 {}.{}.{}", o[0], o[1], o[2])
        }
        IpAddr::V6(v6) if v6.is_loopback() => "this machine".to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("v6 {:x}:{:x}:{:x}", s[0], s[1], s[2])
        }
    };
    Some(key_of(&format!("network\n{network}")))
}

/// The wider block a connection comes from, as a key: the address's IPv4 /16 or IPv6 /32. Not a
/// place (one block holds many independent networks), but placement keeps at most a few pieces
/// of a generation in one (SCATTER.md §5.7). `None` for this machine.
pub(crate) fn block_key(m: &PeerMeta) -> Option<u64> {
    if m.local {
        return None;
    }
    let addr = m.addr.clone().or_else(|| m.dialed.iter().find_map(|r| r.rsplit_once('@').map(|(_, a)| a.to_string())))?;
    let ip = addr.parse::<SocketAddr>().map(|a| a.ip()).or_else(|_| addr.parse::<IpAddr>()).ok()?;
    let block = match ip {
        IpAddr::V4(v4) if v4.is_loopback() => return None,
        IpAddr::V4(v4) => format!("v4 {}.{}", v4.octets()[0], v4.octets()[1]),
        IpAddr::V6(v6) if v6.is_loopback() => return None,
        IpAddr::V6(v6) => format!("v6 {:x}:{:x}", v6.segments()[0], v6.segments()[1]),
    };
    Some(key_of(&format!("block\n{block}")))
}

/// The link is encrypted and its other end was checked: this device dialed a route it chose
/// (the certificate or key it pinned), or the transport authenticated the peer's endpoint.
pub(crate) fn guarded(m: &PeerMeta) -> bool {
    let encrypted = !matches!(crate::net::encryption_words(m).as_str(), "not encrypted" | "unknown");
    encrypted && (m.local || !m.incoming || m.endpoint.is_some())
}

/// Learned member routes younger than this count as remembered holders (PROTOCOL.md §10).
const REMEMBERED_FOR: u64 = 30 * 86_400;
/// A catalog hint is read again at most this often.
const HINT_FOR: u64 = 60;

impl Node {
    /// Everything first hand about one community, for `hoi_sync::warmth::assess`.
    pub(crate) fn observe(&self, c: &Id) -> Observation {
        let now = self.now();
        let records = self.store.in_community(c).filter(|s| s.record.expires_at > now && self.forwardable(s)).count() as u64;
        // One holder per authenticated endpoint: a node reached twice is still one copy.
        let mut holders: BTreeMap<String, Holder> = BTreeMap::new();
        let mut missing = 0u64;
        let mut live = false;
        let mut hinted = 0u8;
        for (key, p) in self.peers.iter().filter(|(_, p)| p.said_hello) {
            let durable = p.features.contains(hoi_sync::FEATURE_KEEPS);
            if durable && let Some(entry) = p.catalog.get(c) {
                hinted = hinted.max(entry.warmth.unwrap_or(0));
            }
            if p.communities.contains(c) {
                live = true;
            }
            let Some(offer) = p.offers.get(c).copied() else { continue };
            // What this peer listed that is still not here: the pages not asked for yet and
            // the page in flight, less anything that arrived from someone else meanwhile. A
            // catch-up that stalled says nothing about the community, only about the link.
            let still_missing = if self.sync_stalled(key) {
                0
            } else {
                p.pending_wants
                    .get(c)
                    .into_iter()
                    .flatten()
                    .chain(p.wants.get(c).into_iter().flat_map(|(_, ids)| ids.iter()))
                    .filter(|id| !self.store.contains(id))
                    .count() as u64
            };
            missing = missing.max(still_missing);
            let claimed = if p.meta.incoming { 0 } else { offer.listed.saturating_sub(still_missing) };
            let held = u64::from(offer.genesis) + claimed + offer.given + offer.delivered;
            let coverage = if records == 0 { if held > 0 { 1.0 } else { 0.0 } } else { (held as f64 / records as f64).min(1.0) };
            let slot = p.meta.endpoint.clone().unwrap_or_else(|| format!("link:{key}"));
            let h = holders.entry(slot).or_insert(Holder { coverage: 0.0, durable: false, operator: None, network: None, guarded: false });
            h.coverage = h.coverage.max(coverage);
            h.durable |= durable;
            h.operator = h.operator.or(p.operator);
            h.network = h.network.or_else(|| network_key(&p.meta));
            h.guarded |= guarded(&p.meta);
        }
        let mut media_wanted = 0u64;
        let mut media_present = 0u64;
        let mut seen = BTreeSet::new();
        for s in self.store.of_kind(c, Kind::Post) {
            let Body::Post { media, .. } = &s.record.body else { continue };
            if s.record.expires_at <= now || !self.forwardable(s) {
                continue;
            }
            for m in media {
                if seen.insert(m.hash) {
                    media_wanted += 1;
                    if self.store.has_blob(&m.hash) {
                        media_present += 1;
                    }
                }
            }
        }
        Observation {
            own_durable: self.can_relay(),
            records,
            missing,
            media_wanted,
            media_present,
            holders: holders.into_values().collect(),
            last_sync: self.state.sync_completed.get(c).copied(),
            now,
            live,
            hinted,
        }
    }

    /// The number a node attaches to its catalog entry for `c`: the durable copies it saw
    /// besides its own, quantised, read at most once a minute so a busy node's catalog stays
    /// cheap to send.
    pub(crate) fn warmth_hint(&mut self, c: &Id) -> u8 {
        let now = self.now();
        if let Some((at, h)) = self.warmth_hints.get(c)
            && now.saturating_sub(*at) < HINT_FOR
        {
            return *h;
        }
        let h = hint(&assess(&self.observe(c)));
        self.warmth_hints.insert(*c, (now, h));
        h
    }

    /// Reads the hints for every community a catalog may list, so building one needs no
    /// arithmetic. Called before a catalog goes out.
    pub(crate) fn refresh_warmth_hints(&mut self) {
        let ids: Vec<Id> = self.state.joined.keys().filter(|c| !self.is_private_community(c)).copied().collect();
        self.warmth_hints.retain(|c, _| ids.contains(c));
        for c in ids {
            self.warmth_hint(&c);
        }
    }

    fn community_warmth_of(&self, c: &Id) -> CommunityWarmth {
        let o = self.observe(c);
        let w = assess(&o);
        let now = self.now();
        let remembered = self
            .state
            .learned
            .get(c)
            .map(|l| {
                l.iter()
                    .filter(|r| now.saturating_sub(r.seen) < REMEMBERED_FOR)
                    .map(|r| r.hint.split('@').next().unwrap_or("").to_string())
                    .collect::<BTreeSet<_>>()
                    .len()
            })
            .unwrap_or(0) as u32;
        let name = self
            .communities
            .get(c)
            .map(|st| st.genesis.name.clone())
            .or_else(|| self.state.public_joins.get(c).and_then(|p| p.name.clone()))
            .unwrap_or_default();
        CommunityWarmth {
            community: c.hex(),
            name,
            level: w.level.as_str().into(),
            score: w.score,
            kept: w.kept,
            places: w.places,
            spread: w.spread,
            guarded: w.guarded,
            copies: w.copies,
            durable_copies: w.durable_copies,
            network_copies: w.network_copies,
            complete: w.complete,
            media: w.media,
            fresh: w.fresh,
            live: o.live,
            records: o.records,
            missing: o.missing,
            holders_durable: w.holders_durable,
            holders_transient: w.holders_transient,
            remembered,
            own_durable: o.own_durable,
            last_sync: o.last_sync,
            carried: self.state.carried.get(c).map(|x| x.origin.clone()),
            pending: self.state.public_joins.contains_key(c) && !self.communities.contains_key(c),
        }
    }

    pub fn community_warmth(&self, id: &str) -> R<CommunityWarmth> {
        let c = parse_id(id)?;
        if !self.state.joined.contains_key(&c) {
            return Err(ApiError::not_found("community you're in"));
        }
        if self.is_private_community(&c) {
            return Err(ApiError::not_allowed("Private communities sync through their encrypted relays; Warmth reads public links only."));
        }
        Ok(self.community_warmth_of(&c))
    }

    /// Every public community this device is in, coldest first, with the directory's own
    /// reading: how many connected nodes share a catalog, how many open communities they list,
    /// how many take carry requests.
    pub fn warmth(&self) -> WarmthView {
        let mut communities: Vec<CommunityWarmth> =
            self.state.joined.keys().filter(|c| !self.is_private_community(c)).map(|c| self.community_warmth_of(c)).collect();
        communities
            .sort_by(|a, b| a.score.cmp(&b.score).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.community.cmp(&b.community)));
        let mut listed: BTreeSet<Id> = BTreeSet::new();
        let mut catalog_nodes = 0u32;
        for p in self.peers.values().filter(|p| p.said_hello) {
            if !p.catalog.is_empty() {
                catalog_nodes += 1;
            }
            listed.extend(p.catalog.keys().copied());
        }
        WarmthView {
            communities,
            directory: DirectoryWarmth { catalog_nodes, discoverable: listed.len() as u32, carry_nodes: self.carry_nodes() },
            own_durable: self.can_relay(),
            links: self.link_warmth(),
        }
    }

    /// The warmth of this device's own connections: every link that said hello to or from a
    /// peer or its home node (devices it serves are its own, not a way into the network), by
    /// independent place and by whether it is guarded.
    pub fn link_warmth(&self) -> LinkWarmthView {
        let links: Vec<Link> = self
            .peers
            .values()
            .filter(|p| p.said_hello && p.role() != Role::Client)
            .map(|p| Link { operator: p.operator, network: network_key(&p.meta), guarded: guarded(&p.meta) })
            .collect();
        let w = assess_links(&links);
        LinkWarmthView {
            level: w.level.as_str().into(),
            score: w.score,
            links: w.links,
            places: w.places,
            effective: w.effective,
            largest: w.largest,
            guarded: w.guarded,
        }
    }
}
