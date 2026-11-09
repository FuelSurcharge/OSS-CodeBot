use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
#[test]
fn extension_permissions_are_reviewed_before_install_and_enable_run_disable_remove_are_persistent()
{
    let dir = std::env::temp_dir().join(format!(
        "codebot-plugins-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = dir.join("plugins");
    let workspace = dir.join("work");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("keep.rs"), "fn keep() {}\n").unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/input-lines/manifest.json");
    let source = source.to_str().unwrap();
    let run = |arguments: &[&str], input: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
            .arg("--plugin-dir")
            .arg(&root)
            .args(arguments)
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
    let preview = run(&["--plugin-install", source, "--plugin-preview"], "");
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains("read-selected-files"));
    assert!(!root.exists());
    assert!(run(&["--plugin-install", source], "no\n").status.success());
    assert!(!root.exists());
    assert!(run(&["--plugin-install", source], "install\n")
        .status
        .success());
    let listed = run(&["--plugin-list"], "");
    let value: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(value[0]["enabled"], false);
    assert!(!run(
        &[
            "--plugin-run",
            "input-lines",
            "--workspace",
            workspace.to_str().unwrap(),
            "a\nb\n"
        ],
        ""
    )
    .status
    .success());
    assert!(run(&["--plugin-enable", "input-lines"], "")
        .status
        .success());
    let output = run(
        &[
            "--plugin-run",
            "input-lines",
            "--workspace",
            workspace.to_str().unwrap(),
            "a\nb\n",
        ],
        "",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 行"));
    assert!(run(
        &[
            "--plugin-run",
            "input-lines",
            "--workspace",
            workspace.to_str().unwrap(),
            "--attach",
            "keep.rs",
            "Count selected input"
        ],
        ""
    )
    .status
    .success());
    assert!(run(&["--plugin-disable", "input-lines"], "")
        .status
        .success());
    assert!(!run(
        &[
            "--plugin-run",
            "input-lines",
            "--workspace",
            workspace.to_str().unwrap(),
            "text"
        ],
        ""
    )
    .status
    .success());
    assert!(
        run(&["--plugin-remove", "input-lines", "--plugin-preview"], "")
            .status
            .success()
    );
    assert!(root.join("input-lines").is_dir());
    assert!(run(&["--plugin-remove", "input-lines"], "no\n")
        .status
        .success());
    assert!(root.join("input-lines").is_dir());
    assert!(run(&["--plugin-remove", "input-lines"], "remove\n")
        .status
        .success());
    assert!(!root.join("input-lines").exists());
    assert_eq!(
        fs::read_to_string(workspace.join("keep.rs")).unwrap(),
        "fn keep() {}\n"
    );
    assert!(Path::new(source).is_file());
    fs::remove_dir_all(dir).unwrap();
}
