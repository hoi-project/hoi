//! Verified DNS handles (M3). A person whose self-chosen name is a domain they control
//! (`mira.example.org`) proves it: `_hoi.<domain>` holds a TXT record `did=<their did:key>`, or
//! `https://<domain>/.well-known/hoi` answers with that did:key. Each device checks for itself;
//! a native host asks DNS (the system's own resolver, no fixed provider) and then HTTPS, a
//! browser HTTPS only, which needs CORS on that domain. A name that merely looks like a domain
//! gets no badge, and the app says it isn't verified. Nothing about the check is published.

use std::collections::BTreeMap;

use hoi_types::Id;
use serde::{Deserialize, Serialize};

use crate::dto::HandleView;
use crate::node::Node;

/// A handle is checked again after a day, a failed one after an hour.
const FRESH: u64 = 86_400;
const RETRY: u64 = 3600;
/// Checks asked for at most per hour, whatever names arrive.
const PER_HOUR: u32 = 60;
/// A check that never came back is given up after this.
const INFLIGHT: u64 = 60;

/// A check the host makes: every answer it got, TXT strings and the well-known file's lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HandleCheck {
    pub domain: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    pub domain: String,
    /// The people the domain named when it was checked.
    pub names: Vec<Id>,
    pub at: u64,
    pub failed: bool,
}

#[derive(Default)]
pub(crate) struct Handles {
    wanted: Vec<HandleCheck>,
    inflight: BTreeMap<String, u64>,
    hour: (u64, u32),
}

/// The name as a domain, lowercased, if it is one: two labels or more of letters, digits and
/// hyphens (punycode for others), a letter top-level label, at most 253 characters.
pub fn as_domain(name: &str) -> Option<String> {
    let d = name.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.len() > 253 || !d.contains('.') {
        return None;
    }
    let labels: Vec<&str> = d.split('.').collect();
    let ok = labels.iter().all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let tld = labels.last()?;
    (ok && labels.len() >= 2 && tld.len() >= 2 && (tld.bytes().all(|b| b.is_ascii_lowercase()) || tld.starts_with("xn--"))).then_some(d)
}

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn base58(bytes: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::new();
    for &b in bytes {
        let mut carry = b as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    std::iter::repeat_n('1', zeros).chain(digits.iter().rev().map(|d| BASE58[*d as usize] as char)).collect()
}

/// A person's key as a did:key (Ed25519, multicodec 0xed01, base58btc).
pub fn did_key(person: &Id) -> String {
    let mut b = vec![0xed, 0x01];
    b.extend_from_slice(&person.0);
    format!("did:key:z{}", base58(&b))
}

/// The people a domain's answers name: `did=did:key:z…` (DNS) or a line `did:key:z…` (HTTPS).
fn named(answers: &[String], candidates: &[Id]) -> Vec<Id> {
    candidates
        .iter()
        .filter(|p| {
            let did = did_key(p);
            answers.iter().any(|a| {
                let a = a.trim();
                a == did || a.strip_prefix("did=").is_some_and(|d| d.trim() == did)
            })
        })
        .copied()
        .collect()
}

impl Node {
    /// Whether this person's name is a domain that names their key, as far as this device knows.
    pub(crate) fn handle_of(&self, root: &Id) -> Option<HandleView> {
        let (name, _) = self.state.names.get(root)?;
        let domain = as_domain(name)?;
        let checked = self.state.handles.get(&domain);
        Some(HandleView { verified: checked.is_some_and(|c| !c.failed && c.names.contains(root)), checking: checked.is_none(), domain })
    }

    /// Asks for a check of the name a person just used, when it is a domain and not checked lately.
    pub(crate) fn want_handle_check(&mut self, name: &str) {
        let Some(domain) = as_domain(name) else { return };
        let now = self.now();
        if self.state.handles.get(&domain).is_some_and(|c| c.at + if c.failed { RETRY } else { FRESH } > now) {
            return;
        }
        let h = &mut self.handles;
        h.inflight.retain(|_, at| *at + INFLIGHT > now);
        if h.inflight.contains_key(&domain) {
            return;
        }
        if h.hour.0 != now / 3600 {
            h.hour = (now / 3600, 0);
        }
        if h.hour.1 >= PER_HOUR {
            return;
        }
        h.hour.1 += 1;
        h.inflight.insert(domain.clone(), now);
        h.wanted.push(HandleCheck { domain });
    }

    /// For the host: the domains to check now.
    pub fn take_handle_checks(&mut self) -> Vec<HandleCheck> {
        std::mem::take(&mut self.handles.wanted)
    }

    /// For the host: what the domain answered (TXT strings of `_hoi.<domain>`, lines of its
    /// well-known file), or nothing when neither answered.
    pub fn handle_checked(&mut self, domain: &str, answers: Vec<String>) {
        if self.handles.inflight.remove(domain).is_none() {
            return;
        }
        let candidates: Vec<Id> = self.state.names.iter().filter(|(_, (n, _))| as_domain(n).as_deref() == Some(domain)).map(|(k, _)| *k).collect();
        let names = named(&answers, &candidates);
        let failed = answers.is_empty();
        self.state.handles.insert(domain.to_string(), Checked { domain: domain.to_string(), names, at: self.now(), failed });
        // a device remembers a few hundred domains
        while self.state.handles.len() > 512 {
            let Some(oldest) = self.state.handles.iter().min_by_key(|(_, c)| c.at).map(|(d, _)| d.clone()) else { break };
            self.state.handles.remove(&oldest);
        }
        self.mark_state();
        self.changed_everywhere();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_names_that_look_like_one() {
        assert_eq!(as_domain("Mira.Example.org").as_deref(), Some("mira.example.org"));
        assert_eq!(as_domain("xn--bcher-kva.example").as_deref(), Some("xn--bcher-kva.example"));
        for not in ["mira", "mira.", "-a.example", "a..example", "a.b", "a.123", "a b.example", "mira@example.org", "über.example"] {
            assert_eq!(as_domain(not), None, "{not}");
        }
    }

    #[test]
    fn a_did_key_is_the_usual_one() {
        // the did:key spec's own Ed25519 example
        let key = Id::from_hex("2e6fcce36701dc791488e0d0b1745cc1e33a4c1c9fcc41c63bd343dbbe0970e6").unwrap();
        assert_eq!(did_key(&key), "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK");
        let other = Id([7; 32]);
        assert_eq!(named(&[format!("did={}", did_key(&key)), "v=spf1".into()], &[key, other]), vec![key]);
        assert_eq!(named(&[did_key(&other)], &[key, other]), vec![other]);
        assert!(named(&["did=did:key:zNope".into()], &[key]).is_empty());
    }
}
