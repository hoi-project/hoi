//! Proof of control for a public name registry. Only the public proof is retained: the
//! community signing key is discarded after creation, as it was before name registration.

use hoi_types::Id;

use crate::api::{R, parse_id};
use crate::dto::{CommunityPublication, CommunityRegistration, NodeAnnouncement};
use crate::{ApiError, Node};

/// Public names use one deliberately small, case-insensitive namespace. Local display names
/// can still use Unicode and spaces without making a public registry claim.
pub fn community_registration_name(name: &str) -> Option<String> {
    let name = name.trim().to_ascii_lowercase();
    let valid = !name.is_empty()
        && name.len() <= 32
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c));
    valid.then_some(name)
}

/// The exact UTF-8 message signed for a name claim; `name` must be canonical first.
pub fn community_registration_statement(name: &str, community: &Id) -> String {
    format!("hoi community name v1\n{name}\n{}", community.hex())
}

/// Unlike a legacy name proof, this also binds the exact protocol and index edition.
pub fn community_publication_statement(name: &str, community: &Id, protocol: &str, index: u32) -> String {
    format!("hoi community index v2\n{name}\n{}\n{protocol}\n{index}", community.hex())
}

/// A build-time snapshot: runtime notice edits must never relabel signed communities.
pub(crate) fn publication_version() -> (String, u32) {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Version {
        protocol_version: String,
        index_version: u32,
    }
    let version: Version =
        serde_json::from_str(include_str!("../../../../release.json")).expect("the compiled release manifest must contain protocolVersion and indexVersion");
    (version.protocol_version, version.index_version)
}

impl Node {
    pub fn community_publication(&self, id: &str) -> R<CommunityPublication> {
        let community = parse_id(id)?;
        if self.is_private_community(&community) {
            return Err(ApiError::not_allowed("Private communities cannot be listed in the public directory."));
        }
        let c = self.community_of(&community)?;
        if c.limits.admission != hoi_types::Admission::Open {
            return Err(ApiError::not_allowed("Only open communities can be published in the public directory."));
        }
        self.state.publication_proofs.get(&community).cloned().ok_or_else(|| ApiError::not_allowed(
            "This device has no versioned publication proof. Publish from the device that created this community; older communities can still be shared by invite.",
        ))
    }

    pub fn community_registration(&self, id: &str) -> R<CommunityRegistration> {
        let community = parse_id(id)?;
        if self.is_private_community(&community) {
            return Err(ApiError::not_allowed("Private communities cannot reserve a public name."));
        }
        let c = self.community_of(&community)?;
        let name = community_registration_name(&c.genesis.name)
            .ok_or_else(|| ApiError::invalid("Public names need 1–32 letters, numbers, dots, underscores or hyphens, starting with a letter or number."))?;
        let signature = self.state.registration_proofs.get(&community).cloned().ok_or_else(|| {
            ApiError::not_allowed(
                "This device has no registration proof for that community. Register from the device that created it; older communities have no proof.",
            )
        })?;
        Ok(CommunityRegistration { name, community: community.hex(), signature })
    }
}

/// Name proofs of each kind a key file carries at most.
pub(crate) const MAX_BACKUP_PROOFS: usize = 64;

impl Node {
    /// The name proofs a key file carries: the community key that signed them was forgotten
    /// at creation, so without them a restored device could never claim or publish those names.
    /// Only for public communities this device holds, at most [`MAX_BACKUP_PROOFS`] of each.
    pub(crate) fn backup_proofs(&self) -> (Vec<CommunityRegistration>, Vec<CommunityPublication>) {
        let held = |c: &Id| self.state.joined.contains_key(c) && !self.is_private_community(c);
        let registrations = self
            .state
            .registration_proofs
            .iter()
            .filter(|(c, _)| held(c))
            .filter_map(|(c, signature)| {
                let name = community_registration_name(&self.communities.get(c)?.genesis.name)?;
                Some(CommunityRegistration { name, community: c.hex(), signature: signature.clone() })
            })
            .take(MAX_BACKUP_PROOFS)
            .collect();
        let publications = self.state.publication_proofs.iter().filter(|(c, _)| held(c)).map(|(_, p)| p.clone()).take(MAX_BACKUP_PROOFS).collect();
        (registrations, publications)
    }

    /// Takes name proofs back from a key file, after its communities were joined. Only proofs the
    /// community key signed for the name its genesis gives are kept; anything else is skipped.
    pub(crate) fn restore_proofs(&mut self, registrations: Option<&serde_json::Value>, publications: Option<&serde_json::Value>) {
        for r in backup_entries::<CommunityRegistration>(registrations) {
            if let Some(c) = self.proof_community(&r.community, &r.name, &r.signature, &community_registration_statement)
                && !self.state.registration_proofs.contains_key(&c)
            {
                self.state.registration_proofs.insert(c, r.signature);
                self.mark_state();
            }
        }
        for p in backup_entries::<CommunityPublication>(publications) {
            let statement = |name: &str, c: &Id| community_publication_statement(name, c, &p.protocol_version, p.index_version);
            if let Some(c) = self.proof_community(&p.community, &p.name, &p.signature, &statement)
                && !self.state.publication_proofs.contains_key(&c)
            {
                self.state.publication_proofs.insert(c, p);
                self.mark_state();
            }
        }
    }

    /// The community a name proof is for, when it holds: a public community joined here whose
    /// genesis name canonicalizes to `name`, and whose key signed `statement(name, community)`.
    fn proof_community(&self, community: &str, name: &str, signature: &str, statement: &dyn Fn(&str, &Id) -> String) -> Option<Id> {
        let c = Id::from_hex(community).ok()?;
        if !self.state.joined.contains_key(&c) || self.is_private_community(&c) {
            return None;
        }
        if community_registration_name(&self.communities.get(&c)?.genesis.name).as_deref() != Some(name) {
            return None;
        }
        let signature = hoi_types::Sig::from_slice(&hoi_types::hex_decode(signature).ok()?).ok()?;
        hoi_crypto::verify(&c, statement(name, &c).as_bytes(), &signature).then_some(c)
    }
}

/// The first [`MAX_BACKUP_PROOFS`] entries of a key file's list that read as `T`.
fn backup_entries<T: serde::de::DeserializeOwned>(list: Option<&serde_json::Value>) -> Vec<T> {
    list.and_then(|v| v.as_array()).into_iter().flatten().take(MAX_BACKUP_PROOFS).filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()
}

/// Open communities a node lists when announcing itself; the registry refuses longer lists.
pub(crate) const MAX_ANNOUNCED_COMMUNITIES: usize = 64;
/// Routes a node lists when announcing itself.
pub(crate) const MAX_ANNOUNCED_ROUTES: usize = 8;

/// The protocol family and index edition this build was compiled with, the same values the
/// publication proof binds. A carrier mirrors only communities published for these.
pub fn release_versions() -> (String, u32) {
    publication_version()
}

/// The checked-in release manifest this build was compiled with (`release.json`), as text. A
/// registry node serves it as its notice and compatibility information unless the operator
/// points it at a replacement file.
pub fn release_manifest() -> &'static str {
    include_str!("../../../../release.json")
}

/// The exact UTF-8 message a node's transport key signs to announce its public routes to one
/// registry (PROTOCOL.md §10): that registry's origin, so no registry can pass on what it was
/// sent to another one, then routes in the order others should try them and community keys
/// sorted, no trailing newline.
pub fn node_announcement_statement(registry: &str, endpoint: &Id, issued_at: u64, routes: &[String], communities: &[Id]) -> String {
    let communities: Vec<String> = communities.iter().map(|c| c.hex()).collect();
    format!("hoi public node v2\n{registry}\n{}\n{issued_at}\n{}\n{}", endpoint.hex(), routes.join(","), communities.join(","))
}

/// Whether `origin` names a registry the way an announcement must: the serialized origin of its
/// URL (`https://host[:port]`, or `http://` for this machine), lowercase, nothing after the
/// authority. Hosts derive it from the registry URL they were given; this check keeps anything
/// that could change the statement's shape (a newline, a comma, a path) out of it.
pub fn registry_origin_ok(origin: &str) -> bool {
    let (tls, authority) = match (origin.strip_prefix("https://"), origin.strip_prefix("http://")) {
        (Some(a), _) => (true, a),
        (None, Some(a)) => (false, a),
        _ => return false,
    };
    let host =
        authority.rsplit_once(':').filter(|(h, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && !h.ends_with(':')).map_or(authority, |x| x.0);
    !authority.is_empty()
        && authority.len() <= 260
        && authority.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.:[]".contains(&b))
        && (tls || matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
}

impl Node {
    /// Signs, for the registry named by `registry` (its origin, see [`registry_origin_ok`]),
    /// where this node can be reached and which open public communities it holds. `routes` are
    /// the host's public routes; native routes naming another endpoint are dropped, and the
    /// transport key is created if it does not exist yet.
    pub fn node_announcement(&mut self, registry: &str, routes: &[String]) -> R<NodeAnnouncement> {
        if !registry_origin_ok(registry) {
            return Err(ApiError::invalid("A node announces itself to a registry named by its https:// origin (http:// only on this machine)."));
        }
        let kp = hoi_crypto::Keypair::from_secret(&self.transport_secret()?);
        let endpoint = kp.public();
        let endpoint_hex = endpoint.hex();
        let mut seen = std::collections::BTreeSet::new();
        let routes: Vec<String> = routes
            .iter()
            .filter(|r| crate::net::valid_peer_hint(r) && (r.starts_with("rtc:") || r.split('@').next() == Some(endpoint_hex.as_str())))
            .filter(|r| seen.insert((*r).clone()))
            .take(MAX_ANNOUNCED_ROUTES)
            .cloned()
            .collect();
        if routes.is_empty() {
            return Err(ApiError::invalid("A node announces at least one public route of its own."));
        }
        let now = self.now();
        let mut carried: Vec<(u64, Id)> = self
            .state
            .joined
            .iter()
            .filter(|(id, _)| !self.is_private_community(id))
            .filter_map(|(id, joined)| {
                let c = self.communities.get(id)?;
                let s = self.store.genesis_of(id)?;
                let hoi_types::Body::Community(g) = &s.record.body else { return None };
                let open = c.limits.admission == hoi_types::Admission::Open && g.limits.admission == hoi_types::Admission::Open;
                (open && s.record.expires_at > now).then_some((joined.at, *id))
            })
            .collect();
        // The most recently joined communities are listed when there are more than fit.
        carried.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let mut communities: Vec<Id> = carried.into_iter().take(MAX_ANNOUNCED_COMMUNITIES).map(|(_, id)| id).collect();
        communities.sort();
        let statement = node_announcement_statement(registry, &endpoint, now, &routes, &communities);
        let signature = hoi_types::hex_encode(&kp.sign(statement.as_bytes()).0);
        Ok(NodeAnnouncement {
            registry: registry.to_string(),
            endpoint: endpoint_hex,
            routes,
            communities: communities.iter().map(|c| c.hex()).collect(),
            issued_at: now,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::registry_origin_ok;

    #[test]
    fn a_registry_is_named_by_its_origin() {
        for good in
            ["https://hoi.sh", "https://registry.example:8443", "https://[2001:db8::1]", "http://127.0.0.1:7845", "http://localhost:7845", "http://[::1]:7845"]
        {
            assert!(registry_origin_ok(good), "{good}");
        }
        for bad in [
            "https://hoi.sh/registry",
            "https://hoi.sh/",
            "https://HOI.sh",
            "http://hoi.sh",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example:80",
            "https://hoi.sh\nhttps://other.example",
            "https://a,b",
            "https://",
            "ftp://hoi.sh",
            "hoi.sh",
        ] {
            assert!(!registry_origin_ok(bad), "{bad}");
        }
    }
}
