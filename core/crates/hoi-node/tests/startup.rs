use std::process::Command;

#[test]
fn startup_refuses_an_invalid_operator_notice_before_serving() {
    let dir = std::env::temp_dir().join(format!("hoi-startup-{}", hoi_types::hex_encode(&hoi_crypto::random_bytes::<8>().unwrap())));
    std::fs::create_dir(&dir).unwrap();
    let notice = dir.join("notice.json");
    std::fs::write(&notice, "invalid JSON").unwrap();
    let data = dir.join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_hoi-node")).arg("--data").arg(&data).arg("--notice").arg(&notice).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a notice file (JSON)"));
    assert!(data.is_dir());
    assert!(!data.join("pairing-link").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
