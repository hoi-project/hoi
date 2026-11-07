use super::*;

#[test]
fn malformed_or_future_local_metadata_never_overwrites_existing_storage() {
    let t = Arc::new(AtomicU64::new(T0));
    let cases: &[(&str, &[u8])] = &[
        ("state", b"{"),
        ("state", b"null"),
        ("state", br#"{"future_identity_model":true}"#),
        ("keys", b"{"),
        ("keys", br#"{"root":"not-a-key"}"#),
        ("keys", br#"{"rtc_cert":["bad!","bad!"]}"#),
        ("keys", br#"{"future_key_model":true}"#),
        ("format", br#"{"version":4294967295}"#),
        ("format", br#"{"version":0}"#),
        ("format", br#"{"version":"1"}"#),
        ("format", br#"{"version":1,"future":true}"#),
    ];
    for (blob, damaged) in cases {
        let memory = Mem::default();
        let mut original = node("web", &t, &memory);
        original.create_identity("Lena").unwrap();
        let community = original.create_community(draft("garden")).unwrap();
        post(&mut original, &community.id, "Keep this even if a later app cannot read the state");
        original.commit().unwrap();
        drop(original);
        memory.0.lock().unwrap().1.insert((*blob).into(), damaged.to_vec());
        let before = memory.0.lock().unwrap().clone();
        let clock = t.clone();
        let error = match Node::open(Box::new(memory.clone()), "web", "https://hoi.sh/app/", Box::new(move || clock.load(Ordering::SeqCst))) {
            Ok(_) => panic!("must refuse incompatible {blob}"),
            Err(error) => error,
        };
        assert_eq!(error.code, "storage-incompatible");
        assert!(error.message.contains("No stored data was changed"));
        assert!(error.message.contains("compatible software"));
        assert!(*memory.0.lock().unwrap() == before, "failed open must leave every original record and blob intact");
    }
}

#[test]
fn unversioned_legacy_storage_is_read_without_replacing_keys_and_gains_a_guard() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut original = node("web", &t, &memory);
    let person = original.create_identity("Lena").unwrap();
    original.commit().unwrap();
    drop(original);
    let keys = memory.0.lock().unwrap().1["keys"].clone();
    memory.0.lock().unwrap().1.remove("format");
    let reopened = node("web", &t, &memory);
    assert_eq!(reopened.me().unwrap().root_key, person.root_key);
    assert_eq!(memory.0.lock().unwrap().1["keys"], keys);
    let format: serde_json::Value = serde_json::from_slice(&memory.0.lock().unwrap().1["format"]).unwrap();
    assert_eq!(format["version"], crate::state::STORAGE_FORMAT_VERSION);
}

#[test]
fn version_one_public_storage_upgrades_without_replacing_identity_or_history() {
    let t = Arc::new(AtomicU64::new(T0));
    let memory = Mem::default();
    let mut original = node("web", &t, &memory);
    let person = original.create_identity("Lena").unwrap();
    let community = original.create_community(draft("garden")).unwrap();
    let kept = post(&mut original, &community.id, "Keep my existing public history");
    original.commit().unwrap();
    drop(original);
    let keys = memory.0.lock().unwrap().1["keys"].clone();
    memory.0.lock().unwrap().1.insert("format".into(), br#"{"version":1}"#.to_vec());
    // Emulate a store written before the private state fields existed.
    let state_bytes = memory.0.lock().unwrap().1["state"].clone();
    let mut state: serde_json::Value = serde_json::from_slice(&state_bytes).unwrap();
    state.as_object_mut().unwrap().remove("private_communities");
    state.as_object_mut().unwrap().remove("private_carriers");
    state.as_object_mut().unwrap().remove("private_blobs");
    memory.0.lock().unwrap().1.insert("state".into(), serde_json::to_vec(&state).unwrap());
    let reopened = node("web", &t, &memory);
    assert_eq!(reopened.me().unwrap().root_key, person.root_key);
    assert!(reopened.store.contains(&Id::from_hex(&kept.id).unwrap()));
    assert_eq!(memory.0.lock().unwrap().1["keys"], keys);
    let format: serde_json::Value = serde_json::from_slice(&memory.0.lock().unwrap().1["format"]).unwrap();
    assert_eq!(format["version"], crate::state::STORAGE_FORMAT_VERSION);
    const { assert!(crate::state::STORAGE_FORMAT_VERSION >= 2, "old format-1 binaries must reject private-capable storage") };
}
