use super::*;
use std::time::Duration;

#[tokio::test]
async fn replacing_a_missing_notice_updates_the_published_hash() {
    let dir = std::env::temp_dir().join(format!("hoi-notice-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
    tokio::fs::create_dir(&dir).await.unwrap();
    let path = dir.join("notice.json");
    let published = dir.join(NOTICE_HASH_FILE);
    let mut node = Node::open(Box::new(hoi_store::Ephemeral), "node", "https://example.invalid/", hoi_core::system_clock()).unwrap();
    let mut notice = hoi_types::NodeNotice { updated: "2026-01-01".into(), ..hoi_types::NodeNotice::template("Home") };
    let old = node.set_notice(Some(notice.clone())).unwrap().unwrap();
    let shared = Shared::new(node, Box::new(|_| {}));
    let update = async {
        while tokio::fs::read_to_string(&published).await.unwrap_or_default().trim() != old {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        notice.operator = "New operator".into();
        tokio::fs::write(&path, serde_json::to_vec(&notice).unwrap()).await.unwrap();
        loop {
            let observed = tokio::fs::read_to_string(&published).await.unwrap();
            let expected = shared.with(|n| n.operator_notice_hash()).unwrap();
            if expected != old && observed.trim() == expected {
                assert_eq!(shared.with(|n| n.operator_notice().unwrap().operator.clone()), "New operator");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let poll = tokio::time::interval(Duration::from_millis(10));
    tokio::select! {
        _ = refresh_notice(shared.clone(), path.clone(), published.clone(), old.clone(), poll) => panic!("notice watcher stopped"),
        result = tokio::time::timeout(Duration::from_secs(5), update) => result.expect("the replacement notice should be published"),
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[tokio::test]
async fn restarting_a_home_node_replaces_stale_pairing_and_public_links() {
    let dir = std::env::temp_dir().join(format!("hoi-restart-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
    tokio::fs::create_dir(&dir).await.unwrap();
    let pairing = dir.join(PAIRING_FILE);
    let public = dir.join(NODE_LINK_FILE);
    tokio::fs::write(&public, "previous public link").await.unwrap();
    let mut node = Node::open(Box::new(hoi_store::Ephemeral), "node", "https://example.invalid/", hoi_core::system_clock()).unwrap();
    node.create_identity("Home").unwrap();
    let old = pairing_text(&node.set_serving(Some("ws://127.0.0.1:7842")).unwrap()).unwrap();
    write_pairing_file(pairing.clone(), old.clone()).await.unwrap();
    let expected = pairing_text(&node.rotate_pairing().unwrap()).unwrap();
    assert!(expected != old);
    let shared = Shared::new(node, Box::new(|_| {}));
    let updated = async {
        while public.exists() || tokio::fs::read_to_string(&pairing).await.unwrap() != expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(tokio::fs::metadata(&pairing).await.unwrap().permissions().mode() & 0o077, 0);
        }
    };
    tokio::select! {
        _ = refresh_pairing_file(shared.clone(), pairing.clone(), old) => panic!("pairing watcher stopped"),
        _ = refresh_node_link(shared, public.clone()) => panic!("node link watcher stopped"),
        result = tokio::time::timeout(Duration::from_secs(5), updated) => result.expect("stale files should be replaced"),
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
