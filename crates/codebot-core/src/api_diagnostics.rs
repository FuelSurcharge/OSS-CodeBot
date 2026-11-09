use crate::{ApiConfig, ApiProtocol, ApiProvider, Backend, ChatMessage};
use serde::Serialize;
use serde_json::Value;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiDiagnosticKind {
    Authentication,
    Permission,
    RateLimit,
    ModelNotFound,
    Unsupported,
    InvalidRequest,
    InvalidResponse,
    Connection,
    Timeout,
    Cancelled,
}
#[derive(Clone, Debug, Serialize)]
pub struct ApiDiagnosticError {
    pub kind: ApiDiagnosticKind,
    pub message: String,
}
impl std::fmt::Display for ApiDiagnosticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ApiDiagnosticError {}
impl ApiDiagnosticError {
    fn new(kind: ApiDiagnosticKind) -> Self {
        let message=match kind {
            ApiDiagnosticKind::Authentication=>"認証できません。APIキーと環境変数の指定を確認してください。",
            ApiDiagnosticKind::Permission=>"権限がありません。プロジェクト・契約・地域・モデルの利用権限を確認してください。",
            ApiDiagnosticKind::RateLimit=>"レート制限です。利用上限と提供元の待機時間を確認してから再実行してください。",
            ApiDiagnosticKind::ModelNotFound=>"指定モデルが見つかりません。モデルIDと地域別の接続先を確認してください。",
            ApiDiagnosticKind::Unsupported=>"この接続先は一覧APIに対応していないか、経路が見つかりません。モデルIDを手入力してください。",
            ApiDiagnosticKind::InvalidRequest=>"要求を受理できません。モデル・プロトコル・出力上限と提供元の要件を確認してください。",
            ApiDiagnosticKind::InvalidResponse=>"応答形式が設定したプロトコルと一致しません。ベースURLとプロトコルを確認してください。",
            ApiDiagnosticKind::Connection=>"接続できません。ネットワーク・TLS・接続先・提供元の稼働状況を確認してください。",
            ApiDiagnosticKind::Timeout=>"接続確認がタイムアウトしました。接続先と待機時間を確認してください。",
            ApiDiagnosticKind::Cancelled=>"接続確認を中止しました。",
        };
        Self {
            kind,
            message: message.into(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ModelCatalog {
    pub models: Vec<String>,
    pub complete: bool,
    pub supported: bool,
    pub note: String,
}

pub fn list_api_models(
    backend: Backend,
    api: &ApiConfig,
    key: Option<&str>,
    cancel: &AtomicBool,
) -> Result<ModelCatalog, ApiDiagnosticError> {
    if matches!(
        backend,
        Backend::Rules | Backend::Local | Backend::Qwen | Backend::Glm | Backend::Mimo
    ) {
        return Ok(ModelCatalog {models:vec![],complete:false,supported:false,note:"このサービスの一覧APIは本アプリでは未確認です。提供元のモデルIDを手入力してください。".into()});
    }
    let mut api = api.clone();
    // A model ID is not needed by GET /models; validate all other settings.
    if api.model.trim().is_empty() {
        api.model = "catalog-probe".into();
    }
    let provider = ApiProvider::new(backend, &api, key).map_err(config_error)?;
    run(cancel, async {
        let (_, headers, _) = provider
            .request_messages(&[ChatMessage::user("Reply OK.")])
            .map_err(|_| ApiDiagnosticError::new(ApiDiagnosticKind::InvalidRequest))?;
        let mut url = provider
            .config
            .validate_url()
            .map_err(|_| ApiDiagnosticError::new(ApiDiagnosticKind::InvalidRequest))?;
        url.path_segments_mut()
            .map_err(|_| ApiDiagnosticError::new(ApiDiagnosticKind::InvalidRequest))?
            .pop_if_empty()
            .push("models");
        let client = client(provider.config.timeout_seconds)?;
        let mut models = Vec::new();
        let mut page: Option<String> = None;
        for _ in 0..10 {
            if cancel.load(Ordering::Relaxed) {
                return Err(ApiDiagnosticError::new(ApiDiagnosticKind::Cancelled));
            }
            let mut next = url.clone();
            match provider.config.protocol {
                ApiProtocol::Claude => {
                    let mut q = next.query_pairs_mut();
                    q.append_pair("limit", "100");
                    if let Some(page) = &page {
                        q.append_pair("after_id", page);
                    }
                }
                ApiProtocol::Gemini => {
                    let mut q = next.query_pairs_mut();
                    q.append_pair("pageSize", "100");
                    if let Some(page) = &page {
                        q.append_pair("pageToken", page);
                    }
                }
                _ => {}
            }
            let response = client
                .get(next)
                .headers(headers.clone())
                .send()
                .await
                .map_err(http_error)?;
            let value = read(response, false).await?;
            let array = match provider.config.protocol {
                ApiProtocol::Gemini => value["models"].as_array(),
                _ => value["data"].as_array(),
            }
            .ok_or_else(|| ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse))?;
            for entry in array {
                if provider.config.protocol == ApiProtocol::Gemini
                    && entry
                        .get("supportedGenerationMethods")
                        .is_some_and(|methods| {
                            !methods
                                .as_array()
                                .is_some_and(|a| a.iter().any(|m| m == "generateContent"))
                        })
                {
                    continue;
                }
                let id = if provider.config.protocol == ApiProtocol::Gemini {
                    entry["name"]
                        .as_str()
                        .map(|s| s.strip_prefix("models/").unwrap_or(s))
                } else {
                    entry["id"].as_str()
                }
                .filter(|s| valid_id(s))
                .ok_or_else(|| ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse))?;
                if provider.contains_credential(id) {
                    return Err(ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse));
                }
                models.push(id.to_owned());
                if models.len() >= 1000 {
                    models.sort();
                    models.dedup();
                    return Ok(catalog(models, false));
                }
            }
            let next = match provider.config.protocol {
                ApiProtocol::Claude if value["has_more"] == true => {
                    Some(value["last_id"].as_str().ok_or_else(|| {
                        ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse)
                    })?)
                }
                ApiProtocol::Gemini => value["nextPageToken"].as_str().filter(|s| !s.is_empty()),
                _ => None,
            };
            match next {
                Some(token)
                    if token.len() <= 1024
                        && !token.chars().any(char::is_control)
                        && page.as_deref() != Some(token) =>
                {
                    page = Some(token.to_owned())
                }
                Some(_) => return Err(ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse)),
                None => {
                    models.sort();
                    models.dedup();
                    return Ok(catalog(models, true));
                }
            }
        }
        models.sort();
        models.dedup();
        Ok(catalog(models, false))
    })
}
fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}
fn catalog(models: Vec<String>, complete: bool) -> ModelCatalog {
    ModelCatalog {models,complete,supported:true,note:if complete {"一覧を取得しました。一覧にない別名や生成権限の可否は、指定モデルの接続確認で判定してください。"} else {"一覧が取得上限に達しました。表示外のモデルIDは手入力できます。"}.into()}
}

/// An explicit generation request: callers must disclose potential API charges.
/// No conversation or generated text is returned by this diagnostic operation.
pub fn test_api_model(
    backend: Backend,
    api: &ApiConfig,
    key: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(), ApiDiagnosticError> {
    if !valid_id(&api.model) {
        return Err(ApiDiagnosticError::new(ApiDiagnosticKind::InvalidRequest));
    }
    let provider = ApiProvider::new(backend, api, key).map_err(config_error)?;
    run(cancel, async {
        let (url, headers, body) = provider
            .request_messages(&[ChatMessage::user("Reply OK.")])
            .map_err(|_| ApiDiagnosticError::new(ApiDiagnosticKind::InvalidRequest))?;
        // Respect the user's configured output limit; some models reject a
        // single-token budget or use reasoning tokens before visible text.
        let response = client(provider.config.timeout_seconds)?
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(http_error)?;
        let value = read(response, true).await?;
        let recognized = match provider.config.protocol {
            ApiProtocol::Openai => value["choices"]
                .as_array()
                .is_some_and(|c| !c.is_empty() && c[0]["message"].is_object()),
            ApiProtocol::Claude => value["type"] == "message" && value["content"].is_array(),
            ApiProtocol::Gemini => value["candidates"]
                .as_array()
                .is_some_and(|c| !c.is_empty()),
        };
        if recognized {
            Ok(())
        } else {
            Err(ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse))
        }
    })
}
fn config_error(error: String) -> ApiDiagnosticError {
    ApiDiagnosticError::new(if error.contains("APIキー") {
        ApiDiagnosticKind::Authentication
    } else {
        ApiDiagnosticKind::InvalidRequest
    })
}
fn client(timeout: u64) -> Result<reqwest::Client, ApiDiagnosticError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiDiagnosticError::new(ApiDiagnosticKind::Connection))
}
fn http_error(error: reqwest::Error) -> ApiDiagnosticError {
    ApiDiagnosticError::new(if error.is_timeout() {
        ApiDiagnosticKind::Timeout
    } else {
        ApiDiagnosticKind::Connection
    })
}
async fn read(mut response: reqwest::Response, model: bool) -> Result<Value, ApiDiagnosticError> {
    let status = response.status().as_u16();
    let kind = match status {
        401 => Some(ApiDiagnosticKind::Authentication),
        403 => Some(ApiDiagnosticKind::Permission),
        429 => Some(ApiDiagnosticKind::RateLimit),
        404 => Some(if model {
            ApiDiagnosticKind::ModelNotFound
        } else {
            ApiDiagnosticKind::Unsupported
        }),
        405 => Some(ApiDiagnosticKind::Unsupported),
        500..=599 => Some(ApiDiagnosticKind::Connection),
        _ => None,
    };
    if let Some(kind) = kind {
        return Err(ApiDiagnosticError::new(kind));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(http_error)? {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err(ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !(200..300).contains(&status) {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let code = value
            .pointer("/error/code")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(ApiDiagnosticError::new(match code {
            "model_not_found" | "NOT_FOUND" => ApiDiagnosticKind::ModelNotFound,
            "invalid_api_key" | "UNAUTHENTICATED" => ApiDiagnosticKind::Authentication,
            "PERMISSION_DENIED" => ApiDiagnosticKind::Permission,
            "RESOURCE_EXHAUSTED" => ApiDiagnosticKind::RateLimit,
            _ => ApiDiagnosticKind::InvalidRequest,
        }));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| ApiDiagnosticError::new(ApiDiagnosticKind::InvalidResponse))
}
fn run<T>(
    cancel: &AtomicBool,
    future: impl std::future::Future<Output = Result<T, ApiDiagnosticError>>,
) -> Result<T, ApiDiagnosticError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ApiDiagnosticError::new(ApiDiagnosticKind::Cancelled));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| ApiDiagnosticError::new(ApiDiagnosticKind::Connection))?;
    runtime.block_on(async {tokio::select! {biased; _=async {while !cancel.load(Ordering::Relaxed) {tokio::time::sleep(Duration::from_millis(25)).await;}}=>Err(ApiDiagnosticError::new(ApiDiagnosticKind::Cancelled)),result=future=>result}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    struct Response {
        status: u16,
        body: Value,
    }
    fn fixture(
        responses: Vec<Response>,
        check: impl Fn(usize, &str, &Value) + Send + 'static,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            for (index, response) in responses.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut raw = Vec::new();
                let mut byte = [0];
                while !raw.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    raw.push(byte[0]);
                }
                let header = String::from_utf8(raw).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                socket.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                check(index, &header, &body);
                let body = serde_json::to_vec(&response.body).unwrap();
                write!(socket,"HTTP/1.1 {} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.status,body.len()).unwrap();
                socket.write_all(&body).unwrap();
            }
        });
        (format!("http://{address}/v1"), worker)
    }
    fn config(base: String, protocol: ApiProtocol) -> ApiConfig {
        ApiConfig {
            base_url: base,
            model: "test-model".into(),
            protocol,
            max_output_tokens: 16,
            ..Default::default()
        }
    }

    #[test]
    fn service_diagnostics_use_the_right_headers_paths_and_only_the_fixed_prompt() {
        for backend in Backend::ALL
            .into_iter()
            .filter(|b| !matches!(b, Backend::Rules | Backend::Local))
        {
            if matches!(backend, Backend::Qwen | Backend::Glm | Backend::Mimo) {
                let report = list_api_models(
                    backend,
                    &ApiConfig::default(),
                    None,
                    &AtomicBool::new(false),
                )
                .unwrap();
                assert!(!report.supported);
                continue;
            }
            let protocol = backend.defaults().protocol;
            let generation = match protocol {
                ApiProtocol::Claude => json!({"type":"message","content":[]}),
                ApiProtocol::Gemini => {
                    json!({"candidates":[{"content":{"parts":[{"text":"OK"}]}}]})
                }
                _ => json!({"choices":[{"message":{"content":"OK"}}]}),
            };
            let models = if protocol == ApiProtocol::Gemini {
                json!({"models":[{"name":"models/test-model","supportedGenerationMethods":["generateContent"]},{"name":"models/embedding","supportedGenerationMethods":["embedContent"]}]})
            } else {
                json!({"data":[{"id":"test-model"}]})
            };
            let (base, worker) =
                fixture(
                    vec![
                        Response {
                            status: 200,
                            body: models,
                        },
                        Response {
                            status: 200,
                            body: generation,
                        },
                    ],
                    move |index, header, body| {
                        let lower = header.to_lowercase();
                        let auth = match protocol {
                            ApiProtocol::Claude => "x-api-key: fixture-key",
                            ApiProtocol::Gemini => "x-goog-api-key: fixture-key",
                            _ => "authorization: bearer fixture-key",
                        };
                        assert!(lower.contains(auth));
                        if index == 0 {
                            assert!(header.starts_with("GET /v1/models"));
                            assert!(body.is_null());
                        } else {
                            assert!(header.starts_with("POST "));
                            assert_eq!(body.to_string().matches("Reply OK.").count(), 1);
                            match protocol {
                                ApiProtocol::Claude => {
                                    assert!(header.starts_with("POST /v1/messages "));
                                    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
                                }
                                ApiProtocol::Gemini => assert!(header
                                    .starts_with("POST /v1/models/test-model:generateContent ")),
                                _ => {
                                    assert!(header.starts_with("POST /v1/chat/completions "));
                                    assert_eq!(body["stream"], false);
                                }
                            }
                        }
                    },
                );
            let api = config(base, protocol);
            let cancel = AtomicBool::new(false);
            let models = list_api_models(backend, &api, Some("fixture-key"), &cancel).unwrap();
            assert_eq!(models.models, ["test-model"]);
            assert!(models.complete);
            test_api_model(backend, &api, Some("fixture-key"), &cancel).unwrap();
            worker.join().unwrap();
        }
    }
    #[test]
    fn gemini_and_claude_pagination_are_bounded_and_keep_manual_ids() {
        for protocol in [ApiProtocol::Gemini, ApiProtocol::Claude] {
            let page1 = if protocol == ApiProtocol::Gemini {
                json!({"models":[{"name":"models/one"}],"nextPageToken":"next"})
            } else {
                json!({"data":[{"id":"one"}],"has_more":true,"last_id":"next"})
            };
            let page2 = if protocol == ApiProtocol::Gemini {
                json!({"models":[{"name":"models/two"}]})
            } else {
                json!({"data":[{"id":"two"}],"has_more":false})
            };
            let (base, worker) = fixture(
                vec![
                    Response {
                        status: 200,
                        body: page1,
                    },
                    Response {
                        status: 200,
                        body: page2,
                    },
                ],
                move |index, header, _| {
                    if index == 1 {
                        assert!(header.contains(if protocol == ApiProtocol::Gemini {
                            "pageToken=next"
                        } else {
                            "after_id=next"
                        }));
                    }
                },
            );
            let api = config(base, protocol);
            let report = list_api_models(
                Backend::Custom,
                &api,
                Some("fixture-key"),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(report.models, ["one", "two"]);
            assert!(report.complete);
            assert_eq!(api.model, "test-model");
            worker.join().unwrap();
        }
    }
    #[test]
    fn error_categories_and_malformed_data_do_not_expose_bodies_or_credentials() {
        for (status, body, expected) in [
            (
                401,
                json!({"error":"private-test-secret"}),
                ApiDiagnosticKind::Authentication,
            ),
            (
                403,
                json!({"error":"private-test-secret"}),
                ApiDiagnosticKind::Permission,
            ),
            (
                429,
                json!({"error":"private-test-secret"}),
                ApiDiagnosticKind::RateLimit,
            ),
            (
                404,
                json!({"error":"private-test-secret"}),
                ApiDiagnosticKind::Unsupported,
            ),
            (
                200,
                json!({"unexpected":"private-test-secret"}),
                ApiDiagnosticKind::InvalidResponse,
            ),
            (
                200,
                json!({"data":[{"id":"private-test-secret"}]}),
                ApiDiagnosticKind::InvalidResponse,
            ),
        ] {
            let (base, worker) = fixture(vec![Response { status, body }], |_, _, _| {});
            let error = list_api_models(
                Backend::Custom,
                &config(base, ApiProtocol::Openai),
                Some("private-test-secret"),
                &AtomicBool::new(false),
            )
            .unwrap_err();
            assert_eq!(error.kind, expected);
            assert!(!error.to_string().contains("private-test-secret"));
            worker.join().unwrap();
        }
        for (status, body, expected) in [
            (
                404,
                json!({"error":"secret"}),
                ApiDiagnosticKind::ModelNotFound,
            ),
            (
                400,
                json!({"error":{"code":"model_not_found","message":"secret"}}),
                ApiDiagnosticKind::ModelNotFound,
            ),
            (200, json!({"data":[]}), ApiDiagnosticKind::InvalidResponse),
        ] {
            let (base, worker) = fixture(vec![Response { status, body }], |_, _, _| {});
            let error = test_api_model(
                Backend::Custom,
                &config(base, ApiProtocol::Openai),
                Some("fixture-key"),
                &AtomicBool::new(false),
            )
            .unwrap_err();
            assert_eq!(error.kind, expected);
            assert!(!error.to_string().contains("secret"));
            worker.join().unwrap();
        }
        assert_eq!(
            list_api_models(
                Backend::Custom,
                &config("http://127.0.0.1:1/v1".into(), ApiProtocol::Openai),
                Some("fixture-key"),
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .kind,
            ApiDiagnosticKind::Cancelled
        );
    }
}
