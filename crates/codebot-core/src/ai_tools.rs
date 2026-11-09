//! Opt-in tool conversations. Requests and results are data; this module never
//! executes a model-proposed command or applies a model-proposed file change.
use crate::{
    ApiConfig, ApiProtocol, ApiProvider, Backend, ChangeProposal, ChatMessage, CommandSpec, Config,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Serialize)]
pub enum AgentEvent {
    Text(String),
    ToolRequest(ToolCall),
}
#[derive(Clone, Serialize)]
pub struct AgentReply {
    pub events: Vec<AgentEvent>,
}
impl AgentReply {
    pub fn text(&self) -> String {
        self.events
            .iter()
            .filter_map(|e| {
                if let AgentEvent::Text(t) = e {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn call(&self) -> Option<&ToolCall> {
        self.events.iter().find_map(|e| {
            if let AgentEvent::ToolRequest(c) = e {
                Some(c)
            } else {
                None
            }
        })
    }
}
pub enum ToolAction {
    Changes(ChangeProposal),
    Command(CommandSpec),
}
impl ToolCall {
    /// Validate a proposal without reading, writing, or executing workspace data.
    pub fn action(&self, root: &Path) -> Result<ToolAction, String> {
        match self.name.as_str() {
            "codebot_propose_changes" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    proposal: ChangeProposal,
                }
                let args: Args = serde_json::from_value(self.arguments.clone())
                    .map_err(|_| "変更ツールの引数形式が不正です")?;
                let proposal = ChangeProposal::parse(
                    &serde_json::to_string(&args.proposal).map_err(|_| "変更案が不正です")?,
                )?;
                Ok(ToolAction::Changes(proposal))
            }
            "codebot_run_command" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    program: String,
                    args: Vec<String>,
                    timeout_seconds: u64,
                }
                let args: Args = serde_json::from_value(self.arguments.clone())
                    .map_err(|_| "コマンドツールの引数形式が不正です")?;
                if args.program.trim().is_empty()
                    || args.program.len() > 4096
                    || args.program.contains('\0')
                    || args.args.len() > 256
                    || args
                        .args
                        .iter()
                        .any(|s| s.len() > 16000 || s.contains('\0'))
                    || !(1..=300).contains(&args.timeout_seconds)
                {
                    return Err("コマンドツールの引数・タイムアウトが上限を超えています".into());
                }
                Ok(ToolAction::Command(CommandSpec {
                    program: args.program,
                    args: args.args,
                    cwd: root.into(),
                    timeout_seconds: args.timeout_seconds,
                    max_output_bytes: 256 * 1024,
                    ..Default::default()
                }))
            }
            _ => Err(
                "このツールには対応していません。変更案またはコマンド提案を使用してください".into(),
            ),
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Succeeded,
    Failed,
    Rejected,
    Cancelled,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    pub status: ToolStatus,
    pub summary: String,
}
struct Pending {
    call: ToolCall,
    assistant: Value,
}
pub struct ToolConversation {
    provider: ApiProvider,
    body: Value,
    root: PathBuf,
    pending: Option<Pending>,
    seen: HashSet<String>,
    seen_ids: HashSet<String>,
    calls: usize,
    requests: usize,
    finished: bool,
    max_chars: usize,
}
impl ToolConversation {
    pub fn new(
        config: &Config,
        backend: Backend,
        api: &ApiConfig,
        key: Option<&str>,
        messages: &[ChatMessage],
        root: &Path,
    ) -> Result<Self, String> {
        crate::validate_messages(config, messages)?;
        let root = crate::Workspace::open(root)?.root().to_path_buf();
        let provider = ApiProvider::new(backend, api, key)?;
        let (_, _, mut body) = provider.request_messages(messages)?;
        body["tools"] = definitions(provider.config.protocol);
        // Keep each proposal independent and individually reviewable.
        if matches!(provider.config.protocol, ApiProtocol::Responses) {
            body["parallel_tool_calls"] = json!(false);
            body["include"] = json!(["reasoning.encrypted_content"]);
        }
        Ok(Self {
            provider,
            body,
            root,
            pending: None,
            seen: HashSet::new(),
            seen_ids: HashSet::new(),
            calls: 0,
            requests: 0,
            finished: false,
            max_chars: config.max_context_chars,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn request(&mut self, cancel: &AtomicBool) -> Result<AgentReply, String> {
        if self.pending.is_some() {
            return Err("前の操作の結果または拒否を確認してから続けてください".into());
        }
        if self.finished || self.requests >= 8 {
            return Err(
                "このツール相談は終了したか、8回の通信上限に達しました。新しく相談してください"
                    .into(),
            );
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("ツール相談を中止しました".into());
        }
        if self.body.to_string().chars().count() > self.max_chars {
            return Err(
                "ツール相談の履歴が文字数上限を超えています。新しく相談してください".into(),
            );
        }
        self.requests += 1;
        let (url, headers, _) = self
            .provider
            .request_messages(&[ChatMessage::user("tool exchange")])?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "通信処理を作成できません")?;
        let future = async {
            let mut response = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(self.provider.config.timeout_seconds))
                .build()
                .map_err(|_| "通信を作成できません")?
                .post(url)
                .headers(headers)
                .json(&self.body)
                .send()
                .await
                .map_err(|_| {
                    "ツール相談に接続できません。ネットワーク・モデル・接続先を確認してください"
                })?;
            if !response.status().is_success() {
                return Err(format!("ツールAPIエラー（HTTP {}）。認証・権限・利用上限と、モデルが選択したAPI形式のツールに対応するかを確認してください",response.status().as_u16()));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "ツール相談の通信が中断しました")?
            {
                if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                    return Err("ツール応答が4 MiBを超えています".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| "ツールAPIの応答がJSONではありません".to_owned())
        };
        let value=runtime.block_on(async {tokio::select! {biased;
            _=async {while !cancel.load(Ordering::Relaxed) {tokio::time::sleep(Duration::from_millis(25)).await;}}=>Err("ツール相談を中止しました".into()),
            result=future=>result
        }})?;
        if cancel.load(Ordering::Relaxed) {
            return Err("ツール相談を中止しました".into());
        }
        let (text, call, assistant) = parse(self.provider.config.protocol, &value, self.calls)?;
        let mut events = Vec::new();
        if !text.trim().is_empty() {
            events.push(AgentEvent::Text(text));
        }
        if let Some(call) = call {
            if self.calls >= 4 {
                return Err(
                    "ツール要求は1回の相談につき4件までです。新しく相談してください".into(),
                );
            }
            call.action(&self.root)?;
            let fingerprint = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&json!([call.name, call.arguments])).unwrap())
            );
            if !self.seen.insert(fingerprint) || !self.seen_ids.insert(call.id.clone()) {
                self.finished = true;
                return Err(
                    "同じツール要求が繰り返されました。循環を止めるため相談を終了します".into(),
                );
            }
            self.calls += 1;
            self.pending = Some(Pending {
                call: call.clone(),
                assistant,
            });
            events.push(AgentEvent::ToolRequest(call));
        } else {
            self.finished = true;
            if events.is_empty() {
                return Err("ツールAPIに表示できるテキスト・対応する操作がありません".into());
            }
        }
        Ok(AgentReply { events })
    }
    /// Called only after the application separately confirms the result to send.
    pub fn resolve(&mut self, id: &str, result: &ToolResult) -> Result<(), String> {
        let pending = self
            .pending
            .as_ref()
            .ok_or("結果待ちのツール要求がありません")?;
        if pending.call.id != id
            || result.summary.chars().count() > 12000
            || result.summary.contains('\0')
        {
            return Err("ツール結果の対象またはサイズが不正です".into());
        }
        let encoded = serde_json::to_string(result).map_err(|_| "ツール結果を変換できません")?;
        let protocol = self.provider.config.protocol;
        let key = match protocol {
            ApiProtocol::Responses => "input",
            ApiProtocol::Gemini => "contents",
            _ => "messages",
        };
        let messages = self.body[key]
            .as_array_mut()
            .ok_or("ツール履歴の形式が不正です")?;
        match protocol {
            ApiProtocol::Openai => {
                messages.push(pending.assistant.clone());
                messages
                    .push(json!({"role":"tool","tool_call_id":pending.call.id,"content":encoded}));
            }
            ApiProtocol::Responses => {
                messages.extend(pending.assistant.as_array().unwrap().clone());
                messages.push(json!({"type":"function_call_output","call_id":pending.call.id,"output":encoded}));
            }
            ApiProtocol::Claude => {
                messages.push(json!({"role":"assistant","content":pending.assistant}));
                messages.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":pending.call.id,"content":encoded,"is_error":!matches!(result.status,ToolStatus::Succeeded)}]}));
            }
            ApiProtocol::Gemini => {
                messages.push(pending.assistant.clone());
                let mut response = json!({"name":pending.call.name,"response":result});
                if let Some(id) = pending.assistant["parts"]
                    .as_array()
                    .and_then(|parts| parts.iter().find_map(|p| p["functionCall"].get("id")))
                {
                    response["id"] = id.clone();
                }
                messages.push(json!({"role":"user","parts":[{"functionResponse":response}]}));
            }
        }
        self.pending = None;
        Ok(())
    }
}
fn parse(
    protocol: ApiProtocol,
    value: &Value,
    index: usize,
) -> Result<(String, Option<ToolCall>, Value), String> {
    let mut calls = Vec::new();
    let mut text = String::new();
    let assistant = match protocol {
        ApiProtocol::Openai => {
            if !matches!(
                value["choices"][0]["finish_reason"].as_str(),
                Some("stop" | "tool_calls")
            ) {
                return Err(
                    "ツール生成が完了しなかったか、モデルが対応する終端状態を返しませんでした"
                        .into(),
                );
            }
            let message = &value["choices"][0]["message"];
            text = message["content"]
                .as_str()
                .or_else(|| message["refusal"].as_str())
                .unwrap_or("")
                .into();
            for call in message["tool_calls"].as_array().into_iter().flatten() {
                calls.push(ToolCall {
                    id: string(&call["id"], 128)?,
                    name: string(&call["function"]["name"], 128)?,
                    arguments: serde_json::from_str(
                        call["function"]["arguments"]
                            .as_str()
                            .ok_or("ツール引数が文字列ではありません")?,
                    )
                    .map_err(|_| "ツール引数がJSONではありません")?,
                });
            }
            let mut assistant = json!({"role":"assistant","content":message["content"],"tool_calls":message["tool_calls"]});
            if let Some(reasoning) = message.get("reasoning_content") {
                assistant["reasoning_content"] = reasoning.clone();
            }
            assistant
        }
        ApiProtocol::Responses => {
            if value["status"] != "completed" {
                return Err("Responsesのツール生成が完了しませんでした".into());
            }
            text = crate::api::extract_text(protocol, value).unwrap_or_default();
            let output = value["output"]
                .as_array()
                .ok_or("Responsesのoutputがありません")?;
            for item in output.iter().filter(|item| item["type"] == "function_call") {
                calls.push(ToolCall {
                    id: string(&item["call_id"], 128)?,
                    name: string(&item["name"], 128)?,
                    arguments: serde_json::from_str(
                        item["arguments"]
                            .as_str()
                            .ok_or("ツール引数が文字列ではありません")?,
                    )
                    .map_err(|_| "ツール引数がJSONではありません")?,
                });
            }
            Value::Array(output.clone())
        }
        ApiProtocol::Claude => {
            if !matches!(
                value["stop_reason"].as_str(),
                Some("end_turn" | "tool_use" | "stop_sequence")
            ) {
                return Err("Claudeのツール生成が完了しませんでした".into());
            }
            let content = value["content"]
                .as_array()
                .ok_or("Claudeのcontentがありません")?;
            for item in content {
                if item["type"] == "text" {
                    text.push_str(item["text"].as_str().unwrap_or(""));
                }
                if item["type"] == "tool_use" {
                    calls.push(ToolCall {
                        id: string(&item["id"], 128)?,
                        name: string(&item["name"], 128)?,
                        arguments: item["input"].clone(),
                    });
                }
            }
            Value::Array(content.clone())
        }
        ApiProtocol::Gemini => {
            if value["candidates"][0]["finishReason"] != "STOP" {
                return Err("Geminiのツール生成が完了しませんでした".into());
            }
            let content = &value["candidates"][0]["content"];
            let parts = content["parts"]
                .as_array()
                .ok_or("Geminiのpartsがありません")?;
            for part in parts {
                if part["thought"] != true {
                    text.push_str(part["text"].as_str().unwrap_or(""));
                }
                if let Some(call) = part.get("functionCall") {
                    calls.push(ToolCall {
                        id: call
                            .get("id")
                            .map(|v| string(v, 128))
                            .transpose()?
                            .unwrap_or_else(|| format!("gemini-call-{index}")),
                        name: string(&call["name"], 128)?,
                        arguments: call["args"].clone(),
                    });
                }
            }
            content.clone()
        }
    };
    if calls.len() > 1 {
        return Err(
            "同時に複数の操作が要求されました。1回に1つのツールを要求するモデルを使用してください"
                .into(),
        );
    }
    Ok((text, calls.pop(), assistant))
}
fn string(value: &Value, max: usize) -> Result<String, String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or("ツールの識別子・名前が不正です".into())
}
fn definitions(protocol: ApiProtocol) -> Value {
    let definitions=[
        ("codebot_propose_changes","Propose file changes for user review. This does not apply changes. Use version 1 changes with operation create/replace/delete. Replace and delete require the attached file's whole-file SHA-256. Paths are relative to the selected workspace.",json!({"type":"object","properties":{"proposal":{"type":"object","properties":{"version":{"type":"integer","enum":[1]},"changes":{"type":"array","items":{"type":"object","properties":{"operation":{"type":"string","enum":["create","replace","delete"]},"path":{"type":"string"},"content":{"type":"string"},"expected_sha256":{"type":"string"},"edits":{"type":"array","items":{"type":"object","properties":{"first_line":{"type":"integer"},"last_line":{"type":"integer"},"content":{"type":"string"}},"required":["first_line","last_line","content"]}}},"required":["operation","path"]}}},"required":["version","changes"]}},"required":["proposal"]})),
        ("codebot_run_command","Propose one executable and literal argv for user review. No implicit shell. The user-selected workspace is fixed; extra environment variables cannot be requested. Maximum timeout 300 seconds.",json!({"type":"object","properties":{"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"timeout_seconds":{"type":"integer","minimum":1,"maximum":300}},"required":["program","args","timeout_seconds"]})),
    ];
    let tools=definitions.into_iter().map(|(name,description,parameters)|match protocol {
        ApiProtocol::Openai=>json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}}),
        ApiProtocol::Responses=>json!({"type":"function","name":name,"description":description,"parameters":parameters}),
        ApiProtocol::Claude=>json!({"name":name,"description":description,"input_schema":parameters}),
        ApiProtocol::Gemini=>json!({"name":name,"description":description,"parameters":parameters}),
    }).collect::<Vec<_>>();
    if protocol == ApiProtocol::Gemini {
        json!([{"functionDeclarations":tools}])
    } else {
        Value::Array(tools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::Instant,
    };
    #[test]
    fn cancellation_during_pending_headers_closes_the_request_without_a_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let worker = thread::spawn(move || {
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
            signal.store(true, Ordering::Relaxed);
        });
        let api = ApiConfig {
            base_url: format!("http://{address}/v1"),
            model: "fixture".into(),
            anonymous: true,
            ..Default::default()
        };
        let mut conversation = conversation(&api, dir.path());
        let start = Instant::now();
        assert!(conversation.request(&cancel).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(conversation.pending.is_none());
        worker.join().unwrap();
    }
    fn fixture(
        protocol: ApiProtocol,
        responses: Vec<(u16, Value)>,
    ) -> (ApiConfig, Arc<Mutex<Vec<Value>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(vec![]));
        let captured = requests.clone();
        let join = thread::spawn(move || {
            for (status, value) in responses {
                let deadline = Instant::now() + Duration::from_secs(3);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("missing fixture request: {e}"),
                    }
                };
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
                let size: usize = header
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = vec![0; size];
                socket.read_exact(&mut body).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(&body).unwrap());
                let bytes = serde_json::to_vec(&value).unwrap();
                write!(socket,"HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
                socket.write_all(&bytes).unwrap();
            }
        });
        (
            ApiConfig {
                base_url: format!("http://{address}/v1"),
                model: "fixture-model".into(),
                anonymous: true,
                protocol,
                ..Default::default()
            },
            requests,
            join,
        )
    }
    fn args(n: usize) -> Value {
        json!({"program":"python3","args":["-c",format!("open('must-not-run-{n}', 'w').write('bad')")],"timeout_seconds":5})
    }
    fn call(protocol: ApiProtocol, id: &str, arguments: Value) -> Value {
        match protocol {
            ApiProtocol::Openai => {
                json!({"choices":[{"finish_reason":"tool_calls","message":{"content":"操作を提案します。","reasoning_content":"preserve-fixture","tool_calls":[{"type":"function","id":id,"function":{"name":"codebot_run_command","arguments":arguments.to_string()}}]}}]})
            }
            ApiProtocol::Responses => {
                json!({"status":"completed","output":[{"type":"reasoning","id":"reasoning-fixture","summary":[],"encrypted_content":"encrypted-fixture"},{"type":"function_call","id":"fc-fixture","call_id":id,"name":"codebot_run_command","arguments":arguments.to_string()}]})
            }
            ApiProtocol::Claude => {
                json!({"type":"message","stop_reason":"tool_use","content":[{"type":"thinking","thinking":"private-fixture","signature":"signature-fixture"},{"type":"text","text":"操作を提案します。"},{"type":"tool_use","id":id,"name":"codebot_run_command","input":arguments}]})
            }
            ApiProtocol::Gemini => {
                json!({"candidates":[{"finishReason":"STOP","content":{"role":"model","parts":[{"text":"private-fixture","thought":true},{"functionCall":{"id":id,"name":"codebot_run_command","args":arguments},"thoughtSignature":"signature-fixture"}]}}]})
            }
        }
    }
    fn answer(protocol: ApiProtocol) -> Value {
        match protocol {
            ApiProtocol::Openai => {
                json!({"choices":[{"finish_reason":"stop","message":{"content":"完了しました。"}}]})
            }
            ApiProtocol::Responses => {
                json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"完了しました。"}]}]})
            }
            ApiProtocol::Claude => {
                json!({"type":"message","stop_reason":"end_turn","content":[{"type":"text","text":"完了しました。"}]})
            }
            ApiProtocol::Gemini => {
                json!({"candidates":[{"finishReason":"STOP","content":{"role":"model","parts":[{"text":"完了しました。"}]}}]})
            }
        }
    }
    fn conversation(api: &ApiConfig, root: &Path) -> ToolConversation {
        ToolConversation::new(
            &Config::default(),
            Backend::Custom,
            api,
            None,
            &[ChatMessage::user("操作を提案してください")],
            root,
        )
        .unwrap()
    }
    #[test]
    fn four_protocols_keep_call_ids_signatures_and_results_without_executing_anything() {
        for protocol in [
            ApiProtocol::Openai,
            ApiProtocol::Responses,
            ApiProtocol::Claude,
            ApiProtocol::Gemini,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (api, requests, worker) = fixture(
                protocol,
                vec![
                    (200, call(protocol, "call-1", args(0))),
                    (200, answer(protocol)),
                ],
            );
            let mut conversation = conversation(&api, dir.path());
            let reply = conversation.request(&AtomicBool::new(false)).unwrap();
            let call = reply.call().unwrap();
            assert_eq!(call.id, "call-1");
            assert!(!reply.text().contains("private-fixture"));
            assert!(!dir.path().join("must-not-run-0").exists());
            assert!(matches!(
                call.action(dir.path()).unwrap(),
                ToolAction::Command(_)
            ));
            assert!(conversation.request(&AtomicBool::new(false)).is_err());
            let result = ToolResult {
                status: ToolStatus::Rejected,
                summary: "User declined this operation.".into(),
            };
            assert!(conversation.resolve("wrong-call", &result).is_err());
            conversation.resolve(&call.id, &result).unwrap();
            assert_eq!(
                conversation
                    .request(&AtomicBool::new(false))
                    .unwrap()
                    .text(),
                "完了しました。"
            );
            assert!(conversation.request(&AtomicBool::new(false)).is_err());
            worker.join().unwrap();
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(
                requests[0]["tools"].as_array().unwrap().len(),
                if protocol == ApiProtocol::Gemini {
                    1
                } else {
                    2
                }
            );
            match protocol {
                ApiProtocol::Openai => {
                    assert_eq!(
                        requests[1]["messages"][1]["reasoning_content"],
                        "preserve-fixture"
                    );
                    assert_eq!(requests[1]["messages"][2]["tool_call_id"], "call-1");
                }
                ApiProtocol::Responses => {
                    assert_eq!(requests[1]["store"], false);
                    assert_eq!(requests[0]["include"][0], "reasoning.encrypted_content");
                    assert_eq!(
                        requests[1]["input"][1]["encrypted_content"],
                        "encrypted-fixture"
                    );
                    assert_eq!(requests[1]["input"][1]["type"], "reasoning");
                    assert_eq!(requests[1]["input"][3]["call_id"], "call-1");
                }
                ApiProtocol::Claude => {
                    assert_eq!(
                        requests[1]["messages"][1]["content"][0]["signature"],
                        "signature-fixture"
                    );
                    assert_eq!(
                        requests[1]["messages"][2]["content"][0]["tool_use_id"],
                        "call-1"
                    );
                    assert_eq!(requests[1]["messages"][2]["content"][0]["is_error"], true);
                }
                ApiProtocol::Gemini => {
                    assert_eq!(
                        requests[1]["contents"][1]["parts"][1]["thoughtSignature"],
                        "signature-fixture"
                    );
                    assert_eq!(
                        requests[1]["contents"][2]["parts"][0]["functionResponse"]["id"],
                        "call-1"
                    );
                }
            }
            assert!(requests[1].to_string().contains("rejected"));
        }
    }
    #[test]
    fn call_limits_repetition_and_reused_ids_stop_before_another_operation_can_run() {
        for repeated in ["none", "arguments", "id"] {
            let dir = tempfile::tempdir().unwrap();
            let protocol = ApiProtocol::Openai;
            let count = if repeated == "none" { 5 } else { 2 };
            let responses = (0..count)
                .map(|i| {
                    (
                        200,
                        call(
                            protocol,
                            &format!("call-{}", if repeated == "id" { 0 } else { i }),
                            args(if repeated == "arguments" { 0 } else { i }),
                        ),
                    )
                })
                .collect();
            let (api, _, worker) = fixture(protocol, responses);
            let mut conversation = conversation(&api, dir.path());
            let cancel = AtomicBool::new(false);
            for _ in 0..count - 1 {
                let reply = conversation.request(&cancel).unwrap();
                let call = reply.call().unwrap();
                conversation
                    .resolve(
                        &call.id,
                        &ToolResult {
                            status: ToolStatus::Failed,
                            summary: "Test result; no command executed.".into(),
                        },
                    )
                    .unwrap();
            }
            let error = conversation.request(&cancel).err().unwrap();
            assert!(error.contains(if repeated == "none" {
                "4件"
            } else {
                "繰り返"
            }));
            worker.join().unwrap();
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn invalid_arguments_extra_permissions_and_incomplete_generations_are_inert() {
        let dir = tempfile::tempdir().unwrap();
        let mut bad = args(0);
        bad["cwd"] = json!("/outside");
        for arguments in [
            bad,
            json!({"program":"python3","args":[],"timeout_seconds":0}),
            json!({"program":"python3","args":[],"timeout_seconds":5,"pass_env":["SECRET"]}),
        ] {
            assert!(ToolCall {
                id: "1".into(),
                name: "codebot_run_command".into(),
                arguments
            }
            .action(dir.path())
            .is_err());
        }
        let changes = ToolCall {
            id: "1".into(),
            name: "codebot_propose_changes".into(),
            arguments: json!({"proposal":{"version":1,"changes":[{"operation":"create","path":"../outside","content":"bad"}]}}),
        };
        let ToolAction::Changes(proposal) = changes.action(dir.path()).unwrap() else {
            panic!()
        };
        assert!(crate::prepare_changes(dir.path(), &proposal).is_err());
        let mut incomplete = call(ApiProtocol::Openai, "id", args(0));
        incomplete["choices"][0]["finish_reason"] = json!("length");
        assert!(parse(ApiProtocol::Openai, &incomplete, 0).is_err());
        let mut unknown = call(ApiProtocol::Claude, "id", args(0));
        unknown["content"][2]["name"] = json!("unapproved_network_tool");
        let (_, call, _) = parse(ApiProtocol::Claude, &unknown, 0).unwrap();
        assert!(call.unwrap().action(dir.path()).is_err());
    }
    #[test]
    fn unsupported_http_errors_cancellation_and_request_budgets_do_not_expose_error_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _, worker) = fixture(
            ApiProtocol::Openai,
            (0..8)
                .map(|_| (400, json!({"error":"private-fixture-key"})))
                .collect(),
        );
        let mut conversation = conversation(&api, dir.path());
        for _ in 0..8 {
            let error = conversation.request(&AtomicBool::new(false)).err().unwrap();
            assert!(error.contains("モデル") && !error.contains("private-fixture-key"));
        }
        assert!(conversation
            .request(&AtomicBool::new(false))
            .err()
            .unwrap()
            .contains("8回"));
        worker.join().unwrap();
        let mut conversation = super::tests::conversation(
            &ApiConfig {
                base_url: "http://127.0.0.1:1/v1".into(),
                model: "fixture".into(),
                anonymous: true,
                ..Default::default()
            },
            dir.path(),
        );
        assert!(conversation
            .request(&AtomicBool::new(true))
            .err()
            .unwrap()
            .contains("中止"));
        conversation.max_chars = 1;
        assert!(conversation
            .request(&AtomicBool::new(false))
            .err()
            .unwrap()
            .contains("文字数"));
    }
}
