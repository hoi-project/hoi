//! Bulk author deletion (RFC-0002): an explicit inventory of the person's known posts and
//! comments, frozen at confirmation and deleted one signed record at a time under the ordinary
//! rules. Gaps are reported and never counted as deleted.
use super::*;
use crate::Published;
use crate::tests::drop_share::{attach, manifest, remote_cap};
use crate::tests::forward::Mesh;
use std::collections::BTreeSet;

fn id(hex: &str) -> Id {
    Id::from_hex(hex).unwrap()
}

/// The signed deletions a node holds, by target.
fn deletions_by_target(n: &Node) -> BTreeMap<Id, usize> {
    let mut out = BTreeMap::new();
    for s in n.store.all().filter(|s| matches!(s.record.body, Body::Delete)) {
        *out.entry(s.record.parent.unwrap()).or_default() += 1;
    }
    out
}

fn listed(p: &BulkDeletionPreview) -> BTreeSet<String> {
    p.communities.iter().flat_map(|c| c.posts.iter().chain(&c.comments).cloned()).collect()
}

fn gap_reasons(gaps: &[BulkDeletionGap]) -> BTreeSet<&str> {
    gaps.iter().map(|g| g.reason.as_str()).collect()
}

fn batch(n: &Node) -> BulkDeletionStatus {
    let mut all = n.bulk_deletions();
    assert_eq!(all.len(), 1, "one saved batch");
    all.pop().unwrap()
}

/// Rounds of the scatter tick, which signs a few targets each time.
fn rounds(n: &mut Node, count: usize) {
    for _ in 0..count {
        n.scatter_tick();
        n.commit().unwrap();
    }
}

fn reopen(platform: &str, t: &Arc<AtomicU64>, memory: &Mem) -> Node {
    node(platform, t, memory)
}

#[test]
fn a_mixed_author_fixture_deletes_only_the_requesters_known_targets() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    home.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let garden = home.create_community(draft("garden")).unwrap().id;
    let kitchen = home.create_community(draft("kitchen")).unwrap().id;
    web.join(&home.invite(&kitchen).unwrap().uri).unwrap();
    pair(&mut home, &mut web, &garden);
    let pa1 = post(&mut home, &garden, "Mira in the garden");
    let pa2 = post(&mut home, &kitchen, "Mira in the kitchen");
    pump(&mut home, &mut web);
    let pb1 = post(&mut web, &garden, "Lena in the garden");
    let pb2 = post(&mut web, &kitchen, "Lena in the kitchen");
    let cb1 = web.reply(&pa1.id, "Lena answers Mira under her post").unwrap();
    let cb2 = web.reply(&pb1.id, "Lena adds a note to her own post").unwrap();
    web.edit(&pb1.id, "Lena edited her garden post", None).unwrap();
    pump(&mut home, &mut web);
    let ca1 = home.reply(&pb1.id, "Mira answers Lena under her post").unwrap();
    pump(&mut home, &mut web);

    let lena: BTreeSet<String> = [&pb1.id, &pb2.id, &cb1.id, &cb2.id].into_iter().cloned().collect();
    let mira: BTreeSet<String> = [&pa1.id, &pa2.id, &ca1.id].into_iter().cloned().collect();
    assert_eq!(listed(&home.preview_bulk_deletion().unwrap()), mira, "another member's preview never lists Lena's records");
    let preview = web.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), lena, "only the requester's posts and comments; the edit follows its post");
    assert_eq!(preview.count, 4);
    assert!(!preview.inventory_complete, "a local scan never claims complete history");
    let in_garden = preview.communities.iter().find(|c| c.id == garden).unwrap();
    assert_eq!(in_garden.name, "garden");
    assert_eq!(in_garden.posts, vec![pb1.id.clone()]);
    assert_eq!(in_garden.comments.iter().collect::<BTreeSet<_>>(), [&cb1.id, &cb2.id].into_iter().collect());
    assert_eq!(preview.communities.iter().find(|c| c.id == kitchen).unwrap().posts, vec![pb2.id.clone()]);
    assert!(deletions_by_target(&web).is_empty(), "a preview changes nothing");

    let status = web.confirm_bulk_deletion(&preview.snapshot).unwrap();
    assert_eq!((status.count, status.selected, status.signed), (4, 4, 0), "saved before anything is signed");
    assert_eq!(web.bulk_deletions(), vec![status]);
    rounds(&mut web, 1);
    let status = batch(&web);
    assert_eq!(status.signed, 4);
    let jobs: BTreeSet<String> = web.deletion_progress().into_iter().map(|p| p.id).collect();
    for target in &status.targets {
        assert_eq!(target.state, "signed");
        assert!(jobs.contains(target.deletion.as_ref().unwrap()), "each signed target has its own RFC-0001 job");
    }
    assert!(deletions_by_target(&web).values().all(|n| *n == 1) && deletions_by_target(&web).len() == 4, "one signed deletion per target");
    pump(&mut home, &mut web);

    for n in [&home, &web] {
        for gone in &lena {
            assert!(!n.store.contains(&id(gone)) && n.state.skeletons.contains_key(&id(gone)), "deleted everywhere it travelled");
        }
        for kept in &mira {
            assert!(n.store.contains(&id(kept)), "other authors' records stay, replies under deleted posts included");
        }
        assert!(!n.store.all().any(|s| matches!(s.record.body, Body::Edit { .. })), "the edit went with its post");
    }

    // the same snapshot resumes the saved batch and signs nothing twice
    let again = web.confirm_bulk_deletion(&preview.snapshot).unwrap();
    assert_eq!(again, batch(&web));
    rounds(&mut web, 2);
    assert_eq!(deletions_by_target(&web).len(), 4);
    assert_eq!(web.preview_bulk_deletion().unwrap().count, 0, "nothing known is left");
}

#[test]
fn forged_altered_and_other_member_requests_change_nothing() {
    let (_t, mut a, cid, c) = owner();
    let x = Actor::new();
    x.intro(&mut a, T0);
    let y = Actor::new();
    y.intro(&mut a, T0);
    let theirs = x.send(&mut a, rec(cid, None, text_post("x writes", "a post by somebody else"), T0)).unwrap();
    let mine = post(&mut a, &c, "Mira's own post");
    let preview = a.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), BTreeSet::from([mine.id.clone()]));

    // snapshots this device did not preview
    let mut altered = preview.snapshot.clone();
    let last = if altered.ends_with('0') { "1" } else { "0" };
    altered.replace_range(63.., last);
    for bad in [Id::ZERO.hex(), altered, "not a snapshot".into()] {
        assert_eq!(a.confirm_bulk_deletion(&bad).unwrap_err().code, "invalid", "{bad}");
    }
    assert_eq!(a.cancel_bulk_deletion(&preview.snapshot).unwrap_err().code, "not-found");
    assert!(a.bulk_deletions().is_empty());

    // another member's deletion, a forged signature and an altered target are refused
    let peer = Source::Peer { peer: "p".into(), live: true };
    let (other, _) = y.sign(rec(cid, Some(id(&mine.id)), Body::Delete, T0 + 1), 8);
    assert!(a.ingest(&other, &peer).is_err(), "only the author deletes");
    let (valid, _) = x.sign(rec(cid, Some(theirs), Body::Delete, T0 + 2), 8);
    let (mut forged, _) = hoi_types::Wire::decode(&valid).unwrap();
    forged.sig.0[0] ^= 1;
    assert!(a.ingest(&forged.encode(), &peer).is_err(), "a forged signature");
    let (mut moved, _) = hoi_types::Wire::decode(&valid).unwrap();
    moved.record.parent = Some(id(&mine.id));
    assert!(a.ingest(&moved.encode(), &peer).is_err(), "a signed deletion moved to another target");
    a.commit().unwrap();
    assert!(a.store.contains(&theirs) && a.store.contains(&id(&mine.id)));

    // the confirmed list deletes its own target only
    a.confirm_bulk_deletion(&preview.snapshot).unwrap();
    rounds(&mut a, 1);
    assert_eq!(batch(&a).signed, 1);
    assert!(a.store.contains(&theirs));
    assert!(!a.store.contains(&id(&mine.id)));
}

#[test]
fn single_deletion_and_moderator_removal_stay_separate_powers() {
    let (t, mut a, cid, c) = owner();
    let x = Actor::new();
    x.intro(&mut a, T0);
    let spam = x.send(&mut a, rec(cid, None, text_post("buy now", "a post the owner removes"), T0)).unwrap();
    let alone = post(&mut a, &c, "deleted on its own");
    let listed_later = post(&mut a, &c, "deleted with the batch");
    a.delete_own(&alone.id).unwrap();
    let preview = a.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), BTreeSet::from([listed_later.id.clone()]), "work already under way is not listed again");
    a.confirm_bulk_deletion(&preview.snapshot).unwrap();
    a.remove(&spam.hex(), "rule 1").unwrap();
    t.store(T0 + 3601, Ordering::SeqCst);
    a.sweep();
    a.commit().unwrap();

    let removal = &a.state.skeletons[&spam];
    assert!(removal.by.is_some() && removal.reason == "rule 1", "the moderator's removal, with its reason");
    assert!(!a.state.skeletons[&id(&alone.id)].by.is_some());
    let status = batch(&a);
    assert_eq!((status.count, status.signed), (1, 1), "the batch never reaches another author's post");
    assert_eq!(deletions_by_target(&a).keys().copied().collect::<BTreeSet<_>>(), BTreeSet::from([id(&alone.id), id(&listed_later.id)]));
}

#[test]
fn a_large_batch_waits_for_rate_windows_and_resumes_after_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let memory = Mem::default();
    let mut web = node("web", &t, &memory);
    home.create_identity("mira").unwrap();
    web.create_identity("lena").unwrap();
    let c = home.create_community(draft("garden")).unwrap().id;
    pair(&mut home, &mut web, &c);
    // a newcomer's whole day: five posts and thirty comments
    let mut targets = BTreeSet::new();
    let posts: Vec<PostSummary> = (0..5).map(|i| post(&mut web, &c, &format!("Lena's thread {i}"))).collect();
    targets.extend(posts.iter().map(|p| p.id.clone()));
    for hour in 0..3u64 {
        t.store(T0 + hour * 3600, Ordering::SeqCst);
        for i in 0..10 {
            targets.insert(web.reply(&posts[i % 5].id, &format!("note {hour}-{i}, in words of its own")).unwrap().id);
        }
    }
    pump(&mut home, &mut web);
    web.net_disconnect("a");
    home.net_disconnect("b");

    let start = T0 + 3 * 3600;
    t.store(start, Ordering::SeqCst);
    let preview = web.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), targets);
    assert_eq!(preview.count, 35);
    web.confirm_bulk_deletion(&preview.snapshot).unwrap();
    rounds(&mut web, 6);
    let status = batch(&web);
    assert_eq!((status.signed, status.queued, status.selected), (30, 5, 0), "deletions keep the 30-an-hour window");
    for target in status.targets.iter().filter(|t| t.state == "queued") {
        assert_eq!(target.reason.as_deref(), Some("rate-limited"));
        assert_eq!(target.next_try, Some(start + 3600), "waits for the window, never backdates");
    }
    assert!(web.store.all().filter(|s| matches!(s.record.body, Body::Delete)).all(|s| s.record.created_at == start), "signed with the current time");
    let progress = web.deletion_progress();
    assert_eq!(progress.len(), 30);
    assert!(progress.iter().all(|p| p.local_removed && !p.record_submitted), "signed is not propagated: nothing has been sent");

    // restart: the same list and the same jobs, and nothing signed twice
    let before = batch(&web);
    drop(web);
    let mut web = reopen("web", &t, &memory);
    assert_eq!(web.bulk_deletions(), vec![before.clone()]);
    assert_eq!(web.confirm_bulk_deletion(&preview.snapshot).unwrap(), before);
    rounds(&mut web, 3);
    assert_eq!(batch(&web).signed, 30, "still inside the window");
    t.store(start + 3600, Ordering::SeqCst);
    rounds(&mut web, 2);
    let status = batch(&web);
    assert_eq!((status.signed, status.queued), (35, 0));
    let signed = deletions_by_target(&web);
    assert_eq!(signed.keys().map(|k| k.hex()).collect::<BTreeSet<_>>(), targets);
    assert!(signed.values().all(|n| *n == 1), "one signed deletion per target");

    // back online, the deletions travel and the receiving node takes them under the same windows
    web.net_connect("a", Role::Home);
    home.net_connect("b", Role::Client);
    pump(&mut home, &mut web);
    for _ in 0..10 {
        rounds(&mut web, 1);
        pump(&mut home, &mut web);
    }
    for target in &targets {
        assert!(!home.store.contains(&id(target)) && home.state.skeletons.contains_key(&id(target)), "{target}");
    }
    assert!(web.deletion_progress().iter().all(|p| p.record_submitted), "submitted once a peer could take them");
}

#[test]
fn owners_keep_their_rate_exemption_in_a_batch() {
    let (_t, mut a, _cid, c) = owner();
    let thread = post(&mut a, &c, "the owner's thread");
    for i in 0..34 {
        a.reply(&thread.id, &format!("owner note {i}, in words of its own")).unwrap();
    }
    let preview = a.preview_bulk_deletion().unwrap();
    assert_eq!(preview.count, 35);
    a.confirm_bulk_deletion(&preview.snapshot).unwrap();
    rounds(&mut a, 5);
    assert_eq!(batch(&a).signed, 35, "owners and moderators are exempt from the deletion window, as for single deletions");
}

#[test]
fn cancelling_stops_unsigned_targets_and_accepted_deletions_continue() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    for i in 0..12 {
        post(&mut n, &c, &format!("post {i}"));
    }
    let preview = n.preview_bulk_deletion().unwrap();
    n.confirm_bulk_deletion(&preview.snapshot).unwrap();
    rounds(&mut n, 1);
    assert_eq!(batch(&n).signed, 8, "a few at a time");
    let cancelled = n.cancel_bulk_deletion(&preview.snapshot).unwrap();
    assert_eq!((cancelled.signed, cancelled.cancelled, cancelled.cancelled_at), (8, 4, Some(T0)));
    rounds(&mut n, 3);
    assert_eq!(deletions_by_target(&n).len(), 8, "nothing more is signed");
    assert_eq!(n.deletion_progress().len(), 8, "accepted deletions keep their jobs");
    drop(n);
    let n = reopen("desktop", &t, &memory);
    assert_eq!(batch(&n), cancelled);
    let again = n.preview_bulk_deletion().unwrap();
    assert_eq!(again.count, 4, "the cancelled targets need a new preview and confirmation");
    assert_ne!(again.snapshot, preview.snapshot);
}

#[test]
fn a_linked_device_deletes_what_it_received_and_resumes_after_restart() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut desk = node("desktop", &t, &Mem::default());
    let memory = Mem::default();
    let mut web = node("web", &t, &memory);
    desk.create_identity("mira").unwrap();
    let c = desk.create_community(draft("garden")).unwrap().id;
    let p1 = post(&mut desk, &c, "Posted on the computer");
    let request = web.link_request("mira's browser").unwrap();
    let approval = desk.link_approve(&request).unwrap();
    web.link_complete(&approval).unwrap();

    // before it hears from the computer, the browser lists nothing and says why
    let early = web.preview_bulk_deletion().unwrap();
    assert_eq!(early.count, 0);
    assert!(early.gaps.iter().any(|g| g.reason == "not-synchronized" && g.community.as_deref() == Some(c.as_str())), "{:?}", early.gaps);

    let pairing = desk.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    web.pair(&pairing).unwrap();
    web.net_connect("a", Role::Home);
    desk.net_connect("b", Role::Client);
    pump(&mut desk, &mut web);
    let p2 = post(&mut web, &c, "Posted on the browser");
    pump(&mut desk, &mut web);
    for n in [&desk, &web] {
        let preview = n.preview_bulk_deletion().unwrap();
        assert_eq!(listed(&preview), BTreeSet::from([p1.id.clone(), p2.id.clone()]), "any of the person's devices lists both");
    }
    let preview = web.preview_bulk_deletion().unwrap();
    assert!(!gap_reasons(&preview.gaps).contains("not-synchronized"), "{:?}", preview.gaps);
    let status = web.confirm_bulk_deletion(&preview.snapshot).unwrap();

    assert_eq!(status.selected, 2);

    // the browser restarts before anything was signed; opening resumes the saved list
    drop(web);
    let mut web = reopen("web", &t, &memory);
    let resumed = batch(&web);
    assert_eq!((resumed.snapshot.as_str(), resumed.signed), (status.snapshot.as_str(), 2));
    rounds(&mut web, 2);
    assert!(deletions_by_target(&web).len() == 2 && deletions_by_target(&web).values().all(|n| *n == 1));
    web.net_connect("a", Role::Home);
    desk.net_connect("b", Role::Client);
    pump(&mut desk, &mut web);
    for p in [&p1, &p2] {
        assert!(desk.state.skeletons.contains_key(&id(&p.id)), "the computer applies the browser's deletion of its post");
    }
    assert!(desk.preview_bulk_deletion().unwrap().count == 0);
}

#[test]
fn drop_authority_from_another_device_counts_and_missing_authority_is_a_gap() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let p = post(&mut n, &c, "placed in pieces by my other device");
    let m = manifest();
    attach(&mut n, &c, &p.id, &m);
    let missing = n.preview_bulk_deletion().unwrap();
    assert_eq!(missing.capabilities, 0);
    let gap = BulkDeletionGap { reason: "capability-missing".into(), community: Some(c.clone()), target: Some(p.id.clone()), count: 1 };
    assert!(missing.gaps.contains(&gap), "{:?}", missing.gaps);

    // the other device shares its capability through the existing own-device channel
    let cap = remote_cap(&m);
    n.save_drop_cap(cap.clone());
    let preview = n.preview_bulk_deletion().unwrap();
    assert_eq!(preview.capabilities, 1);
    assert!(!gap_reasons(&preview.gaps).contains("capability-missing"));
    assert_eq!(preview.snapshot, missing.snapshot, "authority changes the gaps, not the list to confirm");
    let status = n.confirm_bulk_deletion(&missing.snapshot).unwrap();
    assert_eq!(status.gaps, preview.gaps, "the batch keeps the gaps that held when it was confirmed");
    rounds(&mut n, 1);
    let job = n.deletion_for_target(id(&p.id)).unwrap();
    assert_eq!(batch(&n).targets[0].deletion, Some(job.hex()));
    let targets = &n.deletions.saved.jobs[&job].targets;
    assert_eq!(targets.len(), m.holders.len());
    assert!(targets.values().all(|target| target.token == cap.tokens.get(&target.locator).copied()), "the shared tokens, never this device's secret");
}

#[test]
fn identity_recovery_rebuilds_what_it_can_and_reports_what_it_cannot() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let mut robin = node("desktop", &t, &Mem::default());
    robin.create_identity("robin").unwrap();
    let c = robin.create_community(draft("garden")).unwrap().id;
    let mut mira = node("web", &t, &Mem::default());
    mira.create_identity("mira").unwrap();
    mira.join(&robin.invite(&c).unwrap().web).unwrap();
    pair_now(&mut home, &mut robin, "home", "robin");
    pair_now(&mut home, &mut mira, "home", "mira");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira)]);
    let first = post(&mut mira, &c, "Mira, before the laptop was lost");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira)]);
    let line = mira.recovery_key().unwrap();

    // the recovery line alone brings signing authority back, and no inventory
    t.store(T0 + 3600, Ordering::SeqCst);
    let mut back = node("desktop", &t, &Mem::default());
    back.restore_identity(&line, "mira").unwrap();
    let alone = back.preview_bulk_deletion().unwrap();
    assert_eq!((alone.count, alone.inventory_complete), (0, false), "nothing is claimed for history this device never saw");
    back.join(&robin.invite(&c).unwrap().web).unwrap();
    pair_now(&mut home, &mut back, "home", "back");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira), ("back", &mut back)]);

    // once the rotation is known, the lost laptop's old device key signs nothing that counts
    t.store(T0 + 3700, Ordering::SeqCst);
    let fresh = post(&mut back, &c, "Mira, from the new laptop");
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira), ("back", &mut back)]);
    t.store(T0 + 3800, Ordering::SeqCst);
    let stale = mira.preview_bulk_deletion().unwrap();
    mira.confirm_bulk_deletion(&stale.snapshot).unwrap();
    rounds(&mut mira, 1);
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira), ("back", &mut back)]);
    assert!(robin.store.contains(&id(&first.id)), "an old signing key alone deletes nothing");
    assert!(back.store.contains(&id(&first.id)));

    // the post comes back from the network; the lost laptop's piece authority does not
    attach(&mut back, &c, &first.id, &manifest());
    let preview = back.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), BTreeSet::from([first.id.clone(), fresh.id.clone()]), "made under the replaced root, still the same person");
    assert!(preview.gaps.iter().any(|g| g.reason == "capability-missing" && g.target.as_deref() == Some(first.id.as_str())), "{:?}", preview.gaps);
    back.confirm_bulk_deletion(&preview.snapshot).unwrap();
    rounds(&mut back, 1);
    assert_eq!(batch(&back).signed, 2);
    pump_star(&mut home, &mut [("robin", &mut robin), ("mira", &mut mira), ("back", &mut back)]);
    assert!(robin.state.skeletons.contains_key(&id(&first.id)), "the restored identity's deletion validates for the old root's post");
    let progress = back.deletion_progress().into_iter().find(|p| p.target == first.id).unwrap();
    assert!(progress.coverage_gaps > 0, "missing authority stays a gap");
    assert_eq!(progress.acknowledged, 0);
}

#[test]
fn unavailable_private_state_and_pointer_only_posts_are_gaps() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut n = node("desktop", &t, &Mem::default());
    let me = n.create_identity("robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let cid = id(&c);
    let p = post(&mut n, &c, "in a group whose keys are gone");
    // the device knows the community is private but holds no usable group state for it
    n.state.private_communities.insert(cid);
    // and a post of the person's that it knows only by its pointer
    let pointer_only = Id([42; 32]);
    n.scatter.standins.insert(pointer_only, crate::open::Standin { kind: hoi_types::Kind::Post, community: cid, root: id(&me.root_key) });
    let preview = n.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), BTreeSet::from([p.id.clone()]), "a pointer is not enough for ordinary deletion");
    assert!(preview.gaps.contains(&BulkDeletionGap { reason: "private-state-unavailable".into(), community: Some(c.clone()), target: None, count: 1 }));
    assert!(preview.gaps.contains(&BulkDeletionGap { reason: "record-absent".into(), community: Some(c.clone()), target: Some(pointer_only.hex()), count: 1 }));
    // someone else's pointer is not the person's gap
    n.scatter.standins.insert(Id([43; 32]), crate::open::Standin { kind: hoi_types::Kind::Post, community: cid, root: Id([7; 32]) });
    assert_eq!(n.preview_bulk_deletion().unwrap().gaps, preview.gaps);
}

#[test]
fn a_browser_that_signs_through_its_computer_waits_for_each_signature() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("node", &t, &Mem::default());
    home.set_serving(Some("ws://127.0.0.1:7842")).unwrap();
    let mut desk = node("desktop", &t, &Mem::default());
    let mut browser = node("web", &t, &Mem::default());
    desk.create_identity("lena").unwrap();
    let c = desk.create_community(draft("privacy")).unwrap();
    let start = post(&mut desk, &c.id, "Start here");
    desk.pair(&home.net_status().serving.unwrap().pairing).unwrap();
    desk.net_connect("home", Role::Home);
    home.net_connect("d", Role::Client);
    pump_star(&mut home, &mut [("d", &mut desk)]);
    browser.pair(&desk.net_status().home_node.unwrap().pairing_web).unwrap();
    browser.net_connect("home", Role::Home);
    home.net_connect("b", Role::Client);
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    browser.link_home_remote("Work browser").unwrap();
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    let asks = link_asks(&mut desk);
    desk.link_answer(asks[0].id, true).unwrap();
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    assert!(browser.me().unwrap().remote);
    let d = browser.draft_post(&PostDraft { community: c.id.clone(), title: "From the browser".into(), text: "hi".into(), link: None, media: None }).unwrap();
    let nonce = browser.call("mine", serde_json::json!([d.prefix, d.suffix, d.bits])).unwrap().as_u64().unwrap();
    let Published::Signing(from_browser) = browser.publish_draft(d.token, nonce).unwrap() else { panic!("sent to be signed") };
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);

    let preview = browser.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), BTreeSet::from([start.id.clone(), from_browser.clone()]));
    browser.confirm_bulk_deletion(&preview.snapshot).unwrap();
    rounds(&mut browser, 1);
    let waiting = batch(&browser);
    assert_eq!((waiting.queued, waiting.signed), (2, 0), "asked, not yet signed");
    assert!(waiting.targets.iter().all(|t| t.reason.as_deref() == Some("awaiting-signature")));
    rounds(&mut browser, 2);
    assert!(deletions_by_target(&browser).is_empty(), "no second request while the first is pending");
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    rounds(&mut browser, 1);
    let done = batch(&browser);
    assert_eq!(done.signed, 2);
    pump_star(&mut home, &mut [("d", &mut desk), ("b", &mut browser)]);
    for target in [&start.id, &from_browser] {
        assert_eq!(deletions_by_target(&desk).get(&id(target)), Some(&1), "the computer signed each once");
    }
}

#[test]
fn a_local_wipe_loses_batches_and_claims_nothing() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("desktop", &t, &memory);
    n.create_identity("robin").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    post(&mut n, &c, "to be deleted");
    let preview = n.preview_bulk_deletion().unwrap();
    n.confirm_bulk_deletion(&preview.snapshot).unwrap();
    assert!(memory.0.lock().unwrap().1.contains_key(crate::bulk::BLOB));
    n.wipe().unwrap();
    n.commit().unwrap();
    assert!(n.bulk_deletions().is_empty() && n.deletion_progress().is_empty(), "lost with the device's data, never reported as finished");
    assert!(!memory.0.lock().unwrap().1.contains_key(crate::bulk::BLOB));
    drop(n);
    assert!(reopen("desktop", &t, &memory).bulk_deletions().is_empty());
}

#[test]
fn previous_storage_opens_without_batches_and_altered_batches_fail_closed() {
    const { assert!(crate::state::STORAGE_FORMAT_VERSION >= 5, "format-4 binaries must refuse storage that may hold batches") };
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut n = node("web", &t, &memory);
    n.create_identity("lena").unwrap();
    let c = n.create_community(draft("garden")).unwrap().id;
    let early = post(&mut n, &c, "deleted before the upgrade");
    let later = post(&mut n, &c, "deleted after the upgrade");
    n.delete_own(&early.id).unwrap();
    n.commit().unwrap();
    drop(n);
    // what the previous version wrote: format 4, a deletion journal, no batches
    memory.0.lock().unwrap().1.insert("format".into(), br#"{"version":4}"#.to_vec());
    assert!(!memory.0.lock().unwrap().1.contains_key(crate::bulk::BLOB));
    let mut n = reopen("web", &t, &memory);
    assert!(n.bulk_deletions().is_empty(), "old installations start with no historical batches");
    assert_eq!(n.deletion_progress().len(), 1, "the existing deletion journal is kept");
    let format: serde_json::Value = serde_json::from_slice(&memory.0.lock().unwrap().1["format"]).unwrap();
    assert_eq!(format["version"], crate::state::STORAGE_FORMAT_VERSION);
    let preview = n.preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), BTreeSet::from([later.id.clone()]));
    let status = n.confirm_bulk_deletion(&preview.snapshot).unwrap();
    drop(n);
    let resumed = batch(&reopen("web", &t, &memory));
    assert_eq!((resumed.snapshot, resumed.count, resumed.signed), (status.snapshot, 1, 1), "stored, then resumed when opened");

    let saved = memory.0.lock().unwrap().1[crate::bulk::BLOB].clone();
    let mut added: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    let targets = added["batches"].as_object_mut().unwrap().values_mut().next().unwrap()["targets"].as_object_mut().unwrap();
    let one = targets.values().next().unwrap().clone();
    targets.insert(Id([7; 32]).hex(), one);
    let mut future: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    future["version"] = 2.into();
    for damaged in [serde_json::to_vec(&added).unwrap(), serde_json::to_vec(&future).unwrap(), b"{".to_vec()] {
        memory.0.lock().unwrap().1.insert(crate::bulk::BLOB.into(), damaged);
        let before = memory.0.lock().unwrap().clone();
        let clock = t.clone();
        let error = match Node::open(Box::new(memory.clone()), "web", "https://hoi.sh/app/", Box::new(move || clock.load(Ordering::SeqCst))) {
            Ok(_) => panic!("an altered batch must not resume"),
            Err(error) => error,
        };
        assert_eq!(error.code, "storage-incompatible");
        assert!(*memory.0.lock().unwrap() == before, "a refused open changes nothing stored");
    }
}

#[test]
fn holders_drop_the_known_pieces_and_never_learn_who_asked() {
    use crate::tests::forward::{group, kept_pieces, mesh};
    let mut m = mesh(3, 14);
    let c = group(&mut m);
    let mine: Vec<PostSummary> = (0..2).map(|i| post(m.n("b"), &c, &format!("b's post {i}"))).collect();
    let theirs = post(m.n("a"), &c, "a's post stays");
    m.run(120);
    // (placed by this device, placed by someone else) for a post, as `who` knows them
    let placements = |m: &mut Mesh, who: &str, p: &PostSummary| {
        let pid = id(&p.id);
        let kept: Vec<_> = m.n(who).scatter.saved.kept.values().filter(|k| k.records == vec![pid]).cloned().collect();
        let own = kept.iter().filter(|k| k.mine).filter_map(|k| k.newest()).collect::<Vec<_>>();
        let other = kept.iter().filter(|k| !k.mine).filter_map(|k| k.newest()).collect::<Vec<_>>();
        (own, other)
    };
    let mut own = Vec::new();
    let mut expected_gaps = Vec::new();
    for p in &mine {
        let (placed, carried) = placements(&mut m, "b", p);
        assert_eq!(placed.len(), 1);
        own.extend(placed);
        // the group's other member carries it in an object of its own: its authority, not ours
        if !carried.is_empty() {
            expected_gaps.push(BulkDeletionGap {
                reason: "capability-missing".into(),
                community: Some(c.clone()),
                target: Some(p.id.clone()),
                count: carried.len() as u32,
            });
        }
    }
    let (theirs_placed, _) = placements(&mut m, "a", &theirs);
    assert!(own.iter().chain(&theirs_placed).all(|man| kept_pieces(&mut m, man) == man.hashes.len()));

    let preview = m.n("b").preview_bulk_deletion().unwrap();
    assert_eq!(listed(&preview), mine.iter().map(|p| p.id.clone()).collect());
    assert_eq!(preview.capabilities, 2, "this device placed both, with its own drop authority");
    let mut gaps: Vec<_> = preview.gaps.iter().filter(|g| g.reason == "capability-missing").cloned().collect();
    gaps.sort_by(|x, y| x.target.cmp(&y.target));
    expected_gaps.sort_by(|x, y| x.target.cmp(&y.target));
    assert_eq!(gaps, expected_gaps, "pieces only another member can drop are reported, not claimed");
    m.log.clear();
    m.n("b").confirm_bulk_deletion(&preview.snapshot).unwrap();
    m.run(300);
    assert_eq!(m.n("b").bulk_deletions()[0].signed, 2);
    for man in &own {
        assert_eq!(kept_pieces(&mut m, man), 0, "every piece this device could drop was dropped");
    }
    for man in &theirs_placed {
        assert_eq!(kept_pieces(&mut m, man), man.hashes.len(), "another author's pieces stay");
    }
    for p in &mine {
        assert!(m.n("a").state.skeletons.contains_key(&id(&p.id)), "the group applied each deletion");
    }
    assert!(m.n("a").store.contains(&id(&theirs.id)));

    // holders act on locators and authority; nothing they receive or keep names the person
    let root = id(&m.n("b").me().unwrap().root_key);
    let community = id(&c);
    let to_holders: Vec<_> = m.log.iter().filter(|(_, to, _)| to.starts_with('h')).collect();
    assert!(to_holders.iter().any(|(_, _, msg)| matches!(msg, hoi_sync::Msg::Piece { .. })));
    for (_, _, msg) in to_holders {
        let bytes = msg.encode();
        assert!(!bytes.windows(32).any(|w| w == root.0 || w == community.0), "a holder saw the author or the community");
    }
    let holders: Vec<String> = m.nodes.keys().filter(|n| n.starts_with('h')).cloned().collect();
    for h in holders {
        assert!(!m.n(&h).store.all().any(|s| s.author_root == root || s.record.community == Some(community)));
    }
}

/// A computer with a community whose posts last as long as a blessing, and a linked browser
/// that synced with it.
fn linked_pair(t: &Arc<AtomicU64>) -> (Node, Node, String) {
    let mut desk = node("desktop", t, &Mem::default());
    let mut web = node("web", t, &Mem::default());
    desk.create_identity("mira").unwrap();
    let mut long = draft("garden");
    long.limits.as_mut().unwrap().max_retention = Some(crate::api::DELEGATION_LIFETIME);
    let c = desk.create_community(long).unwrap().id;
    let request = web.link_request("mira's browser").unwrap();
    let approval = desk.link_approve(&request).unwrap();
    web.link_complete(&approval).unwrap();
    let pairing = desk.set_serving(Some("ws://127.0.0.1:7842")).unwrap().serving.unwrap().pairing;
    web.pair(&pairing).unwrap();
    web.net_connect("a", Role::Home);
    desk.net_connect("b", Role::Client);
    pump(&mut desk, &mut web);
    (desk, web, c)
}

#[test]
fn a_revoked_device_fails_its_waiting_targets() {
    let t = Arc::new(AtomicU64::new(T0));
    let (mut desk, mut web, c) = linked_pair(&t);
    let p = post(&mut desk, &c, "posted on the computer");
    pump(&mut desk, &mut web);
    let preview = web.preview_bulk_deletion().unwrap();
    web.confirm_bulk_deletion(&preview.snapshot).unwrap();
    let device = web.me().unwrap().device;
    desk.revoke_device(&device).unwrap();
    pump(&mut desk, &mut web);
    t.fetch_add(1, Ordering::SeqCst);
    rounds(&mut web, 1);
    let status = batch(&web);
    assert_eq!((status.failed, status.signed), (1, 0));
    assert_eq!(status.targets[0].reason.as_deref(), Some("not-allowed"), "a changed permission is a failure, never a deletion");
    assert!(deletions_by_target(&web).is_empty());
    assert!(desk.store.contains(&id(&p.id)));
}

#[test]
fn a_lapsed_blessing_waits_instead_of_failing() {
    let t = Arc::new(AtomicU64::new(T0));
    let (_desk, mut web, c) = linked_pair(&t);
    t.store(T0 + 1000, Ordering::SeqCst);
    let p = post(&mut web, &c, "outlives the browser's blessing");
    let preview = web.preview_bulk_deletion().unwrap();
    web.confirm_bulk_deletion(&preview.snapshot).unwrap();
    t.store(T0 + crate::api::DELEGATION_LIFETIME + 10, Ordering::SeqCst);
    web.sweep();
    rounds(&mut web, 2);
    let status = batch(&web);
    assert_eq!((status.selected, status.failed, status.signed), (1, 0, 0), "waits for a renewed blessing");
    assert!(web.store.contains(&id(&p.id)));
}
