use crate::{execute, ApiConfig, ApiProvider, Backend, Config};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocalEngine {
    #[default]
    LlamaCpp,
    Transformers,
}
impl LocalEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::LlamaCpp => "llama.cpp（GGUF）",
            Self::Transformers => "Transformers（公式形式）",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelDevice {
    #[default]
    Auto,
    Cpu,
    Cuda,
    Mps,
}
impl ModelDevice {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
            Self::Mps => "mps",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocalPreset {
    #[default]
    Custom,
    Gemma4,
    DeepseekV4Flash0731,
    Glm53,
    Glm53Flash,
    Qwen37Plus,
}
impl LocalPreset {
    pub const ALL: [Self; 6] = [
        Self::Custom,
        Self::Gemma4,
        Self::DeepseekV4Flash0731,
        Self::Glm53,
        Self::Glm53Flash,
        Self::Qwen37Plus,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Custom => "カスタムモデル",
            Self::Gemma4 => "Gemma 4",
            Self::DeepseekV4Flash0731 => "DeepSeek v4 Flash 0731",
            Self::Glm53 => "GLM 5.3",
            Self::Glm53Flash => "GLM 5.3 Flash",
            Self::Qwen37Plus => "Qwen 3.7 Plus",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalConfig {
    pub preset: LocalPreset,
    pub engine: LocalEngine,
    pub python_path: String,
    pub hf_revision: String,
    pub device: ModelDevice,
    pub model_path: String,
    pub hf_url: String,
    pub sha256: String,
    pub server_path: String,
    pub context_size: u32,
    pub threads: u16,
    pub gpu_layers: u16,
    pub max_output_tokens: u32,
    pub startup_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub max_download_gib: u32,
}
impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            preset: LocalPreset::Custom,
            engine: LocalEngine::default(),
            python_path: if cfg!(windows) { "python" } else { "python3" }.into(),
            hf_revision: "main".into(),
            device: ModelDevice::Auto,
            model_path: String::new(),
            hf_url: String::new(),
            sha256: String::new(),
            server_path: if cfg!(windows) {
                "llama-server.exe"
            } else {
                "llama-server"
            }
            .into(),
            context_size: 4096,
            threads: 4,
            gpu_layers: 0,
            max_output_tokens: 1024,
            startup_timeout_seconds: 180,
            request_timeout_seconds: 120,
            max_download_gib: 64,
        }
    }
}
impl LocalConfig {
    pub fn validate_source(&self) -> Result<(), String> {
        if !self.hf_url.trim().is_empty() {
            match self.engine {
                LocalEngine::LlamaCpp => {
                    normalize_hf_url(&self.hf_url)?;
                }
                LocalEngine::Transformers => {
                    crate::transformers::parse_repository(&self.hf_url, &self.hf_revision)?;
                }
            }
        }
        if !self.sha256.trim().is_empty() && !is_hash(self.sha256.trim()) {
            return Err("SHA-256は64桁の16進数で指定してください".into());
        }
        Ok(())
    }
    pub fn default_cache() -> Option<PathBuf> {
        directories::ProjectDirs::from("org", "OSS-CodeBot", "OSS-CodeBot")
            .map(|d| d.cache_dir().join("models"))
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DownloadProgress {
    pub received: u64,
    pub total: Option<u64>,
}
fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn hash_header(h: &reqwest::header::HeaderMap) -> Option<String> {
    ["x-linked-etag", "etag"]
        .into_iter()
        .filter_map(|k| h.get(k)?.to_str().ok())
        .map(|v| v.trim_matches('"'))
        .find(|s| is_hash(s))
        .map(str::to_ascii_lowercase)
}
fn normalize_hf_url(raw: &str) -> Result<Url, String> {
    let mut u =
        Url::parse(raw.trim()).map_err(|_| "Hugging FaceのGGUFファイルURLを指定してください")?;
    if u.scheme() != "https"
        || u.host_str() != Some("huggingface.co")
        || u.port().is_some_and(|p| p != 443)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err("https://huggingface.co/ から始まるファイルURLを指定してください".into());
    }
    if u.query_pairs()
        .any(|(k, v)| k != "download" || !matches!(v.as_ref(), "true" | "false" | "1"))
    {
        return Err("URLにトークンなどを含めず、HFトークン欄を使用してください".into());
    }
    u.set_query(None);
    let mut parts: Vec<String> = u
        .path_segments()
        .ok_or("ファイルURLが不正です")?
        .map(str::to_owned)
        .collect();
    if parts.len() < 5
        || !matches!(parts[2].as_str(), "resolve" | "blob")
        || !parts
            .last()
            .unwrap()
            .to_ascii_lowercase()
            .ends_with(".gguf")
    {
        return Err(
            "リポジトリのページではなく、GGUFファイルのblob/resolve URLを指定してください".into(),
        );
    }
    if parts
        .last()
        .unwrap()
        .trim_end_matches(".gguf")
        .rsplit_once("-of-")
        .is_some_and(|(_, n)| n.len() == 5 && n.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("分割GGUFの自動取得には未対応です。全パートを同じフォルダに配置し、先頭ファイルのローカルパスを指定してください".into());
    }
    parts[2] = "resolve".into();
    u.set_path(&parts.join("/"));
    Ok(u)
}
pub fn validate_gguf(path: &Path) -> Result<(), String> {
    let mut f = File::open(path)
        .map_err(|_| "GGUFファイルを読み込めません。ローカルパスを確認してください")?;
    let mut header = [0u8; 8];
    f.read_exact(&mut header)
        .map_err(|_| "GGUFファイルが短すぎるか破損しています")?;
    if &header[..4] != b"GGUF"
        || !matches!(u32::from_le_bytes(header[4..].try_into().unwrap()), 2 | 3)
    {
        return Err(
            "GGUF v2/v3ファイルを指定してください（safetensorsは直接読み込めません）".into(),
        );
    }
    if f.metadata()
        .map_err(|_| "モデルのサイズを確認できません")?
        .len()
        < 24
    {
        return Err("GGUFファイルが破損しています".into());
    }
    Ok(())
}
struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn file_hash(path: &Path, cancel: &AtomicBool) -> Result<String, String> {
    let mut f = File::open(path).map_err(|_| "モデルを検証できません")?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("キャンセルしました".into());
        }
        let n = f.read(&mut buf).map_err(|_| "モデルを検証できません")?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
/// Downloads only a user-selected HF GGUF. Redirects may go to HF-owned CDNs;
/// authentication is sent only to huggingface.co. Files are installed after hashing.
pub fn download_hf_model(
    config: &LocalConfig,
    cache: &Path,
    token: Option<&str>,
    cancel: &AtomicBool,
    progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf, String> {
    if config.engine == LocalEngine::Transformers {
        return crate::transformers::download_model(config, cache, token, cancel, progress);
    }
    config.validate_source()?;
    let source = normalize_hf_url(&config.hf_url)?;
    if !(1..=4096).contains(&config.max_download_gib) {
        return Err("ダウンロード上限は1〜4096 GiBで指定してください".into());
    }
    let limit = u64::from(config.max_download_gib) * 1024 * 1024 * 1024;
    let client = Client::builder()
        .timeout(Duration::from_secs(24 * 60 * 60))
        .connect_timeout(Duration::from_secs(20))
        .redirect(Policy::none())
        .build()
        .map_err(|_| "ダウンロード接続を準備できません")?;
    let request = |method: reqwest::Method, url: Url| {
        let r = client.request(method, url.clone());
        if url.host_str() == Some("huggingface.co") {
            if let Some(t) = token.filter(|s| !s.trim().is_empty()) {
                return r.bearer_auth(t);
            }
        }
        r
    };
    let head = request(reqwest::Method::HEAD, source.clone())
        .send()
        .map_err(|_| "Hugging Faceに接続できません。通信設定を確認してください")?;
    if head.status().is_client_error() || head.status().is_server_error() {
        return Err(format!("モデル情報を取得できません（HTTP {}）。URL・公開範囲・利用同意・HFトークンを確認してください",head.status().as_u16()));
    }
    let from_server = hash_header(head.headers());
    let expected = if config.sha256.trim().is_empty() {
        from_server
            .clone()
            .ok_or("配布元からSHA-256を取得できません。配布元のSHA-256を入力してください")?
    } else {
        config.sha256.trim().to_ascii_lowercase()
    };
    if from_server.is_some_and(|h| h != expected) {
        return Err("指定SHA-256と配布元のSHA-256が一致しません".into());
    }
    fs::create_dir_all(cache).map_err(|_| "モデル保存フォルダを作成できません")?;
    let destination = cache.join(format!("{expected}.gguf"));
    if destination.exists() {
        validate_gguf(&destination)?;
        if file_hash(&destination, cancel)? != expected {
            return Err(
                "保存済みモデルのSHA-256が一致しません。破損ファイルを退避して再取得してください"
                    .into(),
            );
        }
        return Ok(destination);
    }
    let lock = cache.join(format!("{expected}.lock"));
    let _lockfile = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|_| {
            "同じモデルを取得中です。以前の取得が中断した場合はキャッシュのlockを確認してください"
        })?;
    let _lock_guard = RemoveOnDrop(lock);
    let mut url = source;
    let mut response = None;
    for _ in 0..8 {
        if cancel.load(Ordering::Relaxed) {
            return Err("キャンセルしました".into());
        }
        let r = request(reqwest::Method::GET, url.clone())
            .send()
            .map_err(|_| "モデルの取得に失敗しました。通信設定を確認してください")?;
        if r.status().is_redirection() {
            let next = r
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("転送先が不正です")?;
            url = url.join(next).map_err(|_| "転送先が不正です")?;
            let host = url.host_str().unwrap_or_default();
            if url.scheme() != "https"
                || !(host == "huggingface.co"
                    || host.ends_with(".huggingface.co")
                    || host.ends_with(".hf.co"))
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("Hugging Face管理外の転送先にはダウンロードしません".into());
            }
            continue;
        }
        if !r.status().is_success() {
            return Err(format!(
                "モデルを取得できません（HTTP {}）",
                r.status().as_u16()
            ));
        }
        response = Some(r);
        break;
    }
    let response = response.ok_or("転送回数が上限を超えました")?;
    install_download(response, &destination, &expected, limit, cancel, progress)
}
fn install_download(
    mut response: reqwest::blocking::Response,
    destination: &Path,
    expected: &str,
    limit: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf, String> {
    let cache = destination.parent().ok_or("モデル保存先が不正です")?;

    let total = response.content_length();
    if total.is_some_and(|n| n > limit) {
        return Err("モデルが設定したダウンロード上限を超えています".into());
    }
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = cache.join(format!("{expected}-{unique}.part"));
    let guard = RemoveOnDrop(temp.clone());
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "モデル保存先を作成できません")?;
    let mut h = Sha256::new();
    let mut received = 0;
    let mut buf = [0u8; 65536];
    progress(DownloadProgress { received, total });
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("キャンセルしました".into());
        }
        let n = response
            .read(&mut buf)
            .map_err(|_| "ダウンロードが中断されました。再取得してください")?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > limit {
            return Err("ダウンロード上限を超えました".into());
        }
        output
            .write_all(&buf[..n])
            .map_err(|_| "モデルを保存できません。空き容量を確認してください")?;
        h.update(&buf[..n]);
        progress(DownloadProgress { received, total });
    }
    if total.is_some_and(|n| received != n) {
        return Err("ダウンロードサイズが一致しません".into());
    }
    if format!("{:x}", h.finalize()) != expected {
        return Err("SHA-256が一致しないためモデルを使用しません".into());
    }
    output.sync_all().map_err(|_| "モデルを保存できません")?;
    drop(output);
    validate_gguf(&temp)?;
    fs::rename(&temp, &destination).map_err(|_| "取得済みモデルを配置できません")?;
    drop(guard);
    Ok(destination.to_path_buf())
}

pub struct LocalServer {
    child: Child,
    connection: LocalConnection,
}
#[derive(Clone)]
pub struct LocalConnection {
    api: ApiConfig,
    key: String,
}
impl std::fmt::Debug for LocalConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LocalConnection { credentials: [REDACTED] }")
    }
}
impl LocalConnection {
    pub fn respond(&self, config: &Config, input: &str) -> Result<String, String> {
        execute(
            config,
            &ApiProvider::new(Backend::Custom, &self.api, Some(&self.key))?,
            input,
        )
    }
}
impl LocalServer {
    pub fn start(config: &LocalConfig, cancel: &AtomicBool) -> Result<Self, String> {
        let path = Path::new(config.model_path.trim());
        match config.engine {
            LocalEngine::LlamaCpp => validate_gguf(path)?,
            LocalEngine::Transformers => crate::transformers::validate_folder(path)?,
        }
        if !(512..=131072).contains(&config.context_size)
            || !(1..=256).contains(&config.threads)
            || !(1..=1800).contains(&config.startup_timeout_seconds)
            || !(1..=128000).contains(&config.max_output_tokens)
            || !(1..=300).contains(&config.request_timeout_seconds)
        {
            return Err("ローカル推論の設定が範囲外です".into());
        }
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|_| "ローカルポートを確保できません")?;
        let port = listener
            .local_addr()
            .map_err(|_| "ポートを確認できません")?
            .port();
        drop(listener);
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce).map_err(|_| "ローカル認証を初期化できません")?;
        let key = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut command = if config.engine == LocalEngine::LlamaCpp {
            Command::new(config.server_path.trim())
        } else {
            let mut command = Command::new(config.python_path.trim());
            command
                .arg(crate::transformers::bridge_path()?)
                .arg("serve");
            command
                .env("HF_HUB_OFFLINE", "1")
                .env("TRANSFORMERS_OFFLINE", "1");
            command
        };
        command
            .arg("--model")
            .arg(path)
            .args(["--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .args(["--alias", "codebot-local", "--api-key"])
            .arg(&key)
            .arg("--ctx-size")
            .arg(config.context_size.to_string())
            .arg("--threads")
            .arg(config.threads.to_string());
        if config.engine == LocalEngine::LlamaCpp {
            command
                .arg("--n-gpu-layers")
                .arg(config.gpu_layers.to_string())
                .args(["--parallel", "1"]);
        } else {
            command.arg("--device").arg(config.device.as_str());
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let child = command.spawn().map_err(|_| {
            "推論エンジンを起動できません。llama-serverまたはPythonの実行ファイルパスを確認してください"
        })?;
        let base = format!("http://127.0.0.1:{port}");
        let connection = LocalConnection {
            key,
            api: ApiConfig {
                model: "codebot-local".into(),
                base_url: format!("{base}/v1"),
                timeout_seconds: config.request_timeout_seconds,
                max_output_tokens: config.max_output_tokens,
                ..Default::default()
            },
        };
        let mut server = Self { child, connection };
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .redirect(Policy::none())
            .build()
            .map_err(|_| "起動確認を準備できません")?;
        let deadline = Instant::now() + Duration::from_secs(config.startup_timeout_seconds);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("読み込みをキャンセルしました".into());
            }
            if server
                .child
                .try_wait()
                .map_err(|_| "推論プロセスを確認できません")?
                .is_some()
            {
                return Err("モデルを読み込めません。形式・Python依存・エンジン対応・メモリ容量を確認してください".into());
            }
            if let Ok(response) = client.get(format!("{base}/health")).send() {
                if response.status().is_success() {
                    return Ok(server);
                }
                if config.engine == LocalEngine::Transformers {
                    if let Ok(value) = response.json::<serde_json::Value>() {
                        if value["status"] == "error" {
                            return Err(crate::transformers::error_message(
                                value["code"].as_str().unwrap_or("load"),
                            ));
                        }
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err("モデル読み込みがタイムアウトしました".into());
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
    pub fn connection(&self) -> LocalConnection {
        self.connection.clone()
    }
    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|s| s.is_none())
    }
}
impl Drop for LocalServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let d = std::env::temp_dir().join(format!(
                "codebot-local-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&d).unwrap();
            Self(d)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0u8; 64];
        bytes[..4].copy_from_slice(b"GGUF");
        bytes[4] = 3;
        bytes
    }
    #[test]
    fn hf_urls_reject_repositories_credentials_and_non_hf_hosts() {
        assert_eq!(
            normalize_hf_url("https://huggingface.co/user/repo/blob/main/model.gguf?download=true")
                .unwrap()
                .as_str(),
            "https://huggingface.co/user/repo/resolve/main/model.gguf"
        );
        for url in [
            "https://huggingface.co/user/repo",
            "https://evil.example/user/repo/resolve/main/model.gguf",
            "https://huggingface.co.evil.example/user/repo/resolve/main/model.gguf",
            "https://huggingface.co/user/repo/resolve/main/model.gguf?token=secret",
            "https://u:secret@huggingface.co/user/repo/resolve/main/model.gguf",
            "https://huggingface.co/user/repo/resolve/main/model.safetensors",
            "https://huggingface.co/user/repo/resolve/main/model-00001-of-00003.gguf",
        ] {
            assert!(normalize_hf_url(url).is_err(), "{url}");
        }
    }
    #[test]
    fn downloaded_checksum_headers_are_sha256_not_git_sha1() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            "etag",
            reqwest::header::HeaderValue::from_static(
                "\"0123456789012345678901234567890123456789\"",
            ),
        );
        assert!(hash_header(&h).is_none());
        h.insert(
            "x-linked-etag",
            reqwest::header::HeaderValue::from_static(
                "\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"",
            ),
        );
        assert!(hash_header(&h).is_some());
    }
    fn response(bytes: Vec<u8>) -> reqwest::blocking::Response {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&bytes);
        });
        Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .unwrap()
    }
    #[test]
    fn downloads_install_only_after_sha256_verification_and_clean_partial_files() {
        let dir = TestDir::new();
        let file = dir.0.join("model.gguf");
        let bytes = fixture();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let cancel = AtomicBool::new(false);
        let mut received = 0;
        install_download(response(bytes.clone()), &file, &hash, 1024, &cancel, |p| {
            received = p.received
        })
        .unwrap();
        assert_eq!(fs::read(&file).unwrap(), bytes);
        assert_eq!(received, 64);
        assert!(validate_gguf(&file).is_ok());
        let invalid = dir.0.join("invalid.gguf");
        assert!(install_download(
            response(bytes.clone()),
            &invalid,
            &"a".repeat(64),
            1024,
            &cancel,
            |_| {}
        )
        .is_err());
        assert!(!invalid.exists());
        let cancelled = AtomicBool::new(true);
        assert!(install_download(
            response(bytes.clone()),
            &invalid,
            &hash,
            1024,
            &cancelled,
            |_| {}
        )
        .is_err());
        assert!(install_download(response(bytes), &invalid, &hash, 8, &cancel, |_| {}).is_err());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    #[test]
    fn bad_local_paths_and_wrong_formats_are_rejected_before_spawning() {
        let dir = TestDir::new();
        let file = dir.0.join("model.gguf");
        fs::write(&file, b"not a model").unwrap();
        assert!(validate_gguf(&file).is_err());
        let c = LocalConfig {
            model_path: file.to_string_lossy().into_owned(),
            server_path: "definitely-not-a-runtime".into(),
            ..Default::default()
        };
        assert!(LocalServer::start(&c, &AtomicBool::new(false))
            .err()
            .unwrap()
            .contains("GGUF"));
    }
    #[cfg(unix)]
    #[test]
    fn owned_local_server_authentication_inference_and_shutdown() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let model = dir.0.join("model with spaces;not-a-command.gguf");
        fs::write(&model, fixture()).unwrap();
        let runner = dir.0.join("fake-llama-server");
        fs::write(&runner,r#"#!/usr/bin/env python3
import sys,json
from http.server import HTTPServer,BaseHTTPRequestHandler
a=sys.argv
port=int(a[a.index('--port')+1]);key=a[a.index('--api-key')+1]
assert a[a.index('--host')+1]=='127.0.0.1'
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  self.send_response(200);self.send_header('Content-Length','2');self.end_headers();self.wfile.write(b'{}')
 def do_POST(self):
  assert self.headers['Authorization']=='Bearer '+key
  body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
  assert body['model']=='codebot-local'
  data=json.dumps({'choices':[{'message':{'content':'mock local inference'}}]}).encode()
  self.send_response(200);self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 def log_message(self,*args):pass
HTTPServer(('127.0.0.1',port),Handler).serve_forever()
"#).unwrap();
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).unwrap();
        let config = LocalConfig {
            model_path: model.to_string_lossy().into_owned(),
            server_path: runner.to_string_lossy().into_owned(),
            startup_timeout_seconds: 10,
            ..Default::default()
        };
        let server = LocalServer::start(&config, &AtomicBool::new(false)).unwrap();
        let connection = server.connection();
        assert!(!format!("{connection:?}").contains(&connection.key));
        assert_eq!(
            connection.respond(&Config::default(), "hello").unwrap(),
            "mock local inference"
        );
        let base = connection.api.base_url.clone();
        drop(server);
        assert!(Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap()
            .get(format!("{base}/health"))
            .send()
            .is_err());
    }
}
