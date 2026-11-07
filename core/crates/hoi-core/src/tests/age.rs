//! Age-proof admission (M5): a newcomer is let in by a moderator's device after it saw a
//! proof of age bound to them; vouches alone don't let anyone in.
use super::*;
use crate::age::tests::{key, presentation, sec1};

#[test]
fn a_newcomer_is_let_in_by_a_proof_of_age_and_vouches_alone_dont() {
    let (t, mut a, cid, c) = owner();
    let issuer = key(1);
    let mut l = a.community(&c).unwrap().limits;
    l.admission = "age-proof".into();
    assert_eq!(a.set_limits(&c, &l).unwrap_err().code, "invalid", "an age-proof community names its issuers");
    l.age_issuers = vec![hoi_types::hex_encode(&sec1(&issuer))];
    a.set_limits(&c, &l).unwrap();
    assert_eq!(a.community(&c).unwrap().limits.admission, "age-proof");

    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    let mine = post(&mut b, &c, "Hello, I'm new");
    assert!(mine.held.as_deref().is_some_and(|h| h.contains("age")), "{:?}", mine.held);
    pump(&mut a, &mut b);
    let shown = |a: &Node| a.feed(Some(&c), "fresh", None).unwrap().iter().any(|p| p.id == mine.id);
    assert!(!shown(&a), "held until they're let in");
    // a vouch is no way in here
    let bkey = b.me().unwrap().root_key;
    a.vouch(&c, &bkey, 1, None).unwrap();
    assert!(!shown(&a));

    // the proof: from b's wallet, for b and this community, just now
    let person = Id::from_hex(&bkey).unwrap();
    let challenge = b.age_challenge(&c).unwrap();
    assert_eq!(challenge.aud, crate::age::audience(&cid));
    let now = t.load(Ordering::SeqCst);
    let wrong = presentation(&issuer, &key(2), &Id([9; 32]), &cid, now, true, &[]);
    assert_eq!(b.age_proof_code(&c, &wrong).unwrap_err().code, "invalid", "a proof made for someone else");
    let proof = presentation(&issuer, &key(2), &person, &cid, now, true, &[]);
    let code = b.age_proof_code(&c, &proof).unwrap();
    assert!(code.starts_with("hoi-age-1-"));
    let entry = a.admit_by_age(&code).unwrap();
    assert_eq!(entry.reason.as_deref(), Some("age proof"), "the entry says that, and nothing about the credential");
    // b learns it's in, and passes on what it held back
    pump(&mut a, &mut b);
    assert!(b.feed(Some(&c), "fresh", None).unwrap().iter().any(|p| p.id == mine.id && p.held.is_none()));
    assert!(shown(&a), "in, and what they wrote shows");
    assert_eq!(a.admit_by_age(&code).unwrap_err().code, "invalid", "once is enough");
    // nothing of the presentation went anywhere: no record holds its issuer's signed part
    let signed = proof.split('~').next().unwrap().as_bytes();
    assert!(!a.store.in_community(&cid).any(|s| s.wire.windows(signed.len()).any(|w| w == signed)));
}
