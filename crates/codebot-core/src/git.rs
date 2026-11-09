use crate::{CommandEvent, CommandReport, CommandSpec, CommandState, PreparedCommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, Serialize)]
pub struct GitFile {
    pub path: String,
    pub original_path: Option<String>,
    pub index: char,
    pub working: char,
    pub conflict: bool,
    pub untracked: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct GitRemote {
    pub name: String,
    pub display_url: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct GitSnapshot {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub unborn: bool,
    pub files: Vec<GitFile>,
    pub unstaged_diff: String,
    pub staged_diff: String,
    pub remotes: Vec<GitRemote>,
    pub stamp: String,
}
impl GitSnapshot {
    pub fn suggest_message(&self) -> String {
        let staged = self
            .files
            .iter()
            .filter(|file| file.index != ' ' && !file.untracked)
            .collect::<Vec<_>>();
        if staged.len() == 1 {
            format!("Update {}", staged[0].path)
                .chars()
                .take(72)
                .collect()
        } else {
            format!("Update {} files", staged.len())
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
pub enum GitOperation {
    Stage { paths: Vec<String> },
    Unstage { paths: Vec<String> },
    CreateBranch { name: String },
    Commit { message: String },
    Push { remote: String, branch: String },
}
pub struct PreparedGitOperation {
    snapshot: GitSnapshot,
    operation: GitOperation,
    summary: String,
    expected_files: BTreeMap<String, Option<String>>,
}
impl PreparedGitOperation {
    pub fn snapshot(&self) -> &GitSnapshot {
        &self.snapshot
    }
    pub fn operation(&self) -> &GitOperation {
        &self.operation
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
}
struct Repo {
    root: PathBuf,
    hooks: tempfile::TempDir,
    filters: Vec<String>,
}
struct Capture {
    out: String,
    err: String,
    report: CommandReport,
}
impl Capture {
    fn ok(&self) -> bool {
        self.report.state == CommandState::Exited && self.report.exit_code == Some(0)
    }
}
impl Repo {
    fn open(root: &Path, cancel: &AtomicBool) -> Result<Self, String> {
        let root = fs::canonicalize(root).map_err(|_| "作業リポジトリを開けません")?;
        if !root.is_dir()
            || root
                .to_str()
                .is_none_or(|s| s.chars().any(char::is_control))
        {
            return Err("UTF-8で制御文字を含まない作業フォルダを指定してください".into());
        }
        let mut repo = Self {
            root,
            hooks: tempfile::tempdir().map_err(|_| "Git操作の一時フォルダを用意できません")?,
            filters: vec![],
        };
        let config = repo.capture(
            &[
                "config",
                "--name-only",
                "--get-regexp",
                r"^filter\..*\.(clean|process|required)$",
            ],
            cancel,
        )?;
        if config.ok() {
            for name in config.out.lines() {
                if let Some(name) = name
                    .strip_prefix("filter.")
                    .and_then(|name| name.rsplit_once('.').map(|(name, _)| name))
                {
                    if name.len() > 512 || name.chars().any(char::is_control) {
                        return Err("Gitフィルター設定の形式が不正です".into());
                    }
                    repo.filters.push(name.to_owned());
                }
            }
            repo.filters.sort();
            repo.filters.dedup();
        }
        let top = repo.capture(&["rev-parse", "--show-toplevel"], cancel)?;
        if !top.ok() {
            return Err(
                "通常のGit作業ツリーを指定してください。所有権とGitの導入状況も確認してください"
                    .into(),
            );
        }
        repo.root = fs::canonicalize(top.out.trim_end_matches('\n'))
            .map_err(|_| "リポジトリのルートを確認できません")?;
        Ok(repo)
    }
    fn command(&self, args: &[String]) -> Result<PreparedCommand, String> {
        let mut base = vec![
            "--no-pager".into(),
            "--no-optional-locks".into(),
            "--literal-pathspecs".into(),
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            format!("core.hooksPath={}", self.hooks.path().display()),
            "-c".into(),
            "commit.gpgSign=false".into(),
            "-c".into(),
            "core.quotePath=false".into(),
            "-c".into(),
            "color.ui=false".into(),
            "-c".into(),
            "protocol.ext.allow=never".into(),
        ];
        for name in &self.filters {
            for (key, value) in [("clean", ""), ("process", ""), ("required", "false")] {
                base.extend(["-c".into(), format!("filter.{name}.{key}={value}")]);
            }
        }
        base.extend_from_slice(args);
        let mut command = crate::prepare_command(&CommandSpec {
            program: "git".into(),
            args: base,
            cwd: self.root.clone(),
            timeout_seconds: 120,
            max_output_bytes: 4 * 1024 * 1024,
            ..Default::default()
        })?;
        command.environment_override("GIT_TERMINAL_PROMPT", "0");
        command.environment_override("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
        for name in [
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "https_proxy",
            "http_proxy",
            "all_proxy",
            "no_proxy",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "GIT_SSL_CAINFO",
            "SSH_AUTH_SOCK",
            "GIT_ASKPASS",
        ] {
            if let Ok(value) = std::env::var(name) {
                command.environment_override(name, &value);
            }
        }
        Ok(command)
    }
    fn capture(&self, args: &[&str], cancel: &AtomicBool) -> Result<Capture, String> {
        self.capture_owned(args.iter().map(|s| s.to_string()).collect(), cancel)
    }
    fn capture_owned(&self, args: Vec<String>, cancel: &AtomicBool) -> Result<Capture, String> {
        let command = self.command(&args)?;
        let mut out = String::new();
        let mut err = String::new();
        let report = crate::run_command(&command, cancel, |event| match event {
            CommandEvent::Stdout(text) => out.push_str(&text),
            CommandEvent::Stderr(text) => err.push_str(&text),
        });
        if report.state != CommandState::Exited {
            return Err(match report.state {
                CommandState::Cancelled => "Git操作を中止しました",
                CommandState::TimedOut => "Git操作がタイムアウトしました",
                CommandState::OutputLimit => {
                    "Gitの出力が4 MiBを超えました。対象を絞って外部で確認してください"
                }
                _ => "Gitを実行できません。実行ファイル・権限・作業場所を確認してください",
            }
            .into());
        }
        Ok(Capture { out, err, report })
    }
    fn require(&self, args: &[&str], cancel: &AtomicBool) -> Result<String, String> {
        let result = self.capture(args, cancel)?;
        if result.ok() {
            Ok(result.out)
        } else {
            Err(git_error(&result.err))
        }
    }
    fn snapshot(&self, cancel: &AtomicBool) -> Result<GitSnapshot, String> {
        let branch = self.capture(&["symbolic-ref", "--quiet", "--short", "HEAD"], cancel)?;
        let branch = branch
            .ok()
            .then(|| branch.out.trim_end_matches('\n').to_owned());
        let head = self.capture(&["rev-parse", "--verify", "HEAD"], cancel)?;
        let head = head
            .ok()
            .then(|| head.out.trim_end_matches('\n').to_owned());
        if branch.is_none() && head.is_none() {
            return Err("HEADを確認できません。リポジトリの破損を確認してください".into());
        }
        let status = self.require(
            &[
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--ignore-submodules=all",
            ],
            cancel,
        )?;
        let mut files = Vec::new();
        for line in status.lines() {
            files.push(parse_status(line)?);
            if files.len() > 2000 {
                return Err("Gitのファイル一覧が2000件を超えました。対象を整理してください".into());
            }
        }
        let unstaged_diff = self.require(
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=all",
            ],
            cancel,
        )?;
        let staged_diff = self.require(
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=all",
            ],
            cancel,
        )?;
        let index = self.require(&["ls-files", "--stage"], cancel)?;
        let mut remotes = Vec::new();
        let mut remote_material = String::new();
        for name in self.require(&["remote"], cancel)?.lines() {
            let url = self.require(&["remote", "get-url", "--push", name], cancel)?;
            remote_material.push_str(name);
            remote_material.push_str(&url);
            remotes.push(GitRemote {
                name: name.into(),
                display_url: redact_url(url.trim()),
            });
        }
        let tracking = self.require(
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/remotes",
            ],
            cancel,
        )?;
        let mut hash = Sha256::new();
        for value in [
            branch.as_deref().unwrap_or(""),
            head.as_deref().unwrap_or(""),
            &status,
            &unstaged_diff,
            &staged_diff,
            &index,
            &remote_material,
            &tracking,
        ] {
            hash.update(value.len().to_le_bytes());
            hash.update(value.as_bytes());
        }
        Ok(GitSnapshot {
            root: self.root.clone(),
            unborn: head.is_none(),
            branch,
            head,
            files,
            unstaged_diff,
            staged_diff,
            remotes,
            stamp: format!("{:x}", hash.finalize()),
        })
    }
}
pub fn git_snapshot(root: &Path, cancel: &AtomicBool) -> Result<GitSnapshot, String> {
    Repo::open(root, cancel)?.snapshot(cancel)
}
fn git_error(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if ["authentication","could not read username","publickey","permission denied","403","401"].iter().any(|s|lower.contains(s)) {"Gitの認証・権限を確認してください。資格情報の設定または対象ブランチへのアクセスが必要です。"}
    else if lower.contains("non-fast-forward") || lower.contains("rejected") {"pushが拒否されました。リモートの変更とブランチ保護を確認してください。強制pushは行いません。"}
    else if lower.contains("conflict") || lower.contains("unmerged") {"競合を解消してから再確認してください。既存のファイルは自動resetしません。"}
    else if lower.contains("unable to auto-detect") || lower.contains("identity unknown") {"Gitのuser.name・user.emailを設定してからコミットしてください。"}
    else {"Git操作に失敗しました。リポジトリ・引数・認証・ネットワーク・ロック状態を確認してください。生のエラー本文は表示しません。"}.into()
}
fn redact_url(raw: &str) -> String {
    if let Ok(mut url) = reqwest::Url::parse(raw) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        url.to_string()
    } else if !Path::new(raw).is_absolute() {
        raw.split_once('@')
            .map(|(_, rest)| rest)
            .unwrap_or(raw)
            .into()
    } else {
        raw.into()
    }
}
fn parse_status(line: &str) -> Result<GitFile, String> {
    let bytes = line.as_bytes();
    if bytes.len() < 4 || !bytes[..2].is_ascii() || bytes[2] != b' ' {
        return Err("Git状態の形式が不正です".into());
    }
    let index = bytes[0] as char;
    let working = bytes[1] as char;
    let raw = &line[3..];
    let mut quoted = false;
    let mut escape = false;
    let mut arrow = None;
    for (i, c) in raw.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && quoted {
            escape = true;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
        }
        if !quoted && raw[i..].starts_with(" -> ") {
            arrow = Some(i);
            break;
        }
    }
    let (path, original_path) = if let Some(i) = arrow {
        (unquote(&raw[i + 4..])?, Some(unquote(&raw[..i])?))
    } else {
        (unquote(raw)?, None)
    };
    Ok(GitFile {
        path,
        original_path,
        index,
        working,
        conflict: index == 'U'
            || working == 'U'
            || matches!((index, working), ('A', 'A') | ('D', 'D')),
        untracked: index == '?' && working == '?',
    })
}
fn unquote(raw: &str) -> Result<String, String> {
    if !raw.starts_with('"') {
        return Ok(raw.into());
    }
    let raw = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("Gitパスの引用形式が不正です")?
        .as_bytes();
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] != b'\\' {
            bytes.push(raw[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i == raw.len() {
            return Err("Gitパスの引用形式が不正です".into());
        }
        if (b'0'..=b'7').contains(&raw[i]) {
            let mut number = 0u16;
            let mut count = 0;
            while i < raw.len() && count < 3 && (b'0'..=b'7').contains(&raw[i]) {
                number = number * 8 + (raw[i] - b'0') as u16;
                i += 1;
                count += 1;
            }
            if number > 255 {
                return Err("Gitパスの引用形式が不正です".into());
            }
            bytes.push(number as u8);
        } else {
            bytes.push(match raw[i] {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'b' => 8,
                b'f' => 12,
                b'v' => 11,
                b'a' => 7,
                b'\\' => b'\\',
                b'"' => b'"',
                _ => return Err("Gitパスの引用形式が不正です".into()),
            });
            i += 1;
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| "UTF-8以外のファイル名には未対応です。外部Gitで操作してください".into())
}
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.chars().any(char::is_control)
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !path.split('/').any(|c| c.eq_ignore_ascii_case(".git"))
}
fn file_fingerprint(root: &Path, path: &str) -> Result<Option<String>, String> {
    if !valid_path(path) {
        return Err("作業ルート内の通常の相対ファイルパスを指定してください".into());
    }
    let full = root.join(path);
    let metadata = match fs::symlink_metadata(&full) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("選択したファイルを確認できません".into()),
    };
    let bytes = if metadata.is_symlink() {
        fs::read_link(&full)
            .map_err(|_| "リンクを確認できません")?
            .to_string_lossy()
            .as_bytes()
            .to_vec()
    } else {
        if !metadata.is_file()
            || metadata.len() > 8 * 1024 * 1024
            || !fs::canonicalize(&full)
                .map_err(|_| "ファイルを確認できません")?
                .starts_with(root)
        {
            return Err("Gitステージは作業ルート内の8 MiB以下のファイルを選択してください".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&full)
            .map_err(|_| "ファイルを開けません")?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "ファイルを読めません")?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("選択したファイルが8 MiBを超えました".into());
        }
        bytes
    };
    Ok(Some(format!("{:x}", Sha256::digest(&bytes))))
}

pub fn prepare_git_operation(
    root: &Path,
    operation: GitOperation,
    cancel: &AtomicBool,
) -> Result<PreparedGitOperation, String> {
    let repo = Repo::open(root, cancel)?;
    let snapshot = repo.snapshot(cancel)?;
    let mut expected_files = BTreeMap::new();
    let summary = match &operation {
        GitOperation::Stage { paths } | GitOperation::Unstage { paths } => {
            if paths.is_empty()
                || paths.len() > 2000
                || paths
                    .iter()
                    .any(|p| !snapshot.files.iter().any(|f| &f.path == p))
            {
                return Err("現在の一覧から操作するファイルを選択してください".into());
            }
            for path in paths {
                expected_files.insert(path.clone(), file_fingerprint(&repo.root, path)?);
            }
            if matches!(operation, GitOperation::Stage { .. }) {
                check_filters(&repo, paths, cancel)?;
            }
            format!(
                "{}：{}ファイル。作業ファイルの削除・自動resetは行いません。",
                if matches!(operation, GitOperation::Stage { .. }) {
                    "ステージ"
                } else {
                    "ステージ解除"
                },
                paths.len()
            )
        }
        GitOperation::CreateBranch { name } => {
            check_branch(&repo, name, cancel)?;
            format!("現在のHEADからブランチ {} を作成して切り替えます。", name)
        }
        GitOperation::Commit { message } => {
            if message.trim().is_empty() || message.len() > 10000 || message.contains('\0') {
                return Err("コミット文を1〜10000バイトで入力してください".into());
            }
            if snapshot.files.iter().any(|f| f.conflict) {
                return Err("競合を解消してからコミットしてください".into());
            }
            if !snapshot
                .files
                .iter()
                .any(|f| f.index != ' ' && !f.untracked)
            {
                return Err("コミット対象がステージされていません".into());
            }
            format!("ブランチ {} のステージ済み差分をコミットします。フック・GPG署名はこの操作では実行しません。",snapshot.branch.as_deref().unwrap_or("detached HEAD"))
        }
        GitOperation::Push { remote, branch } => {
            check_branch(&repo, branch, cancel)?;
            if snapshot.branch.is_none() || snapshot.head.is_none() {
                return Err("pushにはコミットのある作業ブランチが必要です".into());
            }
            let remote_view = snapshot
                .remotes
                .iter()
                .find(|r| &r.name == remote)
                .ok_or("一覧からリモートを選択してください")?;
            check_remote(&repo, remote, cancel)?;
            let tracking = format!("refs/remotes/{remote}/{branch}");
            let known = repo.capture(&["rev-parse", "--verify", &tracking], cancel)?;
            let range = if known.ok() {
                format!("{}..HEAD", known.out.trim())
            } else {
                "HEAD".into()
            };
            let log = repo.require(&["log", "--oneline", "-100", &range], cancel)?;
            let diff = if known.ok() {
                repo.require(&["diff", "--no-ext-diff", "--no-textconv", &range], cancel)?
            } else {
                repo.require(
                    &[
                        "log",
                        "-100",
                        "--format=commit %H%n%s",
                        "--patch",
                        "--no-ext-diff",
                        "--no-textconv",
                        "HEAD",
                    ],
                    cancel,
                )?
            };
            format!("push：{} → {} / refs/heads/{}\n接続先：{}\n強制pushは行いません。以下はローカルの追跡情報に基づくプレビューです。リモートの最新状態は送信時に確認されます。\nコミット（最大100件）：\n{}\n差分：\n{}",snapshot.branch.as_deref().unwrap_or(""),remote,branch,remote_view.display_url,log,diff)
        }
    };
    Ok(PreparedGitOperation {
        snapshot,
        operation,
        summary,
        expected_files,
    })
}
fn check_branch(repo: &Repo, name: &str, cancel: &AtomicBool) -> Result<(), String> {
    if name.starts_with('-')
        || name.chars().any(char::is_control)
        || name.len() > 255
        || name.contains("@{")
        || !repo
            .capture(&["check-ref-format", "--branch", name], cancel)?
            .ok()
    {
        return Err("有効なブランチ名を指定してください".into());
    }
    Ok(())
}
fn check_filters(repo: &Repo, paths: &[String], cancel: &AtomicBool) -> Result<(), String> {
    let mut args = vec!["check-attr".into(), "filter".into(), "--".into()];
    args.extend_from_slice(paths);
    let output = repo.capture_owned(args, cancel)?;
    if !output.ok() {
        return Err(git_error(&output.err));
    }
    if output
        .out
        .lines()
        .any(|line| !line.ends_with(": unspecified") && !line.ends_with(": unset"))
    {
        return Err("フィルターを持つファイルのステージには未対応です。LFS等の変換を保持するため外部Gitでステージしてください".into());
    }
    Ok(())
}
fn check_remote(repo: &Repo, name: &str, cancel: &AtomicBool) -> Result<(), String> {
    if name.starts_with('-') || name.chars().any(char::is_control) {
        return Err("リモート名が不正です".into());
    }
    let url = repo.require(&["remote", "get-url", "--push", name], cancel)?;
    let raw = url.trim();
    if raw.contains("::") || raw.chars().any(char::is_control) {
        return Err("このGitリモート形式には対応していません".into());
    }
    if Path::new(raw).is_absolute() {
        return Ok(());
    }
    if let Ok(url) = reqwest::Url::parse(raw) {
        if !matches!(url.scheme(), "https" | "http" | "ssh" | "file") {
            return Err("HTTPS・SSH・file形式のリモートを指定してください".into());
        }
    }
    Ok(())
}
pub fn execute_git_operation(
    prepared: &PreparedGitOperation,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let repo = Repo::open(&prepared.snapshot.root, cancel)?;
    let current = repo.snapshot(cancel)?;
    if current.stamp != prepared.snapshot.stamp {
        return Err("確認後にHEAD・ブランチ・差分・ステージ・リモートが変わりました。再読み込みして確認し直してください".into());
    }
    for (path, expected) in &prepared.expected_files {
        if &file_fingerprint(&repo.root, path)? != expected {
            return Err("確認後に選択したファイルが変わりました。確認し直してください".into());
        }
    }
    let mut args = match &prepared.operation {
        GitOperation::Stage { paths } => {
            check_filters(&repo, paths, cancel)?;
            vec!["add".into(), "--".into()]
        }
        GitOperation::Unstage { .. } if current.unborn => vec![
            "rm".into(),
            "--cached".into(),
            "--force".into(),
            "--ignore-unmatch".into(),
            "--".into(),
        ],
        GitOperation::Unstage { .. } => vec!["restore".into(), "--staged".into(), "--".into()],
        GitOperation::CreateBranch { name } if current.unborn => vec![
            "symbolic-ref".into(),
            "HEAD".into(),
            format!("refs/heads/{name}"),
        ],
        GitOperation::CreateBranch { name } => {
            vec!["switch".into(), "--create".into(), name.clone()]
        }
        GitOperation::Commit { message } => {
            vec!["commit".into(), "--message".into(), message.clone()]
        }
        GitOperation::Push { remote, branch } => {
            check_remote(&repo, remote, cancel)?;
            vec![
                "push".into(),
                "--porcelain".into(),
                remote.clone(),
                format!("HEAD:refs/heads/{branch}"),
            ]
        }
    };
    if let GitOperation::Stage { paths } | GitOperation::Unstage { paths } = &prepared.operation {
        for path in paths {
            args.push(path.clone());
            if let Some(original) = current
                .files
                .iter()
                .find(|f| &f.path == path)
                .and_then(|f| f.original_path.as_ref())
            {
                if !valid_path(original) {
                    return Err("元のファイルパスが不正です".into());
                }
                args.push(original.clone());
            }
        }
    }
    let result = repo.capture_owned(args, cancel)?;
    if result.ok() {
        Ok(())
    } else {
        Err(git_error(&result.err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    fn git(root: &Path, args: &[&str]) -> bool {
        let hooks = root.join("empty-hooks");
        let _ = fs::create_dir(&hooks);
        Command::new("git")
            .args(["-c", "commit.gpgSign=false", "-c"])
            .arg(format!("core.hooksPath={}", hooks.display()))
            .arg("-C")
            .arg(root)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        assert!(git(dir.path(), &["init", "--initial-branch=main"]));
        assert!(git(dir.path(), &["config", "user.name", "CodeBot Fixture"]));
        assert!(git(
            dir.path(),
            &["config", "user.email", "fixture@example.invalid"]
        ));
        dir
    }
    fn apply(root: &Path, op: GitOperation) {
        execute_git_operation(
            &prepare_git_operation(root, op, &AtomicBool::new(false)).unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap();
    }
    fn initial(root: &Path) {
        fs::write(root.join("source.rs"), "before\n").unwrap();
        apply(
            root,
            GitOperation::Stage {
                paths: vec!["source.rs".into()],
            },
        );
        apply(
            root,
            GitOperation::Commit {
                message: "Initial fixture".into(),
            },
        );
    }

    #[test]
    fn unborn_stage_unstage_commit_and_branch_preserve_unselected_work() {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("space name.txt"), "private working change\n").unwrap();
        fs::write(root.join("日本語.txt"), "日本語\n").unwrap();
        let snapshot = git_snapshot(root, &AtomicBool::new(false)).unwrap();
        assert!(snapshot.unborn);
        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert!(snapshot
            .files
            .iter()
            .any(|f| f.path == "日本語.txt" && f.untracked));
        let prepared = prepare_git_operation(
            root,
            GitOperation::Stage {
                paths: vec!["space name.txt".into()],
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(git_snapshot(root, &AtomicBool::new(false))
            .unwrap()
            .staged_diff
            .is_empty());
        execute_git_operation(&prepared, &AtomicBool::new(false)).unwrap();
        assert!(git_snapshot(root, &AtomicBool::new(false))
            .unwrap()
            .staged_diff
            .contains("+private working change"));
        apply(
            root,
            GitOperation::Unstage {
                paths: vec!["space name.txt".into()],
            },
        );
        assert_eq!(
            fs::read_to_string(root.join("space name.txt")).unwrap(),
            "private working change\n"
        );
        initial(root);
        apply(
            root,
            GitOperation::CreateBranch {
                name: "feature/review".into(),
            },
        );
        let after = git_snapshot(root, &AtomicBool::new(false)).unwrap();
        assert_eq!(after.branch.as_deref(), Some("feature/review"));
        assert!(after.head.is_some());
        assert!(after
            .files
            .iter()
            .any(|f| f.path == "日本語.txt" && f.untracked));
    }
    #[test]
    fn changed_worktree_and_index_invalidate_approvals_without_resetting_files() {
        let dir = repo();
        let root = dir.path();
        initial(root);
        fs::write(root.join("source.rs"), "after\n").unwrap();
        let prepared = prepare_git_operation(
            root,
            GitOperation::Stage {
                paths: vec!["source.rs".into()],
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        fs::write(root.join("source.rs"), "newer\n").unwrap();
        assert!(execute_git_operation(&prepared, &AtomicBool::new(false))
            .unwrap_err()
            .contains("確認後"));
        assert_eq!(
            fs::read_to_string(root.join("source.rs")).unwrap(),
            "newer\n"
        );
        apply(
            root,
            GitOperation::Stage {
                paths: vec!["source.rs".into()],
            },
        );
        let commit = prepare_git_operation(
            root,
            GitOperation::Commit {
                message: "Update fixture".into(),
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        apply(
            root,
            GitOperation::Unstage {
                paths: vec!["source.rs".into()],
            },
        );
        assert!(execute_git_operation(&commit, &AtomicBool::new(false)).is_err());
        assert_eq!(
            fs::read_to_string(root.join("source.rs")).unwrap(),
            "newer\n"
        );
    }
    #[test]
    fn worktrees_detached_head_and_conflicts_are_visible_and_preserved() {
        let dir = repo();
        let root = dir.path();
        initial(root);
        let worktree = tempfile::tempdir().unwrap();
        assert!(git(
            root,
            &[
                "worktree",
                "add",
                "-b",
                "other",
                worktree.path().to_str().unwrap()
            ]
        ));
        assert_eq!(
            git_snapshot(worktree.path(), &AtomicBool::new(false))
                .unwrap()
                .branch
                .as_deref(),
            Some("other")
        );
        fs::write(worktree.path().join("source.rs"), "other\n").unwrap();
        assert!(git(worktree.path(), &["commit", "-am", "Other"]));
        fs::write(root.join("source.rs"), "main\n").unwrap();
        assert!(git(root, &["commit", "-am", "Main"]));
        assert!(!git(root, &["merge", "other"]));
        let conflict = git_snapshot(root, &AtomicBool::new(false)).unwrap();
        assert!(conflict.files.iter().any(|f| f.conflict));
        assert!(prepare_git_operation(
            root,
            GitOperation::Commit {
                message: "Must not commit".into()
            },
            &AtomicBool::new(false)
        )
        .is_err());
        assert!(fs::read_to_string(root.join("source.rs"))
            .unwrap()
            .contains("<<<<<<<"));
        // The detached check uses the independent, clean worktree.
        assert!(git(worktree.path(), &["checkout", "--detach", "HEAD"]));
        let detached = git_snapshot(worktree.path(), &AtomicBool::new(false)).unwrap();
        assert!(detached.branch.is_none());
        assert!(detached.head.is_some());
        assert!(prepare_git_operation(
            worktree.path(),
            GitOperation::Push {
                remote: "origin".into(),
                branch: "main".into()
            },
            &AtomicBool::new(false)
        )
        .is_err());
    }
    #[test]
    fn hooks_filters_remote_credentials_and_unsafe_paths_do_not_escape_the_review_flow() {
        let dir = repo();
        let root = dir.path();
        initial(root);
        let hook = root.join(".git/hooks/post-commit");
        fs::write(&hook, "#!/bin/sh\nprintf bad > hook-executed\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(root.join("source.rs"), "hook check\n").unwrap();
        apply(
            root,
            GitOperation::Stage {
                paths: vec!["source.rs".into()],
            },
        );
        apply(
            root,
            GitOperation::Commit {
                message: "Hook suppression fixture".into(),
            },
        );
        assert!(!root.join("hook-executed").exists());
        assert!(git(
            root,
            &[
                "remote",
                "add",
                "origin",
                "https://private-test-token@example.invalid/repo?token=private-test-token"
            ]
        ));
        let snapshot = git_snapshot(root, &AtomicBool::new(false)).unwrap();
        assert!(!serde_json::to_string(&snapshot.remotes)
            .unwrap()
            .contains("private-test-token"));
        let push = prepare_git_operation(
            root,
            GitOperation::Push {
                remote: "origin".into(),
                branch: "main".into(),
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!push.summary().contains("private-test-token"));
        assert!(git(
            root,
            &[
                "remote",
                "set-url",
                "origin",
                "https://example.invalid/changed"
            ]
        ));
        assert!(execute_git_operation(&push, &AtomicBool::new(false)).is_err());
        fs::write(root.join(".gitattributes"), "*.txt filter=custom\n").unwrap();
        fs::write(root.join("filtered.txt"), "data\n").unwrap();
        assert!(git(
            root,
            &["config", "filter.custom.clean", "touch filter-executed"]
        ));
        let _ = git_snapshot(root, &AtomicBool::new(false)).unwrap();
        assert!(!root.join("filter-executed").exists());
        assert!(prepare_git_operation(
            root,
            GitOperation::Stage {
                paths: vec!["filtered.txt".into()]
            },
            &AtomicBool::new(false)
        )
        .is_err());
        assert!(!root.join("filter-executed").exists());
        for path in [
            "../outside",
            "/absolute",
            ".git/config",
            "dir/../../outside",
        ] {
            assert!(!valid_path(path));
        }
        for raw in [
            "Authentication failed private-test-token",
            "Permission denied private-test-token",
            "rejected private-test-token",
        ] {
            assert!(!git_error(raw).contains("private-test-token"));
        }
    }
    #[test]
    fn quoted_status_paths_keep_spaces_arrows_and_non_ascii_text() {
        let entry = parse_status("R  \"old -> name.txt\" -> \"新しい name.txt\"").unwrap();
        assert_eq!(entry.path, "新しい name.txt");
        assert_eq!(entry.original_path.as_deref(), Some("old -> name.txt"));
        assert_eq!(unquote("\"space\\tname\"").unwrap(), "space\tname");
        assert!(unquote("\"\\377\"").is_err());
    }
    #[test]
    fn pushing_to_a_local_bare_remote_is_reviewed_and_preserves_uncommitted_work() {
        let dir = repo();
        let root = dir.path();
        initial(root);
        let bare = tempfile::tempdir().unwrap();
        assert!(git(bare.path(), &["init", "--bare"]));
        assert!(git(
            root,
            &["remote", "add", "origin", bare.path().to_str().unwrap()]
        ));
        fs::write(root.join("source.rs"), "uncommitted\n").unwrap();
        let prepared = prepare_git_operation(
            root,
            GitOperation::Push {
                remote: "origin".into(),
                branch: "main".into(),
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(prepared.summary().contains("refs/heads/main"));
        assert!(prepared.summary().contains("Initial fixture"));
        execute_git_operation(&prepared, &AtomicBool::new(false)).unwrap();
        assert!(git(
            bare.path(),
            &["rev-parse", "--verify", "refs/heads/main"]
        ));
        assert_eq!(
            fs::read_to_string(root.join("source.rs")).unwrap(),
            "uncommitted\n"
        );
        assert!(
            execute_git_operation(&prepared, &AtomicBool::new(false)).is_err(),
            "tracking changed after push; review must be renewed"
        );
    }
}
