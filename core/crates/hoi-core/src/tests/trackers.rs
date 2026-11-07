//! Trackers communities name: kept in the limits, carried by invites, remembered on
//! joining, announced to by public nodes that carry the community, never chosen by the app.
use super::*;
use crate::CarryPolicy;

fn with_trackers(name: &str, trackers: &[&str]) -> CommunityDraft {
    let mut d = draft(name);
    d.limits.as_mut().unwrap().trackers = Some(trackers.iter().map(|t| t.to_string()).collect());
    d
}

#[test]
fn communities_name_their_trackers_and_invites_carry_them() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut a = node("desktop", &t, &Mem::default());
    a.create_identity("mira").unwrap();
    let c = a.create_community(with_trackers("garden", &["https://tracker.example/", "https://registry.example/registry"])).unwrap();
    assert_eq!(c.limits.trackers, vec!["https://tracker.example", "https://registry.example/registry"]);
    assert_eq!(c.trackers, c.limits.trackers);

    let inv = a.invite(&c.id).unwrap();
    assert_eq!(hoi_types::Invite::parse(&inv.uri).unwrap().trackers, c.limits.trackers);
    let mut b = node("web", &t, &Mem::default());
    b.create_identity("lena").unwrap();
    assert_eq!(b.preview_invite(&inv.web).unwrap().trackers, c.limits.trackers);
    b.join(&inv.web).unwrap();
    assert_eq!(b.community(&c.id).unwrap().trackers, c.limits.trackers);

    // only https addresses (plain http on this machine for tests), two at most
    for bad in [vec!["http://tracker.example"], vec!["https://user@tracker.example"], vec!["https://a.example", "https://b.example", "https://c.example"]] {
        assert_eq!(a.create_community(with_trackers("nope", &bad)).unwrap_err().code, "invalid", "{bad:?}");
    }
    // a community that names none writes none
    let plain = a.create_community(draft("plain")).unwrap();
    assert!(plain.limits.trackers.is_empty());
    assert!(hoi_types::Invite::parse(&a.invite(&plain.id).unwrap().uri).unwrap().trackers.is_empty());
}

#[test]
fn trackers_change_by_consent_like_other_limits() {
    let (_t, mut a, _cid, c) = owner();
    let mut l = a.community(&c).unwrap().limits;
    l.trackers = vec!["https://tracker.example".into()];
    let LedgerResult::Published { entry } = a.set_limits(&c, &l).unwrap() else { panic!("threshold 1") };
    let cv = a.community(&c).unwrap();
    let p = cv.proposals.iter().find(|p| p.id == entry.id).unwrap();
    assert!(p.changes.iter().any(|ch| ch.field == "trackers" && ch.from == "none" && ch.to == "https://tracker.example"));
    // nobody else decides yet: it passes at once
    assert_eq!(cv.limits.trackers, vec!["https://tracker.example"]);
}

#[test]
fn only_public_nodes_announce_to_the_trackers_of_open_communities_they_carry() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut owner = node("desktop", &t, &Mem::default());
    owner.create_identity("mira").unwrap();
    let open = owner.create_community(with_trackers("garden", &["https://tracker.example"])).unwrap();
    let mut closed = with_trackers("club", &["https://club-tracker.example"]);
    closed.limits.as_mut().unwrap().admission = Some("invite".into());
    let closed = owner.create_community(closed).unwrap();
    assert!(owner.tracker_targets(16).is_empty(), "a desktop that isn't a public node announces nowhere");

    let mut n = node("node", &t, &Mem::default());
    n.set_settings(&SettingsPatch { relay: Some(true), relay_notice_seen: Some(true), ..Default::default() }).unwrap();
    n.set_carry_policy(CarryPolicy::Any, 8);
    for c in [&open, &closed] {
        n.join(&owner.invite(&c.id).unwrap().uri).unwrap();
    }
    assert_eq!(n.tracker_targets(16), vec!["https://tracker.example"], "the invite-only community's tracker is never announced to");
    assert!(n.tracker_targets(0).is_empty());
}
