use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[test]
fn tool_actions_and_result_transmission_require_separate_confirmation() {
    for mode in [
        "decline",
        "run-no-send",
        "run-send",
        "change-reject",
        "change-apply",
    ] {
        let root = std::env::temp_dir().join(format!(
            "codebot-tools-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("work");
        fs::create_dir_all(&workspace).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let change = mode.starts_with("change-");
        let sent = mode == "run-send" || mode == "change-apply";
        let (name, arguments) = if change {
            (
                "codebot_propose_changes",
                json!({"proposal":{"version":1,"changes":[{"operation":"create","path":"tool-change.txt","content":"approved change\n"}]}}),
            )
        } else {
            (
                "codebot_run_command",
                json!({"program":"python","args":["-c","from pathlib import Path; Path('tool-ran.txt').write_text('ran'); print('tool-ready')"],"timeout_seconds":5}),
            )
        };
        let worker = thread::spawn(move || {
            for index in 0..if sent { 2 } else { 1 } {
                let deadline = Instant::now() + Duration::from_secs(8);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("missing request: {e}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut raw = Vec::new();
                let mut one = [0];
                while !raw.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut one).unwrap();
                    raw.push(one[0]);
                }
                let header = String::from_utf8(raw).unwrap();
                assert!(header.starts_with("POST /v1/responses "));
                let len: usize = header
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = vec![0; len];
                socket.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body["store"], false);
                if index == 1 {
                    let output = body["input"][2]["output"].as_str().unwrap();
                    assert!(output.contains("succeeded"));
                    if !change {
                        assert!(output.contains("tool-ready"));
                    }
                }
                let response = if index == 0 {
                    json!({"status":"completed","output":[{"type":"function_call","id":"fc-fixture","call_id":"call-fixture","name":name,"arguments":arguments.to_string()}]})
                } else {
                    json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Completed fixture review."}]}]})
                };
                let bytes = serde_json::to_vec(&response).unwrap();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
                socket.write_all(&bytes).unwrap();
            }
            // No follow-up generation happens when the user declines result send.
            thread::sleep(Duration::from_millis(100));
            assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
        });
        let config = root.join("api.toml");
        fs::write(&config,format!("provider='custom'\n[api]\nbase_url='http://{address}/v1'\nmodel='fixture'\nprotocol='responses'\nanonymous=true\n")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
            .args(["--tools", "--config"])
            .arg(&config)
            .arg("--workspace")
            .arg(&workspace)
            .arg("--tools-journal-dir")
            .arg(root.join("journals"))
            .arg("Propose one reviewed operation")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = match mode {
            "decline" | "change-reject" => "no\nno\n",
            "run-no-send" => "run\nno\n",
            "run-send" => "run\nsend\n",
            _ => "apply\nsend\n",
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        worker.join().unwrap();
        assert_eq!(
            workspace.join("tool-ran.txt").exists(),
            mode.starts_with("run-")
        );
        assert_eq!(
            workspace.join("tool-change.txt").exists(),
            mode == "change-apply"
        );
        if mode == "change-apply" {
            assert_eq!(fs::read_dir(root.join("journals")).unwrap().count(), 1);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
