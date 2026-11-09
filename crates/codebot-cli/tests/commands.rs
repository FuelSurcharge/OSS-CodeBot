use std::{
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
#[test]
fn command_preview_and_declined_confirmation_never_execute_and_approved_argv_is_literal() {
    let directory = std::env::temp_dir().join(format!(
        "codebot-command-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let python = if cfg!(windows) { "python" } else { "python3" };
    let invoke = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_codebot-cli")).arg("--workspace").arg(&directory)
        .args(["--run-command",python,"--command-arg=-I","--command-arg=-c","--command-arg=import sys;open('executed','w').write('yes');print(sys.argv[1]);print('日本語')","--command-arg=$(touch unintended); & | >"]).args(extra).stdin(Stdio::null()).output().unwrap()
    };
    let preview = invoke(&["--command-preview"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(!directory.join("executed").exists());
    assert!(String::from_utf8_lossy(&preview.stderr).contains("environment_names"));
    assert!(invoke(&[]).status.success());
    assert!(!directory.join("executed").exists());
    let accepted = invoke(&["--yes"]);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let output = String::from_utf8_lossy(&accepted.stdout);
    assert!(output.contains("日本語"));
    assert!(output.contains("$(touch unintended); & | >"));
    assert!(directory.join("executed").exists());
    assert!(!directory.join("unintended").exists());
    std::fs::remove_dir_all(directory).unwrap();
}
