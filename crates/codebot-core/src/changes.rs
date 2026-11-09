//! Proposals are inert until a caller invokes apply after user review.
use crate::Workspace;
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_TEXT: usize = 64 * 1024;
const MAX_PROPOSAL: usize = 1024 * 1024;
const MAX_JOURNAL: usize = 16 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineEdit {
    pub first_line: usize,
    pub last_line: usize,
    pub content: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "lowercase", deny_unknown_fields)]
pub enum FileChange {
    Create {
        path: String,
        content: String,
    },
    Replace {
        path: String,
        expected_sha256: String,
        edits: Vec<LineEdit>,
    },
    Delete {
        path: String,
        expected_sha256: String,
    },
}
impl FileChange {
    pub fn path(&self) -> &str {
        match self {
            Self::Create { path, .. } | Self::Replace { path, .. } | Self::Delete { path, .. } => {
                path
            }
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "追加",
            Self::Replace { .. } => "更新",
            Self::Delete { .. } => "削除",
        }
    }
    pub fn unit_labels(&self) -> Vec<String> {
        match self {
            Self::Replace { edits, .. } => edits
                .iter()
                .map(|e| format!("{}–{}行", e.first_line, e.last_line))
                .collect(),
            _ => vec!["ファイル全体".into()],
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeProposal {
    pub version: u32,
    pub changes: Vec<FileChange>,
}
impl ChangeProposal {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_PROPOSAL {
            return Err("変更案が1 MiBを超えています".into());
        }
        let normalized = text.replace("\r\n", "\n");
        let text = normalized.trim();
        let json = if let Some((_, block)) = text.split_once("```json\n") {
            block
                .split_once("\n```")
                .map(|(json, _)| json)
                .ok_or("未完了のJSONコードブロックです")?
        } else {
            text
        };
        let proposal: Self = serde_json::from_str(json).map_err(|_| {
            "変更案JSONの形式が不正です。version・changes・operationを確認してください"
        })?;
        if proposal.version != 1 || proposal.changes.is_empty() || proposal.changes.len() > 32 {
            return Err("変更案はversion 1、1〜32ファイルで指定してください".into());
        }
        Ok(proposal)
    }
    pub fn select(&self, selected: &[Vec<bool>]) -> Result<Self, String> {
        if selected.len() != self.changes.len() {
            return Err("変更箇所の選択が一致しません".into());
        }
        let mut changes = Vec::new();
        for (change, flags) in self.changes.iter().zip(selected) {
            if flags.len() != change.unit_labels().len() {
                return Err("変更箇所の選択が一致しません".into());
            }
            match change {
                FileChange::Replace {
                    path,
                    expected_sha256,
                    edits,
                } => {
                    let edits: Vec<_> = edits
                        .iter()
                        .zip(flags)
                        .filter(|(_, flag)| **flag)
                        .map(|(edit, _)| edit.clone())
                        .collect();
                    if !edits.is_empty() {
                        changes.push(FileChange::Replace {
                            path: path.clone(),
                            expected_sha256: expected_sha256.clone(),
                            edits,
                        });
                    }
                }
                _ if flags.first() == Some(&true) => changes.push(change.clone()),
                _ => {}
            }
        }
        if changes.is_empty() {
            return Err("適用する変更を選んでください".into());
        }
        Ok(Self {
            version: self.version,
            changes,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedChange {
    path: String,
    before: Option<String>,
    after: Option<String>,
    mode: Option<u32>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    root: String,
    state: String,
    files: Vec<SavedChange>,
}
pub struct ChangePreview {
    pub path: String,
    pub diff: String,
}
pub struct PreparedChanges {
    root: PathBuf,
    files: Vec<SavedChange>,
}
impl PreparedChanges {
    pub fn previews(&self) -> Vec<ChangePreview> {
        previews(&self.files)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}
fn previews(files: &[SavedChange]) -> Vec<ChangePreview> {
    files
        .iter()
        .map(|file| ChangePreview {
            path: file.path.clone(),
            diff: similar::TextDiff::configure()
                .timeout(std::time::Duration::from_millis(100))
                .diff_lines(
                    file.before.as_deref().unwrap_or(""),
                    file.after.as_deref().unwrap_or(""),
                )
                .unified_diff()
                .context_radius(3)
                .header(&format!("a/{}", file.path), &format!("b/{}", file.path))
                .to_string(),
        })
        .collect()
}
fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn valid_text(text: &str) -> bool {
    text.len() <= MAX_TEXT && !text.contains('\0')
}

fn safe_target(workspace: &Workspace, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains(':')
        || relative.contains('\\')
        || relative.chars().any(char::is_control)
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || crate::workspace::excluded(path)
    {
        return Err("変更先は作業フォルダ内の通常ファイルを相対パスで指定してください".into());
    }
    let mut current = workspace.root().to_owned();
    for part in path.components() {
        let name = part.as_os_str().to_string_lossy();
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if name.starts_with('.')
            || name.ends_with('.')
            || name.ends_with(' ')
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("隠しファイル・デバイス名・OSで曖昧になるパスは変更対象外です".into());
        }
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("シンボリックリンクを通る変更は行いません".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("変更先の情報を取得できません".into()),
        }
    }
    let parent = current.parent().ok_or("変更先の親フォルダがありません")?;
    let canonical = parent
        .canonicalize()
        .map_err(|_| "変更先の親フォルダを先に用意してください")?;
    if !canonical.starts_with(workspace.root()) {
        return Err("作業フォルダ外は変更できません".into());
    }
    Ok(canonical.join(current.file_name().ok_or("ファイル名がありません")?))
}
fn current(workspace: &Workspace, path: &str) -> Result<Option<String>, String> {
    safe_target(workspace, path)?;
    match workspace.dir.symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => workspace.full_text(Path::new(path)).map(Some),
        Ok(_) => Err("通常ファイルだけを変更できます".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("変更対象を読み込めません".into()),
    }
}
fn create_allowed(workspace: &Workspace, path: &Path) -> Result<(), String> {
    let mut directories = vec![PathBuf::new()];
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if !directory.as_os_str().is_empty() {
            directories.push(directory.to_owned());
        }
        parent = directory.parent();
    }
    directories.sort_by_key(|p| p.components().count());
    let mut builder = ignore::gitignore::GitignoreBuilder::new(workspace.root());
    for directory in directories {
        for name in [".gitignore", ".ignore"] {
            let file = workspace.root().join(&directory).join(name);
            if file.exists() && builder.add(file).is_some() {
                return Err("除外規則を読み込めません".into());
            }
        }
    }
    let ignore = builder.build().map_err(|_| "除外規則が不正です")?;
    if ignore
        .matched(workspace.root().join(path), false)
        .is_ignore()
    {
        return Err("除外対象のファイルは作成できません".into());
    }
    Ok(())
}
fn replace_lines(before: &str, edits: &[LineEdit]) -> Result<String, String> {
    if edits.is_empty() || edits.len() > 128 {
        return Err("更新は1〜128箇所で指定してください".into());
    }
    let lines: Vec<_> = if before.is_empty() {
        vec![""]
    } else {
        before.split_inclusive('\n').collect()
    };
    let mut ranges = Vec::new();
    let mut offsets = vec![0];
    for line in &lines {
        offsets.push(offsets.last().unwrap() + line.len());
    }
    for edit in edits {
        if edit.first_line == 0
            || edit.first_line > edit.last_line
            || edit.last_line > lines.len()
            || !valid_text(&edit.content)
        {
            return Err("更新箇所の行範囲・内容が不正です".into());
        }
        let mut content = if before.contains("\r\n") && !before.replace("\r\n", "").contains('\n') {
            edit.content.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            edit.content.clone()
        };
        if lines[edit.last_line - 1].ends_with('\n')
            && !content.is_empty()
            && !content.ends_with('\n')
        {
            content.push_str(if lines[edit.last_line - 1].ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            });
        }
        ranges.push((
            offsets[edit.first_line - 1],
            offsets[edit.last_line],
            content,
        ));
    }
    ranges.sort_by_key(|r| r.0);
    if ranges
        .windows(2)
        .any(|w| w[0].1 > w[1].0 || w[0].0 == w[1].0)
    {
        return Err("更新箇所が重なっています".into());
    }
    let mut after = before.to_owned();
    for (start, end, content) in ranges.into_iter().rev() {
        after.replace_range(start..end, &content);
    }
    if !valid_text(&after) {
        return Err("変更後のファイルが64 KiBを超えるか、バイナリを含みます".into());
    }
    Ok(after)
}
pub fn prepare_changes(root: &Path, proposal: &ChangeProposal) -> Result<PreparedChanges, String> {
    if proposal.version != 1 || proposal.changes.is_empty() || proposal.changes.len() > 32 {
        return Err("変更案のバージョン・件数が不正です".into());
    }
    let workspace = Workspace::open(root)?;
    let creates = proposal
        .changes
        .iter()
        .filter(|c| matches!(c, FileChange::Create { .. }))
        .count();
    if workspace.files()?.len() + creates > 2000 {
        return Err(
            "適用中のファイル数が一覧上限の2000件を超えます。作業フォルダを絞ってください".into(),
        );
    }
    if workspace.root().to_str().is_none() {
        return Err("作業フォルダ名はUTF-8で表せるものを指定してください".into());
    }
    let mut targets = HashSet::new();
    let mut files = Vec::new();
    for change in &proposal.changes {
        let target = safe_target(&workspace, change.path())?;
        if !targets.insert(target) {
            return Err("一つのファイルの変更は同じ項目にまとめてください".into());
        }
        let before = current(&workspace, change.path())?;
        #[allow(unused_mut)]
        let mut mode = None;
        if before.is_some() {
            let file = workspace
                .dir
                .open(change.path())
                .map_err(|_| "変更対象を開けません")?
                .into_std();
            let metadata = file
                .metadata()
                .map_err(|_| "変更対象の属性を取得できません")?;
            if metadata.permissions().readonly() {
                return Err("読み取り専用ファイルは変更できません".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                mode = Some(metadata.permissions().mode() & 0o777);
            }
        }
        let after = match change {
            FileChange::Create { path, content } => {
                if before.is_some() || !valid_text(content) {
                    return Err("追加先が既に存在するか、内容が上限を超えています".into());
                }
                create_allowed(&workspace, Path::new(path))?;
                Some(content.clone())
            }
            FileChange::Replace {
                expected_sha256,
                edits,
                ..
            } => {
                let text = before.as_deref().ok_or("更新対象がありません")?;
                if hash(text) != expected_sha256.to_ascii_lowercase() {
                    return Err(
                        "元ファイルのSHA-256が一致しません。再読み込みして提案を作り直してください"
                            .into(),
                    );
                }
                Some(replace_lines(text, edits)?)
            }
            FileChange::Delete {
                expected_sha256, ..
            } => {
                if before
                    .as_ref()
                    .is_none_or(|text| hash(text) != expected_sha256.to_ascii_lowercase())
                {
                    return Err("削除対象のSHA-256が一致しません".into());
                }
                None
            }
        };
        if before != after {
            files.push(SavedChange {
                path: change.path().to_owned(),
                before,
                after,
                mode,
            });
        }
    }
    if files.is_empty() {
        return Err("現在のファイルとの差分がありません".into());
    }
    Ok(PreparedChanges {
        root: workspace.root().to_owned(),
        files,
    })
}
fn nonce() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| "一時ファイル名を生成できません")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn new_change_journal_path() -> Result<PathBuf, String> {
    Ok(crate::default_config_path()
        .ok_or("変更記録の保存先を取得できません")?
        .with_file_name("changes")
        .join(format!("{}.json", nonce()?)))
}
fn journal_location(root: &Path, path: &Path, create: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let absolute = if parent.is_absolute() {
        parent.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|_| "現在のフォルダを取得できません")?
            .join(parent)
    };
    let existing = absolute
        .ancestors()
        .find(|p| p.exists())
        .ok_or("変更記録の保存先を確認できません")?;
    if existing
        .canonicalize()
        .map_err(|_| "変更記録の保存先を確認できません")?
        .starts_with(root)
    {
        return Err("取り消し用の変更記録は作業フォルダの外へ保存してください".into());
    }
    if create {
        fs::create_dir_all(parent).map_err(|_| "変更記録のフォルダを作成できません")?;
    }
    if parent
        .canonicalize()
        .map_err(|_| "変更記録のフォルダを確認できません")?
        .starts_with(root)
    {
        return Err("取り消し用の変更記録は作業フォルダの外へ保存してください".into());
    }
    Ok(())
}
fn save_journal(path: &Path, journal: &Journal, new: bool) -> Result<(), String> {
    let bytes = serde_json::to_vec(journal).map_err(|_| "変更記録を変換できません")?;
    if bytes.len() > MAX_JOURNAL {
        return Err("変更記録が16 MiBを超えています".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "変更記録の一時ファイルを作成できません")?;
    file.write_all(&bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "変更記録を保存できません。ファイル変更を中断します")?;
    if new {
        file.persist_noclobber(path)
    } else {
        file.persist(path)
    }
    .map_err(|_| "変更記録を保存できません。既存ファイル・権限を確認してください")?;
    Ok(())
}
struct Staged<'a> {
    dir: &'a Dir,
    name: String,
}
impl Drop for Staged<'_> {
    fn drop(&mut self) {
        let _ = self.dir.remove_file(&self.name);
    }
}
fn write_change(workspace: &Workspace, file: &SavedChange) -> Result<(), String> {
    safe_target(workspace, &file.path)?;
    let path = Path::new(&file.path);
    let parent_path = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = workspace
        .dir
        .open_dir(parent_path)
        .map_err(|_| "変更先フォルダを開けません")?;
    let name = path.file_name().ok_or("変更先名がありません")?;
    if let Some(after) = &file.after {
        let staged = Staged {
            dir: &parent,
            name: format!(".codebot-{}.tmp", nonce()?),
        };
        let mut output = parent
            .open_with(
                &staged.name,
                OpenOptions::new().write(true).create_new(true),
            )
            .map_err(|_| "変更用ファイルを作成できません")?
            .into_std();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output
                .set_permissions(fs::Permissions::from_mode(file.mode.unwrap_or(0o600)))
                .map_err(|_| "ファイル属性を設定できません")?;
        }
        output
            .write_all(after.as_bytes())
            .and_then(|_| output.sync_all())
            .map_err(|_| "変更内容を書き込めません")?;
        drop(output);
        if file.before.is_none() {
            // Atomic creation never replaces a concurrently created destination.
            parent.hard_link(&staged.name, &parent, name).map_err(|_| {
                "追加先が存在するか、安全な追加をサポートしていないファイルシステムです"
            })?;
        } else {
            parent
                .rename(&staged.name, &parent, name)
                .map_err(|_| "変更を置き換えられません")?;
        }
    } else {
        parent
            .remove_file(name)
            .map_err(|_| "対象ファイルを削除できません")?;
    }
    Ok(())
}
fn verify_before(workspace: &Workspace, files: &[SavedChange]) -> Result<(), String> {
    for file in files {
        if current(workspace, &file.path)? != file.before {
            return Err("プレビュー後にファイルが変わりました。変更せずに停止しました".into());
        }
        if file.before.is_some() {
            let metadata = workspace
                .dir
                .open(&file.path)
                .map_err(|_| "対象の属性を確認できません")?
                .into_std()
                .metadata()
                .map_err(|_| "対象の属性を確認できません")?;
            if metadata.permissions().readonly() {
                return Err("対象が読み取り専用になっています".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o777 != file.mode.unwrap_or(0o600) {
                    return Err("プレビュー後にファイル属性が変わりました".into());
                }
            }
        }
    }
    Ok(())
}
pub fn apply_changes(prepared: &PreparedChanges, journal_path: &Path) -> Result<usize, String> {
    apply_with(prepared, journal_path, write_change)
}
fn apply_with(
    prepared: &PreparedChanges,
    journal_path: &Path,
    mut write: impl FnMut(&Workspace, &SavedChange) -> Result<(), String>,
) -> Result<usize, String> {
    let workspace = Workspace::open(&prepared.root)?;
    verify_before(&workspace, &prepared.files)?;
    journal_location(&prepared.root, journal_path, true)?;
    let mut journal = Journal {
        version: 1,
        root: prepared
            .root
            .to_str()
            .ok_or("作業フォルダ名を保存できません")?
            .to_owned(),
        state: "prepared".into(),
        files: prepared.files.clone(),
    };
    save_journal(journal_path, &journal, true)?;
    for file in &prepared.files {
        let result = verify_before(&workspace, std::slice::from_ref(file))
            .and_then(|_| write(&workspace, file));
        if let Err(error) = result {
            let rollback = undo_changes(&prepared.root, journal_path);
            return Err(format!(
                "{error}。{}。記録: {}",
                if rollback.is_ok() {
                    "適用した変更を取り消しました"
                } else {
                    "競合または保存エラーのため自動で取り消せません。記録を保管して確認してください"
                },
                journal_path.display()
            ));
        }
    }
    journal.state = "applied".into();
    save_journal(journal_path,&journal,false).map_err(|_|format!("変更は適用済みですが記録の状態更新に失敗しました。preparedの記録から取り消し可能です: {}",journal_path.display()))?;
    Ok(prepared.files.len())
}
fn load_journal(root: &Path, path: &Path) -> Result<(Workspace, Journal), String> {
    let workspace = Workspace::open(root)?;
    journal_location(workspace.root(), path, false)?;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "変更記録を開けません")?
        .take((MAX_JOURNAL + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "変更記録を読み込めません")?;
    if bytes.len() > MAX_JOURNAL {
        return Err("変更記録のサイズが上限を超えています".into());
    }
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|_| "変更記録の形式が不正です")?;
    if journal.version != 1
        || journal.files.is_empty()
        || journal.files.len() > 32
        || journal.root != workspace.root().to_string_lossy()
        || !["prepared", "applied"].contains(&journal.state.as_str())
    {
        return Err(
            "変更記録のバージョン・作業フォルダ・状態が一致しないか、既に取り消し済みです".into(),
        );
    }
    let mut targets = HashSet::new();
    for file in &journal.files {
        if !targets.insert(safe_target(&workspace, &file.path)?)
            || file.before.as_ref().is_some_and(|t| !valid_text(t))
            || file.after.as_ref().is_some_and(|t| !valid_text(t))
            || file.mode.is_some_and(|m| m > 0o777)
        {
            return Err("変更記録のファイル情報が不正です".into());
        }
    }
    Ok((workspace, journal))
}
fn inverse(workspace: &Workspace, journal: &Journal) -> Result<Vec<SavedChange>, String> {
    let mut inverse = Vec::new();
    for file in journal.files.iter().rev() {
        let now = current(workspace, &file.path)?;
        if now == file.before {
            continue;
        }
        if now != file.after {
            return Err(
                "適用後にファイルが変更されています。新しい変更を保護して取り消しを中断します"
                    .into(),
            );
        }
        inverse.push(SavedChange {
            path: file.path.clone(),
            before: file.after.clone(),
            after: file.before.clone(),
            mode: file.mode,
        });
    }
    Ok(inverse)
}
pub fn preview_undo(root: &Path, path: &Path) -> Result<Vec<ChangePreview>, String> {
    let (workspace, journal) = load_journal(root, path)?;
    Ok(previews(&inverse(&workspace, &journal)?))
}
pub fn undo_changes(root: &Path, path: &Path) -> Result<usize, String> {
    let (workspace, mut journal) = load_journal(root, path)?;
    let changes = inverse(&workspace, &journal)?;
    for file in &changes {
        verify_before(&workspace, std::slice::from_ref(file))?;
        write_change(&workspace, file)?;
    }
    journal.state = "undone".into();
    save_journal(path, &journal, false)?;
    Ok(changes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn replace(path: &str, before: &str, line: usize, after: &str) -> FileChange {
        FileChange::Replace {
            path: path.into(),
            expected_sha256: hash(before),
            edits: vec![LineEdit {
                first_line: line,
                last_line: line,
                content: after.into(),
            }],
        }
    }
    #[test]
    fn selected_edits_create_delete_and_undo_preserve_uncommitted_content() {
        let root = tempfile::tempdir().unwrap();
        let records = tempfile::tempdir().unwrap();
        let journal = records.path().join("change.json");
        let before = "uncommitted-first\r\nkeep\r\nthird\r\n";
        fs::write(root.path().join("source.rs"), before).unwrap();
        fs::write(root.path().join("old.txt"), "delete me\n").unwrap();
        let proposal = ChangeProposal {
            version: 1,
            changes: vec![
                FileChange::Replace {
                    path: "source.rs".into(),
                    expected_sha256: hash(before),
                    edits: vec![
                        LineEdit {
                            first_line: 1,
                            last_line: 1,
                            content: "do not select".into(),
                        },
                        LineEdit {
                            first_line: 3,
                            last_line: 3,
                            content: "new-third".into(),
                        },
                    ],
                },
                FileChange::Create {
                    path: "new.txt".into(),
                    content: "new file\n".into(),
                },
                FileChange::Delete {
                    path: "old.txt".into(),
                    expected_sha256: hash("delete me\n"),
                },
            ],
        };
        let selected = proposal
            .select(&[vec![false, true], vec![true], vec![true]])
            .unwrap();
        let prepared = prepare_changes(root.path(), &selected).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join("source.rs")).unwrap(),
            before
        );
        assert!(!journal.exists());
        assert!(prepared.previews()[0].diff.contains("+new-third"));
        assert!(!prepared.previews()[0].diff.contains("do not select"));
        assert_eq!(apply_changes(&prepared, &journal).unwrap(), 3);
        assert_eq!(
            fs::read_to_string(root.path().join("source.rs")).unwrap(),
            "uncommitted-first\r\nkeep\r\nnew-third\r\n"
        );
        assert!(!root.path().join("old.txt").exists());
        assert!(root.path().join("new.txt").exists());
        assert_eq!(preview_undo(root.path(), &journal).unwrap().len(), 3);
        assert_eq!(undo_changes(root.path(), &journal).unwrap(), 3);
        assert_eq!(
            fs::read_to_string(root.path().join("source.rs")).unwrap(),
            before
        );
        assert_eq!(
            fs::read_to_string(root.path().join("old.txt")).unwrap(),
            "delete me\n"
        );
        assert!(!root.path().join("new.txt").exists());
        assert!(undo_changes(root.path(), &journal).is_err());
    }
    #[test]
    fn edits_after_preview_or_application_are_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let records = tempfile::tempdir().unwrap();
        let file = root.path().join("source.rs");
        let journal = records.path().join("change.json");
        fs::write(&file, "base\n").unwrap();
        let proposal = ChangeProposal {
            version: 1,
            changes: vec![replace("source.rs", "base\n", 1, "bot")],
        };
        let prepared = prepare_changes(root.path(), &proposal).unwrap();
        fs::write(&file, "new user edit\n").unwrap();
        assert!(apply_changes(&prepared, &journal).is_err());
        assert!(!journal.exists());
        assert_eq!(fs::read_to_string(&file).unwrap(), "new user edit\n");
        fs::write(&file, "base\n").unwrap();
        apply_changes(&prepared, &journal).unwrap();
        fs::write(&file, "another user edit\n").unwrap();
        assert!(preview_undo(root.path(), &journal).is_err());
        assert!(undo_changes(root.path(), &journal).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "another user edit\n");
    }
    #[test]
    fn a_mid_batch_failure_rolls_back_using_the_saved_originals() {
        let root = tempfile::tempdir().unwrap();
        let records = tempfile::tempdir().unwrap();
        for name in ["a.rs", "b.rs"] {
            fs::write(root.path().join(name), "uncommitted\n").unwrap();
        }
        let proposal = ChangeProposal {
            version: 1,
            changes: vec![
                replace("a.rs", "uncommitted\n", 1, "first"),
                replace("b.rs", "uncommitted\n", 1, "second"),
            ],
        };
        let prepared = prepare_changes(root.path(), &proposal).unwrap();
        let mut count = 0;
        let error = apply_with(
            &prepared,
            &records.path().join("change.json"),
            |workspace, file| {
                count += 1;
                if count == 2 {
                    Err("injected write failure".into())
                } else {
                    write_change(workspace, file)
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("取り消しました"));
        for name in ["a.rs", "b.rs"] {
            assert_eq!(
                fs::read_to_string(root.path().join(name)).unwrap(),
                "uncommitted\n"
            );
        }
    }
    #[test]
    fn unsafe_paths_ignored_files_overlap_and_invalid_json_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".gitignore"), "ignored.txt\n").unwrap();
        for path in [
            "../escape",
            "/outside",
            ".env",
            "ignored.txt",
            "CON",
            "file:stream",
        ] {
            let proposal = ChangeProposal {
                version: 1,
                changes: vec![FileChange::Create {
                    path: path.into(),
                    content: "data".into(),
                }],
            };
            assert!(prepare_changes(root.path(), &proposal).is_err(), "{path}");
        }
        let text = "a\nb\nc\n";
        assert!(replace_lines(
            text,
            &[
                LineEdit {
                    first_line: 1,
                    last_line: 2,
                    content: "x".into()
                },
                LineEdit {
                    first_line: 2,
                    last_line: 3,
                    content: "y".into()
                }
            ]
        )
        .is_err());
        let error = ChangeProposal::parse("private-secret invalid JSON")
            .err()
            .unwrap();
        assert!(!error.contains("private-secret"));
        assert!(ChangeProposal::parse("{\"version\":99,\"changes\":[]}").is_err());
        assert!(ChangeProposal::parse("```json\n{\"version\":1,\"changes\":[{\"operation\":\"create\",\"path\":\"new.rs\",\"content\":\"\"}]}\n```").is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn linked_paths_and_journals_inside_the_project_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let unsafe_proposal = ChangeProposal {
            version: 1,
            changes: vec![FileChange::Create {
                path: "link/file.rs".into(),
                content: "escape".into(),
            }],
        };
        assert!(prepare_changes(root.path(), &unsafe_proposal).is_err());
        assert!(!outside.path().join("file.rs").exists());
        let proposal = ChangeProposal {
            version: 1,
            changes: vec![FileChange::Create {
                path: "safe.rs".into(),
                content: "safe".into(),
            }],
        };
        let prepared = prepare_changes(root.path(), &proposal).unwrap();
        assert!(apply_changes(&prepared, &root.path().join("backups/change.json")).is_err());
        assert!(!root.path().join("safe.rs").exists());
        assert!(!root.path().join("backups").exists());
    }
}
