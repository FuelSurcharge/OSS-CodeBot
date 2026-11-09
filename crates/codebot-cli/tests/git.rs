use std::{
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
#[test]
fn git_status_preview_and_confirmation_preserve_the_worktree_and_stage_only_selected_paths() {
    let directory = std::env::temp_dir().join(format!(
        "codebot-git-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let root = directory.join("project");
    std::fs::create_dir(&root).unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["init", "--initial-branch=main"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success());
    std::fs::write(root.join("selected.rs"), "selected\n").unwrap();
    std::fs::write(root.join("keep.rs"), "keep this working file\n").unwrap();
    let operation = directory.join("operation.json");
    std::fs::write(
        &operation,
        r#"{"operation":"stage","paths":["selected.rs"]}"#,
    )
    .unwrap();
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
            .arg("--workspace")
            .arg(&root)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let status = invoke(&["--git-status"]);
    assert!(status.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(parsed["branch"], "main");
    assert_eq!(parsed["unborn"], true);
    let action = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
            .arg("--workspace")
            .arg(&root)
            .arg("--git-operation")
            .arg(&operation)
            .args(extra)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    assert!(action(&["--git-preview"]).status.success());
    assert!(action(&[]).status.success());
    let status = invoke(&["--git-status"]);
    let parsed: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(parsed["staged_diff"], "");
    let accepted = action(&["--yes"]);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let status = invoke(&["--git-status"]);
    let parsed: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(parsed["staged_diff"]
        .as_str()
        .unwrap()
        .contains("+selected"));
    assert!(!parsed["staged_diff"]
        .as_str()
        .unwrap()
        .contains("keep this"));
    assert_eq!(
        std::fs::read_to_string(root.join("keep.rs")).unwrap(),
        "keep this working file\n"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
