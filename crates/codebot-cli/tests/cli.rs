use std::io::Write;
use std::process::{Command, Stdio};
#[test]
fn argument_success_and_empty_failure() {
    let out = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .args([
            "--config",
            "../../config/config.example.toml",
            "Permission denied",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("所有者"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("Permission denied"));
    let out = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .arg("")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}
#[test]
fn stdin_and_config_error() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"file not found")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("PATH"));
    let out = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .args(["--config", "missing-file.toml", "test"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}
