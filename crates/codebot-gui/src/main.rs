use codebot_core::{
    stream_messages, validate_messages, ApiConfig, ApiProtocol, Backend, ChatMessage, Config,
    StreamEnd, StreamEvent,
};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};
mod api_diagnostics;
mod change_panel;
mod chat_storage;
mod command_panel;
mod git_panel;
mod github_panel;
mod local_diagnostics;
mod local_models;
mod markdown;
mod path_picker;
mod ui_preferences;
mod workspace_panel;
use local_models::LocalModels;
use ui_preferences::{apply_font, FontChoice, UiPreferences};

const SHELL: Color32 = Color32::from_rgb(244, 247, 251);
const SURFACE: Color32 = Color32::WHITE;
const RAISED: Color32 = Color32::from_rgb(248, 250, 253);
const BORDER: Color32 = Color32::from_rgb(210, 220, 233);
const TEXT: Color32 = Color32::from_rgb(28, 40, 58);
const MUTED: Color32 = Color32::from_rgb(88, 104, 125);
const ACCENT: Color32 = Color32::from_rgb(0, 95, 184);
const ACCENT_HOVER: Color32 = Color32::from_rgb(0, 78, 153);
const SELECTED: Color32 = Color32::from_rgb(229, 240, 253);
const DISABLED: Color32 = Color32::from_rgb(219, 227, 238);
const MEASURE: f32 = 680.0;
const COMPOSER_WIDTH: f32 = 740.0;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Session {
    label: String,
    messages: Vec<(bool, String)>,
    draft: String,
    attachments: Vec<codebot_core::Attachment>,
    analysis_action: Option<codebot_core::AnalysisAction>,
    workspace_root: Option<String>,
    history: Vec<ChatMessage>,
    #[serde(skip)]
    history_enabled: bool,
    context_key: Option<String>,
    #[serde(skip)]
    in_flight_input: Option<String>,
    #[serde(skip)]
    retry_input: Option<String>,
    #[serde(skip)]
    notice: Option<String>,
    #[serde(skip)]
    stream_index: Option<usize>,
    generation_incomplete: bool,
}
impl Session {
    fn request_messages(
        &mut self,
        config: &Config,
        destination: &str,
        rules: bool,
    ) -> Result<Vec<ChatMessage>, String> {
        let changed = self
            .context_key
            .as_deref()
            .is_some_and(|key| key != destination);
        if changed && self.history_enabled && !self.history.is_empty() && !rules {
            self.history_enabled = false;
            return Err("送信先またはモデルが変わりました。新しい送信先へ過去の会話を送る場合は「会話履歴を含める」を再度有効にしてください。下書きは保持しています。".into());
        }
        let mut messages = if self.history_enabled && !rules {
            self.history.clone()
        } else {
            vec![]
        };
        let input = self
            .analysis_action
            .map(|action| format!("{}\n\n利用者の依頼:\n{}", action.prompt(), self.draft))
            .unwrap_or_else(|| self.draft.clone());
        messages.push(ChatMessage::user(codebot_core::compose_input(
            &input,
            &self.attachments,
        )?));
        validate_messages(config, &messages)?;
        if changed && !self.history_enabled {
            self.history.clear();
        }
        self.context_key = Some(destination.to_owned());
        Ok(messages)
    }
    fn title(&self) -> String {
        if !self.label.trim().is_empty() {
            return self.label.trim().to_owned();
        }
        self.messages
            .first()
            .map(|(_, s)| {
                let first = s.trim().lines().next().unwrap_or(s).trim();
                let title: String = first.chars().take(24).collect();
                if first.chars().count() > 24 {
                    format!("{title}…")
                } else {
                    title
                }
            })
            .unwrap_or_else(|| "新しいチャット".into())
    }
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.title().to_lowercase().contains(&query)
            || self
                .messages
                .iter()
                .any(|(_, text)| text.to_lowercase().contains(&query))
    }
}
struct App {
    sessions: Vec<Session>,
    active: usize,
    config_path: String,
    backend: Backend,
    api_configs: BTreeMap<Backend, ApiConfig>,
    api_key: String,
    api_diagnostics: api_diagnostics::ApiDiagnostics,
    local: LocalModels,
    font: FontChoice,
    preferences_path: Option<PathBuf>,
    preferences_error: Option<String>,
    settings: bool,
    sidebar: bool,
    copied: Option<(usize, usize)>,
    markdown_cache: egui_commonmark::CommonMarkCache,
    pending: Option<(usize, mpsc::Receiver<StreamEvent>)>,
    request_cancel: Option<Arc<AtomicBool>>,
    archive_last_saved: std::time::Instant,
    save_chats: bool,
    sessions_dirty: bool,
    archive_blocked: bool,
    storage_error: Option<String>,
    search: String,
    delete_confirm: bool,
    export_path: String,
    export_overwrite: bool,
    export_message: Option<String>,
    archive_path_override: String,
    archive_path_edit: String,
    workspace_panel: workspace_panel::WorkspacePanel,
    files_open: bool,
    change_panel: change_panel::ChangePanel,
    changes_open: bool,
    commands_open: bool,
    git_open: bool,
    github_open: bool,
    github_panel: github_panel::GithubPanel,
    git_panel: git_panel::GitPanel,
    command_panel: command_panel::CommandPanel,
}
impl Drop for App {
    fn drop(&mut self) {
        self.stop_generation();
        self.command_panel.shutdown();
        self.git_panel.shutdown();
        self.github_panel.shutdown();
        self.pending = None;
        self.flush_chats();
    }
}
impl Default for App {
    fn default() -> Self {
        Self {
            sessions: vec![Session::default()],
            active: 0,
            config_path: String::new(),
            backend: Backend::Rules,
            api_configs: BTreeMap::new(),
            api_key: String::new(),
            api_diagnostics: api_diagnostics::ApiDiagnostics::default(),
            local: LocalModels::default(),
            font: FontChoice::default(),
            preferences_path: None,
            preferences_error: None,
            settings: false,
            sidebar: true,
            copied: None,
            markdown_cache: egui_commonmark::CommonMarkCache::default(),
            pending: None,
            request_cancel: None,
            archive_last_saved: std::time::Instant::now(),
            save_chats: false,
            sessions_dirty: false,
            archive_blocked: false,
            storage_error: None,
            search: String::new(),
            delete_confirm: false,
            export_path: String::new(),
            export_overwrite: false,
            export_message: None,
            archive_path_override: String::new(),
            archive_path_edit: String::new(),
            workspace_panel: workspace_panel::WorkspacePanel::default(),
            files_open: false,
            change_panel: change_panel::ChangePanel::default(),
            changes_open: false,
            commands_open: false,
            git_open: false,
            github_open: false,
            github_panel: github_panel::GithubPanel::default(),
            git_panel: git_panel::GitPanel::default(),
            command_panel: command_panel::CommandPanel::default(),
        }
    }
}
impl App {
    fn archive_path(&self) -> Option<PathBuf> {
        if !self.archive_path_override.trim().is_empty() {
            let path = PathBuf::from(self.archive_path_override.trim());
            return path.is_absolute().then_some(path);
        }
        self.preferences_path
            .as_ref()
            .map(|path| path.with_file_name("chats.json"))
    }
    fn restore_chats(&mut self, merge: bool) {
        let result = self
            .archive_path()
            .ok_or_else(|| "チャット保存先を取得できません".to_owned())
            .and_then(|path| chat_storage::load(&path));
        match result {
            Ok(Some(mut archive)) => {
                for session in &mut archive.sessions {
                    if session.generation_incomplete {
                        if session.messages.last().is_none_or(|(user, _)| *user) {
                            session.messages.push((false, String::new()));
                        }
                        session.messages.last_mut().unwrap().1.push_str("\n\n---\n生成中にアプリが終了しました。途中の回答は会話履歴へ送信しません。");
                        session.generation_incomplete = false;
                    }
                }
                if merge
                    && self.sessions.iter().any(|s| {
                        !s.messages.is_empty() || !s.draft.is_empty() || !s.label.is_empty()
                    })
                {
                    let old_active = self.active;
                    for (index, session) in self.sessions.drain(..).enumerate() {
                        let position = archive.sessions.iter().position(|saved| {
                            saved.label == session.label
                                && saved.messages == session.messages
                                && saved.draft == session.draft
                                && saved.history == session.history
                                && saved.context_key == session.context_key
                                && saved.attachments == session.attachments
                                && saved.analysis_action == session.analysis_action
                                && saved.workspace_root == session.workspace_root
                        });
                        let position = match position {
                            Some(position) => position,
                            None => {
                                archive.sessions.push(session);
                                archive.sessions.len() - 1
                            }
                        };
                        if index == old_active {
                            self.active = position;
                        }
                    }
                } else {
                    self.active = archive.active;
                }
                self.sessions = archive.sessions;
                self.archive_blocked = false;
                self.storage_error = None;
                self.sessions_dirty = merge;
            }
            Ok(None) => {
                self.archive_blocked = false;
                self.storage_error = None;
                self.sessions_dirty = true;
            }
            Err(error) => {
                self.archive_blocked = true;
                self.storage_error = Some(error);
            }
        }
    }
    fn flush_chats(&mut self) {
        if !self.save_chats || !self.sessions_dirty || self.archive_blocked {
            return;
        }
        if self.pending.is_some()
            && self.archive_last_saved.elapsed() < std::time::Duration::from_millis(500)
        {
            return;
        }
        self.archive_last_saved = std::time::Instant::now();
        self.sessions_dirty = false;
        self.storage_error = self
            .archive_path()
            .ok_or_else(|| "チャット保存先を取得できません".to_owned())
            .and_then(|path| chat_storage::save(&path, &self.sessions, self.active))
            .err();
    }
    fn delete_active_chat(&mut self) {
        if self.pending.is_some() {
            return;
        }
        self.sessions.remove(self.active);
        if self.sessions.is_empty() {
            self.sessions.push(Session::default());
        }
        self.active = self.active.min(self.sessions.len() - 1);
        self.delete_confirm = false;
        self.copied = None;
        self.sessions_dirty = true;
    }
    fn chat_settings(&mut self, ui: &mut egui::Ui) {
        ui.label("保存ファイルのフルパス（空欄はOS標準のchats.json）");
        ui.text_edit_singleline(&mut self.archive_path_edit);
        if ui
            .add_enabled(self.pending.is_none(), egui::Button::new("保存先を適用"))
            .clicked()
        {
            if self.archive_path_edit.trim().is_empty()
                || Path::new(self.archive_path_edit.trim()).is_absolute()
            {
                self.archive_path_override = self.archive_path_edit.trim().to_owned();
                self.save_preferences();
                if self.save_chats {
                    self.restore_chats(true);
                }
            } else {
                self.storage_error = Some("保存先には絶対パスを指定してください".into());
            }
        }
        if ui
            .add_enabled(
                self.pending.is_none(),
                egui::Checkbox::new(&mut self.save_chats, "チャットと下書きをこのPCに保存する"),
            )
            .changed()
        {
            self.save_preferences();
            if self.save_chats {
                self.restore_chats(true);
            }
        }
        ui.label("保存は任意です。無効にしても既存の保存ファイルは保持します。復元した会話の送信には再度チェックが必要です。");
        if let Some(path) = self.archive_path() {
            ui.label(format!("保存先: {}", path.display()));
        }
        if let Some(error) = &self.storage_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if ui
            .add_enabled(
                self.pending.is_none(),
                egui::Button::new("保存・再読み込みを再試行"),
            )
            .clicked()
        {
            if self.archive_blocked {
                self.restore_chats(true);
            } else {
                self.sessions_dirty = true;
                self.flush_chats();
            }
        }
        ui.add_space(8.0);
        ui.label("このチャットの名前");
        if ui
            .text_edit_singleline(&mut self.sessions[self.active].label)
            .changed()
        {
            self.sessions_dirty = true;
        }
        if ui
            .add_enabled(
                self.pending.is_none(),
                egui::Button::new("このチャットを削除…"),
            )
            .clicked()
        {
            self.delete_confirm = true;
        }
        if self.delete_confirm {
            ui.label(format!(
                "「{}」を削除しますか？",
                self.sessions[self.active].title()
            ));
            ui.horizontal(|ui| {
                if ui.button("削除する").clicked() {
                    self.delete_active_chat();
                }
                if ui.button("キャンセル").clicked() {
                    self.delete_confirm = false;
                }
            });
        }
        ui.add_space(8.0);
        ui.label("書き出し先のファイルパス");
        ui.text_edit_singleline(&mut self.export_path);
        ui.checkbox(
            &mut self.export_overwrite,
            "書き出し先の既存ファイルの上書きを許可する",
        );
        ui.horizontal(|ui| {
            for (label, markdown) in [("Markdownで書き出す", true), ("JSONで書き出す", false)]
            {
                if ui
                    .add_enabled(
                        !self.export_path.trim().is_empty(),
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    let path = Path::new(self.export_path.trim());
                    // Exports must never replace the live archive or application settings.
                    let protected = [self.archive_path(), self.preferences_path.clone()]
                        .into_iter()
                        .flatten()
                        .any(|p| {
                            p == path
                                || p.canonicalize()
                                    .ok()
                                    .zip(path.canonicalize().ok())
                                    .is_some_and(|(a, b)| a == b)
                        });
                    let result = if protected {
                        Err(
                            "アプリの保存・設定ファイルを上書きする書き出し先は指定できません"
                                .into(),
                        )
                    } else {
                        chat_storage::export(
                            path,
                            &self.sessions[self.active],
                            markdown,
                            self.export_overwrite,
                        )
                    };
                    self.export_message = Some(match result {
                        Ok(()) => "チャットを書き出しました".into(),
                        Err(error) => error,
                    });
                }
            }
        });
        if let Some(message) = &self.export_message {
            ui.label(message);
        }
    }
    fn load(ctx: &egui::Context) -> Self {
        let mut app = Self::default();
        app.preferences_path =
            codebot_core::default_config_path().map(|p| p.with_file_name("gui-preferences.toml"));
        if let Some(path) = &app.preferences_path {
            match UiPreferences::load(path) {
                Ok(preferences) => {
                    app.font = preferences.font;
                    app.backend = preferences.backend;
                    app.api_configs = preferences.api_configs;
                    app.local.config = preferences.local;
                    app.github_panel.config = preferences.github;
                    app.save_chats = preferences.save_chats;
                    app.archive_path_override = preferences.chat_archive_path;
                    app.archive_path_edit = app.archive_path_override.clone();
                    app.change_panel.restore_location(
                        preferences.last_change_root,
                        preferences.last_change_journal,
                    );
                }
                Err(e) => app.preferences_error = Some(e),
            }
        }
        if app.save_chats {
            app.restore_chats(false);
        }
        apply_font(ctx, app.font);
        app
    }
    fn save_font_choice(&mut self, ctx: &egui::Context) {
        apply_font(ctx, self.font);
        self.save_preferences();
    }
    fn save_preferences(&mut self) {
        let (last_change_root, last_change_journal) =
            self.change_panel.last_location().unwrap_or_default();
        self.preferences_error = match &self.preferences_path {
            Some(path) => UiPreferences {
                font: self.font,
                backend: self.backend,
                api_configs: self.api_configs.clone(),
                local: self.local.config.clone(),
                save_chats: self.save_chats,
                chat_archive_path: self.archive_path_override.clone(),
                last_change_root,
                last_change_journal,
                github: self.github_panel.config.clone(),
            }
            .save(path)
            .err(),
            None => Some("設定の保存先を取得できません。この起動中のみ反映されます。".into()),
        };
    }
    fn new_chat(&mut self) {
        self.sessions.push(Session::default());
        self.active = self.sessions.len() - 1;
        self.settings = false;
        self.files_open = false;
        self.changes_open = false;
        self.commands_open = false;
        self.git_open = false;
        self.github_open = false;
        self.delete_confirm = false;
        self.sessions_dirty = true;
    }
    fn send(&mut self, ctx: &egui::Context) {
        if self.change_panel.is_busy()
            || self.command_panel.is_busy()
            || self.git_panel.is_busy()
            || self.github_panel.is_busy()
        {
            self.sessions[self.active].notice =
                Some("実行中の処理が終わってから送信してください。下書きは保持しています。".into());
            return;
        }
        if self.pending.is_some() || self.sessions[self.active].draft.trim().is_empty() {
            return;
        }
        let local_connection = if self.backend == Backend::Local {
            match self.local.connection() {
                Ok(c) => Some(c),
                Err(e) => {
                    self.local.message = Some(e);
                    self.settings = true;
                    return;
                }
            }
        } else {
            None
        };
        let path = self.config_path.trim();
        let config = match Config::load(if path.is_empty() {
            None
        } else {
            Some(Path::new(path))
        }) {
            Ok(c) => c,
            Err(e) => {
                self.sessions[self.active].notice = Some(e);
                return;
            }
        };
        let destination = self.destination_key();
        let messages = match self.sessions[self.active].request_messages(
            &config,
            &destination,
            self.backend == Backend::Rules,
        ) {
            Ok(messages) => messages,
            Err(e) => {
                self.sessions[self.active].notice = Some(e);
                return;
            }
        };
        self.sessions[self.active].notice = None;
        let input = std::mem::take(&mut self.sessions[self.active].draft);
        self.sessions[self.active].in_flight_input = Some(messages.last().unwrap().content.clone());
        self.sessions_dirty = true;
        let attachments = std::mem::take(&mut self.sessions[self.active].attachments);
        self.sessions[self.active].analysis_action = None;
        let input = if attachments.is_empty() {
            input
        } else {
            format!(
                "{}\n\n添付: {}",
                input,
                attachments
                    .iter()
                    .map(|a| format!("{}（{}–{}行）", a.path, a.first_line, a.last_line))
                    .collect::<Vec<_>>()
                    .join("、")
            )
        };
        self.sessions[self.active]
            .messages
            .push((true, input.clone()));
        let backend = self.backend;
        let api = self
            .api_configs
            .get(&backend)
            .cloned()
            .unwrap_or_else(|| backend.defaults());
        let api_key = self.api_key.clone();
        let (tx, rx) = mpsc::sync_channel(64);
        let cancel = Arc::new(AtomicBool::new(false));
        self.request_cancel = Some(cancel.clone());
        self.sessions[self.active].stream_index = None;
        self.sessions[self.active].generation_incomplete = true;
        self.pending = Some((self.active, rx));
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let emit = |event| {
                if tx.send(event).is_err() {
                    cancel.store(true, Ordering::Relaxed);
                }
                ctx.request_repaint();
            };
            if let Some(connection) = local_connection {
                connection.stream_messages(&config, &messages, &cancel, emit);
            } else {
                stream_messages(
                    &config,
                    backend,
                    &api,
                    Some(&api_key),
                    &messages,
                    &cancel,
                    emit,
                );
            }
        });
    }
    fn stop_generation(&mut self) {
        if let Some(cancel) = &self.request_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    fn poll(&mut self) {
        for _ in 0..256 {
            let Some((id, event)) =
                self.pending
                    .as_ref()
                    .and_then(|(id, rx)| match rx.try_recv() {
                        Ok(event) => Some((*id, event)),
                        Err(mpsc::TryRecvError::Disconnected) => Some((
                            *id,
                            StreamEvent::Finished(StreamEnd::Failed("処理が中断されました".into())),
                        )),
                        Err(mpsc::TryRecvError::Empty) => None,
                    })
            else {
                break;
            };
            let stopped = self
                .request_cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed));
            let session = &mut self.sessions[id];
            match event {
                StreamEvent::Delta(text) if !stopped => {
                    let index = *session.stream_index.get_or_insert_with(|| {
                        session.messages.push((false, String::new()));
                        session.messages.len() - 1
                    });
                    session.messages[index].1.push_str(&text);
                }
                StreamEvent::Delta(_) => continue,
                StreamEvent::Finished(end) => {
                    session.generation_incomplete = false;
                    let end = if stopped { StreamEnd::Cancelled } else { end };
                    let index = session.stream_index.take().unwrap_or_else(|| {
                        session.messages.push((false, String::new()));
                        session.messages.len() - 1
                    });
                    match end {
                        StreamEnd::Complete => {
                            session.retry_input = None;
                            if let Some(input) = session.in_flight_input.take() {
                                session.history.extend([
                                    ChatMessage::user(input),
                                    ChatMessage::assistant(session.messages[index].1.clone()),
                                ]);
                            }
                            eprintln!("INFO request completed");
                        }
                        status => {
                            session.retry_input = session.in_flight_input.take();
                            let notice = match status {
                                StreamEnd::Cancelled => {
                                    "生成を停止しました。途中の回答は会話履歴へ送信しません。"
                                        .into()
                                }
                                StreamEnd::Failed(error) => format!(
                                    "{error}\n設定と入力を確認して、もう一度送信してください。"
                                ),
                                _ => unreachable!(),
                            };
                            session.messages[index]
                                .1
                                .push_str(&format!("\n\n---\n{notice}"));
                            eprintln!("INFO request ended without a complete answer");
                        }
                    }
                    self.pending = None;
                    self.request_cancel = None;
                }
            }
            self.sessions_dirty = true;
        }
    }
    fn destination_key(&self) -> String {
        if self.backend == Backend::Local {
            return format!(
                "local:{:?}:{}",
                self.local.config.engine, self.local.config.model_path
            );
        }
        let api = self
            .api_configs
            .get(&self.backend)
            .cloned()
            .unwrap_or_else(|| self.backend.defaults())
            .resolved(self.backend);
        format!(
            "{}:{:?}:{}:{}",
            self.backend.id(),
            api.protocol,
            api.base_url,
            api.model
        )
    }
    fn composer(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        for (index, attachment) in self.sessions[self.active].attachments.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "添付: {}（{}–{}行）",
                    attachment.path, attachment.first_line, attachment.last_line
                ));
                if ui.small_button("外す").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.sessions[self.active].attachments.remove(index);
            self.sessions_dirty = true;
        }
        let enabled = self.pending.is_none()
            && !self.change_panel.is_busy()
            && !self.command_panel.is_busy()
            && !self.git_panel.is_busy()
            && !self.github_panel.is_busy()
            && !self.sessions[self.active].draft.trim().is_empty();
        let input_height = 2.0 * ui.text_style_height(&egui::TextStyle::Body);
        egui::Frame::new()
            .fill(RAISED)
            .corner_radius(20.0)
            .stroke(egui::Stroke::new(1.0_f32, BORDER))
            .inner_margin(egui::Margin::symmetric(18, 16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let text_width =
                        (ui.available_width() - 36.0 - ui.spacing().item_spacing.x).max(100.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, input_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt(("composer-scroll", self.active))
                                .max_height(input_height)
                                .min_scrolled_height(input_height)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    let response = ui.add(
                                        egui::TextEdit::multiline(
                                            &mut self.sessions[self.active].draft,
                                        )
                                        .id_salt(("draft", self.active))
                                        .hint_text("メッセージを入力")
                                        .font(egui::TextStyle::Body)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(2)
                                        .margin(egui::Vec2::ZERO)
                                        .frame(false),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::TextEdit,
                                            true,
                                            "メッセージ",
                                        )
                                    });
                                    if response.changed() {
                                        self.sessions_dirty = true;
                                    }
                                });
                        },
                    );
                    if self.pending.is_some() {
                        if icon_button(ui, Icon::Stop, "生成を停止", true, true).clicked() {
                            self.stop_generation();
                        }
                    } else if icon_button(ui, Icon::Send, "送信", enabled, true).clicked() {
                        self.send(ui.ctx());
                    }
                });
            });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let notice = if self.pending.is_some() {
                if self
                    .request_cancel
                    .as_ref()
                    .is_some_and(|c| c.load(Ordering::Relaxed))
                {
                    "生成を停止しています…"
                } else {
                    "生成中…"
                }
                .to_string()
            } else if self.backend == Backend::Rules {
                "ローカル診断。入力内容は外部に送信されません。".into()
            } else if self.backend == Backend::Local {
                "ローカルAI。このPC上で推論します。".into()
            } else {
                format!(
                    "{}へ{}を送信します。",
                    self.backend.label(),
                    if self.sessions[self.active].history_enabled {
                        "入力と会話履歴"
                    } else {
                        "今回の入力だけ"
                    }
                )
            };
            ui.label(RichText::new(notice).size(12.0).color(MUTED));
            if ui.available_width() > 120.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(if cfg!(target_os = "macos") {
                            "⌘ Enter"
                        } else {
                            "Ctrl + Enter"
                        })
                        .size(11.0)
                        .color(MUTED),
                    );
                });
            }
        });
        if self.backend != Backend::Rules {
            let destination = self.destination_key();
            ui.horizontal(|ui| {
                let session = &mut self.sessions[self.active];
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        egui::Checkbox::new(&mut session.history_enabled, "会話履歴を含める"),
                    )
                    .changed()
                    && session.history_enabled
                {
                    session.context_key = Some(destination);
                    session.notice = None;
                }
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        egui::Button::new("送信用の履歴を消去"),
                    )
                    .clicked()
                {
                    session.history.clear();
                    session.notice = Some(
                        "送信用の履歴を消去しました。画面の会話と下書きは保持しています。".into(),
                    );
                }
                ui.label(
                    RichText::new(format!(
                        "{}文字",
                        session
                            .history
                            .iter()
                            .map(|m| m.content.chars().count())
                            .sum::<usize>()
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
            });
        }
        if let Some(notice) = &self.sessions[self.active].notice {
            ui.label(RichText::new(notice).size(12.0).color(MUTED));
        }
    }
    fn conversation(&mut self, ui: &mut egui::Ui) {
        let mut reference = None;
        let mut retry = false;
        let mut proposal_open = None;
        egui::ScrollArea::vertical()
            .id_salt(("chat", self.active))
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (index, (user, message)) in
                    self.sessions[self.active].messages.iter().enumerate()
                {
                    if *user {
                        ui.add_space(18.0);
                        let width = (ui.available_width() * 0.88).min(560.0);
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - width - 10.0).max(0.0));
                            egui::Frame::new()
                                .fill(SELECTED)
                                .corner_radius(12.0)
                                .inner_margin(16.0)
                                .show(ui, |ui| {
                                    ui.set_width((width - 32.0).max(50.0));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(message).line_height(Some(22.0)),
                                        )
                                        .wrap()
                                        .selectable(true),
                                    );
                                });
                        });
                    } else {
                        ui.add_space(30.0);
                        ui.label(
                            RichText::new("CodeBot")
                                .size(14.0)
                                .family(egui::FontFamily::Name("ui-bold".into())),
                        );
                        ui.add_space(10.0);
                        // The provider's mode is already visible beneath the composer.
                        let body = message
                            .strip_prefix("ルールベース診断（AI未使用）\n")
                            .unwrap_or(message);
                        if let Some(uri) =
                            markdown::show(ui, &mut self.markdown_cache, (self.active, index), body)
                        {
                            reference = Some(uri);
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if icon_button(ui, Icon::Copy, "回答をコピー", true, false).clicked()
                            {
                                ui.ctx().copy_text(message.clone());
                                self.copied = Some((self.active, index));
                            }
                            if self.copied == Some((self.active, index)) {
                                ui.label(RichText::new("コピーしました").size(12.0).color(MUTED));
                            }
                            if body.contains("\"changes\"")
                                && body.contains("\"version\"")
                                && ui.button("変更案を確認").clicked()
                            {
                                proposal_open = Some(body.to_owned());
                            }
                            if index + 1 == self.sessions[self.active].messages.len()
                                && self.sessions[self.active].retry_input.is_some()
                                && ui
                                    .add_enabled(
                                        self.pending.is_none(),
                                        egui::Button::new("再試行"),
                                    )
                                    .clicked()
                            {
                                retry = true;
                            }
                        });
                    }
                    ui.add_space(14.0);
                }
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|(id, _)| *id == self.active)
                {
                    ui.add_space(20.0);
                    ui.label(RichText::new("生成中…").color(MUTED));
                }
            });
        if let Some(uri) = reference {
            if let Some(root) = &self.sessions[self.active].workspace_root {
                self.workspace_panel.open_location(root, &uri);
                self.files_open = true;
            } else {
                self.sessions[self.active].notice =
                    Some("参照リンクに対応する作業フォルダが選択されていません".into());
            }
        }
        if retry {
            let session = &mut self.sessions[self.active];
            if session.draft.trim().is_empty() && session.attachments.is_empty() {
                session.draft = session.retry_input.clone().unwrap_or_default();
                self.send(ui.ctx());
            } else {
                session.notice = Some(
                    "未送信の下書きがあります。送信または退避してから再試行してください。".into(),
                );
            }
        }
        if let Some(proposal) = proposal_open {
            match self.change_panel.set_proposal(
                self.sessions[self.active].workspace_root.as_deref(),
                &proposal,
            ) {
                Ok(()) => {
                    self.change_panel.preview(ui.ctx());
                    self.changes_open = true;
                    self.files_open = false;
                    self.settings = false;
                }
                Err(error) => self.sessions[self.active].notice = Some(error),
            }
        }
    }
    fn processing_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("処理方式")
                .size(14.0)
                .family(egui::FontFamily::Name("ui-bold".into())),
        );
        let previous = self.backend;
        ui.add_enabled_ui(self.pending.is_none(), |ui| {
            egui::ComboBox::from_id_salt("backend")
                .selected_text(self.backend.label())
                .height(400.0)
                .width(260.0)
                .show_ui(ui, |ui| {
                    for backend in Backend::ALL {
                        ui.selectable_value(&mut self.backend, backend, backend.label());
                    }
                });
        });
        if self.backend != previous {
            self.api_diagnostics.cancel();
            self.api_key.clear();
            if previous == Backend::Local {
                self.local.stop();
            }
            if !matches!(self.backend, Backend::Rules | Backend::Local) {
                self.api_configs
                    .entry(self.backend)
                    .or_insert_with(|| self.backend.defaults());
            }
            self.save_preferences();
        }
        if self.backend == Backend::Local {
            if self.local.ui(ui, self.pending.is_some()) {
                self.save_preferences();
            }
            if let Some(error) = &self.preferences_error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            return;
        }
        if self.backend == Backend::Rules {
            ui.label(
                RichText::new("APIキー不要。入力は外部に送信されません。")
                    .size(13.0)
                    .color(MUTED),
            );
            return;
        }
        let api = self
            .api_configs
            .entry(self.backend)
            .or_insert_with(|| self.backend.defaults());
        ui.add_space(12.0);
        let model = ui.label("モデルID（必須）");
        ui.add(
            egui::TextEdit::singleline(&mut api.model)
                .desired_width(f32::INFINITY)
                .hint_text("契約先で利用可能なモデルID"),
        )
        .labelled_by(model.id);
        if self.backend == Backend::Custom {
            ui.add_space(8.0);
            egui::ComboBox::from_id_salt("protocol")
                .selected_text(api.protocol.label())
                .width(260.0)
                .show_ui(ui, |ui| {
                    for protocol in [
                        ApiProtocol::Openai,
                        ApiProtocol::Gemini,
                        ApiProtocol::Claude,
                    ] {
                        ui.selectable_value(&mut api.protocol, protocol, protocol.label());
                    }
                });
        }
        ui.add_space(8.0);
        let endpoint = ui.label("APIベースURL（地域・契約に合わせて変更可能）");
        let edited = ui
            .add(egui::TextEdit::singleline(&mut api.base_url).desired_width(f32::INFINITY))
            .labelled_by(endpoint.id);
        if edited.changed() {
            self.api_key.clear();
        }
        ui.label(
            RichText::new(match api.protocol {
                ApiProtocol::Openai => "末尾に /chat/completions を自動で追加します。",
                ApiProtocol::Claude => "末尾に /messages を自動で追加します。",
                ApiProtocol::Gemini => {
                    "末尾に /models/{モデルID}:generateContent を自動で追加します。"
                }
            })
            .size(12.0)
            .color(MUTED),
        );
        ui.add_space(8.0);
        let key = ui.label("APIキー（保存されません）");
        ui.add(
            egui::TextEdit::singleline(&mut self.api_key)
                .password(true)
                .desired_width(f32::INFINITY)
                .hint_text("この起動中だけ使用。未入力なら環境変数を参照"),
        )
        .labelled_by(key.id);
        let env = ui.label("APIキーの環境変数名");
        ui.add(egui::TextEdit::singleline(&mut api.api_key_env).desired_width(f32::INFINITY))
            .labelled_by(env.id);
        if self.backend == Backend::Custom {
            ui.checkbox(&mut api.anonymous, "キーなし接続を許可（ローカルAPIなど）");
        }
        ui.horizontal(|ui| {
            ui.label("出力トークン上限");
            ui.add(egui::DragValue::new(&mut api.max_output_tokens).range(1..=128_000));
            ui.label("タイムアウト（秒）");
            ui.add(egui::DragValue::new(&mut api.timeout_seconds).range(1..=300));
        });
        ui.add_space(8.0);
        ui.label(RichText::new("送信時は今回の入力と選択した添付を上記URLに送ります。履歴送信を有効にした場合は過去の会話も含みます。利用料金は各サービスに従います。").size(12.0).color(MUTED));
        self.api_diagnostics
            .ui(ui, self.backend, api, &self.api_key, self.pending.is_none());
        if ui.button("API設定を保存").clicked() {
            self.save_preferences();
        }
        if let Some(error) = &self.preferences_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }
    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(32.0);
        ui.label(RichText::new("設定").size(26.0));
        ui.add_space(28.0);
        ui.collapsing("チャットの保存と管理", |ui| {
            self.chat_settings(ui)
        });
        ui.add_space(16.0);
        self.processing_settings(ui);
        ui.add_space(28.0);
        ui.label(
            RichText::new("フォント")
                .size(14.0)
                .family(egui::FontFamily::Name("ui-bold".into())),
        );
        let previous = self.font;
        ui.radio_value(
            &mut self.font,
            FontChoice::LineSeedJp,
            "LINE Seed JP — Regular 400（標準）",
        );
        ui.radio_value(&mut self.font, FontChoice::BizUdpGothic, "BIZ UDPゴシック");
        if self.font != previous {
            self.save_font_choice(ui.ctx());
        }
        if let Some(error) = &self.preferences_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        } else {
            ui.label(
                RichText::new("すぐに反映され、次回起動にも引き継がれます。")
                    .size(13.0)
                    .color(MUTED),
            );
        }
        ui.add_space(28.0);
        let label = ui.label(
            RichText::new("設定ファイル")
                .size(14.0)
                .family(egui::FontFamily::Name("ui-bold".into())),
        );
        ui.label(
            RichText::new("使用するTOMLファイルのパスを指定します。")
                .size(13.0)
                .color(MUTED),
        );
        ui.add_space(8.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.config_path)
                .desired_width(f32::INFINITY)
                .hint_text("OS標準の設定を使用")
                .margin(egui::vec2(12.0, 10.0)),
        )
        .labelled_by(label.id);
        ui.add_space(8.0);
        ui.label(RichText::new("空欄の場合はOS標準の設定、または既定値を使います。\n変更は次の送信から適用されます。").size(13.0).color(MUTED));
        ui.add_space(28.0);
        if ui.button("チャットに戻る").clicked() {
            self.settings = false;
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        self.github_panel.poll();
        for root in [
            self.change_panel.poll(),
            self.command_panel.poll(),
            self.git_panel.poll(),
        ]
        .into_iter()
        .flatten()
        {
            for session in &mut self.sessions {
                if session.workspace_root.as_deref() == Some(&root) {
                    session.history.clear();
                    session.attachments.clear();
                    session.analysis_action = None;
                    session.history_enabled = false;
                    session.retry_input = None;
                    session.notice = Some("ファイル操作・コマンドを実行しました。参照ファイルを選び直してください。画面の会話と下書きは保持しています。".into());
                }
            }
            self.sessions_dirty = true;
        }
        if self.local.poll() {
            self.save_preferences();
        }
        if self.settings
            && !ctx.memory(|m| m.any_popup_open())
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.settings = false;
        }
        // Consume the shortcut before TextEdit to avoid inserting a newline on submit.
        if !self.settings
            && !self.files_open
            && !self.changes_open
            && !self.commands_open
            && !self.git_open
            && !self.github_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
        {
            self.send(ctx);
        }
        let narrow = ctx.screen_rect().width() < 900.0;
        if self.sidebar && !narrow {
            egui::SidePanel::left("sidebar")
                .exact_width(224.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(SHELL).inner_margin(16.0))
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("CodeBot")
                                .size(20.0)
                                .family(egui::FontFamily::Name("ui-bold".into())),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, Icon::Panel, "サイドバーを閉じる", true, false)
                                .clicked()
                            {
                                self.sidebar = false;
                            }
                        });
                    });
                    ui.add_space(26.0);
                    if new_chat_button(ui).clicked() {
                        self.new_chat();
                    }
                    ui.add_space(30.0);
                    ui.label(RichText::new("チャット").size(12.0).color(MUTED));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("チャットを検索")
                            .desired_width(192.0),
                    );
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .max_height((ui.available_height() - 106.0).max(40.0))
                        .show(ui, |ui| {
                            for (index, session) in self.sessions.iter().enumerate() {
                                if !session.matches(&self.search) {
                                    continue;
                                }
                                let response = ui.add(
                                    egui::Button::new(RichText::new(session.title()).size(13.0))
                                        .truncate()
                                        .min_size(egui::vec2(192.0, 36.0))
                                        .shortcut_text("")
                                        .fill(if index == self.active && !self.settings {
                                            SELECTED
                                        } else {
                                            Color32::TRANSPARENT
                                        }),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::SelectableLabel,
                                        true,
                                        index == self.active,
                                        session.title(),
                                    )
                                });
                                if response.clicked() {
                                    self.active = index;
                                    self.settings = false;
                                    self.files_open = false;
                                    self.changes_open = false;
                                    self.commands_open = false;
                                    self.git_open = false;
                                    self.github_open = false;
                                    self.delete_confirm = false;
                                    self.sessions_dirty = true;
                                }
                            }
                        });
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.label(
                            RichText::new(if self.save_chats {
                                "チャットをこのPCに保存します"
                            } else {
                                "履歴は終了時に消去されます"
                            })
                            .size(11.0)
                            .color(MUTED),
                        );
                        ui.add_space(10.0);
                        if ui
                            .add(
                                egui::Button::new("設定")
                                    .min_size(egui::vec2(192.0, 36.0))
                                    .shortcut_text("")
                                    .fill(if self.settings {
                                        SELECTED
                                    } else {
                                        Color32::TRANSPARENT
                                    }),
                            )
                            .clicked()
                        {
                            self.settings = true;
                            self.files_open = false;
                            self.changes_open = false;
                            self.commands_open = false;
                            self.git_open = false;
                            self.github_open = false;
                        }
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(SHELL).inner_margin(8.0))
            .show(ctx, |ui| {
                let area = ui.max_rect();
                ui.painter().rect_filled(area, 12.0, SURFACE);
                ui.scope_builder(egui::UiBuilder::new().max_rect(area.shrink(20.0)), |ui| {
                    ui.horizontal(|ui| {
                        if !self.sidebar
                            && !narrow
                            && icon_button(ui, Icon::Panel, "サイドバーを開く", true, false)
                                .clicked()
                        {
                            self.sidebar = true;
                        }
                        if narrow {
                            ui.menu_button("チャット", |ui| {
                                if ui.button("新しいチャット").clicked() {
                                    self.new_chat();
                                    ui.close_menu();
                                }
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.search)
                                        .hint_text("チャットを検索"),
                                );
                                for (i, s) in self.sessions.iter().enumerate() {
                                    if !s.matches(&self.search) {
                                        continue;
                                    }
                                    if ui.selectable_label(i == self.active, s.title()).clicked() {
                                        self.active = i;
                                        self.settings = false;
                                        self.files_open = false;
                                        self.changes_open = false; self.commands_open = false; self.git_open=false; self.github_open=false;
                                        self.delete_confirm = false;
                                        self.sessions_dirty = true;
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                        let title = if self.settings { "設定".into() } else if self.github_open { "GitHub".into() } else if self.git_open { "Git".into() } else if self.commands_open { "コマンドの実行".into() } else if self.changes_open { "変更の確認".into() }
                            else if self.files_open { "参照ファイル".into() } else { self.sessions[self.active].title() };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, Icon::Settings, "設定", true, false).clicked() {
                                self.files_open = false;
                                self.changes_open = false; self.commands_open = false; self.git_open=false; self.github_open=false;
                                self.settings = !self.settings;
                            }
                            if icon_button(ui, Icon::New, "新しいチャット", true, false).clicked()
                            {
                                self.new_chat();
                            }
                            if ui.button("ファイル").clicked() { self.files_open = !self.files_open; self.settings = false; self.changes_open = false; self.commands_open = false; self.git_open=false; self.github_open=false; }
                            if ui.button("変更").clicked() { self.changes_open = !self.changes_open; self.settings = false; self.files_open = false; self.commands_open=false; self.git_open=false; self.github_open=false; }
                            if ui.button("実行").clicked() { self.commands_open = !self.commands_open; self.settings=false; self.files_open=false; self.changes_open=false; self.git_open=false; self.github_open=false; self.command_panel.set_initial_root(self.sessions[self.active].workspace_root.as_deref()); }
                            if ui.button("Git").clicked() { self.git_open=!self.git_open; self.github_open=false; self.settings=false; self.files_open=false; self.changes_open=false; self.commands_open=false; self.git_panel.set_initial_root(self.sessions[self.active].workspace_root.as_deref()); }
                            if ui.button("GitHub").clicked() { self.github_open=!self.github_open; self.git_open=false; self.settings=false; self.files_open=false; self.changes_open=false; self.commands_open=false; }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(title).size(13.0).color(MUTED)).truncate());
                            });
                        });
                    });
                    let bounds = ui.available_rect_before_wrap();
                    let width = (bounds.width() - 32.0).min(MEASURE).max(200.0);
                    let left = bounds.center().x - width / 2.0;
                    let composer_width = (bounds.width() - 32.0).min(COMPOSER_WIDTH).max(200.0);
                    let composer_left = bounds.center().x - composer_width / 2.0;
                    if self.github_open {
                        let rect=egui::Rect::from_min_max(egui::pos2(left,bounds.top()),egui::pos2(left+width,bounds.bottom()));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect),|ui| {
                            egui::ScrollArea::vertical().show(ui,|ui| {
                                if let Some(action)=self.github_panel.ui(ui,self.pending.is_none() && !self.change_panel.is_busy() && !self.command_panel.is_busy() && !self.git_panel.is_busy()) {
                                    match action {
                                        github_panel::Action::Save=>self.save_preferences(),
                                        github_panel::Action::Attach(text)=>{
                                            let mut limited=text.chars().take(12000).collect::<String>();if text.chars().count()>12000 {limited.push_str("\n（12000文字以降を省略）");}
                                            let session=&mut self.sessions[self.active];if !session.draft.is_empty() {session.draft.push_str("\n\n");}session.draft.push_str(&limited);session.history_enabled=false;session.notice=Some("GitHubの資料を下書きに追加しました。内容を確認して送信してください。".into());self.sessions_dirty=true;self.github_open=false;
                                        }
                                        github_panel::Action::Branch{branch,source}=>{
                                            match self.git_panel.prepare_issue_branch(ui.ctx(),self.sessions[self.active].workspace_root.as_deref(),&branch,&source) {
                                                Ok(())=>{self.git_open=true;self.github_open=false;},Err(error)=>self.github_panel.report_message(error)
                                            }
                                        }
                                    }
                                }
                                if let Some(error)=&self.preferences_error {ui.colored_label(ui.visuals().error_fg_color,error);}
                                if ui.button("チャットに戻る").clicked() {self.github_open=false;}
                            });
                        });
                    } else if self.git_open {
                        let rect=egui::Rect::from_min_max(egui::pos2(left,bounds.top()),egui::pos2(left+width,bounds.bottom()));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect),|ui| {
                            egui::ScrollArea::vertical().show(ui,|ui| {
                                self.git_panel.ui(ui,self.pending.is_none() && !self.change_panel.is_busy() && !self.command_panel.is_busy() && !self.github_panel.is_busy());
                                if ui.button("チャットに戻る").clicked() {self.git_open=false; self.github_open=false;}
                            });
                        });
                    } else if self.commands_open {
                        let rect=egui::Rect::from_min_max(egui::pos2(left,bounds.top()),egui::pos2(left+width,bounds.bottom()));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect),|ui| {
                            egui::ScrollArea::vertical().show(ui,|ui| {
                                if let Some(output)=self.command_panel.ui(ui,self.pending.is_none() && !self.change_panel.is_busy() && !self.git_panel.is_busy() && !self.github_panel.is_busy()) {
                                    let session=&mut self.sessions[self.active];
                                    if !session.draft.is_empty() {session.draft.push_str("\n\n");}
                                    session.draft.push_str(&output);session.history_enabled=false;self.sessions_dirty=true;self.commands_open=false; self.git_open=false; self.github_open=false;
                                }
                                if ui.button("チャットに戻る").clicked() {self.commands_open=false; self.git_open=false; self.github_open=false;}
                            });
                        });
                    } else if self.changes_open {
                        let rect = egui::Rect::from_min_max(egui::pos2(left,bounds.top()),egui::pos2(left+width,bounds.bottom()));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                self.change_panel.ui(ui,self.pending.is_none() && !self.command_panel.is_busy() && !self.git_panel.is_busy() && !self.github_panel.is_busy());
                                if let Some(error) = &self.preferences_error { ui.colored_label(ui.visuals().error_fg_color,error); }
                                if ui.button("チャットに戻る").clicked() { self.changes_open = false; self.commands_open = false; self.git_open=false; self.github_open=false; }
                            });
                        });
                    } else if self.files_open {
                        let rect = egui::Rect::from_min_max(egui::pos2(left, bounds.top()), egui::pos2(left+width, bounds.bottom()));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                if let Some((attachment, action)) = self.workspace_panel.ui(ui) {
                                    let session = &mut self.sessions[self.active];
                                    session.history_enabled = false;
                                    if session.workspace_root.as_deref().is_some_and(|root| root != attachment.root) {
                                        session.attachments.clear(); session.history.clear();
                                        session.notice = Some("作業フォルダを変更したため、送信用の履歴と以前の添付を消去しました。".into());
                                    }
                                    session.workspace_root = Some(attachment.root.clone());
                                    if session.attachments.len() < 8 {
                                        if session.attachments.iter().any(|a| a.root != attachment.root) { session.attachments.clear(); session.history.clear(); }
                                        session.attachments.push(attachment);
                                        session.analysis_action = Some(action);
                                        if session.draft.trim().is_empty() { session.draft = format!("選択した範囲の{}をお願いします。", action.label()); }
                                        self.sessions_dirty = true; self.files_open = false;
                                    } else { session.notice = Some("添付は8ファイルまでです".into()); }
                                }
                                if ui.button("チャットに戻る").clicked() { self.files_open = false; }
                            });
                        });
                    } else if self.settings {
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(left, bounds.top()),
                            egui::pos2(left + width, bounds.bottom()),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| self.settings_page(ui));
                        });
                    } else if self.sessions[self.active].messages.is_empty() {
                        let top = bounds.top() + (bounds.height() * 0.27).max(12.0);
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(composer_left, top),
                            egui::pos2(composer_left + composer_width, bounds.bottom()),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.label(RichText::new("新しいチャット").size(28.0));
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("コードやログを一緒に確認しましょう。")
                                    .size(14.0)
                                    .color(MUTED),
                            );
                            ui.add_space(28.0);
                            self.composer(ui);
                        });
                    } else {
                        let extra = if self.backend == Backend::Rules {
                            0.0
                        } else {
                            40.0
                        } + if self.sessions[self.active].notice.is_some() {
                            80.0
                        } else {
                            0.0
                        };
                        let extra = extra + self.sessions[self.active].attachments.len() as f32 * 26.0;
                        let composer_top =
                            (bounds.bottom() - 108.0 - extra).max(bounds.top() + 80.0);
                        let history = egui::Rect::from_min_max(
                            egui::pos2(left, bounds.top() + 12.0),
                            egui::pos2(left + width, composer_top - 18.0),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(history), |ui| {
                            self.conversation(ui)
                        });
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(composer_left, composer_top),
                            egui::pos2(composer_left + composer_width, bounds.bottom()),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            self.composer(ui)
                        });
                    }
                });
            });
        if self.change_panel.take_settings_changed() {
            self.save_preferences();
        }
        self.flush_chats();
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Send,
    Stop,
    Copy,
    New,
    Panel,
    Settings,
}

fn new_chat_button(ui: &mut egui::Ui) -> egui::Response {
    let mut label = egui::text::LayoutJob::simple_singleline(
        "新しいチャット".into(),
        egui::FontId::proportional(13.0),
        TEXT,
    );
    // Reserve room for the drawn icon while retaining the native button label.
    label.sections[0].leading_space = 24.0;
    let response = ui.add(
        egui::Button::new(label)
            .min_size(egui::vec2(192.0, 38.0))
            .shortcut_text("")
            .fill(RAISED),
    );
    let center = egui::pos2(response.rect.left() + 18.0, response.rect.center().y);
    let stroke = egui::Stroke::new(1.5_f32, MUTED);
    for offset in [egui::vec2(5.0, 0.0), egui::vec2(0.0, 5.0)] {
        ui.painter()
            .line_segment([center - offset, center + offset], stroke);
    }
    response
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    name: &str,
    enabled: bool,
    primary: bool,
) -> egui::Response {
    let size = if primary { 36.0 } else { 30.0 };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(egui::vec2(size, size))
            .fill(if primary {
                if enabled {
                    ACCENT
                } else {
                    DISABLED
                }
            } else {
                Color32::TRANSPARENT
            })
            .corner_radius(9.0),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    let p = ui.painter();
    let c = response.rect.center();
    if primary && enabled && (response.hovered() || response.is_pointer_button_down_on()) {
        p.rect_filled(response.rect, 9.0, ACCENT_HOVER);
    }
    let color = if primary && enabled {
        Color32::WHITE
    } else {
        MUTED
    };
    let stroke = egui::Stroke::new(1.5_f32, color);
    let line = |a: [f32; 2], b: [f32; 2]| {
        p.line_segment(
            [c + egui::vec2(a[0], a[1]), c + egui::vec2(b[0], b[1])],
            stroke,
        );
    };
    match icon {
        Icon::Stop => {
            p.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(11.0, 11.0)),
                2.0,
                color,
            );
        }
        Icon::Send => {
            line([0.0, 7.0], [0.0, -7.0]);
            line([-5.0, -2.0], [0.0, -7.0]);
            line([0.0, -7.0], [5.0, -2.0]);
        }
        Icon::New => {
            line([-6.0, 0.0], [6.0, 0.0]);
            line([0.0, -6.0], [0.0, 6.0]);
        }
        Icon::Panel => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(16.0, 14.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            line([-2.0, -6.0], [-2.0, 6.0]);
        }
        Icon::Copy => {
            p.rect_stroke(
                egui::Rect::from_center_size(c + egui::vec2(2.0, 2.0), egui::vec2(10.0, 12.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            line([-2.0, -7.0], [-7.0, -7.0]);
            line([-7.0, -7.0], [-7.0, 3.0]);
        }
        Icon::Settings => {
            for (x, y) in [(-5.0, -3.0), (0.0, 4.0), (5.0, -1.0)] {
                line([x, -7.0], [x, 7.0]);
                p.circle_filled(c + egui::vec2(x, y), 2.7, SURFACE);
                p.circle_stroke(c + egui::vec2(x, y), 2.2, stroke);
            }
        }
    }
    if response.has_focus() {
        p.rect_stroke(
            response.rect,
            9.0,
            egui::Stroke::new(1.5_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
        .on_hover_text(name)
        .on_disabled_hover_text(if primary {
            "入力後、診断が完了していれば送信できます"
        } else {
            name
        })
}
fn main() -> eframe::Result {
    eframe::run_native(
        "CodeBot",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 800.0])
                .with_min_inner_size([700.0, 540.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let app = App::load(&cc.egui_ctx);
            cc.egui_ctx.set_theme(egui::Theme::Light);
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::light();
            style.visuals.override_text_color = Some(TEXT);
            style.visuals.panel_fill = SHELL;
            style.visuals.extreme_bg_color = RAISED;
            style.visuals.window_fill = SURFACE;
            style.visuals.selection.bg_fill = SELECTED;
            style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, TEXT);
            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
            style.visuals.widgets.inactive.fg_stroke.color = ACCENT;
            style.visuals.widgets.hovered.fg_stroke.color = ACCENT;
            style.visuals.widgets.active.fg_stroke.color = ACCENT;
            style.visuals.widgets.inactive.weak_bg_fill = RAISED;
            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            style.visuals.widgets.hovered.weak_bg_fill = SELECTED;
            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
            style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
            for widget in [
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
            ] {
                widget.corner_radius = egui::CornerRadius::same(7);
            }
            style.spacing.button_padding = egui::vec2(12.0, 7.0);
            style.spacing.item_spacing = egui::vec2(10.0, 6.0);
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(14.0));
            cc.egui_ctx.set_style(style);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restoring_an_interrupted_generation_marks_partial_text_and_keeps_it_out_of_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::default();
        app.preferences_path = Some(dir.path().join("gui-preferences.toml"));
        app.save_chats = true;
        app.sessions[0].generation_incomplete = true;
        app.sessions[0].messages = vec![(true, "question".into()), (false, "partial".into())];
        app.sessions_dirty = true;
        app.flush_chats();
        app.restore_chats(false);
        assert!(!app.sessions[0].generation_incomplete);
        assert!(app.sessions[0].messages[1]
            .1
            .contains("生成中にアプリが終了"));
        assert!(app.sessions[0].history.is_empty());
        assert!(!app.sessions[0].history_enabled);
    }
    #[test]
    fn stopping_keeps_partial_text_ignores_late_deltas_and_unblocks_the_next_request() {
        let mut app = App::default();
        app.sessions[0].in_flight_input = Some("request".into());
        let (tx, rx) = mpsc::channel();
        app.pending = Some((0, rx));
        app.request_cancel = Some(Arc::new(AtomicBool::new(false)));
        tx.send(StreamEvent::Delta("途中の回答".into())).unwrap();
        app.poll();
        app.new_chat();
        app.sessions[1].draft = "別の下書き".into();
        app.stop_generation();
        tx.send(StreamEvent::Delta("遅延イベント".into())).unwrap();
        tx.send(StreamEvent::Finished(StreamEnd::Complete)).unwrap();
        app.poll();
        assert!(app.pending.is_none());
        assert!(app.sessions[0].history.is_empty());
        assert!(app.sessions[0].messages[0].1.contains("途中の回答"));
        assert!(app.sessions[0].messages[0].1.contains("生成を停止"));
        assert!(!app.sessions[0].messages[0].1.contains("遅延イベント"));
        assert_eq!(app.sessions[1].draft, "別の下書き");
        app.sessions[1].in_flight_input = Some("next request".into());
        let (next_tx, rx) = mpsc::channel();
        app.pending = Some((1, rx));
        next_tx.send(StreamEvent::Delta("次の回答".into())).unwrap();
        next_tx
            .send(StreamEvent::Finished(StreamEnd::Complete))
            .unwrap();
        app.poll();
        assert_eq!(app.sessions[1].history.len(), 2);
    }
    #[test]
    fn archive_reload_preserves_corrupt_files_and_does_not_duplicate_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::default();
        app.preferences_path = Some(dir.path().join("gui-preferences.toml"));
        app.save_chats = true;
        app.sessions[0].draft = "日本語の下書き".into();
        app.sessions_dirty = true;
        app.flush_chats();
        assert!(app.storage_error.is_none());
        app.restore_chats(true);
        app.restore_chats(true);
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].draft, "日本語の下書き");
        let path = app.archive_path().unwrap();
        std::fs::write(&path, "broken data").unwrap();
        app.restore_chats(true);
        assert!(app.archive_blocked);
        app.sessions_dirty = true;
        app.flush_chats();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken data");
    }
    #[test]
    fn delete_and_search_keep_a_valid_active_chat() {
        let mut app = App::default();
        app.sessions[0].label = "Rustの調査".into();
        assert!(app.sessions[0].matches("RUST"));
        assert!(!app.sessions[0].matches("other"));
        app.new_chat();
        app.delete_active_chat();
        assert_eq!(app.active, 0);
        assert_eq!(app.sessions[0].title(), "Rustの調査");
        app.delete_active_chat();
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.active, 0);
        assert!(app.sessions[0].messages.is_empty());
    }
    #[test]
    fn history_is_opt_in_and_changed_destinations_require_a_new_opt_in() {
        let mut session = Session::default();
        session.history = vec![
            ChatMessage::user("old question"),
            ChatMessage::assistant("old answer"),
        ];
        session.context_key = Some("provider A".into());
        session.draft = "new question".into();
        assert_eq!(
            session
                .request_messages(&Config::default(), "provider A", false)
                .unwrap()
                .len(),
            1
        );
        session.history_enabled = true;
        assert_eq!(
            session
                .request_messages(&Config::default(), "provider A", false)
                .unwrap()
                .len(),
            3
        );
        assert!(session
            .request_messages(&Config::default(), "provider B", false)
            .is_err());
        assert!(!session.history_enabled);
        assert_eq!(session.draft, "new question");
        assert_eq!(session.history.len(), 2);
        session.context_key = Some("provider B".into());
        session.history_enabled = true;
        assert_eq!(
            session
                .request_messages(&Config::default(), "provider B", false)
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn failed_answers_never_enter_the_sendable_history() {
        let mut app = App::default();
        app.sessions[0].in_flight_input = Some("failed request".into());
        let (tx, rx) = mpsc::channel();
        app.pending = Some((0, rx));
        tx.send(StreamEvent::Finished(StreamEnd::Failed("HTTP 503".into())))
            .unwrap();
        app.poll();
        assert!(app.sessions[0].history.is_empty());
        app.sessions[0].in_flight_input = Some("good request".into());
        let (tx, rx) = mpsc::channel();
        app.pending = Some((0, rx));
        tx.send(StreamEvent::Delta("answer".into())).unwrap();
        tx.send(StreamEvent::Finished(StreamEnd::Complete)).unwrap();
        app.new_chat();
        app.poll();
        assert_eq!(
            app.sessions[0].history,
            vec![
                ChatMessage::user("good request"),
                ChatMessage::assistant("answer")
            ]
        );
        assert!(app.sessions[1].history.is_empty());
    }

    #[test]
    fn new_chat_preserves_existing_drafts_and_returns_from_settings() {
        let mut app = App::default();
        app.sessions[0].draft = "未送信の調査内容".into();
        app.settings = true;

        app.new_chat();

        assert_eq!(app.active, 1);
        assert!(!app.settings);
        assert_eq!(app.sessions[0].draft, "未送信の調査内容");
        assert!(app.sessions[1].draft.is_empty());
        assert!(app.sessions[1].messages.is_empty());
    }

    #[test]
    fn response_goes_to_its_origin_after_switching_chats() {
        let mut app = App::default();
        app.sessions[0]
            .messages
            .push((true, "first request".into()));
        let (tx, rx) = mpsc::channel();
        app.pending = Some((0, rx));
        app.new_chat();
        app.sessions[1].draft = "second request draft".into();
        tx.send(StreamEvent::Delta("first response".into()))
            .unwrap();
        tx.send(StreamEvent::Finished(StreamEnd::Complete)).unwrap();

        app.poll();

        assert_eq!(app.active, 1);
        assert_eq!(
            app.sessions[0].messages[1],
            (false, "first response".into())
        );
        assert!(app.sessions[1].messages.is_empty());
        assert_eq!(app.sessions[1].draft, "second request draft");
        assert!(app.pending.is_none());
        app.poll();
        assert_eq!(app.sessions[0].messages.len(), 2);
    }

    #[test]
    fn disconnected_worker_adds_one_error_and_unblocks_sending() {
        let mut app = App::default();
        let (tx, rx) = mpsc::channel();
        app.pending = Some((0, rx));
        drop(tx);

        app.poll();

        assert!(app.pending.is_none());
        assert_eq!(app.sessions[0].messages.len(), 1);
        assert!(!app.sessions[0].messages[0].0);
        assert!(app.sessions[0].messages[0]
            .1
            .contains("処理が中断されました"));
        app.poll();
        assert_eq!(app.sessions[0].messages.len(), 1);
    }

    #[test]
    fn empty_or_busy_submission_does_not_erase_a_draft() {
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.sessions[0].draft = " \n\t".into();
        app.send(&ctx);
        assert_eq!(app.sessions[0].draft, " \n\t");
        assert!(app.pending.is_none());
        assert!(app.sessions[0].messages.is_empty());

        let (_tx, rx) = mpsc::channel();
        app.pending = Some((0, rx));
        app.new_chat();
        app.sessions[1].draft = "keep this input".into();
        app.send(&ctx);
        assert_eq!(app.sessions[1].draft, "keep this input");
        assert!(app.sessions[1].messages.is_empty());
        assert_eq!(app.pending.as_ref().unwrap().0, 0);
    }
    #[test]
    fn local_send_without_a_loaded_model_keeps_the_draft() {
        let mut app = App::default();
        app.backend = Backend::Local;
        app.sessions[0].draft = "調査内容".into();
        app.send(&egui::Context::default());
        assert!(app.settings);
        assert!(app.pending.is_none());
        assert_eq!(app.sessions[0].draft, "調査内容");
        assert!(app.sessions[0].messages.is_empty());
    }
}
