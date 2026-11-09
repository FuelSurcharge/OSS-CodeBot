use crate::{DownloadProgress, LocalConfig};
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
const SCRIPT: &str = include_str!("../../../scripts/transformers/runtime.py");

pub(crate) fn error_message(code: &str) -> String {
    match code {
    "dependencies"=>"選択したPython環境にtorch・transformers・accelerate・safetensors・huggingface_hubをインストールしてください",
    "format"=>"config.json・safetensors重み・トークナイザーが揃ったモデルフォルダを指定してください",
    "unsupported"=>"このモデルは現在のTransformersで読み込めません。対応バージョンを確認してください。独自リポジトリコードは実行しません",
    "integrity"=>"配布元のハッシュまたはファイルサイズに一致しません。キャッシュを確認して再取得してください",
    "size"=>"モデルが設定した取得サイズ上限を超えています",
    "disk"=>"モデルの取得に必要な空き容量がありません（全体サイズと64 MiBの余裕を確認します）",
    "device"=>"指定したGPUを使用できません。PyTorch環境またはCPU設定を確認してください",
    "memory"=>"モデルを読み込むメモリが不足しています。モデルサイズやデバイス設定を確認してください",
    _=>"公式形式モデルの処理に失敗しました。Python依存・モデル形式・配布元へのアクセス・利用同意を確認してください",
}.into()
}

pub(crate) fn parse_repository(raw: &str, revision: &str) -> Result<(String, String), String> {
    let raw = raw.trim();
    let mut revision = revision.trim().to_owned();
    let repo = if raw.starts_with("https://") {
        let url = Url::parse(raw).map_err(|_| "Hugging FaceリポジトリURLが不正です")?;
        if url.host_str() != Some("huggingface.co")
            || url.port().is_some_and(|p| p != 443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "認証情報を含まない公式huggingface.coのリポジトリURLを指定してください".into(),
            );
        }
        let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
        if parts.len() == 4 && parts[2] == "tree" {
            revision = parts[3].into();
        } else if parts.len() != 2 {
            return Err("公式形式ではファイルURLではなくリポジトリURLを指定してください".into());
        }
        format!("{}/{}", parts[0], parts[1])
    } else {
        raw.to_owned()
    };
    let valid = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    };
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || !parts.iter().all(|p| valid(p))
        || revision.len() > 200
        || !revision.split('/').all(valid)
    {
        return Err("owner/repo形式のIDと有効なrevisionを指定してください".into());
    }
    Ok((repo, revision))
}
pub(crate) fn validate_folder(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err(error_message("format"));
    }
    let config = fs::read(path.join("config.json")).map_err(|_| error_message("format"))?;
    if config.len() > 1024 * 1024 {
        return Err(error_message("format"));
    }
    let value: Value = serde_json::from_slice(&config).map_err(|_| error_message("format"))?;
    if value["model_type"].as_str().is_none()
        || !fs::read_dir(path)
            .map_err(|_| error_message("format"))?
            .filter_map(Result::ok)
            .any(|e| e.path().extension().is_some_and(|x| x == "safetensors"))
    {
        return Err(error_message("format"));
    }
    Ok(())
}
pub(crate) fn bridge_path() -> Result<PathBuf, String> {
    let dir = LocalConfig::default_cache()
        .ok_or("ランタイム保存先を取得できません")?
        .join("runtimes");
    fs::create_dir_all(&dir).map_err(|_| "ランタイム保存先を作成できません")?;
    let hash = format!("{:x}", Sha256::digest(SCRIPT.as_bytes()));
    let path = dir.join(format!("transformers-{hash}.py"));
    match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(mut file) => {
            file.write_all(SCRIPT.as_bytes())
                .map_err(|_| "ランタイムを配置できません")?;
            file.sync_all().map_err(|_| "ランタイムを保存できません")?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(&path).map_err(|_| "ランタイムを検証できません")? != SCRIPT.as_bytes()
            {
                return Err("キャッシュ内のランタイムが一致しません。該当ファイルを退避して再実行してください".into());
            }
        }
        Err(_) => return Err("ランタイムを配置できません".into()),
    }
    Ok(path)
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub(crate) fn download_model(
    config: &LocalConfig,
    cache: &Path,
    token: Option<&str>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf, String> {
    let (repo, revision) = parse_repository(&config.hf_url, &config.hf_revision)?;
    if !(1..=4096).contains(&config.max_download_gib) {
        return Err("取得サイズ上限は1〜4096 GiBで指定してください".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("キャンセルしました".into());
    }
    let mut command = Command::new(config.python_path.trim());
    command
        .arg(bridge_path()?)
        .arg("download")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = OwnedChild(
        command
            .spawn()
            .map_err(|_| "Pythonを起動できません。実行ファイルのパスを確認してください")?,
    );
    let options = json!({"repo":repo,"revision":revision,"cache":cache.join("huggingface"),"token":token,"max_bytes":u64::from(config.max_download_gib)*1024*1024*1024});
    child
        .0
        .stdin
        .take()
        .ok_or("取得プロセスを準備できません")?
        .write_all(options.to_string().as_bytes())
        .map_err(|_| "取得プロセスに接続できません")?;
    let stdout = child.0.stdout.take().ok_or("取得状態を確認できません")?;
    let (tx, rx) = mpsc::sync_channel(16);
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if tx.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let mut result = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("取得をキャンセルしました（部分キャッシュは次回再開に使用します）".into());
        }
        while let Ok(event) = rx.try_recv() {
            match event["event"].as_str() {
                Some("progress") => progress(DownloadProgress {
                    received: event["received"].as_u64().unwrap_or(0),
                    total: event["total"].as_u64(),
                }),
                Some("done") => result = event["path"].as_str().map(PathBuf::from),
                Some("error") => {
                    return Err(error_message(event["code"].as_str().unwrap_or("download")))
                }
                _ => {}
            }
        }
        if let Some(status) = child
            .0
            .try_wait()
            .map_err(|_| "取得プロセスを確認できません")?
        {
            // A completed process can race the stdout reader: drain until it closes.
            for event in rx.iter() {
                match event["event"].as_str() {
                    Some("done") => result = event["path"].as_str().map(PathBuf::from),
                    Some("error") => {
                        return Err(error_message(event["code"].as_str().unwrap_or("download")))
                    }
                    _ => {}
                }
            }
            if status.success() {
                let path = result.ok_or("取得したフォルダを確認できません")?;
                validate_folder(&path)?;
                return Ok(path);
            }
            return Err(error_message("download"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repository_ids_and_urls_do_not_allow_credentials_or_file_paths() {
        assert_eq!(
            parse_repository("org/model", "main").unwrap(),
            ("org/model".into(), "main".into())
        );
        assert_eq!(
            parse_repository("https://huggingface.co/org/model/tree/release", "main")
                .unwrap()
                .1,
            "release"
        );
        for raw in [
            "https://evil.example/org/model",
            "https://huggingface.co/org/model?token=secret",
            "https://user:secret@huggingface.co/org/model",
            "https://huggingface.co/org/model/resolve/main/model.safetensors",
            "../model",
        ] {
            assert!(parse_repository(raw, "main").is_err());
        }
        assert!(parse_repository("org/model", "../escape").is_err());
    }
}
