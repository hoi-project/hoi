use super::*;
use crate::tests::forward::{Mesh, join, mesh, passed_through, stored};

fn sampled_store(vouchers: [u8; 14]) -> (Mesh, Manifest, Vec<Id>) {
    let mut mesh = mesh(3, 14);
    join(&mut mesh, "a", "198.51.100.10:7843", &[0, 1, 2]);
    let ids: Vec<_> = (0..14).map(|i| Id::from_hex(&mesh.eps[&format!("h{i}")]).unwrap()).collect();
    for (place, name) in ["f0", "f1", "f2"].into_iter().enumerate() {
        let allowed: BTreeSet<_> = ids.iter().zip(vouchers).filter(|(_, mask)| mask & (1 << place) != 0).map(|(id, _)| *id).collect();
        mesh.n("a").peers.get_mut(name).unwrap().scatter.vouches.retain(|v| allowed.contains(&v.endpoint));
    }
    let plan = mesh.n("a").plan();
    assert_eq!((plan.places(), plan.forwarder_places()), (14, 3));
    assert!(plan.supports_puts(5, 12));
    let object = sc::new_object_id().unwrap();
    let key = sc::new_secret().unwrap();
    let (class, sealed) = sc::seal_object(b"flexible holders must leave room for restricted holders", &object, &key).unwrap();
    let shards = Shards::new(&sealed, class, 5).unwrap();
    // Supply a particular possible random holder sample, preserving its piece order. The
    // rest of the path uses the real signed vouches, encrypted PUTs and holder receipts.
    let manifest = Manifest {
        object,
        key,
        class,
        sealed: sc::sealed_hash(&sealed),
        generation: 0,
        level: 10,
        places: 14,
        k: 5,
        n: 12,
        m: 14,
        hashes: (0..12).map(|i| sc::piece_hash(&object, 0, i, &shards.piece(i))).collect(),
        holders: ids[..12].to_vec(),
        keys: Vec::new(),
        loc_secret: sc::new_secret().unwrap(),
        expires_at: mesh.n("a").now() + DAY,
    };
    mesh.log.clear();
    let op = mesh.n("a").begin_store(manifest, shards, 0, plan, 5).unwrap();
    let result = stored(mesh.wait("a", op, 60));
    assert_eq!(passed_through(&mesh, PieceOp::Put), [("f0".into(), 4), ("f1".into(), 4), ("f2".into(), 4)].into());
    (mesh, result, ids)
}

#[test]
fn flexible_first_puts_leave_room_for_holders_with_one_voucher() {
    let (mut mesh, manifest, ids) = sampled_store([3, 3, 3, 3, 1, 1, 1, 1, 4, 4, 4, 4, 1, 1]);
    assert_eq!(manifest.holders, ids[..12], "retain the sampled holders and their index order");
    for (i, _) in ids.iter().enumerate() {
        assert_eq!(mesh.n(&format!("h{i}")).piece_stats().unwrap().count, u64::from(i < 12));
    }
}

#[test]
fn only_an_unroutable_unsent_index_uses_a_diverse_reserve_holder() {
    // Five selected holders need F0 and only three use F1. Of the two reserve
    // holders, only h12 can absorb the remaining index through F1.
    let (mut mesh, manifest, ids) = sampled_store([1, 1, 1, 1, 1, 2, 2, 2, 4, 4, 4, 4, 2, 1]);
    let mut expected = ids[..12].to_vec();
    expected[4] = ids[12];
    assert_eq!(manifest.holders, expected);
    assert_eq!(mesh.n("h4").piece_stats().unwrap().count, 0);
    assert_eq!(mesh.n("h12").piece_stats().unwrap().count, 1);
    assert_eq!(mesh.n("h13").piece_stats().unwrap().count, 0);
}
