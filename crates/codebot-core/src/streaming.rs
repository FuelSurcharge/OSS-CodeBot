use crate::{ApiConfig, ApiProtocol, ApiProvider, Backend, ChatMessage, Config};
use serde_json::Value;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEnd {
    Complete,
    Cancelled,
    Failed(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Delta(String),
    Finished(StreamEnd),
}
const MAX: usize = 4 * 1024 * 1024;

/// Emits text deltas followed by exactly one terminal event. No response bodies
/// or credentials are included in diagnostic errors. Cancellation drops HTTP I/O.
pub fn stream_messages(
    config: &Config,
    backend: Backend,
    api: &ApiConfig,
    key: Option<&str>,
    messages: &[ChatMessage],
    cancel: &AtomicBool,
    mut emit: impl FnMut(StreamEvent),
) {
    let result = (|| {
        crate::validate_messages(config, messages)?;
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        if backend == Backend::Rules {
            let answer = crate::execute(config, &crate::Rules, &messages.last().unwrap().content)?;
            emit(StreamEvent::Delta(answer));
            return Ok(());
        }
        let server;
        let provider = if backend == Backend::Local {
            let mut local = config.local.clone();
            if local.model_path.trim().is_empty() && !local.hf_url.trim().is_empty() {
                let cache =
                    crate::LocalConfig::default_cache().ok_or("モデル保存先を取得できません")?;
                let token = std::env::var("HF_TOKEN").ok();
                local.model_path =
                    crate::download_hf_model(&local, &cache, token.as_deref(), cancel, |_| {})?
                        .to_string_lossy()
                        .into_owned();
            }
            server = crate::LocalServer::start(&local, cancel)?;
            server.connection().api_provider()?
        } else {
            ApiProvider::new(backend, api, key)?
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "通信ランタイムを初期化できません")?;
        runtime.block_on(async {
            tokio::select! {
                biased;
                _ = cancelled(cancel) => Ok(()),
                result = provider.stream(messages, &mut emit) => result,
            }
        })
    })();
    let end = if cancel.load(Ordering::Relaxed) {
        StreamEnd::Cancelled
    } else {
        match result {
            Ok(()) => StreamEnd::Complete,
            Err(error) => StreamEnd::Failed(error),
        }
    };
    emit(StreamEvent::Finished(end));
}
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

impl ApiProvider {
    async fn stream(
        &self,
        messages: &[ChatMessage],
        emit: &mut impl FnMut(StreamEvent),
    ) -> Result<(), String> {
        let (mut url, headers, mut body) = self.request_messages(messages)?;
        self.stream_request(&mut url, headers, &mut body, emit)
            .await
    }
    async fn stream_request(
        &self,
        url: &mut reqwest::Url,
        headers: reqwest::header::HeaderMap,
        body: &mut Value,
        emit: &mut impl FnMut(StreamEvent),
    ) -> Result<(), String> {
        if self.config.protocol == ApiProtocol::Gemini {
            let model = self.config.model.trim().trim_start_matches("models/");
            url.path_segments_mut()
                .map_err(|_| "API URLが不正です")?
                .pop()
                .push(&format!("{model}:streamGenerateContent"));
            url.set_query(Some("alt=sse"));
        } else {
            body["stream"] = Value::Bool(true);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.config.timeout_seconds))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "HTTPクライアントを初期化できません")?;
        let mut response = client
            .post(url.clone())
            .headers(headers)
            .json(body)
            .send()
            .await
            .map_err(http_error)?;
        if !response.status().is_success() {
            return Err(format!(
                "APIエラー（HTTP {}）。認証情報・利用上限・モデル・接続先を確認してください。",
                response.status().as_u16()
            ));
        }
        let json_response = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|mime| mime.trim() == "application/json")
            });
        let mut parser = SseParser::default();
        let mut raw = Vec::new();
        let mut total = 0;
        let mut displayed = false;
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            total += chunk.len();
            if total > MAX {
                return Err("API応答がサイズ上限を超えました".into());
            }
            if json_response {
                raw.extend_from_slice(&chunk);
                continue;
            }
            for data in parser.push(&chunk)? {
                let (text, done) = delta(self.config.protocol, &data)?;
                if !text.is_empty() {
                    displayed = true;
                    emit(StreamEvent::Delta(text));
                }
                if done {
                    if !displayed && self.config.protocol == ApiProtocol::Responses {
                        let value: Value = serde_json::from_str(&data)
                            .map_err(|_| "Responsesの終端イベントが不正です")?;
                        if let Ok(text) =
                            crate::api::extract_text(ApiProtocol::Responses, &value["response"])
                        {
                            emit(StreamEvent::Delta(text));
                            displayed = true;
                        }
                    }
                    return if displayed {
                        Ok(())
                    } else {
                        Err("表示できるテキストがありません".into())
                    };
                }
            }
        }
        if json_response {
            let value =
                serde_json::from_slice(&raw).map_err(|_| "APIが有効なJSONを返しませんでした")?;
            emit(StreamEvent::Delta(crate::api::extract_text(
                self.config.protocol,
                &value,
            )?));
            return Ok(());
        }
        Err("接続が終端イベントの前に切断されました。途中の回答を保持しています".into())
    }
}
fn http_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "APIがタイムアウトしました。途中の回答を保持しています"
    } else {
        "APIとの通信が失敗しました。接続先・ネットワーク・TLS設定を確認してください"
    }
    .into()
}

#[derive(Default)]
struct SseParser {
    pending: Vec<u8>,
    data: Vec<String>,
}
impl SseParser {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut consumed = 0;
        for (index, byte) in self.pending.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let line = std::str::from_utf8(&self.pending[consumed..index])
                .map_err(|_| "SSEの文字コードが不正です")?
                .trim_end_matches('\r');
            consumed = index + 1;
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                self.data
                    .push(value.strip_prefix(' ').unwrap_or(value).to_owned());
            }
        }
        self.pending.drain(..consumed);
        Ok(events)
    }
}
fn delta(protocol: ApiProtocol, data: &str) -> Result<(String, bool), String> {
    if protocol == ApiProtocol::Openai && data.trim() == "[DONE]" {
        return Ok((String::new(), true));
    }
    let value: Value = serde_json::from_str(data).map_err(|_| "SSEイベントの形式が不正です")?;
    if value.get("error").is_some() || value["type"] == "error" {
        return Err("APIが生成中のエラーを返しました。途中の回答を保持しています".into());
    }
    Ok(match protocol {
        ApiProtocol::Responses => match value["type"].as_str() {
            Some("response.output_text.delta" | "response.refusal.delta") => {
                (value["delta"].as_str().unwrap_or("").into(), false)
            }
            Some("response.completed") if value["response"]["status"] == "completed" => {
                (String::new(), true)
            }
            Some("response.failed" | "response.incomplete") => {
                return Err(
                    "Responses APIの生成が完了しませんでした。途中の回答を保持しています".into(),
                )
            }
            Some("response.completed") => return Err("Responses APIの終端状態が不正です".into()),
            _ => (String::new(), false),
        },
        ApiProtocol::Openai => {
            let choice = &value["choices"][0];
            (
                choice["delta"]["content"]
                    .as_str()
                    .or_else(|| choice["delta"]["refusal"].as_str())
                    .unwrap_or("")
                    .into(),
                choice.get("finish_reason").is_some_and(|v| !v.is_null()),
            )
        }
        ApiProtocol::Claude => {
            let text = match value["type"].as_str() {
                Some("content_block_delta") if value["delta"]["type"] == "text_delta" => {
                    value["delta"]["text"].as_str()
                }
                Some("content_block_start") if value["content_block"]["type"] == "text" => {
                    value["content_block"]["text"].as_str()
                }
                _ => None,
            };
            (text.unwrap_or("").into(), value["type"] == "message_stop")
        }
        ApiProtocol::Gemini => {
            let candidate = &value["candidates"][0];
            let text = candidate["content"]["parts"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter(|part| part["thought"] != true)
                        .filter_map(|part| part["text"].as_str())
                        .collect::<String>()
                })
                .unwrap_or_default();
            (
                text,
                candidate["finishReason"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{mpsc, Arc},
        thread,
        time::Instant,
    };
    #[test]
    fn responses_incomplete_streams_keep_partial_text_and_never_report_success() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = api(listener.local_addr().unwrap(), ApiProtocol::Responses);
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_request(&mut socket);
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
            socket
                .write_all(
                    frame(json!({"type":"response.output_text.delta","delta":"途中の回答"}))
                        .as_bytes(),
                )
                .unwrap();
            socket.write_all(frame(json!({"type":"response.incomplete","response":{"status":"incomplete","error":"private-fixture-key"}})).as_bytes()).unwrap();
        });
        let mut events = vec![];
        stream_messages(
            &Config::default(),
            Backend::Custom,
            &api,
            None,
            &[ChatMessage::user("input")],
            &AtomicBool::new(false),
            |e| events.push(e),
        );
        worker.join().unwrap();
        assert_eq!(events[0], StreamEvent::Delta("途中の回答".into()));
        assert!(
            matches!(events.last(),Some(StreamEvent::Finished(StreamEnd::Failed(error))) if !error.contains("private-fixture-key"))
        );
        assert!(crate::api::extract_text(
            ApiProtocol::Responses,
            &json!({"status":"incomplete","output":[]})
        )
        .is_err());
    }

    fn read_request(stream: &mut TcpStream) -> (String, Value) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut raw = Vec::new();
        let mut byte = [0];
        while !raw.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            raw.push(byte[0]);
        }
        let header = String::from_utf8(raw).unwrap();
        let size: usize = header
            .lines()
            .find_map(|s| {
                s.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|s| s.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; size];
        stream.read_exact(&mut body).unwrap();
        (header, serde_json::from_slice(&body).unwrap())
    }
    fn api(address: std::net::SocketAddr, protocol: ApiProtocol) -> ApiConfig {
        ApiConfig {
            base_url: format!("http://{address}/v1"),
            model: "test-model".into(),
            protocol,
            anonymous: true,
            ..Default::default()
        }
    }
    fn frame(value: Value) -> String {
        format!("data: {value}\r\n\r\n")
    }

    #[test]
    fn all_protocols_deliver_split_japanese_before_the_terminal_event_and_keep_roles() {
        for protocol in [
            ApiProtocol::Openai,
            ApiProtocol::Responses,
            ApiProtocol::Claude,
            ApiProtocol::Gemini,
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let config = api(listener.local_addr().unwrap(), protocol);
            let (seen_tx, seen_rx) = mpsc::channel();
            let worker = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let (header, body) = read_request(&mut socket);
                let first = match protocol {
                    ApiProtocol::Responses => {
                        assert!(header.starts_with("POST /v1/responses "));
                        assert_eq!(body["input"][1]["role"], "assistant");
                        assert_eq!(body["store"], false);
                        json!({"type":"response.output_text.delta","delta":"日本語"})
                    }
                    ApiProtocol::Openai => {
                        assert!(header.starts_with("POST /v1/chat/completions "));
                        assert_eq!(body["stream"], true);
                        assert_eq!(body["messages"][1]["role"], "assistant");
                        json!({"choices":[{"delta":{"content":"日本語"},"finish_reason":null}]})
                    }
                    ApiProtocol::Claude => {
                        assert!(header.starts_with("POST /v1/messages "));
                        assert_eq!(body["stream"], true);
                        assert_eq!(body["messages"][1]["role"], "assistant");
                        json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"日本語"}})
                    }
                    ApiProtocol::Gemini => {
                        assert!(header.starts_with(
                            "POST /v1/models/test-model:streamGenerateContent?alt=sse "
                        ));
                        assert_eq!(body["contents"][1]["role"], "model");
                        json!({"candidates":[{"content":{"parts":[{"text":"秘密の推論","thought":true},{"text":"日本語"}]}}]})
                    }
                };
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n: heartbeat\r\n\r\n").unwrap();
                for byte in frame(first).bytes() {
                    socket.write_all(&[byte]).unwrap();
                }
                // This cannot complete unless text reaches the caller before EOF.
                seen_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                let last = match protocol {
                    ApiProtocol::Responses => frame(
                        json!({"type":"response.completed","response":{"status":"completed"}}),
                    ),
                    ApiProtocol::Openai => "data: [DONE]\r\n\r\n".to_owned(),
                    ApiProtocol::Claude => frame(json!({"type":"message_stop"})),
                    ApiProtocol::Gemini => frame(json!({"candidates":[{"finishReason":"STOP"}]})),
                };
                socket.write_all(last.as_bytes()).unwrap();
            });
            let messages = [
                ChatMessage::user("前"),
                ChatMessage::assistant("回答"),
                ChatMessage::user("次"),
            ];
            let mut events = Vec::new();
            stream_messages(
                &Config::default(),
                Backend::Custom,
                &config,
                None,
                &messages,
                &AtomicBool::new(false),
                |event| {
                    if matches!(&event, StreamEvent::Delta(text) if text == "日本語") {
                        seen_tx.send(()).unwrap();
                    }
                    events.push(event);
                },
            );
            worker.join().unwrap();
            assert_eq!(
                events,
                vec![
                    StreamEvent::Delta("日本語".into()),
                    StreamEvent::Finished(StreamEnd::Complete)
                ]
            );
        }
    }

    #[test]
    fn cancellation_closes_io_during_headers_and_body_and_emits_one_terminal_event() {
        for send_text in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let config = api(listener.local_addr().unwrap(), ApiProtocol::Openai);
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let worker = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                read_request(&mut socket);
                if send_text {
                    socket.write_all("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"途中\"}}]}\n\n".as_bytes()).unwrap();
                } else {
                    worker_cancel.store(true, Ordering::Relaxed);
                }
                let mut buf = [0; 1];
                assert!(matches!(socket.read(&mut buf), Ok(0)));
            });
            let mut events = Vec::new();
            let start = Instant::now();
            stream_messages(
                &Config::default(),
                Backend::Custom,
                &config,
                None,
                &[ChatMessage::user("input")],
                &cancel,
                |event| {
                    if matches!(event, StreamEvent::Delta(_)) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    events.push(event);
                },
            );
            assert!(start.elapsed() < Duration::from_secs(1));
            assert_eq!(
                events.last(),
                Some(&StreamEvent::Finished(StreamEnd::Cancelled))
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, StreamEvent::Finished(_)))
                    .count(),
                1
            );
            assert_eq!(events.len(), if send_text { 2 } else { 1 });
            worker.join().unwrap();
        }
    }

    #[test]
    fn premature_eof_timeout_and_api_errors_preserve_partial_text_without_error_bodies() {
        for mode in ["eof", "timeout", "error"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut config = api(listener.local_addr().unwrap(), ApiProtocol::Openai);
            config.timeout_seconds = 1;
            let worker = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                read_request(&mut socket);
                let data = if mode == "error" {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 16\r\nConnection: close\r\n\r\nprivate-test-key"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"途中\"}}]}\n\n"
                };
                socket.write_all(data.as_bytes()).unwrap();
                if mode == "timeout" {
                    let _ = socket.read(&mut [0]);
                }
            });
            let mut events = Vec::new();
            stream_messages(
                &Config::default(),
                Backend::Custom,
                &config,
                Some("private-test-key"),
                &[ChatMessage::user("input")],
                &AtomicBool::new(false),
                |e| events.push(e),
            );
            worker.join().unwrap();
            let Some(StreamEvent::Finished(StreamEnd::Failed(error))) = events.last() else {
                panic!("expected a failure: {events:?}");
            };
            assert!(!error.contains("private-test-key"));
            if mode == "timeout" {
                assert!(error.contains("タイムアウト"));
            }
            if mode != "error" {
                assert_eq!(events[0], StreamEvent::Delta("途中".into()));
            }
        }
    }
    #[test]
    fn sse_buffers_utf8_and_multiline_data_and_ignores_comments() {
        let raw = ": comment\r\nevent: content\r\ndata: {\"a\":\r\ndata: \"日本語\"}\r\n\r\n";
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        for byte in raw.bytes() {
            events.extend(parser.push(&[byte]).unwrap());
        }
        assert_eq!(events, ["{\"a\":\n\"日本語\"}"]);
        assert!(parser.push(b"data: \xff\n\n").is_err());
    }
}
