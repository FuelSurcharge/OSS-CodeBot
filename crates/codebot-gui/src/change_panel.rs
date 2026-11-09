use codebot_core::{ChangePreview, ChangeProposal, PreparedChanges};
use eframe::egui;
use std::{path::Path, sync::mpsc};

enum Event {
    Prepared(PreparedChanges, Vec<ChangePreview>),
    UndoPreview(Vec<ChangePreview>),
    Applied(usize),
    Undone(usize),
}
#[derive(Default)]
pub struct ChangePanel {
    root: String,
    text: String,
    proposal: Option<ChangeProposal>,
    selected: Vec<Vec<bool>>,
    prepared: Option<PreparedChanges>,
    previews: Vec<ChangePreview>,
    undo_mode: bool,
    confirmed: bool,
    journal: String,
    message: Option<String>,
    task: Option<mpsc::Receiver<Result<Event, String>>>,
    operation_root: Option<String>,
    last_location: Option<(String, String)>,
    settings_changed: bool,
}
impl ChangePanel {
    pub fn last_location(&self) -> Option<(String, String)> {
        self.last_location.clone()
    }
    pub fn restore_location(&mut self, root: String, journal: String) {
        if !root.is_empty() && !journal.is_empty() {
            self.root = root.clone();
            self.journal = journal.clone();
            self.last_location = Some((root, journal));
        }
    }
    pub fn take_settings_changed(&mut self) -> bool {
        std::mem::take(&mut self.settings_changed)
    }
    pub fn is_busy(&self) -> bool {
        self.task.is_some()
    }
    pub fn set_proposal(&mut self, root: Option<&str>, text: &str) -> Result<(), String> {
        if self.task.is_some() {
            return Err("ファイル操作を処理中です。完了してから変更案を開いてください".into());
        }
        let proposal = ChangeProposal::parse(text)?;
        self.text = text.to_owned();
        self.root = root.unwrap_or("").to_owned();
        self.selected = proposal
            .changes
            .iter()
            .map(|change| vec![true; change.unit_labels().len()])
            .collect();
        self.proposal = Some(proposal);
        self.reset_preview();
        self.journal = codebot_core::new_change_journal_path()?
            .to_string_lossy()
            .into_owned();
        Ok(())
    }
    fn reset_preview(&mut self) {
        self.prepared = None;
        self.previews.clear();
        self.undo_mode = false;
        self.confirmed = false;
    }
    pub fn preview(&mut self, ctx: &egui::Context) {
        let proposal = self
            .proposal
            .as_ref()
            .ok_or_else(|| "変更案を読み込んでください".to_owned())
            .and_then(|proposal| proposal.select(&self.selected));
        match proposal {
            Ok(proposal) => {
                let root = self.root.clone();
                self.start(ctx, None, move || {
                    let prepared = codebot_core::prepare_changes(Path::new(&root), &proposal)?;
                    let previews = prepared.previews();
                    Ok(Event::Prepared(prepared, previews))
                });
            }
            Err(error) => self.message = Some(error),
        }
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        operation_root: Option<String>,
        job: impl FnOnce() -> Result<Event, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.task = Some(rx);
        if let Some(root) = &operation_root {
            self.last_location = Some((root.clone(), self.journal.clone()));
            self.settings_changed = true;
        }
        self.operation_root = operation_root;
        self.reset_preview();
        self.message = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }
    /// Returns the affected workspace after apply/undo, including partial errors.
    pub fn poll(&mut self) -> Option<String> {
        let result = self.task.as_ref().and_then(|task| match task.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "ファイル操作が中断しました。変更記録を確認してください".into(),
            )),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        let result = result?;
        self.task = None;
        match result {
            Ok(Event::Prepared(prepared, previews)) => {
                self.root = prepared.root().to_string_lossy().into_owned();
                self.prepared = Some(prepared);
                self.previews = previews;
            }
            Ok(Event::UndoPreview(previews)) => {
                self.undo_mode = true;
                self.previews = previews;
            }
            Ok(Event::Applied(count)) => {
                self.message = Some(format!(
                    "{count}ファイルへ適用しました。変更記録から取り消し可能です。"
                ));
            }
            Ok(Event::Undone(count)) => {
                self.last_location = None;
                self.settings_changed = true;
                self.message = Some(format!("{count}ファイルの変更を取り消しました。"));
            }
            Err(error) => self.message = Some(error),
        }
        self.operation_root.take()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, can_mutate: bool) {
        ui.heading("変更の確認と適用");
        let busy = self.task.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            ui.label("作業フォルダのフルパス");
            if ui
                .add(egui::TextEdit::singleline(&mut self.root).desired_width(f32::INFINITY))
                .changed()
            {
                self.reset_preview();
            }
            egui::CollapsingHeader::new("変更案データ（JSON）")
                .id_salt(("proposal-data", self.proposal.is_some()))
                .default_open(self.proposal.is_none())
                .show(ui, |ui| {
                    ui.label("修正提案の回答を貼り付けるか、回答の「変更案を確認」から開きます。");
                    if ui
                        .add(
                            egui::TextEdit::multiline(&mut self.text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(5)
                                .char_limit(1024 * 1024),
                        )
                        .changed()
                    {
                        self.proposal = None;
                        self.reset_preview();
                    }
                    if ui.button("変更案を読み込む").clicked() {
                        let text = self.text.clone();
                        let root = self.root.clone();
                        if let Err(error) = self.set_proposal(Some(&root), &text) {
                            self.message = Some(error);
                        } else {
                            self.preview(ui.ctx());
                        }
                    }
                });
            let mut changed = false;
            if let Some(proposal) = &self.proposal {
                for (file, change) in proposal.changes.iter().enumerate() {
                    ui.label(format!("{}: {}", change.label(), change.path()));
                    ui.horizontal_wrapped(|ui| {
                        for (edit, label) in change.unit_labels().iter().enumerate() {
                            changed |= ui
                                .add_enabled(
                                    self.task.is_none(),
                                    egui::Checkbox::new(&mut self.selected[file][edit], label),
                                )
                                .changed();
                        }
                    });
                }
            }
            if changed {
                self.reset_preview();
            }
            if ui
                .add_enabled(
                    self.proposal.is_some() && self.task.is_none(),
                    egui::Button::new("選択した差分をプレビュー"),
                )
                .clicked()
            {
                self.preview(ui.ctx());
            }
            ui.add_space(8.0);
            ui.label("取り消し用の変更記録（原本を含みます。作業フォルダの外に保存）");
            if ui
                .add_enabled(
                    self.task.is_none(),
                    egui::TextEdit::singleline(&mut self.journal).desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.confirmed = false;
                self.undo_mode = false;
            }
            if ui
                .add_enabled(
                    !self.journal.trim().is_empty() && self.task.is_none(),
                    egui::Button::new("この記録の取り消し差分を確認"),
                )
                .clicked()
            {
                let root = self.root.clone();
                let journal = self.journal.clone();
                self.start(ui.ctx(), None, move || {
                    Ok(Event::UndoPreview(codebot_core::preview_undo(
                        Path::new(&root),
                        Path::new(&journal),
                    )?))
                });
            }
            if let Some((root, journal)) = self.last_location.clone() {
                ui.label(format!("直前の操作: {root}"));
                if ui
                    .add_enabled(
                        self.task.is_none(),
                        egui::Button::new("直前の操作の取り消し差分を確認"),
                    )
                    .clicked()
                {
                    self.root = root.clone();
                    self.journal = journal.clone();
                    self.start(ui.ctx(), None, move || {
                        Ok(Event::UndoPreview(codebot_core::preview_undo(
                            Path::new(&root),
                            Path::new(&journal),
                        )?))
                    });
                }
            }
            for preview in &self.previews {
                ui.horizontal(|ui| {
                    ui.label(&preview.path);
                    if ui.small_button("差分をコピー").clicked() {
                        ui.ctx().copy_text(preview.diff.clone());
                    }
                });
                egui::ScrollArea::both()
                    .id_salt(("change-diff", &preview.path))
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(diff_job(&preview.diff))
                                .selectable(true)
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
            }
            if self.prepared.is_some() || self.undo_mode {
                ui.checkbox(
                    &mut self.confirmed,
                    "表示した差分と作業フォルダを確認しました",
                );
                if ui
                    .add_enabled(
                        can_mutate && self.confirmed && !self.journal.trim().is_empty(),
                        egui::Button::new(if self.undo_mode {
                            "表示した取り消しを実行"
                        } else {
                            "選択した変更を適用"
                        }),
                    )
                    .clicked()
                {
                    let journal = self.journal.clone();
                    if self.undo_mode {
                        let root = self.root.clone();
                        let canonical = Path::new(&root)
                            .canonicalize()
                            .ok()
                            .map(|p| p.to_string_lossy().into_owned());
                        self.start(ui.ctx(), canonical, move || {
                            Ok(Event::Undone(codebot_core::undo_changes(
                                Path::new(&root),
                                Path::new(&journal),
                            )?))
                        });
                    } else if let Some(prepared) = self.prepared.take() {
                        let root = prepared.root().to_string_lossy().into_owned();
                        self.start(ui.ctx(), Some(root), move || {
                            Ok(Event::Applied(codebot_core::apply_changes(
                                &prepared,
                                Path::new(&journal),
                            )?))
                        });
                    }
                }
            }
            if !can_mutate {
                ui.label("AIの応答中はファイルの適用・取り消しを実行できません。");
            }
        });
        if busy {
            ui.spinner();
            ui.label("差分・ファイル操作を処理しています…");
        }
        if let Some(message) = &self.message {
            ui.label(message);
        }
    }
}

fn diff_job(text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for line in text.split_inclusive('\n') {
        let color = if line.starts_with("--- ") || line.starts_with("+++ ") {
            crate::MUTED
        } else if line.starts_with('+') {
            egui::Color32::from_rgb(22, 113, 61)
        } else if line.starts_with('-') {
            egui::Color32::from_rgb(180, 35, 24)
        } else if line.starts_with("@@") {
            crate::ACCENT
        } else {
            crate::TEXT
        };
        job.append(
            line,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::monospace(14.0),
                color,
                ..Default::default()
            },
        );
    }
    job
}
