use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[test]
fn interactive_history_trim_clear_and_failed_turns_reach_the_server_correctly() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut requests = Vec::new();
        while requests.len() < 6 {
            let (mut socket, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "client did not send all six turns"
                    );
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let size = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|size| size.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + size {
                        requests.push(
                            String::from_utf8(bytes[end + 4..end + 4 + size].to_vec()).unwrap(),
                        );
                        break;
                    }
                }
            }
            let index = requests.len();
            let (status, body) = if index == 4 {
                ("503 Service Unavailable", "private error body".to_owned())
            } else {
                (
                    "200 OK",
                    format!(r#"{{"choices":[{{"message":{{"content":"answer-{index}"}}}}]}}"#),
                )
            };
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let path = std::env::temp_dir().join(format!(
        "codebot-interactive-{}-{}.toml",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, format!("provider='custom'\n[api]\nmodel='test'\nbase_url='http://{address}/v1'\nanonymous=true\ntimeout_seconds=3\n")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .args(["--interactive", "--config"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"first\nsecond\n/trim\nthird\nfail\nfourth\n/clear\nfifth\n/quit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let requests = server.join().unwrap();
    assert!(!requests[0].contains("answer-"));
    assert!(requests[1].contains("first") && requests[1].contains("answer-1"));
    assert!(!requests[2].contains("first") && requests[2].contains("second"));
    assert!(!requests[4].contains("fail") && requests[4].contains("third"));
    assert!(!requests[5].contains("fourth") && requests[5].contains("fifth"));
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostics.contains("503"));
    assert!(!diagnostics.contains("private error body"));
}
