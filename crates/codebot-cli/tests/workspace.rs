use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn review_preview_includes_only_the_selected_lines_without_network() {
    let root = std::env::temp_dir().join(format!(
        "codebot-workspace-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("source.rs"),
        "not-selected-first\nselected-code\nnot-selected-last\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .arg("--workspace")
        .arg(&root)
        .args([
            "--attach",
            "source.rs",
            "--line-start",
            "2",
            "--line-end",
            "2",
            "--action",
            "review",
            "--preview",
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("selected-code") && text.contains("レビュー"));
    assert!(!text.contains("not-selected-first") && !text.contains("not-selected-last"));
    assert!(text.contains("source.rs (2-2行)"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("selected-code"));
}
