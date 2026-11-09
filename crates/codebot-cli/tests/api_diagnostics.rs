use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn api_generation_probe_requires_confirmation_and_sends_only_the_fixed_diagnostic() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let config = std::env::temp_dir().join(format!(
        "codebot-api-probe-{}-{}.toml",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&config,format!("provider='custom'\n[api]\nbase_url='http://{address}/v1'\nmodel='test-model'\nanonymous=true\nmax_output_tokens=16\ntimeout_seconds=3\n")).unwrap();
    let declined = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .args(["--test-api", "--config"])
        .arg(&config)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(declined.status.success());
    assert!(String::from_utf8_lossy(&declined.stderr).contains("API料金"));
    listener.set_nonblocking(true).unwrap();
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
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
        assert!(header.starts_with("POST /v1/chat/completions "));
        let count: usize = header
            .lines()
            .find_map(|s| {
                s.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|s| s.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; count];
        socket.read_exact(&mut body).unwrap();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("Reply OK."));
        assert_eq!(text.matches("\"role\"").count(), 1);
        let body = r#"{"choices":[{"message":{"content":"OK"}}]}"#;
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let accepted = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .args(["--test-api", "--yes", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert!(String::from_utf8_lossy(&accepted.stdout).contains("受理"));
    server.join().unwrap();
    std::fs::remove_file(config).unwrap();
}
