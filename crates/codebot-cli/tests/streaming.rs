#[cfg(unix)]
#[test]
fn ctrl_c_cancels_streaming_and_the_next_turn_excludes_the_partial_answer() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let config = std::env::temp_dir().join(format!(
        "codebot-stream-{}-{}.toml",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&config, format!("provider = 'custom'\n[api]\nmodel='test'\nbase_url='http://{address}/v1'\nanonymous=true\ntimeout_seconds=3\n")).unwrap();
    let server = thread::spawn(move || {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).unwrap();
            let size: usize = header
                .lines()
                .find_map(|s| {
                    s.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; size];
            stream.read_exact(&mut body).unwrap();
            let expected = if index == 0 { "first" } else { "second" };
            // The cancelled first turn must not enter the next request.
            let text = String::from_utf8(body).unwrap();
            assert!(text.contains(&format!("\"content\":\"{expected}\"")));
            assert_eq!(text.matches("\"role\"").count(), 1);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
            if index == 0 {
                stream
                    .write_all(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"途中\\n\"}}]}\n\n"
                            .as_bytes(),
                    )
                    .unwrap();
                assert!(matches!(stream.read(&mut byte), Ok(0)));
            } else {
                stream.write_all("data: {\"choices\":[{\"delta\":{\"content\":\"次の回答\"}}]}\n\ndata: [DONE]\n\n".as_bytes()).unwrap();
            }
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .args(["--interactive", "--stream", "--config"])
        .arg(&config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (out_tx, out_rx) = mpsc::channel();
    let output = thread::spawn(move || {
        let mut all = String::new();
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            all.push_str(&line);
            let _ = out_tx.send(line);
        }
        all
    });
    let (err_tx, err_rx) = mpsc::channel();
    let diagnostics = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let _ = err_tx.send(line.unwrap());
        }
    });
    input.write_all(b"first\n").unwrap();
    assert_eq!(out_rx.recv_timeout(Duration::from_secs(4)).unwrap(), "途中");
    assert!(Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap()
        .success());
    loop {
        if err_rx
            .recv_timeout(Duration::from_secs(4))
            .unwrap()
            .contains("生成を停止")
        {
            break;
        }
    }
    input.write_all(b"second\n/quit\n").unwrap();
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("CLI did not finish after cancellation");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
    assert!(output.join().unwrap().contains("次の回答"));
    diagnostics.join().unwrap();
    server.join().unwrap();
    std::fs::remove_file(config).unwrap();
}
