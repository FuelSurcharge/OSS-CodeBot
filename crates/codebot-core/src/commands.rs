use process_wrap::std::{ChildWrapper, CommandWrap};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub pass_env: Vec<String>,
    pub timeout_seconds: u64,
    pub max_output_bytes: usize,
}
impl Default for CommandSpec {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: vec![],
            cwd: PathBuf::new(),
            pass_env: vec![],
            timeout_seconds: 120,
            max_output_bytes: 1024 * 1024,
        }
    }
}
#[derive(Clone, Serialize)]
pub struct CommandPreview {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub environment_names: Vec<String>,
    pub timeout_seconds: u64,
    pub max_output_bytes: usize,
}
pub struct PreparedCommand {
    preview: CommandPreview,
    canonical_program: PathBuf,
    executable_len: u64,
    modified: Option<SystemTime>,
    environment: BTreeMap<OsString, OsString>,
    secrets: Vec<Vec<u8>>,
}
impl PreparedCommand {
    pub fn preview(&self) -> &CommandPreview {
        &self.preview
    }
}
const BASE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "SystemRoot",
    "SYSTEMROOT",
    "WINDIR",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "TERM",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "NUMBER_OF_PROCESSORS",
    "PATHEXT",
    "ComSpec",
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "VCINSTALLDIR",
    "VCToolsInstallDir",
    "VSINSTALLDIR",
    "WindowsSdkDir",
    "WindowsSDKVersion",
    "UniversalCRTSdkDir",
    "UCRTVersion",
];
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || i > 0 && b.is_ascii_digit())
}
fn sensitive(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "KEY",
        "PASS",
        "CREDENTIAL",
        "AUTHORIZATION",
    ]
    .iter()
    .any(|part| name.contains(part))
}

pub fn prepare_command(spec: &CommandSpec) -> Result<PreparedCommand, String> {
    if spec.program.trim().is_empty()
        || spec.program.len() > 4096
        || spec.program.contains('\0')
        || spec.args.len() > 256
        || spec
            .args
            .iter()
            .any(|a| a.len() > 65536 || a.contains('\0'))
        || spec.args.iter().map(String::len).sum::<usize>() > 1024 * 1024
    {
        return Err("実行ファイル・引数が空、不正、または上限を超えています".into());
    }
    if !spec.cwd.is_absolute()
        || !(1..=3600).contains(&spec.timeout_seconds)
        || !(1024..=4 * 1024 * 1024).contains(&spec.max_output_bytes)
        || spec.pass_env.len() > 100
        || spec.pass_env.iter().any(|n| !valid_name(n))
    {
        return Err(
            "作業フォルダの絶対パス、タイムアウト、出力上限、環境変数名を確認してください".into(),
        );
    }
    let cwd = fs::canonicalize(&spec.cwd).map_err(|_| "作業フォルダを開けません")?;
    if !cwd.is_dir() {
        return Err("作業フォルダを指定してください".into());
    }
    let program = resolve_program(&spec.program, &cwd)?;
    if program
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
    {
        return Err(".bat/.cmdの暗黙のシェル実行は使用できません。実行するインタープリターと引数を明示してください".into());
    }
    let canonical_program =
        fs::canonicalize(&program).map_err(|_| "実行ファイルを確認できません")?;
    let metadata = fs::metadata(&program).map_err(|_| "実行ファイルを確認できません")?;
    if !metadata.is_file() {
        return Err("通常の実行ファイルを指定してください".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("実行権限がありません".into());
        }
    }
    let mut environment = BTreeMap::new();
    if spec
        .pass_env
        .iter()
        .any(|name| std::env::var_os(name).is_none())
    {
        return Err("明示した環境変数が未設定です。変数名と実行環境を確認してください".into());
    }
    for name in BASE_ENV
        .iter()
        .copied()
        .chain(spec.pass_env.iter().map(String::as_str))
    {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(OsString::from(name), value);
        }
    }
    let mut secrets = environment
        .iter()
        .filter(|(name, _)| sensitive(&name.to_string_lossy()))
        .filter_map(|(_, value)| value.to_str())
        .filter(|v| !v.is_empty())
        .map(|v| v.as_bytes().to_vec())
        .collect::<Vec<_>>();
    secrets.sort_by_key(|v| std::cmp::Reverse(v.len()));
    secrets.dedup();
    let preview = CommandPreview {
        executable: program,
        args: spec.args.clone(),
        cwd,
        environment_names: environment
            .keys()
            .map(|v| v.to_string_lossy().into_owned())
            .collect(),
        timeout_seconds: spec.timeout_seconds,
        max_output_bytes: spec.max_output_bytes,
    };
    Ok(PreparedCommand {
        preview,
        canonical_program,
        executable_len: metadata.len(),
        modified: metadata.modified().ok(),
        environment,
        secrets,
    })
}
fn resolve_program(program: &str, cwd: &Path) -> Result<PathBuf, String> {
    let input = Path::new(program);
    if input.is_absolute() {
        return Ok(input.to_owned());
    }
    if input.components().count() > 1 {
        return Ok(cwd.join(input));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        // Relative PATH components are resolved against the selected directory.
        let directory = if directory.is_absolute() {
            directory
        } else {
            cwd.join(directory)
        };
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Ok(candidate);
        }
        #[cfg(windows)]
        {
            if input.extension().is_none() {
                for ext in ["exe", "com"] {
                    let candidate = candidate.with_extension(ext);
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                }
            }
        }
    }
    Err("実行ファイルがPATHにありません。フルパスで指定してください".into())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandState {
    Exited,
    Cancelled,
    TimedOut,
    OutputLimit,
    Failed,
}
#[derive(Clone, Debug, Serialize)]
pub struct CommandReport {
    pub state: CommandState,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub output_bytes: usize,
    pub error: Option<String>,
}
pub enum CommandEvent {
    Stdout(String),
    Stderr(String),
}
struct OwnedChild(Box<dyn ChildWrapper>, bool);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.0.start_kill();
            let _ = self.0.wait();
        }
    }
}
enum PipeEvent {
    Data(bool, Vec<u8>),
    End,
    Error,
}
fn reader(
    pipe: impl Read + Send + 'static,
    stderr: bool,
    tx: mpsc::SyncSender<PipeEvent>,
    total: Arc<AtomicUsize>,
    limit: Arc<AtomicBool>,
    max: usize,
) {
    thread::spawn(move || {
        let mut pipe = pipe;
        let mut buffer = [0; 4096];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let previous = total.fetch_add(n, Ordering::Relaxed);
                    let kept = n.min(max.saturating_sub(previous));
                    if kept > 0
                        && tx
                            .send(PipeEvent::Data(stderr, buffer[..kept].to_vec()))
                            .is_err()
                    {
                        return;
                    }
                    if previous + n > max {
                        limit.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(PipeEvent::Error);
                    break;
                }
            }
        }
        let _ = tx.send(PipeEvent::End);
    });
}

/// The caller approves the exact preview before invoking this method. The
/// command runs as the current user, with explicit argv and a filtered env.
pub fn run_command(
    prepared: &PreparedCommand,
    cancel: &AtomicBool,
    mut emit: impl FnMut(CommandEvent),
) -> CommandReport {
    let start = Instant::now();
    let preview = &prepared.preview;
    let mut report = CommandReport {
        state: CommandState::Failed,
        exit_code: None,
        elapsed_ms: 0,
        output_bytes: 0,
        error: None,
    };
    let result = (|| -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) {
            report.state = CommandState::Cancelled;
            return Ok(());
        }
        let metadata =
            fs::metadata(&preview.executable).map_err(|_| "実行ファイルを確認できません")?;
        if fs::canonicalize(&preview.executable).ok().as_ref() != Some(&prepared.canonical_program)
            || metadata.len() != prepared.executable_len
            || metadata.modified().ok() != prepared.modified
            || fs::canonicalize(&preview.cwd).ok().as_ref() != Some(&preview.cwd)
        {
            return Err(
                "確認後に実行ファイルまたは作業場所が変わりました。内容を確認し直してください"
                    .into(),
            );
        }
        let mut command = Command::new(&preview.executable);
        command
            .args(&preview.args)
            .current_dir(&preview.cwd)
            .env_clear()
            .envs(&prepared.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut command = CommandWrap::from(command);
        #[cfg(unix)]
        {
            command.wrap(process_wrap::std::ProcessGroup::leader());
        }
        #[cfg(windows)]
        {
            command.wrap(process_wrap::std::CreationFlags(
                windows::Win32::System::Threading::CREATE_NO_WINDOW,
            ));
            command.wrap(process_wrap::std::JobObject);
        }
        let mut child = OwnedChild(
            command
                .spawn()
                .map_err(|_| "コマンドを起動できません。実行ファイルと権限を確認してください")?,
            true,
        );
        let (tx, rx) = mpsc::sync_channel(64);
        let total = Arc::new(AtomicUsize::new(0));
        let limit = Arc::new(AtomicBool::new(false));
        reader(
            child.0.stdout().take().unwrap(),
            false,
            tx.clone(),
            total.clone(),
            limit.clone(),
            preview.max_output_bytes,
        );
        reader(
            child.0.stderr().take().unwrap(),
            true,
            tx.clone(),
            total.clone(),
            limit.clone(),
            preview.max_output_bytes,
        );
        drop(tx);
        let mut out = Redactor::new(prepared.secrets.clone());
        let mut err = Redactor::new(prepared.secrets.clone());
        let mut ended = 0;
        let mut status = None;
        let mut killed = false;
        let mut read_failed = false;
        loop {
            if !killed {
                let reason = if cancel.load(Ordering::Relaxed) {
                    Some(CommandState::Cancelled)
                } else if limit.load(Ordering::Relaxed) {
                    Some(CommandState::OutputLimit)
                } else if start.elapsed() >= Duration::from_secs(preview.timeout_seconds) {
                    Some(CommandState::TimedOut)
                } else {
                    None
                };
                if let Some(reason) = reason {
                    report.state = reason;
                    child
                        .0
                        .start_kill()
                        .map_err(|_| "子プロセスを停止できませんでした")?;
                    killed = true;
                }
            }
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(PipeEvent::Data(stderr, bytes)) => {
                    let text = if stderr {
                        err.push(&bytes, false)
                    } else {
                        out.push(&bytes, false)
                    };
                    if !text.is_empty() {
                        emit(if stderr {
                            CommandEvent::Stderr(text)
                        } else {
                            CommandEvent::Stdout(text)
                        });
                    }
                }
                Ok(PipeEvent::End) => ended += 1,
                Ok(PipeEvent::Error) => read_failed = true,
                Err(mpsc::RecvTimeoutError::Disconnected) => ended = 2,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if status.is_none() {
                status = child.0.try_wait().map_err(|_| "終了状態を確認できません")?;
            }
            // A normally exited leader must not leave its build/test children
            // or inherited output handles alive after this operation.
            if status.is_some() && !killed {
                let _ = child.0.start_kill();
                killed = true;
                report.state = CommandState::Exited;
            }
            if ended >= 2 && status.is_some() {
                break;
            }
        }
        for (stderr, text) in [(false, out.push(&[], true)), (true, err.push(&[], true))] {
            if !text.is_empty() {
                emit(if stderr {
                    CommandEvent::Stderr(text)
                } else {
                    CommandEvent::Stdout(text)
                });
            }
        }
        child.1 = false;
        report.exit_code = status.and_then(|s| s.code());
        report.output_bytes = total.load(Ordering::Relaxed).min(preview.max_output_bytes);
        if limit.load(Ordering::Relaxed) && report.state == CommandState::Exited {
            report.state = CommandState::OutputLimit;
        }
        if read_failed {
            return Err("コマンド出力を読み込めませんでした".into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        report.state = CommandState::Failed;
        report.error = Some(error);
    }
    report.elapsed_ms = start.elapsed().as_millis().min(u64::MAX as u128) as u64;
    report
}

struct Redactor {
    pending: Vec<u8>,
    utf8: Vec<u8>,
    secrets: Vec<Vec<u8>>,
    hold: usize,
}
impl Redactor {
    fn new(secrets: Vec<Vec<u8>>) -> Self {
        let hold = secrets
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(1)
            .saturating_sub(1);
        Self {
            pending: vec![],
            utf8: vec![],
            secrets,
            hold,
        }
    }
    fn push(&mut self, bytes: &[u8], flush: bool) -> String {
        self.pending.extend_from_slice(bytes);
        let end = if flush {
            self.pending.len()
        } else {
            self.pending.len().saturating_sub(self.hold)
        };
        let mut index = 0;
        while index < end {
            if flush
                && self
                    .secrets
                    .iter()
                    .any(|secret| secret.starts_with(&self.pending[index..]))
            {
                self.utf8.extend_from_slice(b"[REDACTED]");
                index = self.pending.len();
                break;
            }
            if let Some(secret) = self
                .secrets
                .iter()
                .find(|secret| self.pending[index..].starts_with(secret))
            {
                self.utf8.extend_from_slice(b"[REDACTED]");
                index += secret.len();
            } else {
                self.utf8.push(self.pending[index]);
                index += 1;
            }
        }
        self.pending.drain(..index);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.utf8) {
                Ok(text) => {
                    output.push_str(text);
                    self.utf8.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    output.push_str(std::str::from_utf8(&self.utf8[..valid]).unwrap());
                    self.utf8.drain(..valid);
                    if let Some(length) = error.error_len() {
                        output.push('\u{fffd}');
                        self.utf8.drain(..length);
                    } else if flush {
                        output.push('\u{fffd}');
                        self.utf8.clear();
                        break;
                    } else {
                        break;
                    }
                }
            }
        }
        output
            .chars()
            .map(|c| {
                if c == '\r' {
                    '\n'
                } else if c.is_control() && c != '\n' && c != '\t' {
                    '\u{fffd}'
                } else {
                    c
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn python_spec(root: &Path, script: &str) -> CommandSpec {
        CommandSpec {
            program: if cfg!(windows) { "python" } else { "python3" }.into(),
            args: vec!["-I".into(), "-c".into(), script.into()],
            cwd: root.to_owned(),
            ..Default::default()
        }
    }
    #[test]
    fn preview_does_not_execute_and_argv_is_literal_with_filtered_environment() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("executed");
        let mut spec=python_spec(dir.path(),"import os,sys;open('executed','w').write('yes'); print(sys.argv[1]); print('OPENAI_API_KEY' in os.environ); print('日本語')");
        spec.args
            .push("$(touch unintended); `echo injected` & | >".into());
        let prepared = prepare_command(&spec).unwrap();
        assert!(!marker.exists());
        assert!(!prepared
            .preview()
            .environment_names
            .iter()
            .any(|n| n == "OPENAI_API_KEY"));
        let mut output = String::new();
        let report = run_command(&prepared, &AtomicBool::new(false), |event| {
            if let CommandEvent::Stdout(text) = event {
                output.push_str(&text);
            }
        });
        assert_eq!(report.state, CommandState::Exited);
        assert_eq!(report.exit_code, Some(0));
        assert!(marker.exists());
        assert!(!dir.path().join("unintended").exists());
        assert!(output.contains("$(touch unintended); `echo injected` & | >"));
        assert!(output.contains("False"));
        assert!(output.contains("日本語"));
    }
    #[test]
    fn nonzero_exit_and_output_limits_are_reported_without_unbounded_buffers() {
        let dir = tempfile::tempdir().unwrap();
        let mut output = String::new();
        let report = run_command(
            &prepare_command(&python_spec(
                dir.path(),
                "import sys;print('test failure',file=sys.stderr);sys.exit(7)",
            ))
            .unwrap(),
            &AtomicBool::new(false),
            |event| {
                if let CommandEvent::Stderr(text) = event {
                    output.push_str(&text);
                }
            },
        );
        assert_eq!(report.state, CommandState::Exited);
        assert_eq!(report.exit_code, Some(7));
        assert!(output.contains("test failure"));
        let mut spec = python_spec(dir.path(), "print('a'*1000000)");
        spec.max_output_bytes = 1024;
        let mut displayed = 0;
        let report = run_command(
            &prepare_command(&spec).unwrap(),
            &AtomicBool::new(false),
            |event| match event {
                CommandEvent::Stdout(s) | CommandEvent::Stderr(s) => displayed += s.len(),
            },
        );
        assert_eq!(report.state, CommandState::OutputLimit);
        assert_eq!(report.output_bytes, 1024);
        assert_eq!(displayed, 1024);
    }
    #[test]
    fn timeout_cancellation_and_normal_exit_clean_up_owned_descendants() {
        for mode in ["timeout", "cancel", "parent-exit"] {
            let dir = tempfile::tempdir().unwrap();
            let child = "import time;time.sleep(1.5);open('descendant-survived','w').write('bad')";
            let script=format!("import subprocess,sys,time;subprocess.Popen([sys.executable,'-I','-c',{child:?}]);print('started',flush=True);{}",if mode=="parent-exit" {"sys.exit(0)"} else {"time.sleep(20)"});
            let mut spec = python_spec(dir.path(), &script);
            spec.timeout_seconds = 1;
            let cancel = AtomicBool::new(false);
            let started = Instant::now();
            let report = run_command(&prepare_command(&spec).unwrap(), &cancel, |event| {
                if mode == "cancel"
                    && matches!(event,CommandEvent::Stdout(ref s) if s.contains("started"))
                {
                    cancel.store(true, Ordering::Relaxed);
                }
            });
            assert!(started.elapsed() < Duration::from_secs(4));
            assert_eq!(
                report.state,
                match mode {
                    "cancel" => CommandState::Cancelled,
                    "timeout" => CommandState::TimedOut,
                    _ => CommandState::Exited,
                }
            );
            thread::sleep(Duration::from_millis(1600));
            assert!(
                !dir.path().join("descendant-survived").exists(),
                "owned descendant survived {mode}"
            );
        }
    }
    #[test]
    fn chunked_utf8_and_secret_values_are_redacted_before_display() {
        let mut redactor = Redactor::new(vec![b"private-test-token".to_vec()]);
        let mut text = String::new();
        for byte in "日本語 private-test-token 最後\u{1b}[31m".as_bytes() {
            text.push_str(&redactor.push(&[*byte], false));
        }
        text.push_str(&redactor.push(&[], true));
        assert!(text.starts_with("日本語 [REDACTED] 最後"));
        assert!(!text.contains("private-test-token"));
        assert!(!text.contains('\u{1b}'));
        let mut truncated = Redactor::new(vec![b"private-test-token".to_vec()]);
        assert_eq!(truncated.push(b"private-test", false), "");
        assert_eq!(truncated.push(&[], true), "[REDACTED]");
    }
    #[test]
    fn changed_executables_invalid_paths_and_batch_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("test.exe");
        fs::write(&executable, b"initial").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let spec = CommandSpec {
            program: executable.to_string_lossy().into_owned(),
            cwd: dir.path().to_owned(),
            ..Default::default()
        };
        let prepared = prepare_command(&spec).unwrap();
        fs::write(&executable, b"changed length").unwrap();
        assert_eq!(
            run_command(&prepared, &AtomicBool::new(false), |_| panic!(
                "must not start"
            ))
            .state,
            CommandState::Failed
        );
        let mut invalid = spec.clone();
        invalid.cwd = PathBuf::from("relative");
        assert!(prepare_command(&invalid).is_err());
        invalid = spec;
        invalid.program = dir.path().join("run.cmd").to_string_lossy().into_owned();
        assert!(prepare_command(&invalid).is_err());
    }
}
