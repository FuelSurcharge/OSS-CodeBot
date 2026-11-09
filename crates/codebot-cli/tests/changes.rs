use std::{
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn changes_require_apply_and_confirmation_and_can_be_undone() {
    let directory = std::env::temp_dir().join(format!(
        "codebot-cli-changes-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = directory.join("project");
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("source.rs");
    std::fs::write(&file, "before-a\nmiddle\nbefore-c\n").unwrap();
    let hash = codebot_core::Workspace::open(&root)
        .unwrap()
        .read(std::path::Path::new("source.rs"), 1, None)
        .unwrap()
        .sha256;
    let proposal = directory.join("proposal.json");
    std::fs::write(&proposal,format!(r#"{{"version":1,"changes":[{{"operation":"replace","path":"source.rs","expected_sha256":"{hash}","edits":[{{"first_line":1,"last_line":1,"content":"after-a"}},{{"first_line":3,"last_line":3,"content":"after-c"}}]}}]}}"#)).unwrap();
    let journal = directory.join("changes.json");
    let invoke = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
            .arg("--workspace")
            .arg(&root)
            .arg("--proposal")
            .arg(&proposal)
            .args(["--select", "1:2"])
            .arg("--journal")
            .arg(&journal)
            .args(extra)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let preview = invoke(&[]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains("+after-c"));
    assert!(!String::from_utf8_lossy(&preview.stdout).contains("+after-a"));
    assert!(!journal.exists());
    assert!(invoke(&["--apply"]).status.success());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "before-a\nmiddle\nbefore-c\n"
    );
    assert!(!journal.exists());
    assert!(invoke(&["--apply", "--yes"]).status.success());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "before-a\nmiddle\nafter-c\n"
    );
    assert!(journal.exists());
    let undo = Command::new(env!("CARGO_BIN_EXE_codebot-cli"))
        .arg("--workspace")
        .arg(&root)
        .arg("--undo")
        .arg(&journal)
        .arg("--yes")
        .output()
        .unwrap();
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "before-a\nmiddle\nbefore-c\n"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
