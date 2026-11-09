use crate::{ApiConfig, ApiProvider, Backend, Config};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
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
pub(crate) fn normalize_hf_url(raw: &str) -> Result<Url, String> {
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
    parts[2] = "resolve".into();
    u.set_path(&parts.join("/"));
    Ok(u)
}
pub fn validate_gguf(path: &Path) -> Result<(), String> {
    if let Some(names) = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(crate::gguf_download::split_names)
        .transpose()?
        .flatten()
    {
        for name in names {
            validate_gguf_header(&path.with_file_name(name))?;
        }
    }
    validate_gguf_header(path)
}
pub(crate) fn validate_gguf_header(path: &Path) -> Result<(), String> {
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
pub fn download_hf_model(
    config: &LocalConfig,
    cache: &Path,
    token: Option<&str>,
    cancel: &AtomicBool,
    progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf, String> {
    if config.engine == LocalEngine::Transformers {
        let _lease = crate::model_cache::lease(cache, true)?;
        return crate::transformers::download_model(config, cache, token, cancel, progress);
    }
    crate::gguf_download::download(config, cache, token, cancel, progress)
}

pub struct LocalServer {
    child: Child,
    connection: LocalConnection,
    _cache_lease: Option<crate::model_cache::CacheLease>,
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
    pub(crate) fn api_provider(&self) -> Result<ApiProvider, String> {
        ApiProvider::new(Backend::Custom, &self.api, Some(&self.key))
    }
    pub fn stream_messages(
        &self,
        config: &Config,
        messages: &[crate::ChatMessage],
        cancel: &AtomicBool,
        emit: impl FnMut(crate::StreamEvent),
    ) {
        crate::stream_messages(
            config,
            Backend::Custom,
            &self.api,
            Some(&self.key),
            messages,
            cancel,
            emit,
        );
    }
    pub fn respond(&self, config: &Config, input: &str) -> Result<String, String> {
        self.respond_messages(config, &[crate::ChatMessage::user(input.trim())])
    }
    pub fn respond_messages(
        &self,
        config: &Config,
        messages: &[crate::ChatMessage],
    ) -> Result<String, String> {
        crate::validate_messages(config, messages)?;
        ApiProvider::new(Backend::Custom, &self.api, Some(&self.key))?.respond_messages(messages)
    }
}
impl LocalServer {
    pub fn start(config: &LocalConfig, cancel: &AtomicBool) -> Result<Self, String> {
        let path = Path::new(config.model_path.trim());
        let cache_lease = crate::model_cache::model_lease(path)?;
        if config.engine == LocalEngine::LlamaCpp {
            crate::gguf_download::validate_managed(path, cancel)?;
        }
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
        let mut server = Self {
            child,
            connection,
            _cache_lease: cache_lease,
        };
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
    use std::fs;
    use std::sync::atomic::AtomicU64;
    use std::time::{SystemTime, UNIX_EPOCH};
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
        ] {
            assert!(normalize_hf_url(url).is_err(), "{url}");
        }
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
        drop(crate::model_cache::lease(&dir.0, true).unwrap());
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
  if len(body['messages'])>1:
   assert [m['role'] for m in body['messages']]==['user','assistant','user']
   assert body['messages'][0]['content']=='previous question'
  if body.get('stream'):
   self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
   self.wfile.write(b'data: {"choices":[{"delta":{"content":"mock local stream"}}]}\n\ndata: [DONE]\n\n');self.wfile.flush()
   return
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
        assert!(crate::model_cache::lease(&dir.0, true).is_err());
        let connection = server.connection();
        assert!(!format!("{connection:?}").contains(&connection.key));
        assert_eq!(
            connection.respond(&Config::default(), "hello").unwrap(),
            "mock local inference"
        );
        assert_eq!(
            connection
                .respond_messages(
                    &Config::default(),
                    &[
                        crate::ChatMessage::user("previous question"),
                        crate::ChatMessage::assistant("previous answer"),
                        crate::ChatMessage::user("follow up"),
                    ]
                )
                .unwrap(),
            "mock local inference"
        );
        let mut events = Vec::new();
        connection.stream_messages(
            &Config::default(),
            &[crate::ChatMessage::user("stream")],
            &AtomicBool::new(false),
            |event| events.push(event),
        );
        assert_eq!(
            events,
            vec![
                crate::StreamEvent::Delta("mock local stream".into()),
                crate::StreamEvent::Finished(crate::StreamEnd::Complete)
            ]
        );
        let base = connection.api.base_url.clone();
        drop(server);
        assert!(crate::model_cache::lease(&dir.0, true).is_ok());
        assert!(Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap()
            .get(format!("{base}/health"))
            .send()
            .is_err());
    }
}
