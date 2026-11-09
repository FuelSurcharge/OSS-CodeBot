use fs2::FileExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Advisory OS lock: released by the OS even after a crashed process.
pub(crate) struct CacheLease {
    _file: fs::File,
}
pub(crate) fn lease(root: &Path, exclusive: bool) -> Result<CacheLease, String> {
    fs::create_dir_all(root).map_err(|_| "キャッシュの保存先を作成できません")?;
    let path = root.join(".codebot-cache.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("キャッシュのロックがリンクです。保存先を確認してください".into());
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|_| "キャッシュのロックを開けません")?;
    let result = if exclusive {
        FileExt::try_lock_exclusive(&file)
    } else {
        FileExt::try_lock_shared(&file)
    };
    result.map_err(|_| {
        "キャッシュはモデルの読み込み・推論・取得・削除で使用中です。停止してから再実行してください"
    })?;
    Ok(CacheLease { _file: file })
}
pub(crate) fn model_lease(path: &Path) -> Result<Option<CacheLease>, String> {
    let path = fs::canonicalize(path).map_err(|_| "モデルのパスを確認できません")?;
    // Also recognize caches explicitly selected by CLI callers.
    for root in path.ancestors().skip(1).take(64) {
        if root.join(".codebot-cache.lock").exists() {
            return lease(root, false).map(Some);
        }
    }
    let Some(root) = crate::LocalConfig::default_cache() else {
        return Ok(None);
    };
    if !root.exists() {
        return Ok(None);
    }
    let root = fs::canonicalize(root).map_err(|_| "キャッシュを確認できません")?;
    if path.starts_with(&root) {
        lease(&root, false).map(Some)
    } else {
        Ok(None)
    }
}
pub(crate) fn require_space(root: &Path, bytes: u64) -> Result<(), String> {
    let free = fs2::available_space(root).map_err(|_| "キャッシュの空き容量を確認できません")?;
    if free < bytes.saturating_add(64 * 1024 * 1024) {
        return Err(
            "モデルの取得に必要な空き容量がありません（取得残量と64 MiBの余裕が必要です）".into(),
        );
    }
    Ok(())
}
#[derive(Clone, Serialize)]
pub struct ModelCacheEntry {
    pub id: String,
    pub format: String,
    pub bytes: u64,
    pub files: usize,
    pub file_names: Vec<String>,
    pub ready: bool,
}
#[derive(Clone, Serialize)]
pub struct ModelCacheInventory {
    pub root: PathBuf,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub in_use: bool,
    pub entries: Vec<ModelCacheEntry>,
}
fn hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn allowed(id: &str) -> bool {
    if let Some(name) = id.strip_prefix("huggingface/models--") {
        return !name.is_empty()
            && name.len() <= 220
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    }
    if let Some(name) = id.strip_prefix("gguf-") {
        return hex(name.strip_suffix(".partial").unwrap_or(name));
    }
    id.strip_suffix(".gguf").is_some_and(hex)
        || id.strip_suffix(".part").is_some_and(|s| {
            s.split('-').next().is_some_and(hex)
                && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        })
}
fn ready(id: &str, names: &[String]) -> bool {
    if id.starts_with("huggingface/") {
        return names
            .iter()
            .filter_map(|n| n.strip_suffix("/config.json"))
            .filter(|n| n.starts_with("snapshots/"))
            .any(|prefix| {
                names
                    .iter()
                    .any(|n| n.starts_with(&format!("{prefix}/")) && n.ends_with(".safetensors"))
            });
    }
    !id.ends_with(".partial") && !id.ends_with(".part")
}
// Never follow links while measuring or deleting a Hugging Face repository.
fn measure(path: &Path) -> Result<(u64, usize, String, Vec<String>), String> {
    let mut pending = vec![(path.to_path_buf(), 0)];
    let mut bytes = 0u64;
    let mut files = 0usize;
    let mut seen = 0usize;
    let mut hash = Sha256::new();
    let mut file_names = Vec::new();
    while let Some((next, depth)) = pending.pop() {
        seen += 1;
        if depth > 64 || seen > 100_000 {
            return Err("キャッシュのファイル数・階層が上限を超えています".into());
        }
        let m = fs::symlink_metadata(&next).map_err(|_| "キャッシュを読み込めません")?;
        let relative = next
            .strip_prefix(path)
            .unwrap()
            .to_str()
            .ok_or("キャッシュのパスがUTF-8ではありません")?;
        hash.update((relative.len() as u64).to_le_bytes());
        if !m.is_dir() || m.file_type().is_symlink() {
            file_names.push(if relative.is_empty() {
                next.file_name().unwrap().to_string_lossy().into_owned()
            } else {
                relative.replace('\\', "/")
            });
        }
        hash.update(relative.as_bytes());
        hash.update([m.is_dir() as u8, m.file_type().is_symlink() as u8]);
        hash.update(m.len().to_le_bytes());
        hash.update(
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or_default()
                .to_le_bytes(),
        );
        if m.file_type().is_symlink() {
            hash.update(
                fs::read_link(&next)
                    .map_err(|_| "キャッシュのリンクを確認できません")?
                    .to_string_lossy()
                    .as_bytes(),
            );
            files += 1;
        } else if m.is_dir() {
            let mut children = fs::read_dir(&next)
                .map_err(|_| "キャッシュを読み込めません")?
                .take(100_001)
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "キャッシュを読み込めません")?;
            if children.len() > 100_000 {
                return Err("キャッシュのファイル数が上限を超えています".into());
            }
            children.sort();
            pending.extend(children.into_iter().rev().map(|p| (p, depth + 1)));
        } else if m.is_file() {
            bytes = bytes
                .checked_add(m.len())
                .ok_or("容量の上限を超えています")?;
            files += 1;
        } else {
            return Err("キャッシュに通常のファイル以外が含まれています".into());
        }
    }
    Ok((bytes, files, format!("{:x}", hash.finalize()), file_names))
}
fn cache_root(root: &Path) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("キャッシュは絶対パスで指定してください".into());
    }
    fs::create_dir_all(root).map_err(|_| "キャッシュを作成できません")?;
    fs::canonicalize(root).map_err(|_| "キャッシュを確認できません".into())
}
fn entry_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !allowed(id) {
        return Err("アプリが管理するモデルのIDを選択してください".into());
    }
    let path = root.join(id);
    if path
        .components()
        .any(|p| matches!(p, std::path::Component::ParentDir))
        || fs::symlink_metadata(&path)
            .map_err(|_| "対象がありません")?
            .file_type()
            .is_symlink()
        || path
            .parent()
            .is_some_and(|p| fs::canonicalize(p).ok().as_deref() != Some(p))
    {
        return Err("キャッシュの対象・親フォルダがリンクか、変更されています".into());
    }
    Ok(path)
}
pub fn model_cache_inventory(root: &Path) -> Result<ModelCacheInventory, String> {
    let root = cache_root(root)?;
    let guard = lease(&root, true).ok();
    let in_use = guard.is_none();
    let mut ids = Vec::new();
    for parent in [&root, &root.join("huggingface")] {
        if !parent.exists()
            || fs::symlink_metadata(parent)
                .map_err(|_| "キャッシュを確認できません")?
                .file_type()
                .is_symlink()
        {
            continue;
        }
        for entry in fs::read_dir(parent)
            .map_err(|_| "キャッシュを確認できません")?
            .take(2001)
        {
            let path = entry.map_err(|_| "キャッシュを確認できません")?.path();
            let id = path
                .strip_prefix(&root)
                .unwrap()
                .to_str()
                .ok_or("キャッシュのパスがUTF-8ではありません")?
                .replace('\\', "/");
            if allowed(&id) {
                ids.push(id)
            }
            if ids.len() > 2000 {
                return Err("キャッシュの項目数が上限を超えています".into());
            }
        }
    }
    ids.sort();
    let mut entries = Vec::new();
    let mut total_bytes = 0u64;
    for id in ids {
        let path = entry_path(&root, &id)?;
        let (bytes, files, _, file_names) = measure(&path)?;
        if entries
            .iter()
            .map(|e: &ModelCacheEntry| e.files)
            .sum::<usize>()
            + files
            > 100_000
        {
            return Err("キャッシュの総ファイル数が上限を超えています".into());
        }
        total_bytes = total_bytes.saturating_add(bytes);
        entries.push(ModelCacheEntry {
            format: if id.starts_with("huggingface/") {
                "Transformers"
            } else {
                "GGUF"
            }
            .into(),
            ready: ready(&id, &file_names),
            id,
            bytes,
            files,
            file_names,
        });
    }
    Ok(ModelCacheInventory {
        free_bytes: fs2::available_space(&root).map_err(|_| "空き容量を確認できません")?,
        root,
        total_bytes,
        in_use,
        entries,
    })
}
pub struct PreparedCacheDelete {
    root: PathBuf,
    entry: ModelCacheEntry,
    stamp: String,
}
impl PreparedCacheDelete {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn entry(&self) -> &ModelCacheEntry {
        &self.entry
    }
}
pub fn prepare_cache_delete(root: &Path, id: &str) -> Result<PreparedCacheDelete, String> {
    let root = cache_root(root)?;
    let _guard = lease(&root, true)?;
    let path = entry_path(&root, id)?;
    let (bytes, files, stamp, file_names) = measure(&path)?;
    Ok(PreparedCacheDelete {
        root,
        entry: ModelCacheEntry {
            id: id.into(),
            bytes,
            files,
            ready: ready(id, &file_names),
            file_names,
            format: if id.starts_with("huggingface/") {
                "Transformers"
            } else {
                "GGUF"
            }
            .into(),
        },
        stamp,
    })
}
pub fn delete_model_cache(plan: &PreparedCacheDelete) -> Result<(), String> {
    let _guard = lease(&plan.root, true)?;
    let path = entry_path(&plan.root, &plan.entry.id)?;
    if measure(&path)?.2 != plan.stamp {
        return Err("確認後に対象が変更されました。再度一覧と削除対象を確認してください".into());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|_| "キャッシュの削除が完了していません。一覧を再取得してください".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locks_protect_live_models_and_downloads_and_release_without_stale_pid_files() {
        let dir = tempfile::tempdir().unwrap();
        let shared = lease(dir.path(), false).unwrap();
        assert!(lease(dir.path(), true).is_err());
        assert!(model_cache_inventory(dir.path()).unwrap().in_use);
        let second = lease(dir.path(), false).unwrap();
        drop(shared);
        drop(second);
        let exclusive = lease(dir.path(), true).unwrap();
        assert!(lease(dir.path(), false).is_err());
        drop(exclusive);
        assert!(lease(dir.path(), true).is_ok());
    }
    #[test]
    fn cache_deletion_requires_an_unchanged_managed_entry_and_preserves_external_files() {
        let dir = tempfile::tempdir().unwrap();
        let id = format!("{}.gguf", "a".repeat(64));
        fs::write(dir.path().join(&id), b"model").unwrap();
        fs::write(dir.path().join("user.txt"), b"keep").unwrap();
        assert_eq!(model_cache_inventory(dir.path()).unwrap().total_bytes, 5);
        for bad in [
            "../user.txt",
            "user.txt",
            "huggingface/../../user.txt",
            "gguf-foo",
        ] {
            assert!(prepare_cache_delete(dir.path(), bad).is_err())
        }
        let plan = prepare_cache_delete(dir.path(), &id).unwrap();
        fs::write(dir.path().join(&id), b"changed model").unwrap();
        assert!(delete_model_cache(&plan).is_err());
        let plan = prepare_cache_delete(dir.path(), &id).unwrap();
        let shared = lease(dir.path(), false).unwrap();
        assert!(delete_model_cache(&plan).is_err());
        drop(shared);
        delete_model_cache(&plan).unwrap();
        assert_eq!(fs::read(dir.path().join("user.txt")).unwrap(), b"keep");
        assert!(require_space(dir.path(), u64::MAX).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn repository_symlinks_are_not_followed_during_measurement_or_deletion() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep"), b"external").unwrap();
        let repo = dir.path().join("huggingface/models--owner--repo");
        fs::create_dir_all(&repo).unwrap();
        symlink(outside.path(), repo.join("linked")).unwrap();
        let plan = prepare_cache_delete(dir.path(), "huggingface/models--owner--repo").unwrap();
        assert_eq!(plan.entry.bytes, 0);
        delete_model_cache(&plan).unwrap();
        assert_eq!(fs::read(outside.path().join("keep")).unwrap(), b"external");
    }
}
