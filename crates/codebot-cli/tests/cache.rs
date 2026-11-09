use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
#[test]
fn cache_inventory_preview_decline_and_confirmed_deletion_preserve_unselected_files() {
    let dir = std::env::temp_dir().join(format!(
        "codebot-cache-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let id = format!("{}.gguf", "a".repeat(64));
    fs::write(dir.join(&id), b"model fixture").unwrap();
    fs::write(dir.join("keep.txt"), b"keep").unwrap();
    let run = |action: &[&str], input: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
            .args(["--cache-dir", dir.to_str().unwrap()])
            .args(action)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let listed = run(&["--cache-list"], "");
    assert!(listed.status.success());
    let json: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(json["entries"][0]["id"], id);
    assert_eq!(json["total_bytes"], 13);
    assert_eq!(json["entries"][0]["file_names"][0], id);
    assert!(run(&["--cache-delete", &id, "--cache-preview"], "")
        .status
        .success());
    assert!(dir.join(&id).exists());
    assert!(run(&["--cache-delete", &id], "no\n").status.success());
    assert!(dir.join(&id).exists());
    assert!(run(&["--cache-delete", &id], "delete\n").status.success());
    assert!(!dir.join(&id).exists());
    assert_eq!(fs::read(dir.join("keep.txt")).unwrap(), b"keep");
    assert!(!run(&["--cache-delete", "../keep.txt", "--yes"], "")
        .status
        .success());
    fs::remove_dir_all(dir).unwrap();
}
