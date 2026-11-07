//! Members-only communities as MLS groups, phase S4's first step (SCATTER.md §10): a
//! private group admitted by invitation or approval follows its own records. The owner's device
//! adds the people the records admit and removes those they ban; every member refuses a commit
//! the records do not back; members pass join requests on and hand the invitations back.
use super::*;
use crate::tests::forward::{Mesh, join, kept_pieces, mesh};
use hoi_types::scatter::Manifest;

const DAY: u64 = 86_400;

fn members_only(name: &str) -> CommunityDraft {
    let mut d = draft(name);
    let l = d.limits.as_mut().unwrap();
    l.admission = Some("invite".into());
    l.ladder.as_mut().unwrap().admit_vouches = Some(1);
    d
}

/// `a` owns a members-only private group and admits `b` by hand; both reach holders through
/// forwarders. Returns the group's id.
fn group_of_two(m: &mut Mesh) -> String {
    join(m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    join(m, "b", "192.0.2.20:7843", &[0, 1, 2]);
    let c = m.n("a").create_private_community(members_only("a members-only garden")).unwrap().id;
    m.run(10);
    let request = m.n("b").private_join_request().unwrap();
    let invitation = m.n("a").private_invite(&c, &request).unwrap();
    m.n("b").private_join(&invitation).unwrap();
    m.run(60);
    c
}

#[test]
fn the_owner_admits_by_hand_through_the_ledger_and_members_follow() {
    let mut m = mesh(3, 14);
    let c = group_of_two(&mut m);
    let ca = Id::from_hex(&c).unwrap();
    assert!(m.n("a").private_group(&c).unwrap().follows_records);
    // the owner's choice is in the ledger, which b holds and checked the commit against
    let b_root = m.n("b").state.me.as_ref().unwrap().root;
    assert!(m.n("b").communities[&ca].is_admitted(&b_root));
    let p = post(m.n("a"), &c, "members only, in pieces");
    m.run(120);
    assert_eq!(m.n("b").post(&p.id).unwrap().summary.title, "members only, in pieces");
}

/// A device whose join request names no pickup holders (it made it before it linked to any), so
/// its invitation is handed over by hand.
fn request_unlinked(m: &mut Mesh, name: &str, addr: &str) -> String {
    m.device(name, addr);
    let request = m.n(name).private_join_request().unwrap();
    for f in 0..3 {
        m.dial(name, &format!("f{f}"));
    }
    m.pump();
    request
}

#[test]
fn a_member_passes_a_request_on_and_the_records_decide() {
    let mut m = mesh(3, 14);
    let c = group_of_two(&mut m);
    // b becomes established, so its vouch counts
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    m.run(30);
    let request = request_unlinked(&mut m, "c", "203.0.113.30:7843");
    // b hands it on: b vouches (in a community admitted by invitation, the vouch is the
    // invitation) and the request goes into the group
    let who = m.n("b").private_pass_request(&c, &request).unwrap();
    m.run(180);
    // the owner's device found the records admit c, added c, and sent the invitation back
    let ready = m.n("b").private_group(&c).unwrap().invitations;
    let inv = ready.iter().find(|r| r.device == who.device).expect("an invitation for c came back to b");
    m.n("c").private_join(&inv.invitation).unwrap();
    m.run(120);
    let p = post(m.n("a"), &c, "welcome, c");
    m.run(120);
    assert_eq!(m.n("c").post(&p.id).unwrap().summary.title, "welcome, c");
    assert!(m.n("b").private_group(&c).unwrap().invitations.is_empty() || m.n("a").private_group(&c).unwrap().members.len() == 3);
}

#[test]
fn members_refuse_a_commit_the_records_do_not_back() {
    let mut m = mesh(3, 14);
    let c = group_of_two(&mut m);
    let ca = Id::from_hex(&c).unwrap();
    let epoch = m.n("b").private_group(&c).unwrap().epoch;
    // the owner's device misbehaves: it treats the community as open and adds someone the
    // records never admitted, without an `admit` in the ledger
    join(&mut m, "x", "203.0.113.40:7843", &[0, 1, 2]);
    let request = m.n("x").private_join_request().unwrap();
    m.n("a").communities.get_mut(&ca).unwrap().limits.admission = hoi_types::Admission::Open;
    m.n("a").private_invite(&c, &request).unwrap();
    m.n("a").communities.get_mut(&ca).unwrap().limits.admission = hoi_types::Admission::Invite;
    m.run(180);
    // b holds the records and does not follow: it stays in the epoch before
    assert_eq!(m.n("b").private_group(&c).unwrap().epoch, epoch, "b refused the commit");
    let p = post(m.n("a"), &c, "after an unbacked addition");
    m.run(120);
    assert!(m.n("b").post(&p.id).is_err(), "b does not read the epoch it refused");
}

#[test]
fn removal_follows_a_ban_and_only_a_ban() {
    let mut m = mesh(3, 14);
    let c = group_of_two(&mut m);
    let b = m.n("b").me().unwrap();
    let e = m.n("a").private_remove(&c, &b.device).unwrap_err();
    assert!(e.message.contains("ban"), "{}", e.message);
    m.n("a").ban(&c, &b.root, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * 3600, Ordering::SeqCst);
    m.run(300);
    let members = m.n("a").private_group(&c).unwrap().members;
    assert!(!members.iter().any(|x| x.device == b.device), "the owner's device removed the banned member by itself");
    assert!(!m.n("b").private_group(&c).unwrap().active, "and b read its removal");
}

#[test]
fn an_owner_makes_a_private_copy_of_a_members_only_community() {
    let mut m = mesh(1, 0);
    join(&mut m, "a", "198.51.100.10:7843", &[0]);
    let old = m.n("a").create_community(members_only("an old members-only one")).unwrap();
    assert!(old.private_copy, "offered to its owner");
    let copy = m.n("a").private_copy(&old.id).unwrap();
    assert_ne!(copy.id, old.id);
    assert_eq!(copy.name, old.name);
    assert_eq!(copy.limits.admission, "invite");
    assert!(!copy.private_copy, "a private community needs no copy");
    assert!(m.n("a").private_group(&copy.id).unwrap().follows_records);
    // an open community is public: no private copy
    let open = m.n("a").create_community(draft("an open one")).unwrap();
    assert!(!open.private_copy);
    assert!(m.n("a").private_copy(&open.id).is_err());
}

// ---- several committers: the owner's and moderators' devices, in their order

/// `a` owns a members-only private group and admits every named device by hand.
fn group_of(m: &mut Mesh, names: &[(&str, &str)]) -> String {
    join(m, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let c = m.n("a").create_private_community(members_only("a members-only garden")).unwrap().id;
    m.run(10);
    for (name, addr) in names {
        join(m, name, addr, &[0, 1, 2]);
        let request = m.n(name).private_join_request().unwrap();
        let invitation = invite_when_ready(m, "a", &c, &request);
        m.n(name).private_join(&invitation).unwrap();
        m.run(60);
    }
    c
}

/// `by` invites a request by hand, once what it sent before has its place in the slots.
fn invite_when_ready(m: &mut Mesh, by: &str, c: &str, request: &str) -> String {
    for _ in 0..20 {
        match m.n(by).private_invite(c, request) {
            Ok(invitation) => return invitation,
            Err(e) if e.code == "private-pending" => m.run(15),
            Err(e) => panic!("{}", e.message),
        }
    }
    panic!("never ready to invite");
}

/// `a` makes each named member a moderator, a minute apart, in this order, and lets the line
/// settle: the owner's device gives each moderator's the group's drop key.
fn appoint(m: &mut Mesh, c: &str, mods: &[&str]) {
    for name in mods {
        let root = m.n(name).me().unwrap().root;
        m.n("a").add_mod(c, &root).unwrap();
        m.t.fetch_add(60, Ordering::SeqCst);
        m.run(60);
    }
    m.run(120);
    for name in mods {
        assert!(until(m, 1800, |m| holds_listed_key(m, name, c).is_some()), "{name} was given the drop key");
    }
}

#[test]
fn the_line_is_the_owner_then_moderators_by_how_long_they_have_been_ones() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("m1", "192.0.2.21:7843"), ("m2", "192.0.2.22:7843"), ("b", "192.0.2.20:7843")]);
    // m2 became a moderator first
    appoint(&mut m, &c, &["m2", "m1"]);
    for (name, rank) in [("a", Some(0)), ("m2", Some(1)), ("m1", Some(2)), ("b", None)] {
        let g = m.n(name).private_group(&c).unwrap();
        assert_eq!(g.committers, 3, "{name} sees the same line");
        assert_eq!(g.rank, rank, "{name}");
    }
    // a moderator who is no longer one leaves the line
    let m2 = m.n("m2").me().unwrap().root;
    m.n("a").remove_mod(&c, &m2).unwrap();
    m.run(120);
    for name in ["a", "m1", "m2", "b"] {
        assert_eq!(m.n(name).private_group(&c).unwrap().committers, 2, "{name}");
    }
    assert_eq!(m.n("m1").private_group(&c).unwrap().rank, Some(1));
    assert_eq!(m.n("m2").private_group(&c).unwrap().rank, None);
}

#[test]
fn a_moderator_admits_once_the_owner_stays_silent_for_its_turn() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    appoint(&mut m, &c, &["md"]);
    // b becomes established, so its vouch counts
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    m.run(60);
    // the owner's device goes quiet
    m.cut.insert("a".into());
    let request = request_unlinked(&mut m, "c", "203.0.113.30:7843");
    let who = m.n("b").private_pass_request(&c, &request).unwrap();
    m.run(300);
    assert!(m.n("b").private_group(&c).unwrap().invitations.is_empty(), "the moderator's device waits its turn");
    m.t.fetch_add(crate::private::TURN, Ordering::SeqCst);
    m.run(300);
    // then it adds c, and its invitation (with what makes it a moderator) comes back to b
    let ready = m.n("b").private_group(&c).unwrap().invitations;
    let inv = ready.iter().find(|r| r.device == who.device).expect("the moderator's device added c");
    m.n("c").private_join(&inv.invitation).unwrap();
    m.run(120);
    let p = post(m.n("md"), &c, "while the owner is away");
    for name in ["c", "b"] {
        assert!(until(&mut m, 600, |m| m.n(name).post(&p.id).is_ok()), "{name} reads the moderator's post");
    }
    // the owner's device, back, reads what happened before it changes anything: it follows the
    // moderator's change rather than adding c a second time
    m.cut.remove("a");
    assert!(until(&mut m, 900, |m| m.n("a").private_group(&c).unwrap().room == m.n("c").private_group(&c).unwrap().room));
    let p = post(m.n("a"), &c, "the owner is back");
    assert!(until(&mut m, 600, |m| m.n("c").post(&p.id).is_ok()));
}

/// Runs half a minute at a time until `done`, at most `secs`.
fn until(m: &mut Mesh, secs: u64, done: impl Fn(&mut Mesh) -> bool) -> bool {
    for _ in 0..secs.div_ceil(30) {
        if done(m) {
            return true;
        }
        m.run(30);
    }
    done(m)
}

#[test]
fn a_member_s_moderator_cannot_add_whom_the_records_do_not_admit() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    appoint(&mut m, &c, &["md"]);
    let ca = Id::from_hex(&c).unwrap();
    let epoch = m.n("b").private_group(&c).unwrap().epoch;
    // a moderator's device that treats the community as open adds someone nobody admitted
    join(&mut m, "x", "203.0.113.40:7843", &[0, 1, 2]);
    let request = m.n("x").private_join_request().unwrap();
    m.n("md").communities.get_mut(&ca).unwrap().limits.admission = hoi_types::Admission::Open;
    let _ = m.n("md").private_invite(&c, &request);
    m.n("md").communities.get_mut(&ca).unwrap().limits.admission = hoi_types::Admission::Invite;
    m.run(180);
    assert_eq!(m.n("b").private_group(&c).unwrap().epoch, epoch, "b refused it");
    assert_eq!(m.n("a").private_group(&c).unwrap().epoch, epoch, "and so did the owner's device");
}

#[test]
fn when_two_committers_change_one_epoch_the_one_earlier_in_line_counts() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    appoint(&mut m, &c, &["md"]);
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    m.run(60);
    let epoch = m.n("b").private_group(&c).unwrap().epoch;
    // the owner's device is cut off; the moderator's takes over in its turn and adds c
    m.cut.insert("a".into());
    let request = request_unlinked(&mut m, "c", "203.0.113.30:7843");
    let c_device = m.n("b").private_pass_request(&c, &request).unwrap().device;
    m.run(120);
    m.t.fetch_add(crate::private::TURN, Ordering::SeqCst);
    m.run(300);
    let inv = m.n("b").private_group(&c).unwrap().invitations.into_iter().find(|r| r.device == c_device).expect("added by the moderator");
    m.n("c").private_join(&inv.invitation).unwrap();
    m.run(60);
    assert_eq!(m.n("b").private_group(&c).unwrap().epoch, epoch + 1, "b followed the moderator's commit");
    // meanwhile the owner's device, unaware, admitted x by hand in the epoch the others left
    join(&mut m, "x", "203.0.113.40:7843", &[0, 1, 2]);
    let x_request = m.n("x").private_join_request().unwrap();
    let x_inv = invite_when_ready(&mut m, "a", &c, &x_request);
    m.n("x").private_join(&x_inv).unwrap();
    // it comes back: its commit was first in line, so it counts, for the moderator's device too
    m.cut.remove("a");
    m.run(900);
    let room = m.n("a").private_group(&c).unwrap().room;
    for name in ["b", "md", "x"] {
        assert_eq!(m.n(name).private_group(&c).unwrap().room, room, "{name} follows the owner's commit");
    }
    let x_device = m.n("x").me().unwrap().device;
    let members = m.n("b").private_group(&c).unwrap().members;
    assert!(members.iter().any(|w| w.device == x_device) && !members.iter().any(|w| w.device == c_device));
    let p = post(m.n("md"), &c, "back on one branch");
    for name in ["a", "b", "x"] {
        assert!(until(&mut m, 600, |m| m.n(name).post(&p.id).is_ok()), "{name} reads the moderator's post on the owner's branch");
    }
    // c, added by the commit that lost, asks again, and its app picks the new invitation up
    let again = m.n("c").private_join_request().unwrap();
    m.n("b").private_pass_request(&c, &again).unwrap();
    assert!(until(&mut m, 900, |m| m.n("c").private_group(&c).unwrap().room == m.n("a").private_group(&c).unwrap().room), "added again, and joined by itself");
    let p = post(m.n("a"), &c, "welcome back, c");
    assert!(until(&mut m, 600, |m| m.n("c").post(&p.id).is_ok()));
}

// ---- a drop key every committer's device holds

/// Waits until `name` holds the drop key `group`'s list names, and returns it.
fn holds_listed_key(m: &mut Mesh, name: &str, c: &str) -> Option<[u8; 32]> {
    let ca = Id::from_hex(c).unwrap();
    let n = m.n(name);
    let listed = n.community_drop_key(ca)?;
    n.held_drop_key(ca, &listed).map(|_| listed)
}

/// The newest generation of the object that carries post `p`, as `name` keeps it.
fn generation_of(m: &mut Mesh, name: &str, p: &str) -> Manifest {
    let pid = Id::from_hex(p).unwrap();
    m.n(name).scatter.saved.kept.values().find(|k| k.records == vec![pid]).unwrap().newest().unwrap()
}

#[test]
fn a_moderator_s_device_takes_the_drop_key_and_drops_a_removed_post_in_its_turn() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    // the owner's device gives the moderator's its key, sealed to its leaf alone
    appoint(&mut m, &c, &["md"]);
    let key = holds_listed_key(&mut m, "a", &c).expect("the owner's device still holds it");
    assert_eq!(holds_listed_key(&mut m, "md", &c), Some(key), "the same key: pieces stored before stay droppable");
    let (a, md) = (m.n("a").me().unwrap().device, m.n("md").me().unwrap().device);
    let holders = m.n("b").private_epoch(ca).unwrap().holders;
    assert!(holders.iter().any(|h| h.hex() == a) && holders.iter().any(|h| h.hex() == md), "the list names both as holders");
    assert!(m.n("b").held_drop_keys(ca).is_empty(), "a member's device holds no drop key");

    let p = post(m.n("b"), &c, "against the rules");
    assert!(until(&mut m, 600, |m| m.n("md").post(&p.id).is_ok()));
    let man = generation_of(&mut m, "md", &p.id);
    assert_eq!(kept_pieces(&mut m, &man), man.hashes.len());
    // the owner's device and the author's are away; the moderator removes the post
    m.cut.insert("a".into());
    m.cut.insert("b".into());
    m.n("md").remove(&p.id, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * 3600, Ordering::SeqCst);
    m.run(120);
    let pid = Id::from_hex(&p.id).unwrap();
    assert!(!m.n("md").scatter.saved.kept.values().any(|k| k.records == vec![pid]), "let go");
    assert_eq!(kept_pieces(&mut m, &man), man.hashes.len(), "second in line, it waits its turn");
    m.t.fetch_add(crate::private::TURN, Ordering::SeqCst);
    m.run(120);
    assert_eq!(kept_pieces(&mut m, &man), 0, "then it finds the pieces still there and drops them");
}

#[test]
fn a_new_drop_key_once_a_moderator_leaves_the_line_and_older_pieces_stay_droppable() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    appoint(&mut m, &c, &["md"]);
    let old = holds_listed_key(&mut m, "a", &c).unwrap();
    let p = post(m.n("b"), &c, "stored with the first key");
    assert!(until(&mut m, 600, |m| m.n("a").post(&p.id).is_ok()));
    let man = generation_of(&mut m, "a", &p.id);

    // md is a moderator no longer: its device forgets the keys, and the owner's makes a new one
    let md_root = m.n("md").me().unwrap().root;
    m.n("a").remove_mod(&c, &md_root).unwrap();
    assert!(until(&mut m, 1800, |m| m.n("b").community_drop_key(ca).is_some_and(|k| k != old)), "a new key is listed");
    let new = m.n("b").community_drop_key(ca).unwrap();
    assert!(m.n("md").held_drop_keys(ca).is_empty(), "a device out of the line keeps no drop key");
    assert!(m.n("a").held_drop_key(ca, &new).is_some() && m.n("a").held_drop_key(ca, &old).is_some(), "the owner's device keeps the old one too");
    let md = m.n("md").me().unwrap().device;
    assert!(!m.n("b").private_epoch(ca).unwrap().holders.iter().any(|h| h.hex() == md));

    // the post stored with the old key is removed: the owner's device drops it with that key
    m.n("a").remove(&p.id, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * 3600, Ordering::SeqCst);
    m.run(120);
    assert_eq!(kept_pieces(&mut m, &man), 0, "dropped with the key it was stored with");
}

#[test]
fn with_the_owner_s_device_gone_the_first_moderator_makes_a_key_of_its_own() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    // the owner's device goes away for good before it could give anyone the key
    m.cut.insert("a".into());
    let old = m.n("b").community_drop_key(ca).unwrap();
    let md_root = m.n("md").me().unwrap().root;
    // the appointment reaches md through b, whom the owner's device told before going
    m.cut.remove("a");
    m.n("a").add_mod(&c, &md_root).unwrap();
    m.run(15);
    m.cut.insert("a".into());
    m.run(300);
    assert_eq!(m.n("md").private_group(&c).unwrap().rank, Some(1));
    assert!(m.n("md").held_drop_keys(ca).is_empty());
    // second in line and without the key, it waits its turn and one more, then makes one
    m.t.fetch_add(crate::private::TURN, Ordering::SeqCst);
    m.run(700);
    assert_eq!(m.n("b").community_drop_key(ca), Some(old), "not yet");
    m.t.fetch_add(crate::private::TURN, Ordering::SeqCst);
    assert!(until(&mut m, 1800, |m| m.n("b").community_drop_key(ca).is_some_and(|k| k != old)), "a key of its own is listed");
    let new = m.n("b").community_drop_key(ca).unwrap();
    assert!(m.n("md").held_drop_key(ca, &new).is_some());
    // posts are stored with it, and the moderator's device drops them once removed
    let p = post(m.n("b"), &c, "after the owner left");
    assert!(until(&mut m, 600, |m| m.n("md").post(&p.id).is_ok()));
    let man = generation_of(&mut m, "md", &p.id);
    m.n("md").remove(&p.id, "spam").unwrap();
    m.run(60);
    m.t.fetch_add(2 * 3600, Ordering::SeqCst);
    m.run(120);
    m.t.fetch_add(crate::private::TURN, Ordering::SeqCst);
    m.run(120);
    assert_eq!(kept_pieces(&mut m, &man), 0);
}

#[test]
fn only_a_committer_s_device_gives_a_drop_key_and_only_to_one() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    let ca = Id::from_hex(&c).unwrap();
    appoint(&mut m, &c, &["md"]);
    let md = Id::from_hex(&m.n("md").me().unwrap().device).unwrap();
    let held = m.n("md").held_drop_keys(ca);
    // the member's device heard the moderator's seal key, as every member does
    let seal = m.n("a").private.seal_keys[&ca][&md].clone();
    m.n("b").private.seal_keys.entry(ca).or_default().insert(md, seal);
    // a member's device offers the moderator's a key of its own: refused
    let forged = m.n("b").sealed_drop_key(ca, md, [9; 32]);
    assert!(m.n("md").private_receive(&c, &forged).is_err(), "not from a committer's device");
    assert_eq!(m.n("md").held_drop_keys(ca), held);
    // a key for one device is nothing to another
    let for_md = m.n("a").sealed_drop_key(ca, md, [8; 32]);
    m.n("b").private_receive(&c, &for_md).unwrap();
    assert!(m.n("b").held_drop_keys(ca).is_empty(), "a member's device holds no drop key");
    m.n("md").private_receive(&c, &for_md).unwrap();
    assert_eq!(m.n("md").held_drop_keys(ca).len(), held.len() + 1, "the one it was for takes it");
}

#[test]
fn a_request_passed_on_in_an_epoch_a_commit_ends_goes_again() {
    let mut m = mesh(3, 14);
    let c = group_of(&mut m, &[("md", "192.0.2.21:7843"), ("b", "192.0.2.20:7843")]);
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    m.run(60);
    let request = request_unlinked(&mut m, "c", "203.0.113.30:7843");
    // b passes c's request on while it is cut off; meanwhile the owner's device commits
    // (it gives a new moderator the drop key), so the epoch b passed it in ends unread
    m.cut.insert("b".into());
    let epoch = m.n("b").private_group(&c).unwrap().epoch;
    let who = m.n("b").private_pass_request(&c, &request).unwrap();
    appoint(&mut m, &c, &["md"]);
    assert!(m.n("a").private_group(&c).unwrap().epoch > epoch);
    m.cut.remove("b");
    // b follows, passes the request again in the epoch it moved to, and the invitation comes back
    assert!(until(&mut m, 900, |m| m.n("b").private_group(&c).unwrap().invitations.iter().any(|r| r.device == who.device)), "c was added");
}

// ---- the clean break

#[test]
fn a_commit_found_ahead_waits_until_the_writer_s_slots_before_it_are_read() {
    use crate::mailbox::MailRequest;
    use hoi_crypto::envelope::Reply;
    let mut m = mesh(3, 14);
    let c = group_of_two(&mut m);
    let ca = Id::from_hex(&c).unwrap();
    m.run(120);
    let epoch = m.n("b").private_epoch(ca).unwrap().epoch;
    let leaf = m.n("a").private_epoch(ca).unwrap().own;
    let next = m.n("b").scatter.saved.mailboxes[&ca].read.get(&leaf).copied().unwrap_or(0);
    // a writes a record whole (its identicon color, which goes out with its next post), then a
    // commit no record backs (a new drop key), and nobody reads them yet
    m.n("a").set_avatar_color(Some("lilac")).unwrap();
    post(m.n("a"), &c, "with a color");
    m.n("a").private_outbox(&c).unwrap();
    let list = m.n("a").private_epoch(ca).unwrap().mailboxes;
    m.n("a").private_set_mailboxes(ca, list, crate::private::KeyWork::Make).unwrap();
    let pending = m.n("a").private_pending(ca);
    let slots = m.n("a").scatter.saved.mailboxes[&ca].slots.clone();
    let mut old: Vec<(u32, Vec<u8>)> =
        pending.iter().filter_map(|o| slots.get(&o.id).filter(|s| s.epoch == epoch).map(|s| (s.seq, hoi_types::uri::unb64(&o.body).unwrap()))).collect();
    old.sort();
    assert!(old.len() >= 2 && old[0].0 == next, "{:?}", old.iter().map(|o| o.0).collect::<Vec<_>>());
    let (commit_seq, commit) = old.pop().unwrap();
    // b finds the commit first, ahead of the slots before it: it holds it
    let fetch = |seq| MailRequest::Fetch { c: ca, epoch, leaf, seq };
    m.n("b").on_mail_answer(fetch(commit_seq), Some(Reply::Got(Some(commit))));
    assert_eq!(m.n("b").private_epoch(ca).unwrap().epoch, epoch, "a commit found ahead is not followed yet");
    // then the slots before it, in order: the record is read in the epoch it was written in, and
    // the commit follows once its turn comes
    for (seq, body) in old {
        m.n("b").on_mail_answer(fetch(seq), Some(Reply::Got(Some(body))));
    }
    assert!(m.n("b").store.of_kind(&ca, hoi_types::Kind::ProfileColor).count() > 0, "the writer's earlier slot was read");
    assert_eq!(m.n("b").private_epoch(ca).unwrap().epoch, epoch + 1, "and the commit followed it");
}

// ---- invitations picked up by the newcomer

#[test]
fn a_newcomer_picks_up_its_invitation_by_itself() {
    let mut m = mesh(3, 14);
    let c = group_of_two(&mut m);
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    m.run(30);
    join(&mut m, "c", "203.0.113.30:7843", &[0, 1, 2]);
    let request = m.n("c").private_join_request().unwrap();
    assert!(!m.n("c").private_pickup().unwrap().1.is_empty(), "the request names holders c reaches");
    m.n("b").private_pass_request(&c, &request).unwrap();
    // nobody hands c anything: its app finds the invitation on the holders its request named
    assert!(until(&mut m, 900, |m| m.n("c").private_group(&c).is_ok_and(|g| g.active)), "c joined by itself");
    assert!(m.n("c").private_pickup().is_none(), "and stopped looking");
    let p = post(m.n("a"), &c, "welcome, c");
    assert!(until(&mut m, 600, |m| m.n("c").post(&p.id).is_ok()));
    let p = post(m.n("c"), &c, "thanks");
    assert!(until(&mut m, 600, |m| m.n("b").post(&p.id).is_ok()));
    // the stored invitation is gone from the committer's list once its holders took it
    m.run(60);
    assert!(m.n("a").scatter.saved.pickups.is_empty());
}

#[test]
fn a_filled_pickup_slot_moves_the_invitation_to_the_next() {
    let mut m = mesh(3, 14);
    let c = group_of_two(&mut m);
    m.t.fetch_add(DAY + 60, Ordering::SeqCst);
    m.run(30);
    join(&mut m, "c", "203.0.113.30:7843", &[0, 1, 2]);
    let request = m.n("c").private_join_request().unwrap();
    // someone who saw the request fills slot 0 on every holder it names with junk sealed to the
    // pickup key: it opens, but holds no invitation c can follow
    let (key, holders) = m.n("c").private_pickup().unwrap();
    let junk = crate::pickup::seal_invitation(&key.public(), "hoi:private:junk").unwrap();
    for h in &holders {
        let name = m.eps.iter().find(|(_, e)| **e == h.endpoint.hex()).map(|(n, _)| n.clone()).unwrap();
        let loc = crate::pickup::pickup_loc(&key.public(), 0);
        let day = (m.n(&name).now() / 86_400) as u32 + 7;
        let stamp = hoi_crypto::scatter::mine_stamp(&h.endpoint.0, &loc, junk.len(), day);
        let put = hoi_crypto::envelope::Request::Put {
            loc,
            reply: hoi_crypto::envelope::XKey::generate().unwrap().public(),
            expiry_day: day,
            drop_hash: [0; 32],
            drop_key_id: [0; 16],
            stamp,
            piece: junk.clone(),
        };
        let xk = m.n(&name).holder_hello().unwrap().xk;
        let env = hoi_crypto::envelope::seal_request(&h.endpoint.0, &xk, &put).unwrap();
        m.n(&name).on_piece("someone", hoi_sync::PieceOp::Put, 1, None, env);
    }
    m.n("b").private_pass_request(&c, &request).unwrap();
    assert!(until(&mut m, 1200, |m| m.n("c").private_group(&c).is_ok_and(|g| g.active)), "c joined from the next slot");
}
