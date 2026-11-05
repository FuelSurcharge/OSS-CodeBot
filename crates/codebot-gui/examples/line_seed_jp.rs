// Isolated font preview snapshot. Default src/main.rs is intentionally unchanged.
// Launch: cargo run -p codebot-gui --example line_seed_jp
use codebot_core::{execute, Config, Rules};
use eframe::egui::{self, Color32, RichText};
use std::{path::Path, sync::mpsc};

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
    settings: bool,
    sidebar: bool,
    copied: Option<(usize, usize)>,
    pending: Option<(usize, mpsc::Receiver<Result<String, String>>)>,
}
impl Default for App {
    fn default() -> Self {
        Self {
            sessions: vec![Session::default()],
            active: 0,
            config_path: String::new(),
            settings: false,
            sidebar: true,
            copied: None,
            pending: None,
        }
    }
}
impl App {
    fn new_chat(&mut self) {
        self.sessions.push(Session::default());
        self.active = self.sessions.len() - 1;
        self.settings = false;
    }
    fn send(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.sessions[self.active].draft.trim().is_empty() {
            return;
        }
        let input = std::mem::take(&mut self.sessions[self.active].draft);
        self.sessions[self.active]
            .messages
            .push((true, input.clone()));
        let path = self.config_path.clone();
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
            .and_then(|c| execute(&c, &Rules, &input));
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
            ui.label(
                RichText::new(if self.pending.is_some() {
                    "診断中…"
                } else {
                    "AI未接続。入力内容は外部に送信されません。"
                })
                .size(12.0)
                .color(MUTED),
            );
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
                                .family(egui::FontFamily::Name("ui-regular".into())),
                        );
                        ui.add_space(10.0);
                        // The provider's mode is already visible beneath the composer.
                        let body = message
                            .strip_prefix("ルールベース診断（AI未使用）\n")
                            .unwrap_or(message);
                        ui.add(
                            egui::Label::new(
                                RichText::new(body).size(16.0).line_height(Some(25.0)),
                            )
                            .wrap()
                            .selectable(true),
                        );
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
    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(32.0);
        ui.label(RichText::new("設定").size(26.0));
        ui.add_space(28.0);
        let label = ui.label(
            RichText::new("設定ファイル")
                .size(14.0)
                .family(egui::FontFamily::Name("ui-regular".into())),
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
        ui.add_space(32.0);
        ui.label(
            RichText::new("処理方式")
                .size(14.0)
                .family(egui::FontFamily::Name("ui-regular".into())),
        );
        ui.label("ローカル診断（ルールベース）");
        ui.label(
            RichText::new("AI接続とコード編集は未対応です。")
                .size(13.0)
                .color(MUTED),
        );
        ui.add_space(28.0);
        if ui.button("チャットに戻る").clicked() {
            self.settings = false;
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        if self.settings && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
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
                                .family(egui::FontFamily::Name("ui-regular".into())),
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
        "CodeBot — LINE Seed JP 400 Preview",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 800.0])
                .with_min_inner_size([700.0, 540.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "japanese".into(),
                egui::FontData::from_static(include_bytes!(
                    "../../../assets/fonts/line-seed-jp/LINESeedJP-Regular.ttf"
                ))
                .into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "japanese".into());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("japanese".into());
            // All UI text, including section labels, uses actual Regular 400.
            let regular_family = fonts.families[&egui::FontFamily::Proportional].clone();
            fonts
                .families
                .insert(egui::FontFamily::Name("ui-regular".into()), regular_family);
            cc.egui_ctx.set_fonts(fonts);
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
            Ok(Box::<App>::default())
        }),
    )
}
