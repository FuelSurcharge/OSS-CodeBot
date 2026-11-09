use codebot_core::{Attachment, Workspace};
use eframe::egui;
use std::path::Path;

#[derive(Default)]
pub struct WorkspacePanel {
    root: String,
    files: Vec<String>,
    selected: String,
    first: usize,
    last: usize,
    preview: Option<Attachment>,
    message: Option<String>,
    action: codebot_core::AnalysisAction,
}
impl WorkspacePanel {
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<(Attachment, codebot_core::AnalysisAction)> {
        let mut attachment = None;
        ui.heading("参照ファイル");
        ui.horizontal(|ui| {
            for action in [
                codebot_core::AnalysisAction::Explain,
                codebot_core::AnalysisAction::Diagnose,
                codebot_core::AnalysisAction::Review,
                codebot_core::AnalysisAction::Fix,
            ] {
                ui.selectable_value(&mut self.action, action, action.label());
            }
        });
        ui.label("作業フォルダのフルパス");
        if ui.text_edit_singleline(&mut self.root).changed() {
            self.preview = None;
            self.files.clear();
            self.selected.clear();
        }
        if ui.button("ファイル一覧を読み込む").clicked() {
            self.preview = None;
            match Workspace::open(Path::new(self.root.trim())).and_then(|w| w.files()) {
                Ok(files) => {
                    self.files = files;
                    self.message = None;
                }
                Err(error) => {
                    self.files.clear();
                    self.message = Some(error);
                }
            }
        }
        ui.label(".gitignore・秘密情報の既定除外を適用します。選択したUTF-8テキストだけを添付できます（64 KiB/ファイル、最大8件）。");
        egui::ScrollArea::vertical()
            .id_salt("workspace-files")
            .max_height(140.0)
            .show(ui, |ui| {
                for file in &self.files {
                    if ui.selectable_label(self.selected == *file, file).clicked() {
                        self.selected = file.clone();
                        self.first = 1;
                        self.last = 0;
                        self.preview = None;
                    }
                }
            });
        ui.label(format!("選択: {}", self.selected));
        ui.horizontal(|ui| {
            ui.label("開始行");
            if ui
                .add(egui::DragValue::new(&mut self.first).range(1..=1_000_000))
                .changed()
            {
                self.preview = None;
            }
            ui.label("終了行（0は末尾）");
            if ui
                .add(egui::DragValue::new(&mut self.last).range(0..=1_000_000))
                .changed()
            {
                self.preview = None;
            }
        });
        if ui
            .add_enabled(
                !self.selected.is_empty(),
                egui::Button::new("選択範囲をプレビュー"),
            )
            .clicked()
        {
            match Workspace::open(Path::new(self.root.trim())).and_then(|w| {
                w.read(
                    Path::new(&self.selected),
                    self.first.max(1),
                    (self.last != 0).then_some(self.last),
                )
            }) {
                Ok(preview) => {
                    self.preview = Some(preview);
                    self.message = None;
                }
                Err(error) => {
                    self.preview = None;
                    self.message = Some(error);
                }
            }
        }
        if let Some(preview) = &self.preview {
            // This is the exact immutable snapshot that the attachment action adds.
            ui.label(format!(
                "{} / {}（{}–{}行）",
                preview.root, preview.path, preview.first_line, preview.last_line
            ));
            egui::ScrollArea::both()
                .id_salt("file-preview")
                .max_height(160.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&preview.content).monospace())
                            .selectable(true),
                    );
                });
            if ui.button("この内容を現在のチャットに添付").clicked() {
                attachment = Some((preview.clone(), self.action));
            }
        }
        if let Some(message) = &self.message {
            ui.colored_label(ui.visuals().error_fg_color, message);
        }
        attachment
    }
    pub fn open_location(&mut self, root: &str, uri: &str) {
        self.root = root.to_owned();
        let result = codebot_core::citation_location(uri).and_then(|(path, line)| {
            self.selected = path.to_string_lossy().into_owned();
            self.first = line;
            self.last = line;
            Workspace::open(Path::new(root))?.read(&path, line, Some(line))
        });
        match result {
            Ok(preview) => {
                self.preview = Some(preview);
                self.message = None;
            }
            Err(error) => {
                self.preview = None;
                self.message = Some(error);
            }
        }
    }
}
