use codebot_core::{execute, Config, Rules};
use eframe::egui;
use std::{path::Path, sync::mpsc};
#[derive(Default)]
struct App {
    input: String,
    output: String,
    error: String,
    config_path: String,
    pending: Option<mpsc::Receiver<Result<String, String>>>,
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(s) => {
                            self.output = s;
                            eprintln!("INFO request completed");
                        }
                        Err(e) => {
                            self.error = e;
                            eprintln!("ERROR request failed");
                        }
                    }
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = "処理スレッドが終了しました".into();
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("OSS CodeBot");
            ui.label("オフライン・ルールベースのログ診断 / AI未使用");
            ui.collapsing("設定", |ui| {
                ui.label("TOML設定パス（空欄はOS標準設定または既定値）");
                ui.text_edit_singleline(&mut self.config_path);
            });
            ui.label("エラーログや質問");
            ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            );
            if ui
                .add_enabled(self.pending.is_none(), egui::Button::new("実行"))
                .clicked()
            {
                self.error.clear();
                self.output.clear();
                let input = self.input.clone();
                let path = self.config_path.clone();
                let (tx, rx) = mpsc::channel();
                self.pending = Some(rx);
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
            if self.pending.is_some() {
                ui.spinner();
                ui.label("処理中…");
            }
            if !self.error.is_empty() {
                ui.colored_label(egui::Color32::RED, &self.error);
            }
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(&self.output);
            });
        });
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "OSS CodeBot",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "japanese".into(),
                egui::FontData::from_static(include_bytes!("../../../assets/NotoSansJP.ttf"))
                    .into(),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("japanese".into());
            }
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::<App>::default())
        }),
    )
}
