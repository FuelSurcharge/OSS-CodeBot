use crate::{execute, Config, Provider, Rules};
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue},
    redirect::Policy,
    Url,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    #[default]
    Rules,
    Local,
    Openai,
    Gemini,
    Claude,
    Deepseek,
    Qwen,
    Glm,
    Grok,
    Mimo,
    Kimi,
    Custom,
}
impl Backend {
    pub const ALL: [Self; 12] = [
        Self::Rules,
        Self::Local,
        Self::Openai,
        Self::Gemini,
        Self::Claude,
        Self::Deepseek,
        Self::Qwen,
        Self::Glm,
        Self::Grok,
        Self::Mimo,
        Self::Kimi,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Rules => "ローカル診断",
            Self::Local => "ローカルAI",
            Self::Openai => "OpenAI",
            Self::Gemini => "Gemini",
            Self::Claude => "Claude",
            Self::Deepseek => "DeepSeek",
            Self::Qwen => "Qwen",
            Self::Glm => "GLM",
            Self::Grok => "Grok",
            Self::Mimo => "MiMo",
            Self::Kimi => "Kimi",
            Self::Custom => "カスタム",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Local => "local",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Deepseek => "deepseek",
            Self::Qwen => "qwen",
            Self::Glm => "glm",
            Self::Grok => "grok",
            Self::Mimo => "mimo",
            Self::Kimi => "kimi",
            Self::Custom => "custom",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == id)
    }
    pub fn defaults(self) -> ApiConfig {
        let (base_url, key_env, protocol) = match self {
            Self::Openai => (
                "https://api.openai.com/v1",
                "OPENAI_API_KEY",
                ApiProtocol::Openai,
            ),
            Self::Gemini => (
                "https://generativelanguage.googleapis.com/v1beta",
                "GEMINI_API_KEY",
                ApiProtocol::Gemini,
            ),
            Self::Claude => (
                "https://api.anthropic.com/v1",
                "ANTHROPIC_API_KEY",
                ApiProtocol::Claude,
            ),
            Self::Deepseek => (
                "https://api.deepseek.com/v1",
                "DEEPSEEK_API_KEY",
                ApiProtocol::Openai,
            ),
            Self::Qwen => (
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                "DASHSCOPE_API_KEY",
                ApiProtocol::Openai,
            ),
            Self::Glm => (
                "https://api.z.ai/api/paas/v4",
                "ZHIPU_API_KEY",
                ApiProtocol::Openai,
            ),
            Self::Grok => ("https://api.x.ai/v1", "XAI_API_KEY", ApiProtocol::Openai),
            Self::Mimo => (
                "https://api.xiaomimimo.com/v1",
                "MIMO_API_KEY",
                ApiProtocol::Openai,
            ),
            Self::Kimi => (
                "https://api.moonshot.ai/v1",
                "MOONSHOT_API_KEY",
                ApiProtocol::Openai,
            ),
            _ => ("", "", ApiProtocol::Openai),
        };
        ApiConfig {
            base_url: base_url.into(),
            api_key_env: key_env.into(),
            protocol,
            ..Default::default()
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiProtocol {
    #[default]
    Openai,
    Gemini,
    Claude,
}
impl ApiProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Openai => "OpenAI互換",
            Self::Gemini => "Gemini",
            Self::Claude => "Claude Messages",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiConfig {
    pub model: String,
    pub base_url: String,
    pub api_key_env: String,
    pub protocol: ApiProtocol,
    pub max_output_tokens: u32,
    pub timeout_seconds: u64,
    pub anonymous: bool,
}
impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            base_url: String::new(),
            api_key_env: String::new(),
            protocol: ApiProtocol::Openai,
            max_output_tokens: 2048,
            timeout_seconds: 60,
            anonymous: false,
        }
    }
}
impl ApiConfig {
    pub fn resolved(&self, backend: Backend) -> Self {
        let defaults = backend.defaults();
        let mut c = self.clone();
        if c.base_url.trim().is_empty() {
            c.base_url = defaults.base_url;
        }
        if c.api_key_env.trim().is_empty() {
            c.api_key_env = defaults.api_key_env;
        }
        if backend != Backend::Custom {
            c.protocol = defaults.protocol;
            c.anonymous = false;
        }
        c
    }
    pub fn validate_url(&self) -> Result<Url, String> {
        let u =
            Url::parse(self.base_url.trim()).map_err(|_| "APIのベースURLが不正です".to_string())?;
        let local = matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if u.scheme() != "https" && !(u.scheme() == "http" && local) {
            return Err(
                "APIにはHTTPS URLを指定してください（HTTPはループバックのみ使用可能）".into(),
            );
        }
        if !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
        {
            return Err("API URLに認証情報・クエリ・フラグメントを含めないでください".into());
        }
        Ok(u)
    }
    fn validate(&self) -> Result<(), String> {
        self.validate_url()?;
        if self.model.trim().is_empty() {
            return Err("モデルIDを入力してください".into());
        }
        if !(1..=128_000).contains(&self.max_output_tokens)
            || !(1..=300).contains(&self.timeout_seconds)
        {
            return Err("APIの出力上限またはタイムアウト設定が範囲外です".into());
        }
        if !self.api_key_env.is_empty()
            && !self
                .api_key_env
                .bytes()
                .enumerate()
                .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
        {
            return Err("APIキー環境変数名が不正です".into());
        }
        Ok(())
    }
}

pub struct ApiProvider {
    backend: Backend,
    config: ApiConfig,
    key: String,
}
impl std::fmt::Debug for ApiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiProvider")
            .field("backend", &self.backend)
            .field("key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
impl ApiProvider {
    pub fn new(
        backend: Backend,
        config: &ApiConfig,
        explicit_key: Option<&str>,
    ) -> Result<Self, String> {
        if matches!(backend, Backend::Rules | Backend::Local) {
            return Err("APIサービスを選択してください".into());
        }
        let config = config.resolved(backend);
        config.validate()?;
        let key = explicit_key
            .filter(|k| !k.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| {
                std::env::var(&config.api_key_env)
                    .ok()
                    .filter(|k| !k.trim().is_empty())
            })
            .unwrap_or_default();
        if key.is_empty() && !(backend == Backend::Custom && config.anonymous) {
            return Err("APIキーを入力するか、指定の環境変数に設定してください".into());
        }
        Ok(Self {
            backend,
            config,
            key,
        })
    }
    fn request(&self, input: &str) -> Result<(Url, HeaderMap, Value), String> {
        let mut url = self.config.validate_url()?;
        let protocol = self.config.protocol;
        {
            let mut path = url.path_segments_mut().map_err(|_| "API URLが不正です")?;
            path.pop_if_empty();
            match protocol {
                ApiProtocol::Openai => {
                    path.push("chat").push("completions");
                }
                ApiProtocol::Claude => {
                    path.push("messages");
                }
                ApiProtocol::Gemini => {
                    path.push("models").push(&format!(
                        "{}:generateContent",
                        self.config.model.trim().trim_start_matches("models/")
                    ));
                }
            }
        }
        let mut headers = HeaderMap::new();
        if !self.key.is_empty() {
            let (name, value) = match protocol {
                ApiProtocol::Gemini => ("x-goog-api-key", self.key.clone()),
                ApiProtocol::Claude => ("x-api-key", self.key.clone()),
                ApiProtocol::Openai if self.backend == Backend::Mimo => {
                    ("api-key", self.key.clone())
                }
                _ => ("authorization", format!("Bearer {}", self.key)),
            };
            let mut value = HeaderValue::from_str(&value).map_err(|_| "APIキーの形式が不正です")?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        if protocol == ApiProtocol::Claude {
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        }
        let model = self.config.model.trim();
        let body = match protocol {
            ApiProtocol::Openai => {
                let mut body = json!({"model":model,"messages":[{"role":"user","content":input}],"stream":false});
                let name = if self.backend == Backend::Openai {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                };
                body[name] = json!(self.config.max_output_tokens);
                body
            }
            ApiProtocol::Claude => {
                json!({"model":model,"max_tokens":self.config.max_output_tokens,"messages":[{"role":"user","content":input}],"stream":false})
            }
            ApiProtocol::Gemini => {
                json!({"contents":[{"role":"user","parts":[{"text":input}]}],"generationConfig":{"maxOutputTokens":self.config.max_output_tokens}})
            }
        };
        Ok((url, headers, body))
    }
}
impl Provider for ApiProvider {
    fn respond(&self, input: &str) -> Result<String, String> {
        let (url, headers, body) = self.request(input)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(self.config.timeout_seconds))
            .redirect(Policy::none())
            .build()
            .map_err(|_| "HTTPクライアントを初期化できません")?;
        let response = client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_timeout() {
                    "APIがタイムアウトしました。接続先・モデル・タイムアウト設定を確認してください"
                } else {
                    "APIに接続できません。ネットワーク・接続先・TLS設定を確認してください"
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "APIエラー（HTTP {}）。認証情報・利用上限・モデル・接続先を確認してください。",
                status.as_u16()
            ));
        }
        const MAX: u64 = 4 * 1024 * 1024;
        let mut raw = Vec::new();
        response
            .take(MAX + 1)
            .read_to_end(&mut raw)
            .map_err(|_| "API応答を読み込めません")?;
        if raw.len() as u64 > MAX {
            return Err("API応答がサイズ上限を超えました".into());
        }
        let value: Value =
            serde_json::from_slice(&raw).map_err(|_| "APIが有効なJSONを返しませんでした")?;
        extract_text(self.config.protocol, &value)
    }
}
fn extract_text(protocol: ApiProtocol, value: &Value) -> Result<String, String> {
    let text = match protocol {
        ApiProtocol::Openai => value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .pointer("/choices/0/message/refusal")
                    .and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_owned(),
        ApiProtocol::Gemini => value
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter(|p| p["thought"] != true)
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
        ApiProtocol::Claude => value["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter(|p| p["type"] == "text")
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
    };
    if text.trim().is_empty() {
        Err(
            "API応答に表示できるテキストがありません。モデルや安全設定、出力上限を確認してください"
                .into(),
        )
    } else {
        Ok(text)
    }
}
pub fn execute_backend(
    config: &Config,
    backend: Backend,
    api: &ApiConfig,
    key: Option<&str>,
    input: &str,
) -> Result<String, String> {
    // Validate before constructing a client, so invalid input never reaches an API.
    if input.trim().is_empty() {
        return Err("入力が空です".into());
    }
    if input.trim().chars().count() > config.max_input_chars {
        return Err("入力が文字数の上限を超えています".into());
    }
    if backend == Backend::Rules {
        execute(config, &Rules, input)
    } else if backend == Backend::Local {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut local = config.local.clone();
        if local.model_path.trim().is_empty() && !local.hf_url.trim().is_empty() {
            let cache =
                crate::LocalConfig::default_cache().ok_or("モデル保存先を取得できません")?;
            let token = std::env::var("HF_TOKEN").ok();
            local.model_path =
                crate::download_hf_model(&local, &cache, token.as_deref(), &cancel, |_| {})?
                    .to_string_lossy()
                    .into_owned();
        }
        let server = crate::LocalServer::start(&local, &cancel)?;
        server.connection().respond(config, input)
    } else {
        execute(config, &ApiProvider::new(backend, api, key)?, input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    #[test]
    fn every_service_has_a_valid_preset_and_expected_authentication() {
        for backend in Backend::ALL
            .into_iter()
            .filter(|b| *b != Backend::Rules && *b != Backend::Local && *b != Backend::Custom)
        {
            let mut config = backend.defaults();
            config.model = "test-model".into();
            let p = ApiProvider::new(backend, &config, Some("test-api-key")).unwrap();
            let (url, headers, body) = p.request("日本語の入力").unwrap();
            assert_eq!(url.scheme(), "https");
            let name = match backend {
                Backend::Gemini => "x-goog-api-key",
                Backend::Claude => "x-api-key",
                Backend::Mimo => "api-key",
                _ => "authorization",
            };
            assert!(headers[name].is_sensitive());
            assert!(!format!("{p:?}").contains("test-api-key"));
            assert!(!url.as_str().contains("test-api-key"));
            match backend {
                Backend::Gemini => {
                    assert!(url.path().ends_with("models/test-model:generateContent"));
                    assert_eq!(body["contents"][0]["parts"][0]["text"], "日本語の入力");
                }
                Backend::Claude => {
                    assert!(url.path().ends_with("messages"));
                    assert_eq!(headers["anthropic-version"], "2023-06-01");
                    assert_eq!(body["max_tokens"], 2048);
                }
                _ => {
                    assert!(url.path().ends_with("chat/completions"));
                    assert_eq!(body["messages"][0]["content"], "日本語の入力");
                }
            }
        }
    }
    #[test]
    fn url_and_credentials_validation() {
        for url in [
            "http://example.com/v1",
            "https://user:secret@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/#secret",
            "file:///tmp/key",
        ] {
            assert!(ApiConfig {
                base_url: url.into(),
                ..Default::default()
            }
            .validate_url()
            .is_err());
        }
        let c = ApiConfig {
            model: "model".into(),
            base_url: "http://127.0.0.1:8080/v1".into(),
            anonymous: true,
            ..Default::default()
        };
        let p = ApiProvider::new(Backend::Custom, &c, None).unwrap();
        assert!(p.request("hi").unwrap().1.is_empty());
        let mut bad = c.clone();
        bad.anonymous = false;
        bad.api_key_env = "CODEBOT_TEST_UNSET_KEY_DO_NOT_SET".into();
        assert!(ApiProvider::new(Backend::Custom, &bad, None).is_err());
        assert_eq!(
            execute_backend(
                &Config::default(),
                Backend::Openai,
                &ApiConfig::default(),
                None,
                " "
            )
            .unwrap_err(),
            "入力が空です"
        );
    }
    #[test]
    fn response_formats_and_empty_responses() {
        assert_eq!(
            extract_text(
                ApiProtocol::Openai,
                &json!({"choices":[{"message":{"content":"answer"}}]})
            )
            .unwrap(),
            "answer"
        );
        assert_eq!(extract_text(ApiProtocol::Claude,&json!({"content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"answer"}]})).unwrap(),"answer");
        assert_eq!(extract_text(ApiProtocol::Gemini,&json!({"candidates":[{"content":{"parts":[{"thought":true,"text":"hidden"},{"text":"answer"}]}}]})).unwrap(),"answer");
        for protocol in [
            ApiProtocol::Openai,
            ApiProtocol::Claude,
            ApiProtocol::Gemini,
        ] {
            assert!(extract_text(protocol, &json!({})).is_err());
        }
    }
    fn mock_request(
        backend: Backend,
        protocol: ApiProtocol,
        status: &str,
        response: &str,
    ) -> (Result<String, String>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let response = response.to_owned();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
                if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let h = String::from_utf8_lossy(&raw[..end]);
                    let size = h
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if raw.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let output=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/never-follow\r\nConnection: close\r\n\r\n{response}",response.len());
            stream.write_all(output.as_bytes()).unwrap();
            String::from_utf8(raw).unwrap()
        });
        let config = ApiConfig {
            model: "test-model".into(),
            base_url: format!("http://{address}/v1"),
            protocol,
            ..Default::default()
        };
        let p = ApiProvider::new(backend, &config, Some("mock-secret-key")).unwrap();
        let result = p.respond("input text");
        // A failed client assertion must not wait forever for an uncontacted server.
        if result.as_ref().is_err_and(|e| e.contains("接続できません")) {
            panic!("mock server was not contacted: {result:?}");
        }
        (result, server.join().unwrap())
    }
    #[test]
    fn actual_http_roundtrips_cover_three_protocols_and_mimo_header() {
        for (backend, protocol, reply, header) in [
            (
                Backend::Custom,
                ApiProtocol::Openai,
                r#"{"choices":[{"message":{"content":"mock answer"}}]}"#,
                "authorization: Bearer mock-secret-key",
            ),
            (
                Backend::Gemini,
                ApiProtocol::Gemini,
                r#"{"candidates":[{"content":{"parts":[{"text":"mock answer"}]}}]}"#,
                "x-goog-api-key: mock-secret-key",
            ),
            (
                Backend::Claude,
                ApiProtocol::Claude,
                r#"{"content":[{"type":"text","text":"mock answer"}]}"#,
                "x-api-key: mock-secret-key",
            ),
            (
                Backend::Mimo,
                ApiProtocol::Openai,
                r#"{"choices":[{"message":{"content":"mock answer"}}]}"#,
                "api-key: mock-secret-key",
            ),
        ] {
            let (result, request) = mock_request(backend, protocol, "200 OK", reply);
            assert_eq!(result.unwrap(), "mock answer");
            assert!(request
                .to_ascii_lowercase()
                .contains(&header.to_ascii_lowercase()));
            assert!(request.contains("input text"));
        }
    }
    #[test]
    fn error_bodies_are_not_exposed_and_redirects_are_not_followed() {
        let (result, _) = mock_request(
            Backend::Custom,
            ApiProtocol::Openai,
            "401 Unauthorized",
            "mock-secret-key private error text",
        );
        let error = result.unwrap_err();
        assert!(error.contains("401"));
        assert!(!error.contains("mock-secret-key"));
        assert!(!error.contains("private"));
        let (result, _) = mock_request(
            Backend::Custom,
            ApiProtocol::Openai,
            "302 Found",
            "redirect",
        );
        assert!(result.unwrap_err().contains("302"));
        let (result, _) = mock_request(
            Backend::Custom,
            ApiProtocol::Openai,
            "200 OK",
            "not-json mock-secret-key",
        );
        assert!(!result.unwrap_err().contains("mock-secret-key"));
    }
}
