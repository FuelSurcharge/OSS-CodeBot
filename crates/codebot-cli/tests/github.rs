use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
#[test]
fn github_post_preview_decline_and_cross_process_duplicate_protection() {
    let directory = std::env::temp_dir().join(format!(
        "codebot-github-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let ledger = directory.join("records");
    let payload = directory.join("post.json");
    std::fs::write(
        &payload,
        r#"{"operation":"create-issue","title":"CLI fixture","body":"private-body-fixture"}"#,
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let invoke = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
            .args([
                "--github-repository",
                "owner/repo",
                "--github-api-url",
                &base,
                "--github-token-env",
                "CODEBOT_CLI_GITHUB_TOKEN",
                "--github-ledger-dir",
            ])
            .arg(&ledger)
            .arg("--github-write")
            .arg(&payload)
            .args(extra)
            .env("CODEBOT_CLI_GITHUB_TOKEN", "fixture-private-key")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    assert!(invoke(&["--github-preview"]).status.success());
    assert!(!ledger.exists());
    assert!(invoke(&[]).status.success());
    assert!(!ledger.exists());
    listener.set_nonblocking(true).unwrap();
    assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
    listener.set_nonblocking(false).unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let header = String::from_utf8(header).unwrap();
        assert!(header.starts_with("POST /repos/owner/repo/issues "));
        let length: usize = header
            .lines()
            .find_map(|s| {
                s.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|s| s.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["title"], "CLI fixture");
        assert_eq!(value["body"], "private-body-fixture");
        let body = r#"{"number":77}"#;
        write!(socket,"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let first = invoke(&["--yes"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("77"));
    server.join().unwrap();
    let duplicate = invoke(&["--yes"]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("同じ内容"));
    for entry in std::fs::read_dir(&ledger).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(!text.contains("fixture-private-key"));
        assert!(!text.contains("private-body-fixture"));
    }
    let history = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .args([
            "--github-repository",
            "owner/repo",
            "--github-history",
            "--github-ledger-dir",
        ])
        .arg(&ledger)
        .output()
        .unwrap();
    assert!(history.status.success());
    assert!(String::from_utf8_lossy(&history.stdout).contains("confirmed"));
    std::fs::remove_dir_all(directory).unwrap();
}
