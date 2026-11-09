use crate::{DownloadProgress, LocalConfig};
use reqwest::{
    header::{CONTENT_RANGE, ETAG, IF_RANGE, RANGE},
    Method, Url,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 240
        && s != "."
        && s != ".."
        && !s.ends_with(['.', ' '])
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && !matches!(
            s.split('.').next().unwrap().to_ascii_uppercase().as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
}
/// llama.cpp's portable split filename convention. Only the first shard is loadable.
pub(crate) fn split_names(name: &str) -> Result<Option<Vec<String>>, String> {
    let Some(stem) = name.strip_suffix(".gguf") else {
        return Ok(None);
    };
    let Some((prefix, total)) = stem.rsplit_once("-of-") else {
        return Ok(None);
    };
    let Some((base, index)) = prefix.rsplit_once('-') else {
        return Err("分割GGUFのファイル名が不正です".into());
    };
    if total.len() != 5
        || index.len() != 5
        || !total.bytes().all(|b| b.is_ascii_digit())
        || !index.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("分割GGUFの番号は5桁で指定してください".into());
    }
    let count: usize = total.parse().map_err(|_| "分割数が不正です")?;
    if index != "00001" || !(2..=1024).contains(&count) || !safe_name(base) {
        return Err("分割GGUFは先頭の00001パートを指定してください（最大1024パート）".into());
    }
    Ok(Some(
        (1..=count)
            .map(|n| format!("{base}-{n:05}-of-{count:05}.gguf"))
            .collect(),
    ))
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Part {
    name: String,
    source: String,
    size: u64,
    sha256: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    repository: String,
    revision: String,
    parts: Vec<Part>,
}
impl Manifest {
    fn key(&self) -> String {
        digest(&serde_json::to_vec(self).unwrap())
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !hash(&self.revision, 40)
            || self.parts.is_empty()
            || self.parts.len() > 1024
            || self
                .parts
                .iter()
                .any(|p| !safe_name(&p.name) || p.size < 24 || !hash(&p.sha256, 64))
        {
            return Err("モデルの検証情報が不正です".into());
        }
        let names =
            split_names(&self.parts[0].name)?.unwrap_or_else(|| vec![self.parts[0].name.clone()]);
        if names
            != self
                .parts
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
        {
            return Err("分割GGUFのパートが欠けているか、順序が不正です".into());
        }
        Ok(())
    }
}
struct Source {
    owner: String,
    repo: String,
    revision: String,
    folder: Vec<String>,
    name: String,
}
impl Source {
    fn parse(config: &LocalConfig) -> Result<Self, String> {
        let url = crate::local::normalize_hf_url(&config.hf_url)?;
        let segments = url
            .path_segments()
            .ok_or("ファイルURLが不正です")?
            .map(|s| {
                percent_encoding::percent_decode_str(s)
                    .decode_utf8()
                    .map(|s| s.into_owned())
                    .map_err(|_| "URLの文字が不正です")
            })
            .collect::<Result<Vec<_>, _>>()?;
        if segments.len() < 5
            || segments
                .iter()
                .enumerate()
                .any(|(i, s)| i != 2 && !s.split('/').all(safe_name))
            || segments[0].contains('/')
            || segments[1].contains('/')
            || segments[4..].iter().any(|s| s.contains('/'))
        {
            return Err(
                "モデルURLには安全なowner/repo・revision・ファイル名を指定してください".into(),
            );
        }
        Ok(Self {
            owner: segments[0].clone(),
            repo: segments[1].clone(),
            revision: segments[3].clone(),
            folder: segments[4..segments.len() - 1].to_vec(),
            name: segments.last().unwrap().clone(),
        })
    }
    fn url(&self, base: &Url, revision: Option<&str>, file: Option<&str>) -> Url {
        let mut url = base.clone();
        let mut path = url.path_segments_mut().unwrap();
        path.pop_if_empty();
        if let Some(file) = file {
            path.push(&self.owner)
                .push(&self.repo)
                .push("resolve")
                .push(revision.unwrap());
            for p in &self.folder {
                path.push(p);
            }
            path.push(file);
        } else {
            path.push("api")
                .push("models")
                .push(&self.owner)
                .push(&self.repo)
                .push("revision")
                .push(&self.revision);
        }
        drop(path);
        if file.is_none() {
            url.query_pairs_mut().append_pair("blobs", "true");
        }
        url
    }
}
struct Transport {
    base: Url,
    client: reqwest::Client,
    token: Option<String>,
}
impl Transport {
    fn new(base: Url, token: Option<&str>) -> Result<Self, String> {
        Ok(Self {
            base,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(20))
                .read_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(86400))
                .build()
                .map_err(|_| "取得用の接続を作成できません")?,
            token: token.filter(|s| !s.trim().is_empty()).map(str::to_owned),
        })
    }
    async fn request(
        &self,
        method: Method,
        mut url: Url,
        range: Option<(u64, &str)>,
    ) -> Result<reqwest::Response, String> {
        for _ in 0..8 {
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .header("accept-encoding", "identity");
            // Never forward a Hugging Face token to a CDN or response-selected host.
            if url.origin() == self.base.origin() {
                if let Some(token) = &self.token {
                    request = request.bearer_auth(token);
                }
            }
            if let Some((start, etag)) = range {
                request = request
                    .header(RANGE, format!("bytes={start}-"))
                    .header(IF_RANGE, etag);
            }
            let response = request
                .send()
                .await
                .map_err(|_| "モデルの通信が中断しました。再実行で検証可能な部分から再開します")?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or("転送先が不正です")?;
                let next = url.join(location).map_err(|_| "転送先が不正です")?;
                let host = next.host_str().unwrap_or("");
                let allowed = next.scheme() == "https"
                    && (host == "huggingface.co"
                        || host.ends_with(".huggingface.co")
                        || host.ends_with(".hf.co"));
                #[cfg(test)]
                let allowed = allowed || next.origin() == self.base.origin();
                if !allowed
                    || !next.username().is_empty()
                    || next.password().is_some()
                    || next
                        .port()
                        .is_some_and(|p| p != 443 && next.origin() != self.base.origin())
                {
                    return Err("Hugging Face管理外の転送先には取得しません".into());
                }
                url = next;
                continue;
            }
            if !response.status().is_success() {
                return Err(format!("Hugging Faceから取得できません（HTTP {}）。URL・権限・利用同意を確認してください",response.status().as_u16()));
            }
            return Ok(response);
        }
        Err("転送回数が上限を超えました".into())
    }
    async fn manifest(&self, source: &Source, specified: &str) -> Result<Manifest, String> {
        let mut response = self
            .request(Method::GET, source.url(&self.base, None, None), None)
            .await?;
        let mut raw = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "配布情報の取得が中断しました")?
        {
            if raw.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err("配布情報が上限を超えています".into());
            }
            raw.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&raw).map_err(|_| "配布情報のJSON形式が不正です")?;
        let revision = value["sha"]
            .as_str()
            .filter(|s| hash(s, 40))
            .ok_or("配布元の正確なコミットを取得できません")?
            .to_owned();
        let names = split_names(&source.name)?.unwrap_or_else(|| vec![source.name.clone()]);
        if names.len() > 1 && !specified.trim().is_empty() {
            return Err("分割GGUFのSHA-256は全パートの配布メタデータから取得します。単一ファイル用のSHA-256欄は空にしてください".into());
        }
        let siblings = value["siblings"]
            .as_array()
            .ok_or("配布ファイル一覧がありません")?;
        let mut parts = Vec::new();
        for name in names {
            let path = source
                .folder
                .iter()
                .cloned()
                .chain([name.clone()])
                .collect::<Vec<_>>()
                .join("/");
            let matches = siblings
                .iter()
                .filter(|s| s["rfilename"].as_str() == Some(&path))
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err("分割GGUFに欠損・重複があるか、指定ファイルがありません".into());
            }
            let entry = matches[0];
            let size = entry["size"]
                .as_u64()
                .ok_or("配布元からファイルサイズを取得できません")?;
            let supplied = entry
                .pointer("/lfs/sha256")
                .and_then(Value::as_str)
                .filter(|s| hash(s, 64));
            let sha256 = if specified.trim().is_empty() {
                supplied
                    .ok_or("配布元のSHA-256がありません。単一GGUFではSHA-256を指定できます")?
                    .to_owned()
            } else {
                let explicit = specified.trim().to_ascii_lowercase();
                if !hash(&explicit, 64)
                    || supplied.is_some_and(|h| !h.eq_ignore_ascii_case(&explicit))
                {
                    return Err("指定SHA-256と配布元の情報が一致しません".into());
                }
                explicit
            };
            parts.push(Part {
                name,
                source: path,
                size,
                sha256: sha256.to_ascii_lowercase(),
            });
        }
        let manifest = Manifest {
            version: 1,
            repository: format!("{}/{}", source.owner, source.repo),
            revision,
            parts,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}
fn file_hash(path: &Path, cancel: &AtomicBool) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|_| "モデルの検証ファイルを開けません")?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("取得をキャンセルしました。部分ファイルは次回の再開用に残します".into());
        }
        let n = file.read(&mut buf).map_err(|_| "モデルを検証できません")?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn regular(path: &Path) -> Result<Option<u64>, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() => Ok(Some(m.len())),
        Ok(_) => Err("キャッシュに通常ファイル以外が含まれています".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("キャッシュの状態を確認できません".into()),
    }
}
fn verify(root: &Path, manifest: &Manifest, cancel: &AtomicBool) -> Result<(), String> {
    manifest.validate()?;
    for part in &manifest.parts {
        let path = root.join(&part.name);
        if regular(&path)? != Some(part.size) || file_hash(&path, cancel)? != part.sha256 {
            return Err(
                "保存済みGGUFのサイズ・SHA-256が一致しません。キャッシュを確認してください".into(),
            );
        }
        crate::local::validate_gguf_header(&path)?;
    }
    Ok(())
}
pub(crate) fn validate_managed(path: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let Some(name) = parent
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| s.starts_with("gguf-"))
    else {
        return Ok(());
    };
    let key = name
        .strip_prefix("gguf-")
        .unwrap()
        .strip_suffix(".partial")
        .unwrap_or(&name[5..]);
    if !hash(key, 64) {
        return Ok(());
    }
    if name.ends_with(".partial") {
        return Err("取得途中のGGUFは読み込めません。取得を再開してください".into());
    }
    let manifest_path = parent.join("manifest.json");
    if regular(&manifest_path)?.is_none_or(|n| n > 2 * 1024 * 1024) {
        return Err("GGUFの取得完了情報がないか、上限を超えています".into());
    }
    let raw = fs::read(manifest_path).map_err(|_| "GGUFの取得完了情報を読めません")?;
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|_| "GGUFの取得情報が不正です")?;
    manifest.validate()?;
    if name != format!("gguf-{}", manifest.key())
        || path.file_name().and_then(|s| s.to_str()) != Some(&manifest.parts[0].name)
    {
        return Err("GGUFの取得情報または先頭パートが一致しません".into());
    }
    verify(parent, &manifest, cancel)
}
pub(crate) fn download(
    config: &LocalConfig,
    cache: &Path,
    token: Option<&str>,
    cancel: &AtomicBool,
    progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf, String> {
    config.validate_source()?;
    let transport = Transport::new(Url::parse("https://huggingface.co").unwrap(), token)?;
    download_with(config, cache, cancel, progress, transport)
}
fn download_with(
    config: &LocalConfig,
    cache: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(DownloadProgress),
    transport: Transport,
) -> Result<PathBuf, String> {
    if !(1..=4096).contains(&config.max_download_gib) {
        return Err("取得上限は1〜4096 GiBで指定してください".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("取得をキャンセルしました".into());
    }
    let source = Source::parse(config)?;
    let _lease = crate::model_cache::lease(cache, true)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "取得処理を作成できません")?;
    let future = async {
        let manifest = transport.manifest(&source, &config.sha256).await?;
        let total = manifest
            .parts
            .iter()
            .try_fold(0u64, |n, p| n.checked_add(p.size))
            .ok_or("配布サイズが上限を超えています")?;
        if total > u64::from(config.max_download_gib) * 1024 * 1024 * 1024 {
            return Err("モデルが取得サイズ上限を超えています".into());
        }
        let key = manifest.key();
        let destination = cache.join(format!("gguf-{key}"));
        if destination.exists() {
            if fs::symlink_metadata(&destination)
                .map_err(|_| "キャッシュを確認できません")?
                .file_type()
                .is_symlink()
            {
                return Err("モデルの保存先がリンクです".into());
            }
            verify(&destination, &manifest, cancel)?;
            progress(DownloadProgress {
                received: total,
                total: Some(total),
            });
            return Ok(destination.join(&manifest.parts[0].name));
        }
        let partial = cache.join(format!("gguf-{key}.partial"));
        if partial.exists()
            && fs::symlink_metadata(&partial)
                .map_err(|_| "部分キャッシュを確認できません")?
                .file_type()
                .is_symlink()
        {
            return Err("部分キャッシュがリンクです".into());
        }
        fs::create_dir_all(&partial).map_err(|_| "部分ファイルの保存先を作成できません")?;
        let present = manifest.parts.iter().try_fold(0u64, |n, p| {
            regular(&partial.join(&p.name)).map(|v| n + v.unwrap_or(0).min(p.size))
        })?;
        crate::model_cache::require_space(cache, total.saturating_sub(present))?;
        let mut complete = 0;
        for part in &manifest.parts {
            let output = partial.join(&part.name);
            if regular(&output)? == Some(part.size) && file_hash(&output, cancel)? == part.sha256 {
                crate::local::validate_gguf_header(&output)?;
                complete += part.size;
                continue;
            }
            let url = source.url(&transport.base, Some(&manifest.revision), Some(&part.name));
            let head = transport.request(Method::HEAD, url.clone(), None).await?;
            if head
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                != Some(part.size)
            {
                return Err("配布元のサイズが固定したメタデータと一致しません".into());
            }
            let etag = head
                .headers()
                .get(ETAG)
                .and_then(|v| v.to_str().ok())
                .filter(|s| s.starts_with('"') && s.ends_with('"') && s.len() <= 1024)
                .map(str::to_owned);
            let validator = partial.join(format!("{}.etag", part.name));
            if regular(&validator)?.is_some_and(|n| n > 1024) {
                return Err("再開情報が不正です".into());
            }
            let old = fs::read(&validator)
                .ok()
                .filter(|b| b.len() <= 1024)
                .and_then(|v| String::from_utf8(v).ok());
            let existing = regular(&output)?.unwrap_or(0);
            let start = if existing < part.size && etag.is_some() && old == etag {
                existing
            } else {
                0
            };
            // A changed/missing validator never reuses old bytes.
            if start == 0 && existing > 0 {
                fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&output)
                    .map_err(|_| "部分ファイルを初期化できません")?;
            }
            if let Some(etag) = &etag {
                fs::write(&validator, etag).map_err(|_| "再開情報を保存できません")?;
            } else {
                let _ = fs::remove_file(&validator);
            }
            crate::model_cache::require_space(cache, part.size - start)?;
            let mut response = transport
                .request(
                    Method::GET,
                    url,
                    if start > 0 {
                        Some((start, etag.as_deref().unwrap()))
                    } else {
                        None
                    },
                )
                .await?;
            let mut received = if response.status().as_u16() == 206 {
                start
            } else {
                0
            };
            if response.status().as_u16() == 206 {
                let expected = format!("bytes {start}-{}/{}", part.size - 1, part.size);
                if response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    != Some(&expected)
                    || start > 0
                        && response.headers().get(ETAG).and_then(|v| v.to_str().ok())
                            != etag.as_deref()
                {
                    return Err(
                        "再開応答の範囲・ETagが一致しません。部分ファイルは使用しません".into(),
                    );
                }
            }
            if response.status().as_u16() != 200 && response.status().as_u16() != 206 {
                return Err("モデルの取得応答が不正です".into());
            }
            let mut file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(received == 0)
                .append(received > 0)
                .open(&output)
                .map_err(|_| "モデルを保存できません")?;
            progress(DownloadProgress {
                received: complete + received,
                total: Some(total),
            });
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                "通信が中断しました。部分ファイルを残して終了しました。再実行で再開します"
            })? {
                if cancel.load(Ordering::Relaxed) {
                    return Err("取得をキャンセルしました。部分ファイルは再開用に残します".into());
                }
                received = received
                    .checked_add(chunk.len() as u64)
                    .ok_or("取得サイズが不正です")?;
                if received > part.size {
                    return Err("配布元の取得サイズが一致しません".into());
                }
                file.write_all(&chunk)
                    .map_err(|_| "モデルを保存できません。空き容量を確認してください")?;
                progress(DownloadProgress {
                    received: complete + received,
                    total: Some(total),
                });
            }
            file.sync_all()
                .map_err(|_| "モデルの保存を確定できません")?;
            drop(file);
            if received != part.size || file_hash(&output, cancel)? != part.sha256 {
                let _ = fs::remove_file(&output);
                let _ = fs::remove_file(&validator);
                return Err(
                    "モデルのサイズ・SHA-256が一致しません。不一致のパートを破棄しました".into(),
                );
            }
            crate::local::validate_gguf_header(&output)?;
            let _ = fs::remove_file(&validator);
            complete += part.size;
        }
        regular(&partial.join("manifest.json"))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(partial.join("manifest.json"))
            .map_err(|_| "完了情報を作成できません")?;
        serde_json::to_writer(&mut file, &manifest).map_err(|_| "完了情報を保存できません")?;
        file.sync_all().map_err(|_| "完了情報を保存できません")?;
        drop(file);
        verify(&partial, &manifest, cancel)?;
        fs::rename(&partial, &destination).map_err(|_| "検証済みモデルを配置できません")?;
        progress(DownloadProgress {
            received: total,
            total: Some(total),
        });
        Ok(destination.join(&manifest.parts[0].name))
    };
    runtime.block_on(async {tokio::select! {biased;
        _=async {while !cancel.load(Ordering::Relaxed) {tokio::time::sleep(Duration::from_millis(25)).await;}}=>Err("取得をキャンセルしました。部分ファイルは次回の再開用に残します".into()),
        result=future=>result
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };
    fn bytes(seed: u8) -> Vec<u8> {
        let mut bytes = vec![seed; 128];
        bytes[..4].copy_from_slice(b"GGUF");
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        bytes
    }
    fn metadata(files: &[(&str, Vec<u8>)], revision: char) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"sha":revision.to_string().repeat(40),"siblings":files.iter().map(|(name,b)|serde_json::json!({"rfilename":name,"size":b.len(),"lfs":{"sha256":digest(b)}})).collect::<Vec<_>>()})).unwrap()
    }
    struct Reply {
        status: u16,
        headers: String,
        body: Vec<u8>,
        declared: usize,
    }
    fn reply(status: u16, body: Vec<u8>, headers: &str) -> Reply {
        let declared = body.len();
        Reply {
            status,
            body,
            headers: headers.into(),
            declared,
        }
    }
    fn head(size: usize, etag: &str) -> Reply {
        Reply {
            status: 200,
            body: vec![],
            headers: format!("ETag: \"{etag}\"\r\n"),
            declared: size,
        }
    }
    fn fixture(
        responses: Vec<Reply>,
    ) -> (Transport, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(vec![]));
        let captured = requests.clone();
        let join = thread::spawn(move || {
            for response in responses {
                listener.set_nonblocking(true).unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("fixture did not receive expected request: {e}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut one = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut one).unwrap();
                    request.push(one[0]);
                }
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                write!(
                    socket,
                    "HTTP/1.1 {} Response\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    response.status, response.headers, response.declared
                )
                .unwrap();
                socket.write_all(&response.body).unwrap();
            }
        });
        (
            Transport::new(
                Url::parse(&format!("http://{address}")).unwrap(),
                Some("fixture-hf-token"),
            )
            .unwrap(),
            requests,
            join,
        )
    }
    fn config(name: &str) -> LocalConfig {
        LocalConfig {
            hf_url: format!("https://huggingface.co/owner/repo/resolve/main/{name}"),
            ..Default::default()
        }
    }
    fn interrupted(meta: Vec<u8>, data: &[u8], etag: &str) -> Vec<Reply> {
        vec![
            reply(200, meta, ""),
            head(data.len(), etag),
            Reply {
                status: 200,
                headers: format!("ETag: \"{etag}\"\r\n"),
                body: data[..32].to_vec(),
                declared: data.len(),
            },
        ]
    }
    #[test]
    fn interrupted_download_resumes_from_disk_with_range_and_if_range_then_checks_every_byte() {
        // Invoke this same test in a separate process for the interrupted half.
        // The parent then starts a fresh downloader from the child's disk state.
        if let Ok(root) = std::env::var("CODEBOT_GGUF_RESTART_TEST_ROOT") {
            let transport = Transport::new(
                Url::parse(&std::env::var("CODEBOT_GGUF_RESTART_TEST_ORIGIN").unwrap()).unwrap(),
                Some("fixture-hf-token"),
            )
            .unwrap();
            assert!(download_with(
                &config("model.gguf"),
                Path::new(&root),
                &AtomicBool::new(false),
                |_| {},
                transport
            )
            .is_err());
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = bytes(8);
        let meta = metadata(&[("model.gguf", data.clone())], 'a');
        let (transport, _, worker) = fixture(interrupted(meta.clone(), &data, "one"));
        let child=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","gguf_download::tests::interrupted_download_resumes_from_disk_with_range_and_if_range_then_checks_every_byte","--test-threads=1"])
            .env("CODEBOT_GGUF_RESTART_TEST_ROOT",dir.path()).env("CODEBOT_GGUF_RESTART_TEST_ORIGIN",transport.base.as_str()).output().unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        worker.join().unwrap();
        let partial = fs::read_dir(dir.path())
            .unwrap()
            .map(Result::unwrap)
            .find(|e| e.file_name().to_string_lossy().ends_with(".partial"))
            .unwrap()
            .path();
        assert_eq!(fs::metadata(partial.join("model.gguf")).unwrap().len(), 32);
        let (transport, requests, worker) = fixture(vec![
            reply(200, meta, ""),
            head(data.len(), "one"),
            reply(
                206,
                data[32..].to_vec(),
                "ETag: \"one\"\r\nContent-Range: bytes 32-127/128\r\n",
            ),
        ]);
        let mut progress = 0;
        let path = download_with(
            &config("model.gguf"),
            dir.path(),
            &AtomicBool::new(false),
            |p| progress = p.received,
            transport,
        )
        .unwrap();
        worker.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), data);
        assert_eq!(progress, 128);
        assert!(!partial.exists());
        let requests = requests.lock().unwrap();
        assert!(requests[2].to_lowercase().contains("range: bytes=32-\r\n"));
        assert!(requests[2].to_lowercase().contains("if-range: \"one\""));
        assert!(requests[1].contains(&format!("/resolve/{}/model.gguf", "a".repeat(40))));
        validate_managed(&path, &AtomicBool::new(false)).unwrap();
        fs::write(&path, bytes(9)).unwrap();
        assert!(validate_managed(&path, &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn changed_validators_restart_and_servers_ignoring_range_never_append_a_full_response() {
        for changed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let data = bytes(3);
            let meta = metadata(&[("model.gguf", data.clone())], 'b');
            let (transport, _, worker) = fixture(interrupted(meta.clone(), &data, "old"));
            assert!(download_with(
                &config("model.gguf"),
                dir.path(),
                &AtomicBool::new(false),
                |_| {},
                transport
            )
            .is_err());
            worker.join().unwrap();
            let tag = if changed { "new" } else { "old" };
            let (transport, requests, worker) = fixture(vec![
                reply(200, meta, ""),
                head(128, tag),
                reply(200, data.clone(), &format!("ETag: \"{tag}\"\r\n")),
            ]);
            let path = download_with(
                &config("model.gguf"),
                dir.path(),
                &AtomicBool::new(false),
                |_| {},
                transport,
            )
            .unwrap();
            worker.join().unwrap();
            assert_eq!(fs::read(path).unwrap(), data);
            assert_eq!(
                requests.lock().unwrap()[2]
                    .to_lowercase()
                    .contains("range: bytes="),
                !changed
            );
        }
    }
    #[test]
    fn split_gguf_is_pinned_as_a_set_and_incomplete_or_corrupt_parts_cannot_be_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let names = ["model-00001-of-00002.gguf", "model-00002-of-00002.gguf"];
        let data = [bytes(1), bytes(2)];
        let meta = metadata(
            &[(names[0], data[0].clone()), (names[1], data[1].clone())],
            'c',
        );
        let (transport, requests, worker) = fixture(vec![
            reply(200, meta, ""),
            head(128, "a"),
            reply(200, data[0].clone(), "ETag: \"a\"\r\n"),
            head(128, "b"),
            reply(200, data[1].clone(), "ETag: \"b\"\r\n"),
        ]);
        let path = download_with(
            &config(names[0]),
            dir.path(),
            &AtomicBool::new(false),
            |_| {},
            transport,
        )
        .unwrap();
        worker.join().unwrap();
        assert!(requests.lock().unwrap()[1..]
            .iter()
            .all(|r| r.contains(&format!("/resolve/{}/", "c".repeat(40)))));
        validate_managed(&path, &AtomicBool::new(false)).unwrap();
        crate::validate_gguf(&path).unwrap();
        fs::remove_file(path.with_file_name(names[1])).unwrap();
        assert!(crate::validate_gguf(&path).is_err());
        assert!(validate_managed(&path, &AtomicBool::new(false)).is_err());
        let (transport, _, worker) = fixture(vec![reply(
            200,
            metadata(&[(names[0], data[0].clone())], 'c'),
            "",
        )]);
        assert!(download_with(
            &config(names[0]),
            dir.path(),
            &AtomicBool::new(false),
            |_| {},
            transport
        )
        .is_err());
        worker.join().unwrap();
        assert!(split_names(names[1]).is_err());
    }
    #[test]
    fn wrong_ranges_hashes_and_changed_revisions_do_not_install_or_reuse_old_partial_content() {
        let dir = tempfile::tempdir().unwrap();
        let data = bytes(4);
        let meta = metadata(&[("model.gguf", data.clone())], 'd');
        let (transport, _, worker) = fixture(interrupted(meta.clone(), &data, "one"));
        assert!(download_with(
            &config("model.gguf"),
            dir.path(),
            &AtomicBool::new(false),
            |_| {},
            transport
        )
        .is_err());
        worker.join().unwrap();
        let (transport, _, worker) = fixture(vec![
            reply(200, meta, ""),
            head(128, "one"),
            reply(
                206,
                data[32..].to_vec(),
                "ETag: \"one\"\r\nContent-Range: bytes 0-95/128\r\n",
            ),
        ]);
        assert!(download_with(
            &config("model.gguf"),
            dir.path(),
            &AtomicBool::new(false),
            |_| {},
            transport
        )
        .is_err());
        worker.join().unwrap();
        let (transport, requests, worker) = fixture(vec![
            reply(200, metadata(&[("model.gguf", data.clone())], 'e'), ""),
            head(128, "new"),
            reply(200, bytes(5), "ETag: \"new\"\r\n"),
        ]);
        assert!(download_with(
            &config("model.gguf"),
            dir.path(),
            &AtomicBool::new(false),
            |_| {},
            transport
        )
        .is_err());
        worker.join().unwrap();
        assert!(!requests.lock().unwrap()[2]
            .to_lowercase()
            .contains("range: bytes="));
        assert_eq!(
            fs::read_dir(dir.path())
                .unwrap()
                .map(Result::unwrap)
                .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
                .count(),
            2
        );
        assert!(!crate::model_cache_inventory(dir.path())
            .unwrap()
            .entries
            .iter()
            .any(|e| e.ready));
    }
    #[test]
    fn cancellation_aborts_pending_network_and_releases_the_cache_lease() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let worker = thread::spawn(move || {
            let (_socket, _) = listener.accept().unwrap();
            signal.store(true, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(200));
        });
        let start = std::time::Instant::now();
        let transport =
            Transport::new(Url::parse(&format!("http://{address}")).unwrap(), None).unwrap();
        assert!(download_with(
            &config("model.gguf"),
            dir.path(),
            &cancel,
            |_| {},
            transport
        )
        .unwrap_err()
        .contains("キャンセル"));
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(crate::model_cache::lease(dir.path(), true).is_ok());
        worker.join().unwrap();
    }
}
