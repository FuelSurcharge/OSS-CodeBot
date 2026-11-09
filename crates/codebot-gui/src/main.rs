use codebot_core::{execute_backend, ApiConfig, ApiProtocol, Backend, Config};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc,
};
mod local_models;
mod markdown;
mod ui_preferences;
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

#[derive(Default)]
struct Session {
    messages: Vec<(bool, String)>,
    draft: String,
}
impl Session {
    fn title(&self) -> String {
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
}
struct App {
    sessions: Vec<Session>,
    active: usize,
    config_path: String,
    backend: Backend,
    api_configs: BTreeMap<Backend, ApiConfig>,
    api_key: String,
    local: LocalModels,
    font: FontChoice,
    preferences_path: Option<PathBuf>,
    preferences_error: Option<String>,
    settings: bool,
    sidebar: bool,
    copied: Option<(usize, usize)>,
    markdown_cache: egui_commonmark::CommonMarkCache,
    pending: Option<(usize, mpsc::Receiver<Result<String, String>>)>,
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
            local: LocalModels::default(),
            font: FontChoice::default(),
            preferences_path: None,
            preferences_error: None,
            settings: false,
            sidebar: true,
            copied: None,
            markdown_cache: egui_commonmark::CommonMarkCache::default(),
            pending: None,
        }
    }
}
impl App {
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
                }
                Err(e) => app.preferences_error = Some(e),
            }
        }
        apply_font(ctx, app.font);
        app
    }
    fn save_font_choice(&mut self, ctx: &egui::Context) {
        apply_font(ctx, self.font);
        self.save_preferences();
    }
    fn save_preferences(&mut self) {
        self.preferences_error = match &self.preferences_path {
            Some(path) => UiPreferences {
                font: self.font,
                backend: self.backend,
                api_configs: self.api_configs.clone(),
                local: self.local.config.clone(),
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
    }
    fn send(&mut self, ctx: &egui::Context) {
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
        let input = std::mem::take(&mut self.sessions[self.active].draft);
        self.sessions[self.active]
            .messages
            .push((true, input.clone()));
        let path = self.config_path.clone();
        let backend = self.backend;
        let api = self
            .api_configs
            .get(&backend)
            .cloned()
            .unwrap_or_else(|| backend.defaults());
        let api_key = self.api_key.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some((self.active, rx));
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let path = path.trim();
            let result = Config::load(if path.is_empty() {
                None
            } else {
                Some(Path::new(path))
            })
            .and_then(|c| {
                if let Some(connection) = local_connection {
                    connection.respond(&c, &input)
                } else {
                    execute_backend(&c, backend, &api, Some(&api_key), &input)
                }
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        let result = self
            .pending
            .as_ref()
            .and_then(|(id, rx)| match rx.try_recv() {
                Ok(r) => Some((*id, r)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some((*id, Err("処理が中断されました".into())))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some((id, result)) = result {
            let message = match result {
                Ok(s) => {
                    eprintln!("INFO request completed");
                    s
                }
                Err(e) => {
                    eprintln!("ERROR request failed");
                    format!("{e}\n設定と入力を確認して、もう一度送信してください。")
                }
            };
            self.sessions[id].messages.push((false, message));
            self.pending = None;
        }
    }
    fn composer(&mut self, ui: &mut egui::Ui) {
        let enabled = self.pending.is_none() && !self.sessions[self.active].draft.trim().is_empty();
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
                                        .hint_text("エラーログを貼り付けてください")
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
                                });
                        },
                    );
                    if icon_button(ui, Icon::Send, "送信", enabled, true).clicked() {
                        self.send(ui.ctx());
                    }
                });
            });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let notice = if self.pending.is_some() {
                "応答を待っています…".to_string()
            } else if self.backend == Backend::Rules {
                "ローカル診断。入力内容は外部に送信されません。".into()
            } else if self.backend == Backend::Local {
                "ローカルAI。このPC上で推論します。".into()
            } else {
                format!(
                    "{}へ入力を送信します（会話履歴は送信しません）。",
                    self.backend.label()
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
    }
    fn conversation(&mut self, ui: &mut egui::Ui) {
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
                        markdown::show(ui, &mut self.markdown_cache, (self.active, index), body);
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
                    ui.label(RichText::new("ログを確認しています…").color(MUTED));
                }
            });
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
        ui.label(RichText::new("送信ボタンを押すと、今回の入力が上記URLに送信されます。利用料金は各サービスに従います。").size(12.0).color(MUTED));
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
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .max_height((ui.available_height() - 106.0).max(40.0))
                        .show(ui, |ui| {
                            for (index, session) in self.sessions.iter().enumerate() {
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
                                }
                            }
                        });
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.label(
                            RichText::new("履歴は終了時に消去されます")
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
                                for (i, s) in self.sessions.iter().enumerate() {
                                    if ui.selectable_label(i == self.active, s.title()).clicked() {
                                        self.active = i;
                                        self.settings = false;
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                        ui.label(
                            RichText::new(if self.settings {
                                "設定".into()
                            } else {
                                self.sessions[self.active].title()
                            })
                            .size(13.0)
                            .color(MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, Icon::Settings, "設定", true, false).clicked() {
                                self.settings = !self.settings;
                            }
                            if icon_button(ui, Icon::New, "新しいチャット", true, false).clicked()
                            {
                                self.new_chat();
                            }
                        });
                    });
                    let bounds = ui.available_rect_before_wrap();
                    let width = (bounds.width() - 32.0).min(MEASURE).max(200.0);
                    let left = bounds.center().x - width / 2.0;
                    let composer_width = (bounds.width() - 32.0).min(COMPOSER_WIDTH).max(200.0);
                    let composer_left = bounds.center().x - composer_width / 2.0;
                    if self.settings {
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
                                RichText::new("ログを貼り付けて、原因を調べましょう。")
                                    .size(14.0)
                                    .color(MUTED),
                            );
                            ui.add_space(28.0);
                            self.composer(ui);
                        });
                    } else {
                        let composer_top = bounds.bottom() - 108.0;
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
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Send,
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
            cc.egui_ctx.set_style(style);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        tx.send(Ok("first response".into())).unwrap();

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
