//! Rules and wiki text as Loro documents (M3).
use super::*;
use hoi_types::Verdict;

fn entry_id(r: LedgerResult) -> Id {
    match r {
        LedgerResult::Published { entry } => Id::from_hex(&entry.id).unwrap(),
        other => panic!("published at once: {other:?}"),
    }
}

#[test]
fn two_rules_changes_written_against_the_same_version_both_count() {
    let (t, mut a, cid, c) = owner();
    let ms = members(&mut a, &c, 2);
    let now = T0 + DAY + 10;
    t.store(now, Ordering::SeqCst);
    a.sweep();
    for (i, m) in ms.iter().enumerate() {
        m.send(&mut a, rec(cid, None, text_post(&format!("hello {i}"), "here and reading along"), now + i as u64)).unwrap();
    }
    assert_eq!(a.community(&c).unwrap().rules.unwrap().text, "1. Be kind.");
    // both written while the rules say only "Be kind": neither knows of the other
    let first = entry_id(a.set_rules(&c, "1. Be kind.\n2. No spam.").unwrap());
    let second = entry_id(a.set_rules(&c, "Rules:\n1. Be kind.").unwrap());
    for p in [first, second] {
        for (i, m) in ms.iter().enumerate() {
            answer(m, &mut a, &t, cid, p, Verdict::Ok, now + 60 + i as u64);
        }
    }
    let rules = a.community(&c).unwrap().rules.unwrap();
    assert_eq!(rules.text, "Rules:\n1. Be kind.\n2. No spam.", "both changes, merged");
    assert_eq!(rules.version, 3);
}

#[test]
fn members_edit_a_page_on_two_devices_and_both_edits_stay() {
    let (t, mut a, _cid, c) = owner();
    let mut b = node("desktop", &t, &Mem::default());
    b.create_identity("lu").unwrap();
    b.join(&a.invite(&c).unwrap().web).unwrap();
    let bkey = b.me().unwrap().root_key;
    a.vouch(&c, &bkey, 1, None).unwrap();
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    assert!(a.wiki_page(&c, "start").unwrap().text.is_empty());
    a.edit_wiki(&c, "start", "Welcome.\nRead the rules.").unwrap();
    pump(&mut a, &mut b);
    let seen = b.wiki_page(&c, "start").unwrap();
    assert_eq!(seen.text, "Welcome.\nRead the rules.");
    assert!(seen.can_edit, "b was let in");
    // apart, both edit the same page
    a.net_disconnect("b");
    b.net_disconnect("a");
    t.store(T0 + 600, Ordering::SeqCst);
    a.edit_wiki(&c, "start", "Welcome to the garden.\nRead the rules.").unwrap();
    b.edit_wiki(&c, "start", "Welcome.\nRead the rules.\nAsk in the weekly thread.").unwrap();
    a.net_connect("b", Role::Peer);
    b.net_connect("a", Role::Peer);
    pump(&mut a, &mut b);
    let (ta, tb) = (a.wiki_page(&c, "start").unwrap().text, b.wiki_page(&c, "start").unwrap().text);
    assert_eq!(ta, tb, "the same page on both");
    assert_eq!(ta, "Welcome to the garden.\nRead the rules.\nAsk in the weekly thread.");
    let pages = a.wiki_pages(&c).unwrap();
    assert_eq!((pages[0].page.as_str(), pages[0].editors), ("start", 2));
    assert_eq!(a.wiki_page(&c, "start").unwrap().edits, 3);
    // names and size
    assert_eq!(a.edit_wiki(&c, "Start Here", "x").unwrap_err().code, "invalid");
    assert_eq!(a.edit_wiki(&c, "big", &"x".repeat(70_000)).unwrap_err().code, "invalid");
}

#[test]
fn only_members_edit_and_a_broken_edit_is_refused() {
    let (t, mut a, cid, c) = owner();
    let mut n = node("desktop", &t, &Mem::default());
    n.create_identity("new here").unwrap();
    n.join(&a.invite(&c).unwrap().web).unwrap();
    assert_eq!(n.edit_wiki(&c, "start", "hi").unwrap_err().code, "not-allowed", "a newcomer waits until let in");
    assert!(!n.wiki_page(&c, "start").unwrap().can_edit);
    // a stranger's edit is refused where it arrives
    let stranger = Actor::new();
    stranger.intro(&mut a, T0);
    let doc = loro::LoroDoc::new();
    doc.get_text("body").insert(0, "spam").unwrap();
    let update = doc.export(loro::ExportMode::all_updates()).unwrap();
    let e = stranger.send(&mut a, rec(cid, None, Body::Wiki { page: "start".into(), update: update.clone() }, T0)).unwrap_err();
    assert!(matches!(e, hoi_validate::Reject::NotAllowed(_)), "{e:?}");
    // a member's bytes that aren't an update are refused as content
    let member = members(&mut a, &c, 1).remove(0);
    let e = member.send(&mut a, rec(cid, None, Body::Wiki { page: "start".into(), update: b"not loro at all".to_vec() }, T0 + 1)).unwrap_err();
    assert!(matches!(e, hoi_validate::Reject::Content(_)), "{e:?}");
    assert!(member.send(&mut a, rec(cid, None, Body::Wiki { page: "start".into(), update }, T0 + 2)).is_ok());
    assert_eq!(a.wiki_page(&c, "start").unwrap().text, "spam", "a member's edit counts; undoing it is editing");
}
