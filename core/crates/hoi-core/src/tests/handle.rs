//! Verified DNS handles (M3): a name that is a domain shows as verified on a device
//! only once that device found the domain naming the key.
use super::*;
use crate::handle::did_key;

#[test]
fn a_domain_name_is_verified_by_what_the_domain_says() {
    let (t, mut a, _cid, c) = owner();
    a.set_name("mira.example.org").unwrap();
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    post(&mut a, &c, "Hello from my own domain");
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    let key = a.me().unwrap().root_key;
    let seen = |b: &Node| b.person_view(&key, Some(&c)).unwrap().person.handle;
    let before = seen(&b).expect("the name is a domain");
    assert_eq!((before.domain.as_str(), before.verified, before.checking), ("mira.example.org", false, true));
    // the host checks once
    let checks = b.take_handle_checks();
    assert_eq!(checks.iter().map(|c| c.domain.as_str()).collect::<Vec<_>>(), vec!["mira.example.org"]);
    assert!(b.take_handle_checks().is_empty());
    // the domain names someone else: not verified
    b.handle_checked("mira.example.org", vec![format!("did={}", did_key(&Id([9; 32])))]);
    let wrong = seen(&b).unwrap();
    assert!(!wrong.verified && !wrong.checking);
    // a day later it names mira's key
    t.store(T0 + 2 * DAY, Ordering::SeqCst);
    b.sweep();
    assert_eq!(b.take_handle_checks().len(), 1, "checked again after a day");
    b.handle_checked("mira.example.org", vec!["v=spf1 -all".into(), format!("did={}", did_key(&Id::from_hex(&key).unwrap()))]);
    assert!(seen(&b).unwrap().verified);
    // an answer nobody asked for changes nothing
    b.handle_checked("other.example", vec![]);
    assert!(!b.state.handles.contains_key("other.example"));
}

#[test]
fn a_name_that_is_no_domain_has_no_handle() {
    let (_t, mut a, _cid, c) = owner();
    post(&mut a, &c, "Just a post");
    let key = a.me().unwrap().root_key;
    assert!(a.person_view(&key, Some(&c)).unwrap().person.handle.is_none());
    assert!(a.take_handle_checks().is_empty());
}
