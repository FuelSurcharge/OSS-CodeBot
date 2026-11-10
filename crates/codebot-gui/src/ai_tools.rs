use codebot_core::{
    AgentReply, ApiConfig, Backend, ChatMessage, Config, ToolAction, ToolCall, ToolConversation,
    ToolResult, ToolStatus,
};
use eframe::egui;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};
struct Task {
    rx: Option<mpsc::Receiver<(ToolConversation, Result<AgentReply, String>)>>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.rx = None;
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
#[derive(Default)]
pub struct ToolPanel {
    root: String,
    prompt: String,
    destination: String,
    conversation: Option<ToolConversation>,
    task: Option<Task>,
    replies: Vec<String>,
    call: Option<ToolCall>,
    result: Option<ToolResult>,
    confirm_result: bool,
    command: crate::command_panel::CommandPanel,
    changes: crate::change_panel::ChangePanel,
    kind: u8,
    message: Option<String>,
    markdown: egui_commonmark::CommonMarkCache,
}
impl ToolPanel {
    pub fn take_location(&mut self) -> Option<(String, String)> {
        if self.changes.take_settings_changed() {
            Some(self.changes.last_location().unwrap_or_default())
        } else {
            None
        }
    }
    pub fn is_busy(&self) -> bool {
        self.task.is_some() || self.command.is_busy() || self.changes.is_busy()
    }
    pub fn shutdown(&mut self) {
        self.task = None;
        self.command.shutdown();
    }
    pub fn set_initial(&mut self, root: Option<&str>, prompt: &str) {
        if self.conversation.is_none() && !self.is_busy() {
            self.root = root.unwrap_or("").into();
            self.prompt = prompt.into();
        }
    }
    fn request(&mut self, ctx: &egui::Context, conversation: ToolConversation) {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let ctx = ctx.clone();
        let join = std::thread::spawn(move || {
            let mut conversation = conversation;
            let result = conversation.request(&worker_cancel);
            let _ = tx.send((conversation, result));
            ctx.request_repaint();
        });
        self.task = Some(Task {
            rx: Some(rx),
            cancel,
            join: Some(join),
        });
        self.message = None;
        self.call = None;
        self.result = None;
        self.confirm_result = false;
    }
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<String> {
        let roots = [self.command.poll(), self.changes.poll()]
            .into_iter()
            .flatten()
            .collect();
        let ready = self
            .task
            .as_ref()
            .and_then(|task| task.rx.as_ref())
            .and_then(|rx| match rx.try_recv() {
                Ok(value) => Some(Ok(value)),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("ツール相談が中断しました")),
                _ => None,
            });
        if let Some(ready) = ready {
            let task = self.task.take().unwrap();
            let cancelled = task.cancel.load(Ordering::Relaxed);
            drop(task);
            match ready {
                Ok((conversation, result)) => {
                    self.conversation = Some(conversation);
                    match result {
                        _ if cancelled => {
                            self.message =
                                Some("ツール相談を中止しました。提案は実行していません。".into())
                        }
                        Ok(reply) => {
                            let text = reply.text();
                            if !text.is_empty() {
                                self.replies.push(text);
                            }
                            if let Some(call) = reply.call() {
                                self.call = Some(call.clone());
                                match call.action(Path::new(&self.root)) {
                                    Ok(ToolAction::Command(spec)) => {
                                        self.kind = 1;
                                        if let Err(error) = self.command.prepare_tool(ctx, &spec) {
                                            self.message = Some(error);
                                        }
                                    }
                                    Ok(ToolAction::Changes(proposal)) => {
                                        self.kind = 2;
                                        match self.changes.set_proposal(
                                            Some(&self.root),
                                            &serde_json::to_string(&proposal).unwrap(),
                                        ) {
                                            Ok(()) => {
                                                self.changes.lock_root();
                                                self.changes.preview(ctx)
                                            }
                                            Err(error) => self.message = Some(error),
                                        }
                                    }
                                    Err(error) => self.message = Some(error),
                                }
                            } else {
                                self.message =
                                    Some("回答が完了しました。新しく相談できます。".into());
                            }
                        }
                        Err(error) => self.message = Some(error),
                    }
                }
                Err(error) => self.message = Some(error.into()),
            }
        }
        if self.call.is_some() && self.result.is_none() {
            self.result = if self.kind == 1 {
                self.command.tool_result()
            } else {
                self.changes.tool_result()
            };
        }
        roots
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        backend: Backend,
        api: &ApiConfig,
        key: &str,
        config: &Config,
        can_mutate: bool,
    ) {
        ui.heading("AIに作業を相談");
        ui.add(egui::Label::new("変更案・コマンドの提案を受け取ります。操作ごとに内容を確認し、結果をAIへ送る前にも確認します。最大4操作・8回の通信で終了します。").wrap());
        let busy = self.is_busy();
        if self.conversation.is_none() && self.task.is_none() {
            ui.add_enabled_ui(!busy, |ui| {
                ui.label("作業フォルダ");
                ui.add(egui::TextEdit::singleline(&mut self.root).desired_width(f32::INFINITY));
                ui.label("相談する内容");
                ui.add(
                    egui::TextEdit::multiline(&mut self.prompt)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                ui.add(
                    egui::Label::new(format!(
                        "送信先：{} / {} / {}",
                        api.resolved(backend).base_url,
                        api.model,
                        api.resolved(backend).protocol.label()
                    ))
                    .wrap(),
                );
                if ui.button("内容をAIへ送って相談").clicked() {
                    match ToolConversation::new(
                        config,
                        backend,
                        api,
                        Some(key),
                        &[ChatMessage::user(self.prompt.clone())],
                        Path::new(&self.root),
                    ) {
                        Ok(conversation) => {
                            self.root = conversation.root().to_string_lossy().into_owned();
                            self.destination = format!(
                                "{} / {} / {}",
                                api.resolved(backend).base_url,
                                api.model,
                                api.resolved(backend).protocol.label()
                            );
                            self.request(ui.ctx(), conversation);
                        }
                        Err(error) => self.message = Some(error),
                    }
                }
            });
        } else {
            ui.add(egui::Label::new(format!("作業フォルダ：{}", self.root)).wrap());
            ui.collapsing("最初の相談", |ui| {
                ui.add(egui::Label::new(&self.prompt).wrap());
            });
        }
        if !self.destination.is_empty() {
            ui.add(egui::Label::new(format!("この相談の送信先：{}", self.destination)).wrap());
        }
        for (index, text) in self.replies.iter().enumerate() {
            let _ = crate::markdown::show(ui, &mut self.markdown, (usize::MAX - 2, index), text);
        }
        if let Some(call) = &self.call {
            ui.separator();
            ui.label(if self.kind == 1 {
                "提案：コマンドの実行"
            } else {
                "提案：ファイルの変更"
            });
            ui.collapsing("要求の詳細", |ui| {
                ui.add(egui::Label::new(serde_json::to_string_pretty(call).unwrap()).wrap());
            });
            if self.result.is_none() {
                if self.kind == 1 {
                    let _ = self.command.ui(ui, can_mutate);
                } else {
                    self.changes.ui(ui, can_mutate);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("この提案を拒否"))
                    .clicked()
                {
                    self.result = Some(ToolResult {
                        status: ToolStatus::Rejected,
                        summary: "利用者がこの提案を拒否しました。実行していません。".into(),
                    });
                    self.confirm_result = false;
                }
            }
        }
        if let Some(result) = &self.result {
            ui.separator();
            ui.label("AIへ送る結果");
            egui::ScrollArea::vertical()
                .id_salt("tool-result")
                .min_scrolled_height(80.)
                .max_height(180.)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(&result.summary).wrap());
                });
            ui.add_enabled(
                !busy,
                crate::checkbox::Checkbox::new(
                    &mut self.confirm_result,
                    "この結果を上記のAIへ送ることを確認しました",
                ),
            );
            if ui
                .add_enabled(
                    !busy && self.confirm_result,
                    egui::Button::new("確認した結果を送って続ける"),
                )
                .clicked()
            {
                if let (Some(mut conversation), Some(call)) = (self.conversation.take(), &self.call)
                {
                    match conversation.resolve(&call.id, result) {
                        Ok(()) => self.request(ui.ctx(), conversation),
                        Err(error) => {
                            self.message = Some(error);
                            self.conversation = Some(conversation);
                        }
                    }
                }
            }
        }
        if let Some(task) = &self.task {
            ui.label("AIへ問い合わせ中…");
            if ui.button("問い合わせを停止").clicked() {
                task.cancel.store(true, Ordering::Relaxed);
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if ui
            .add_enabled(
                !busy && self.conversation.is_some(),
                egui::Button::new("相談を終了して新しく始める"),
            )
            .clicked()
        {
            self.conversation = None;
            self.call = None;
            self.result = None;
            self.replies.clear();
            self.destination.clear();
            self.confirm_result = false;
            self.message = None;
            self.markdown = Default::default();
        }
        if let Some(message) = &self.message {
            ui.add(egui::Label::new(message).wrap());
        }
    }
}
