use codebot_core::{GitOperation, GitSnapshot, PreparedGitOperation};
use eframe::egui;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};
enum Event {
    Snapshot(Result<GitSnapshot, String>),
    Prepared(Result<PreparedGitOperation, String>),
    Executed(Result<(), String>, Result<GitSnapshot, String>),
}
struct Task {
    rx: Option<mpsc::Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    root: Option<String>,
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
pub struct GitPanel {
    root: String,
    snapshot: Option<GitSnapshot>,
    selected: BTreeSet<String>,
    message: Option<String>,
    plan: Option<PreparedGitOperation>,
    confirmed: bool,
    new_branch: String,
    commit_message: String,
    remote: String,
    push_branch: String,
    task: Option<Task>,
    focus_review: bool,
    focus_status: bool,
}
impl GitPanel {
    pub fn prepare_issue_branch(
        &mut self,
        ctx: &egui::Context,
        root: Option<&str>,
        branch: &str,
        source: &str,
    ) -> Result<(), String> {
        if self.is_busy() {
            return Err("Git処理の完了後にブランチを準備してください".into());
        }
        let root = root.ok_or("先にファイル画面から作業フォルダを選択してください")?;
        self.root = root.to_owned();
        self.new_branch = branch.to_owned();
        self.start(
            ctx,
            Some(GitOperation::CreateBranch {
                name: branch.into(),
            }),
        );
        self.message = Some(format!("参照Issue：{source}"));
        Ok(())
    }
    pub fn is_busy(&self) -> bool {
        self.task.is_some()
    }
    pub fn shutdown(&mut self) {
        self.task = None;
    }
    pub fn set_initial_root(&mut self, root: Option<&str>) {
        if self.root.is_empty() {
            self.root = root.unwrap_or("").into();
        }
    }
    fn reset(&mut self) {
        self.plan = None;
        self.confirmed = false;
    }
    fn load(&mut self, snapshot: GitSnapshot) {
        let previous = self
            .snapshot
            .as_ref()
            .and_then(|s| s.branch.as_deref())
            .unwrap_or("");
        if self.push_branch.is_empty() || self.push_branch == previous {
            self.push_branch = snapshot.branch.clone().unwrap_or_default();
        }
        if !snapshot.remotes.iter().any(|r| r.name == self.remote) {
            self.remote = snapshot
                .remotes
                .first()
                .map(|r| r.name.clone())
                .unwrap_or_default();
        }
        self.selected
            .retain(|path| snapshot.files.iter().any(|f| &f.path == path));
        self.root = snapshot.root.to_string_lossy().into_owned();
        self.snapshot = Some(snapshot);
    }
    fn start(&mut self, ctx: &egui::Context, operation: Option<GitOperation>) {
        let root = PathBuf::from(&self.root);
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let ctx = ctx.clone();
        let join = std::thread::spawn(move || {
            let event = match operation {
                Some(op) => Event::Prepared(codebot_core::prepare_git_operation(
                    &root,
                    op,
                    &worker_cancel,
                )),
                None => Event::Snapshot(codebot_core::git_snapshot(&root, &worker_cancel)),
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
        self.task = Some(Task {
            rx: Some(rx),
            cancel,
            join: Some(join),
            root: None,
        });
        self.reset();
        self.message = None;
    }
    fn execute(&mut self, ctx: &egui::Context) {
        let Some(plan) = self.plan.take() else {
            return;
        };
        let root = plan.snapshot().root.clone();
        let operation_root = root.to_string_lossy().into_owned();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let ctx = ctx.clone();
        let join = std::thread::spawn(move || {
            let result = codebot_core::execute_git_operation(&plan, &worker_cancel);
            let snapshot = codebot_core::git_snapshot(&root, &worker_cancel);
            let _ = tx.send(Event::Executed(result, snapshot));
            ctx.request_repaint();
        });
        self.task = Some(Task {
            rx: Some(rx),
            cancel,
            join: Some(join),
            root: Some(operation_root),
        });
        self.confirmed = false;
        self.message = None;
    }
    pub fn poll(&mut self) -> Option<String> {
        let event = self
            .task
            .as_ref()
            .and_then(|task| task.rx.as_ref())
            .and_then(|rx| match rx.try_recv() {
                Ok(e) => Some(e),
                Err(mpsc::TryRecvError::Disconnected) => Some(Event::Snapshot(Err(
                    "Git処理が中断しました。状態を再読み込みしてください".into(),
                ))),
                _ => None,
            })?;
        let task = self.task.take().unwrap();
        let root = task.root.clone();
        let cancelled = task.cancel.load(Ordering::Relaxed);
        drop(task);
        match event {
            Event::Snapshot(Ok(snapshot)) if !cancelled => self.load(snapshot),
            Event::Prepared(Ok(plan)) if !cancelled => {
                self.snapshot = Some(plan.snapshot().clone());
                self.plan = Some(plan);
                self.confirmed = false;
                self.focus_review = true;
            }
            Event::Executed(result, snapshot) => {
                self.focus_status = true;
                self.message = Some(match result {
                    Ok(()) => "Git操作を完了しました。".into(),
                    Err(error) => error,
                });
                match snapshot {
                    Ok(snapshot) => self.load(snapshot),
                    Err(error) => {
                        self.snapshot = None;
                        self.message = Some(format!(
                            "{} 状態の再読み込みが必要です：{error}",
                            self.message.as_deref().unwrap_or("")
                        ));
                    }
                }
            }
            Event::Snapshot(Err(error)) | Event::Prepared(Err(error)) => {
                self.message = Some(error);
                self.reset();
            }
            _ => self.message = Some("Git操作の確認を中止しました".into()),
        }
        root
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, can_mutate: bool) {
        if std::mem::take(&mut self.focus_status) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        ui.heading("Git");
        if let Some(message) = &self.message {
            ui.add(egui::Label::new(message).wrap());
        }
        let busy = self.is_busy();
        ui.add_enabled_ui(!busy, |ui| {
            ui.label("リポジトリの作業フォルダ");
            if ui
                .add(egui::TextEdit::singleline(&mut self.root).desired_width(f32::INFINITY))
                .changed()
            {
                self.snapshot = None;
                self.selected.clear();
                self.message = None;
                self.reset();
            }
            if ui.button("状態と差分を読み込む").clicked() {
                self.start(ui.ctx(), None);
            }
        });
        let mut operation = None;
        let mut changed = false;
        ui.add_enabled_ui(!busy && self.task.is_none(), |ui| {
            if let Some(snapshot) = &self.snapshot {
                let branch = snapshot.branch.as_deref().unwrap_or("detached HEAD");
                ui.label(format!("ブランチ：{branch}"));
                ui.label(if snapshot.unborn {
                    "初回コミット前".into()
                } else {
                    format!("HEAD：{}", snapshot.head.as_deref().unwrap_or(""))
                });
                if snapshot.files.iter().any(|f| f.conflict) {
                    ui.label("競合があります。作業ファイルを解決してからコミットしてください。");
                }
                egui::ScrollArea::vertical()
                    .id_salt("git-file-list")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for file in &snapshot.files {
                            let mut selected = self.selected.contains(&file.path);
                            let label = format!(
                                "{} [{}{}] {}",
                                if file.conflict {
                                    "競合"
                                } else if file.untracked {
                                    "未追跡"
                                } else {
                                    "変更"
                                },
                                file.index,
                                file.working,
                                serde_json::to_string(&file.path).unwrap_or_default()
                            );
                            if ui
                                .add(crate::checkbox::Checkbox::new(&mut selected, label))
                                .changed()
                            {
                                if selected {
                                    self.selected.insert(file.path.clone());
                                } else {
                                    self.selected.remove(&file.path);
                                }
                                changed = true;
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.selected.is_empty(),
                            egui::Button::new("選択したファイルをステージ確認"),
                        )
                        .clicked()
                    {
                        operation = Some(GitOperation::Stage {
                            paths: self.selected.iter().cloned().collect(),
                        });
                    }
                    if ui
                        .add_enabled(
                            !self.selected.is_empty(),
                            egui::Button::new("ステージ解除を確認"),
                        )
                        .clicked()
                    {
                        operation = Some(GitOperation::Unstage {
                            paths: self.selected.iter().cloned().collect(),
                        });
                    }
                });
                ui.collapsing("未ステージの差分", |ui| {
                    diff(ui, "unstaged", &snapshot.unstaged_diff)
                });
                ui.collapsing("ステージ済みの差分", |ui| {
                    diff(ui, "staged", &snapshot.staged_diff)
                });
                ui.add_space(8.0);
                ui.label("新しいブランチ名");
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.new_branch)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                if ui.button("ブランチの作成を確認").clicked() {
                    operation = Some(GitOperation::CreateBranch {
                        name: self.new_branch.clone(),
                    });
                }
                ui.add_space(8.0);
                ui.label("コミット文");
                changed |= ui
                    .add(
                        egui::TextEdit::multiline(&mut self.commit_message)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                ui.horizontal(|ui| {
                    if ui.button("コミット文を提案").clicked() {
                        self.commit_message = snapshot.suggest_message();
                        changed = true;
                    }
                    if ui.button("コミットする差分を確認").clicked() {
                        operation = Some(GitOperation::Commit {
                            message: self.commit_message.clone(),
                        });
                    }
                });
                ui.add_space(8.0);
                ui.label("push先");
                egui::ComboBox::from_id_salt("git-remote")
                    .selected_text(if self.remote.is_empty() {
                        "リモート未設定"
                    } else {
                        &self.remote
                    })
                    .show_ui(ui, |ui| {
                        for remote in &snapshot.remotes {
                            changed |= ui
                                .selectable_value(
                                    &mut self.remote,
                                    remote.name.clone(),
                                    format!("{}：{}", remote.name, remote.display_url),
                                )
                                .changed();
                        }
                    });
                ui.label("送信先ブランチ名");
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.push_branch)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                if ui
                    .add_enabled(
                        !self.remote.is_empty(),
                        egui::Button::new("pushするコミットと差分を確認"),
                    )
                    .clicked()
                {
                    operation = Some(GitOperation::Push {
                        remote: self.remote.clone(),
                        branch: self.push_branch.clone(),
                    });
                }
            }
        });
        if changed {
            self.reset();
        }
        if let Some(op) = operation {
            self.start(ui.ctx(), Some(op));
        }
        if let Some(plan) = &self.plan {
            if std::mem::take(&mut self.focus_review) {
                ui.scroll_to_cursor(Some(egui::Align::Min));
            }
            ui.separator();
            ui.label("実行前の確認");
            if matches!(plan.operation(), GitOperation::Push { .. }) {
                diff(ui, "push-review", plan.summary());
            } else {
                ui.add(egui::Label::new(plan.summary()).wrap());
            }
            match plan.operation() {
                GitOperation::Stage { paths } | GitOperation::Unstage { paths } => {
                    ui.label("対象ファイル");
                    for path in paths {
                        ui.label(path);
                    }
                }
                GitOperation::Commit { message } => {
                    ui.add(egui::Label::new(format!("コミット文：\n{message}")).wrap());
                }
                _ => {}
            }
            ui.label(format!("作業場所：{}", plan.snapshot().root.display()));
            ui.label(format!(
                "対象ブランチ：{}",
                plan.snapshot().branch.as_deref().unwrap_or("detached HEAD")
            ));
            if matches!(plan.operation(), GitOperation::Commit { .. }) {
                diff(ui, "commit-review", &plan.snapshot().staged_diff);
            }
            ui.add_enabled(
                can_mutate,
                crate::checkbox::Checkbox::new(
                    &mut self.confirmed,
                    "対象・ブランチ・差分を確認しました",
                ),
            );
            if ui
                .add_enabled(
                    can_mutate && self.confirmed && self.task.is_none(),
                    egui::Button::new("確認したGit操作を実行"),
                )
                .clicked()
            {
                self.execute(ui.ctx());
            }
        }
        if let Some(task) = &self.task {
            ui.horizontal(|ui| {
                ui.label("Gitを処理中…");
                if ui.button("Git処理を中止").clicked() {
                    task.cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.add(egui::Label::new("Gitのフック・外部diff・署名を実行しません。LFS等のフィルターを持つファイルは外部Gitでステージしてください。").wrap());
    }
}
pub(crate) fn diff(ui: &mut egui::Ui, id: &str, text: &str) {
    if text.is_empty() {
        ui.label("差分はありません。");
        return;
    }
    if ui.button("差分をコピー").clicked() {
        ui.ctx().copy_text(text.to_owned());
    }
    let lines = text.lines().collect::<Vec<_>>();
    egui::ScrollArea::both()
        .id_salt(("git-diff", id))
        .max_height(220.0)
        .min_scrolled_height(160.0)
        .auto_shrink([false, true])
        .show_rows(
            ui,
            ui.text_style_height(&egui::TextStyle::Monospace),
            lines.len(),
            |ui, range| {
                for index in range {
                    let line = lines[index];
                    let end = line
                        .char_indices()
                        .nth(8000)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    let color = if line.starts_with('+') && !line.starts_with("+++") {
                        egui::Color32::from_rgb(0, 110, 45)
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        egui::Color32::from_rgb(175, 30, 30)
                    } else if line.starts_with("@@") {
                        egui::Color32::from_rgb(0, 95, 184)
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(if end < line.len() {
                                format!(
                                    "{} …（長い行の表示を省略。コピーで全体を確認）",
                                    &line[..end]
                                )
                            } else {
                                line.to_owned()
                            })
                            .monospace()
                            .color(color),
                        )
                        .selectable(true),
                    );
                }
            },
        );
}
