//! Forging raw records the way a real attacker's tool would: no client in the way, no rules
//! obeyed on the sending side. A [`Forger`] owns a root identity and a blessed device and can
//! sign anything, well-formed or not. The attacks here are what "pentest the protocol" means —
//! each produces wire bytes that a correct node must refuse at a named stage of the pipeline
//! (TECHNICAL.md §6). Feed them to a node's `ingest` (the local oracle) and to a hub, then check
//! that the honest network never shows them.

use hoi_crypto::Keypair;
use hoi_types::{Body, Id, LedgerAction, LedgerBody, MediaRef, Record, Verdict, Wire};

/// How a forged record should fare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// A correct node must refuse it.
    Refused,
    /// A correct node accepts it but holds it or counts it late — it must never reach the feed.
    Contained,
}

/// One forged artifact: a label for the report, the wire bytes, and what should happen to it.
#[derive(Debug, Clone)]
pub struct Forged {
    pub label: String,
    pub wire: Vec<u8>,
    pub expect: Expect,
    pub note: String,
}

/// An attacker identity that signs raw records, honestly or not.
pub struct Forger {
    pub root: Keypair,
    pub device: Keypair,
    pub work: u64,
}

impl Default for Forger {
    fn default() -> Self {
        Self::new()
    }
}

impl Forger {
    /// A fresh identity with the cheapest acceptable work (4 bits), so a Sybil horde is quick to make.
    pub fn new() -> Forger {
        let root = Keypair::generate().expect("rng");
        let work = hoi_crypto::identity::mine(&root.public(), hoi_crypto::identity::MIN_BITS);
        Forger { root, device: Keypair::generate().expect("rng"), work }
    }

    pub fn root_key(&self) -> Id {
        self.root.public()
    }

    pub fn device_key(&self) -> Id {
        self.device.public()
    }

    /// The identity record (root-signed), stamped to the global floor.
    pub fn identity_wire(&self, now: u64) -> Vec<u8> {
        let mut r = Record {
            v: 1,
            community: None,
            author: self.root.public(),
            parent: None,
            created_at: now,
            expires_at: now + hoi_validate::KEEP,
            body: Body::Identity { work: self.work, rc: None, of: None },
            nonce: 0,
        };
        r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
        self.root.sign_record(r).0.encode()
    }

    /// The device delegation (device-signed, root-blessed), stamped to the global floor.
    pub fn delegation_wire(&self, now: u64) -> Vec<u8> {
        let exp = now + 365 * 86_400;
        let scope = vec!["*".to_string()];
        let root_sig = hoi_crypto::bless(&self.root, &self.device.public(), &scope, exp);
        let mut r = Record {
            v: 1,
            community: None,
            author: self.device.public(),
            parent: None,
            created_at: now,
            expires_at: exp,
            body: Body::Delegation { root: self.root.public(), scope, expires_at: exp, root_sig, seal: None },
            nonce: 0,
        };
        r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
        self.device.sign_record(r).0.encode()
    }

    /// The identity + delegation this device needs before anything it signs is accepted.
    pub fn credentials(&self, now: u64) -> Vec<Vec<u8>> {
        vec![self.identity_wire(now), self.delegation_wire(now)]
    }

    /// A device-authored record skeleton in `community`.
    pub fn record(&self, community: Id, parent: Option<Id>, body: Body, now: u64, ttl: u64) -> Record {
        Record { v: 1, community: Some(community), author: self.device.public(), parent, created_at: now, expires_at: now + ttl, body, nonce: 0 }
    }

    /// Stamps `r` to `bits` and signs it with the device key. Honest output.
    pub fn sign(&self, mut r: Record, bits: u32) -> (Vec<u8>, Id) {
        r.author = self.device.public();
        r.nonce = hoi_crypto::pow::mine(&r, bits);
        let (w, id) = self.device.sign_record(r);
        (w.encode(), id)
    }

    /// Stamps and signs, then replaces the signature with a wrong one — valid everything, bad sig.
    pub fn sign_badly(&self, mut r: Record, bits: u32) -> Vec<u8> {
        r.author = self.device.public();
        r.nonce = hoi_crypto::pow::mine(&r, bits);
        let id = r.id();
        let mut sig = self.device.sign(id.as_bytes());
        sig.0[0] ^= 0xff; // flip the signature
        Wire { record: r, sig }.encode()
    }
}

fn post_body(title: &str, text: &str) -> Body {
    Body::Post { title: title.into(), text: text.into(), link: None, name: None, media: Vec::new() }
}

/// The catalogue of protocol attacks. `bits` is the community's `pow_bits` (so stamped forgeries
/// clear the stamp and reach the stage they are meant to test). `now` is the receiver's clock.
pub fn catalogue() -> &'static [&'static str] {
    &["bad-sig", "future", "expired", "unstamped", "oversize", "noncanonical", "dup-flood", "no-identity", "wrong-root-delegation"]
}

/// Builds the forgeries for one attack. Most return a single artifact; `dup-flood` returns several.
pub fn forge(attack: &str, f: &Forger, community: Id, now: u64, bits: u32) -> Vec<Forged> {
    match attack {
        // A post whose signature doesn't match its id. Stage 3 (signature) must refuse it.
        "bad-sig" => {
            let r = f.record(community, None, post_body("free money", "click here, it's real"), now, 86_400);
            vec![Forged {
                label: "post with a forged signature".into(),
                wire: f.sign_badly(r, bits),
                expect: Expect::Refused,
                note: "signature doesn't verify over the id".into(),
            }]
        }
        // created_at an hour into the future. Stage 6 (time) must refuse it on live gossip.
        "future" => {
            let r = f.record(community, None, post_body("from tomorrow", "i am ahead of my time"), now + 3600, 86_400);
            let (wire, _) = f.sign(r, bits);
            vec![Forged { label: "post dated in the future".into(), wire, expect: Expect::Refused, note: "created_at > now + clock skew".into() }]
        }
        // Already expired. Stage 6 must refuse it.
        "expired" => {
            let mut r = f.record(community, None, post_body("stale", "should already be gone"), now - 7200, 0);
            r.expires_at = now - 3600;
            let (wire, _) = f.sign(r, bits);
            vec![Forged { label: "already-expired post".into(), wire, expect: Expect::Refused, note: "expires_at in the past".into() }]
        }
        // nonce 0: below the stamp floor. Stage 2/4 must refuse it (and cheaply).
        "unstamped" => {
            let r = f.record(community, None, post_body("no stamp", "i didn't pay the toll"), now, 86_400);
            let (w, _) = f.device.sign_record({
                let mut r = r;
                r.author = f.device.public();
                r
            });
            vec![Forged { label: "unstamped post (nonce 0)".into(), wire: w.encode(), expect: Expect::Refused, note: "no proof-of-work stamp".into() }]
        }
        // A post past the 64 KiB cap. Stage 1 (decode/size) must refuse it.
        "oversize" => {
            let big = "A".repeat(70_000);
            let r = f.record(community, None, post_body("too big", &big), now, 86_400);
            let (wire, _) = f.sign(r, bits);
            vec![Forged { label: "oversized post (>64 KiB)".into(), wire, expect: Expect::Refused, note: "exceeds the kind's size cap".into() }]
        }
        // Valid wire with a trailing byte: not canonical. Stage 1 (strict decode) must refuse it.
        "noncanonical" => {
            let r = f.record(community, None, post_body("garbled", "trailing junk"), now, 86_400);
            let (mut wire, _) = f.sign(r, bits);
            wire.push(0x00);
            vec![Forged {
                label: "non-canonical bytes (trailing junk)".into(),
                wire,
                expect: Expect::Refused,
                note: "strict decoder refuses trailing bytes".into(),
            }]
        }
        // The same text many times from one author. The first is fine; the rest hit the
        // duplicate fingerprint / rate windows (stage 9).
        "dup-flood" => {
            let mut out = Vec::new();
            for i in 0..6 {
                let r = f.record(community, None, post_body("buy now", "the exact same copy-pasted pitch, again and again"), now + i, 86_400);
                let (wire, _) = f.sign(r, bits);
                out.push(Forged {
                    label: format!("copy-paste flood #{}", i + 1),
                    wire,
                    expect: if i == 0 { Expect::Contained } else { Expect::Refused },
                    note: "same text fingerprint from one author within 7 days".into(),
                });
            }
            out
        }
        // A post from a device whose root never published an identity. Stage 5 must hold/refuse it.
        "no-identity" => {
            let r = f.record(community, None, post_body("who am i", "no identity work backs this key"), now, 86_400);
            let (wire, _) = f.sign(r, bits);
            vec![Forged {
                label: "post from a key with no identity record".into(),
                wire,
                expect: Expect::Refused,
                note: "author's root has no identity work on this node".into(),
            }]
        }
        // A delegation claiming a device key the attacker generated but that a different root
        // might later claim: a device belongs to its first root (§19). Here it simply tests that a
        // self-consistent delegation is required; the mismatch is the wrong-root case in battle.
        "wrong-root-delegation" => {
            let victim = Keypair::generate().expect("rng");
            let exp = now + 365 * 86_400;
            let scope = vec!["*".to_string()];
            // Attacker's root blesses the victim's device key without the victim's consent...
            let root_sig = hoi_crypto::bless(&f.root, &victim.public(), &scope, exp);
            let mut r = Record {
                v: 1,
                community: None,
                author: victim.public(),
                parent: None,
                created_at: now,
                expires_at: exp,
                body: Body::Delegation { root: f.root.public(), scope, expires_at: exp, root_sig, seal: None },
                nonce: 0,
            };
            r.nonce = hoi_crypto::pow::mine(&r, hoi_types::GLOBAL_POW_BITS);
            // ...but the attacker signs it with their own device, not the victim's: no consent.
            let bad = f.device.sign_record(r).0.encode();
            vec![Forged {
                label: "delegation without the device's consent".into(),
                wire: bad,
                expect: Expect::Refused,
                note: "record not signed by the device it delegates".into(),
            }]
        }
        other => vec![Forged {
            label: format!("unknown attack {other:?}"),
            wire: Vec::new(),
            expect: Expect::Refused,
            note: format!("attacks: {}", catalogue().join(", ")),
        }],
    }
}

/// A picture that is simply random-looking bytes claimed as a PNG — used to test media checks.
/// (Images are earned, so this is mostly exercised in the battle.)
pub fn fake_media_ref() -> MediaRef {
    MediaRef { hash: Id([7; 32]), mime: "image/png".into(), size: 1234, w: Some(16), h: Some(16), alt: None }
}

/// A vouch record from this forger for `person`, backdated by `age` seconds (a node that
/// had caught up counts it from arrival, not from this claimed time).
pub fn backdated_vouch(f: &Forger, community: Id, person: Id, seq: u64, now: u64, age: u64, bits: u32) -> Vec<u8> {
    let r = f.record(community, Some(person), Body::Vouch { value: 1, seq, reason: None }, now.saturating_sub(age), 365 * 86_400);
    let discount = 8; // vouch gets the 8-bit kind discount
    f.sign(r, bits.saturating_sub(discount)).0
}

/// A hostile `limits` proposal (e.g. raising the identity bar out of reach) — used by the
/// governance-capture strategy. Owner-signed; needs the community's consent to take effect (§6.1).
pub fn hostile_ledger(seq: u64, action: LedgerAction, target: Option<Id>, reason: &str) -> LedgerBody {
    LedgerBody { seq, action, target, reason: Some(reason.into()), evidence: None, cosign: None, text: None, limits: None, drop_key: None, patch: None }
}

impl Forger {
    /// A review record (hide/veto/ok) from this forger — used by brigading strategies.
    #[allow(clippy::too_many_arguments)] // a forging helper; every field is a distinct wire value
    pub fn review(&self, community: Id, target: Id, verdict: Verdict, seq: u64, reason: Option<&str>, now: u64, bits: u32) -> Vec<u8> {
        let r = self.record(community, Some(target), Body::Review { verdict, seq, reason: reason.map(String::from) }, now, 86_400);
        self.sign(r, bits.saturating_sub(8)).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalogued_attack_builds_something_refusable() {
        let f = Forger::new();
        let community = Id([5; 32]);
        for attack in catalogue() {
            let items = forge(attack, &f, community, 1_790_000_000, 8);
            assert!(!items.is_empty(), "{attack} produced nothing");
            assert!(items.iter().any(|i| !i.wire.is_empty()), "{attack} produced no wire");
            // At least one artifact of each attack is meant to be refused outright.
            assert!(items.iter().any(|i| i.expect == Expect::Refused), "{attack} has no refusable artifact");
        }
    }
}
