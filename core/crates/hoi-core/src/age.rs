//! Anonymous age credentials (M5). A community whose admission is `age-proof` lets a
//! newcomer in once a moderator saw a proof that they are over 18 and nothing else: an SD-JWT VC
//! presentation (from an identity wallet) that discloses only `age_over_18`, signed by an issuer
//! the community trusts (`limits.age_issuers`), bound by its key-binding JWT to this person's key
//! and this community (its `aud` and `nonce`), made in the last 15 minutes. The newcomer's device
//! checks it and makes a code; they send the code to a moderator however they like; the
//! moderator's device checks it again and signs an `admit` entry. The presentation is never
//! published, and the entry says only "age proof".

use hoi_types::{Id, LedgerAction};
use p256::ecdsa::signature::Verifier;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::api::{R, parse_id};
use crate::dto::LedgerEntryView;
use crate::error::ApiError;
use crate::node::Node;

const CODE: &str = "hoi-age-1-";
/// A key-binding JWT is good for this long after it was made.
const FRESH: u64 = 15 * 60;
const SKEW: u64 = 60;
/// A presentation at most.
const MAX_PRESENTATION: usize = 16 * 1024;
/// The claim, and the one disclosure a presentation may carry.
const CLAIM: &str = "age_over_18";

/// What a wallet needs to bind its presentation to this person and community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AgeChallenge {
    pub aud: String,
    pub nonce: String,
    /// The issuers the community trusts, hex.
    pub issuers: Vec<String>,
}

pub fn audience(community: &Id) -> String {
    format!("hoi:{}", community.hex())
}

pub fn nonce(person: &Id, community: &Id) -> String {
    let mut k = blake3::Hasher::new_derive_key("hoi age proof nonce v1");
    k.update(&person.0);
    k.update(&community.0);
    hoi_types::uri::b64(&k.finalize().as_bytes()[..16])
}

fn part(s: &str) -> Result<Json, String> {
    let bytes = hoi_types::uri::unb64(s).map_err(|_| "not base64url".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "not JSON".to_string())
}

fn b64_hash(data: &[u8]) -> String {
    hoi_types::uri::b64(&Sha256::digest(data))
}

/// Checks a JWS (`header.payload.sig`) signed with `alg` by a raw key: ES256 with a P-256 SEC1
/// key, EdDSA with an Ed25519 key.
fn signed_by(jwt: &str, key: &[u8]) -> Result<(Json, Json), String> {
    let mut it = jwt.split('.');
    let (Some(h), Some(p), Some(s), None) = (it.next(), it.next(), it.next(), it.next()) else { return Err("not a JWT".into()) };
    let header = part(h)?;
    let payload = part(p)?;
    let sig = hoi_types::uri::unb64(s).map_err(|_| "a signature that isn't base64url".to_string())?;
    let input = format!("{h}.{p}");
    let ok = match (header.get("alg").and_then(Json::as_str), key.len()) {
        (Some("ES256"), 33 | 65) => {
            let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(key).map_err(|_| "a bad P-256 key".to_string())?;
            let sig = p256::ecdsa::Signature::from_slice(&sig).map_err(|_| "a bad ES256 signature".to_string())?;
            vk.verify(input.as_bytes(), &sig).is_ok()
        }
        (Some("EdDSA") | Some("Ed25519"), 32) => {
            let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) else { return Err("a bad EdDSA signature".into()) };
            hoi_crypto::verify(&Id(key.try_into().expect("32 bytes")), input.as_bytes(), &hoi_types::Sig(sig))
        }
        _ => false,
    };
    if !ok {
        return Err("the signature doesn't check".into());
    }
    Ok((header, payload))
}

/// The raw key a `cnf.jwk` names: P-256 as SEC1 uncompressed, Ed25519 as its 32 bytes.
fn jwk_key(jwk: &Json) -> Result<Vec<u8>, String> {
    let get = |k: &str| jwk.get(k).and_then(Json::as_str).ok_or_else(|| format!("a holder key without {k}"));
    match (get("kty")?, get("crv")?) {
        ("EC", "P-256") => {
            let x = hoi_types::uri::unb64(get("x")?).map_err(|_| "a bad holder key".to_string())?;
            let y = hoi_types::uri::unb64(get("y")?).map_err(|_| "a bad holder key".to_string())?;
            if x.len() != 32 || y.len() != 32 {
                return Err("a bad holder key".into());
            }
            Ok([&[4u8][..], &x, &y].concat())
        }
        ("OKP", "Ed25519") => {
            let x = hoi_types::uri::unb64(get("x")?).map_err(|_| "a bad holder key".to_string())?;
            if x.len() != 32 {
                return Err("a bad holder key".into());
            }
            Ok(x)
        }
        _ => Err("a holder key of a kind not taken here".into()),
    }
}

/// Checks a presentation for `person` in `community`: signed by one of `issuers`, not expired,
/// disclosing `age_over_18: true` and nothing else, its key binding made by the credential's
/// holder for this person and community within the last 15 minutes.
pub fn check(presentation: &str, person: &Id, community: &Id, issuers: &[Vec<u8>], now: u64) -> Result<(), String> {
    if presentation.len() > MAX_PRESENTATION {
        return Err("the presentation is too large".into());
    }
    let parts: Vec<&str> = presentation.trim().split('~').collect();
    let [jwt, disclosures @ .., kb] = parts.as_slice() else { return Err("not an SD-JWT presentation".into()) };
    if kb.is_empty() {
        return Err("the presentation has no key binding: it could be anyone's".into());
    }
    // the issuer: one the community trusts
    let (_, payload) = issuers.iter().find_map(|k| signed_by(jwt, k).ok()).ok_or("not signed by an issuer this community trusts")?;
    if payload.get("exp").and_then(Json::as_u64).is_some_and(|e| e <= now) {
        return Err("the credential has expired".into());
    }
    if payload.get("_sd_alg").and_then(Json::as_str).is_some_and(|a| a != "sha-256") {
        return Err("a digest other than SHA-256".into());
    }
    // the disclosures: only age_over_18, and it says true
    let digests: Vec<&str> = payload.get("_sd").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).collect()).unwrap_or_default();
    let mut over_18 = payload.get(CLAIM) == Some(&Json::Bool(true));
    for d in disclosures.iter().filter(|d| !d.is_empty()) {
        if !digests.contains(&b64_hash(d.as_bytes()).as_str()) {
            return Err("a disclosure the issuer didn't sign".into());
        }
        let Json::Array(items) = part(d)? else { return Err("a disclosure that isn't one".into()) };
        match items.as_slice() {
            [_, Json::String(name), value] if name == CLAIM => over_18 |= *value == Json::Bool(true),
            _ => return Err("the presentation shows more than the age: only age_over_18 may be disclosed".into()),
        }
    }
    if !over_18 {
        return Err("the credential doesn't say over 18".into());
    }
    // the key binding: the credential's own holder, for this person and community, just now
    let holder = jwk_key(payload.get("cnf").and_then(|c| c.get("jwk")).ok_or("a credential without a holder key")?)?;
    let (kh, kp) = signed_by(kb, &holder)?;
    if kh.get("typ").and_then(Json::as_str) != Some("kb+jwt") {
        return Err("the key binding isn't one".into());
    }
    if kp.get("aud").and_then(Json::as_str) != Some(audience(community).as_str())
        || kp.get("nonce").and_then(Json::as_str) != Some(nonce(person, community).as_str())
    {
        return Err("the proof was made for someone else, or another community".into());
    }
    let iat = kp.get("iat").and_then(Json::as_u64).ok_or("a key binding without a time")?;
    if iat + FRESH < now || iat > now + SKEW {
        return Err("the proof is too old; make a new one".into());
    }
    let presented = &presentation.trim()[..presentation.trim().len() - kb.len()];
    if kp.get("sd_hash").and_then(Json::as_str) != Some(b64_hash(presented.as_bytes()).as_str()) {
        return Err("the key binding is for another presentation".into());
    }
    Ok(())
}

impl Node {
    /// What a wallet needs to make a proof for this person and community.
    pub fn age_challenge(&self, community: &str) -> R<AgeChallenge> {
        let me = self.need_me()?;
        let c = parse_id(community)?;
        let st = self.community_of(&c)?;
        Ok(AgeChallenge { aud: audience(&c), nonce: nonce(&me.root, &c), issuers: st.limits.age_issuers.iter().map(|k| hoi_types::hex_encode(k)).collect() })
    }

    /// Checks a wallet's presentation here and makes the code to send to a moderator.
    pub fn age_proof_code(&self, community: &str, presentation: &str) -> R<String> {
        let me = self.need_me()?;
        let c = parse_id(community)?;
        let st = self.community_of(&c)?;
        check(presentation, &me.root, &c, &st.limits.age_issuers, self.now()).map_err(|e| ApiError::invalid(format!("That proof won't do: {e}.")))?;
        let mut b = Vec::with_capacity(64 + presentation.len());
        b.extend_from_slice(&me.root.0);
        b.extend_from_slice(&c.0);
        b.extend_from_slice(presentation.trim().as_bytes());
        Ok(format!("{CODE}{}", hoi_types::uri::b64(&b)))
    }

    /// On a moderator's device: checks a newcomer's code and lets them in with an `admit` entry.
    pub fn admit_by_age(&mut self, code: &str) -> R<LedgerEntryView> {
        let raw = code.trim().strip_prefix(CODE).ok_or_else(|| ApiError::invalid("That isn't an age proof code."))?;
        let b = hoi_types::uri::unb64(raw).map_err(|_| ApiError::invalid("That code is damaged."))?;
        if b.len() < 65 {
            return Err(ApiError::invalid("That code is damaged."));
        }
        let person = Id(b[..32].try_into().expect("32"));
        let c = Id(b[32..64].try_into().expect("32"));
        let presentation = std::str::from_utf8(&b[64..]).map_err(|_| ApiError::invalid("That code is damaged."))?;
        let st = self.community_of(&c)?;
        if st.limits.admission != hoi_types::Admission::AgeProof {
            return Err(ApiError::not_allowed("This community doesn't ask for age proofs."));
        }
        if st.is_admitted(&person) {
            return Err(ApiError::invalid("They're in already."));
        }
        check(presentation, &person, &c, &st.limits.age_issuers, self.now()).map_err(|e| ApiError::invalid(format!("That proof won't do: {e}.")))?;
        self.mod_action(&c, LedgerAction::Admit, person, Some("age proof".into()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer;

    pub(crate) fn key(n: u8) -> SigningKey {
        SigningKey::from_slice(&[n; 32]).unwrap()
    }

    pub(crate) fn sec1(k: &SigningKey) -> Vec<u8> {
        k.verifying_key().to_sec1_bytes().to_vec()
    }

    fn jws(k: &SigningKey, header: Json, payload: Json) -> String {
        let h = hoi_types::uri::b64(header.to_string().as_bytes());
        let p = hoi_types::uri::b64(payload.to_string().as_bytes());
        let sig: p256::ecdsa::Signature = k.sign(format!("{h}.{p}").as_bytes());
        format!("{h}.{p}.{}", hoi_types::uri::b64(&sig.to_bytes()))
    }

    fn disclosure(name: &str, value: Json) -> String {
        salted("salt-salt-salt", name, value)
    }

    fn salted(salt: &str, name: &str, value: Json) -> String {
        hoi_types::uri::b64(serde_json::json!([salt, name, value]).to_string().as_bytes())
    }

    /// A presentation as a wallet makes it: `extra` disclosures besides `age_over_18: over`.
    pub(crate) fn presentation(issuer: &SigningKey, holder: &SigningKey, person: &Id, community: &Id, iat: u64, over: bool, extra: &[(&str, Json)]) -> String {
        let point = holder.verifying_key().to_sec1_point(false);
        let raw = point.as_bytes();
        let jwk = serde_json::json!({ "kty": "EC", "crv": "P-256", "x": hoi_types::uri::b64(&raw[1..33]), "y": hoi_types::uri::b64(&raw[33..65]) });
        let mut ds = vec![disclosure(CLAIM, Json::Bool(over))];
        ds.extend(extra.iter().map(|(n, v)| disclosure(n, v.clone())));
        let sd: Vec<String> = ds.iter().map(|d| b64_hash(d.as_bytes())).collect();
        let jwt = jws(
            issuer,
            serde_json::json!({ "alg": "ES256", "typ": "dc+sd-jwt" }),
            serde_json::json!({ "iss": "https://issuer.example", "vct": "urn:eudi:pid:1", "iat": iat - 3600, "exp": iat + 86_400 * 365, "_sd": sd, "_sd_alg": "sha-256", "cnf": { "jwk": jwk } }),
        );
        let presented = format!("{jwt}~{}~", ds.join("~"));
        let kb = jws(
            holder,
            serde_json::json!({ "alg": "ES256", "typ": "kb+jwt" }),
            serde_json::json!({ "iat": iat, "aud": audience(community), "nonce": nonce(person, community), "sd_hash": b64_hash(presented.as_bytes()) }),
        );
        format!("{presented}{kb}")
    }

    #[test]
    fn a_wallets_proof_of_age_and_nothing_else_checks() {
        let (issuer, holder, other) = (key(1), key(2), key(3));
        let (person, community, now) = (Id([4; 32]), Id([5; 32]), 1_800_000_000);
        let issuers = vec![sec1(&issuer)];
        let good = presentation(&issuer, &holder, &person, &community, now - 10, true, &[]);
        assert_eq!(check(&good, &person, &community, &issuers, now), Ok(()));
        // not for anyone else, nowhere else, not later
        assert!(check(&good, &Id([6; 32]), &community, &issuers, now).unwrap_err().contains("someone else"));
        assert!(check(&good, &person, &Id([7; 32]), &issuers, now).unwrap_err().contains("someone else"));
        assert!(check(&good, &person, &community, &issuers, now + 16 * 60).unwrap_err().contains("too old"));
        // an issuer the community doesn't trust
        assert!(check(&good, &person, &community, &[sec1(&other)], now).unwrap_err().contains("issuer"));
        // under 18, or more than the age shown
        let young = presentation(&issuer, &holder, &person, &community, now, false, &[]);
        assert!(check(&young, &person, &community, &issuers, now).unwrap_err().contains("over 18"));
        let chatty = presentation(&issuer, &holder, &person, &community, now, true, &[("family_name", Json::String("Muster".into()))]);
        assert!(check(&chatty, &person, &community, &issuers, now).unwrap_err().contains("only age_over_18"));
        // without its key binding, or with someone else's
        let unbound = format!("{}~", good.rsplit_once('~').unwrap().0);
        assert!(check(&unbound, &person, &community, &issuers, now).unwrap_err().contains("key binding"));
        let stolen = presentation(&issuer, &other, &person, &community, now, true, &[]);
        let (head, _) = good.rsplit_once('~').unwrap();
        let (_, their_kb) = stolen.rsplit_once('~').unwrap();
        assert!(check(&format!("{head}~{their_kb}"), &person, &community, &issuers, now).is_err());
        // a disclosure changed after signing
        let forged = good.replacen(&disclosure(CLAIM, Json::Bool(true)), &salted("another-salt", CLAIM, Json::Bool(true)), 1);
        assert!(check(&forged, &person, &community, &issuers, now).unwrap_err().contains("didn't sign"));
    }
}
