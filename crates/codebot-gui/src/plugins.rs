use codebot_core::{
    Attachment, PluginEntry, PluginRun, PreparedPluginInstall, PreparedPluginRemove, ToolAction,
    ToolResult, ToolStatus,
};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};
enum Event {
    List(Vec<PluginEntry>, String),
    Install(PreparedPluginInstall),
    Remove(PreparedPluginRemove),
    Run(PluginRun),
}
struct Task {
    rx: Option<mpsc::Receiver<Result<Event, String>>>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    cancellable: bool,
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
pub struct PluginPanel {
    manifest_path: String,
    entries: Vec<PluginEntry>,
    selected: Option<String>,
    install: Option<PreparedPluginInstall>,
    remove: Option<PreparedPluginRemove>,
    confirmed: bool,
    root: String,
    prompt: String,
    files: Vec<Attachment>,
    output: Option<PluginRun>,
    result: Option<ToolResult>,
    kind: u8,
    command: crate::command_panel::CommandPanel,
    changes: crate::change_panel::ChangePanel,
    task: Option<Task>,
    message: Option<String>,
    picker: Option<crate::path_picker::PathPicker>,
}
impl PluginPanel {
    pub fn is_busy(&self) -> bool {
        self.task.is_some() || self.command.is_busy() || self.changes.is_busy()
    }
    pub fn shutdown(&mut self) {
        self.task = None;
        self.command.shutdown();
    }
    pub fn take_location(&mut self) -> Option<(String, String)> {
        if self.changes.take_settings_changed() {
            Some(self.changes.last_location().unwrap_or_default())
        } else {
            None
        }
    }
    pub fn set_initial(&mut self, root: Option<&str>, prompt: &str, files: &[Attachment]) {
        if !self.is_busy() && self.output.is_none() {
            self.root = root.unwrap_or("").into();
            self.prompt = prompt.into();
            self.files = files.to_vec();
        }
    }
    fn clear_run(&mut self) {
        self.output = None;
        self.result = None;
        self.command = Default::default();
        self.changes = Default::default();
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce(&AtomicBool) -> Result<Event, String> + Send + 'static,
    ) {
        self.clear_run();
        self.install = None;
        self.remove = None;
        self.confirmed = false;
        self.message = None;
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let ctx = ctx.clone();
        let join = std::thread::spawn(move || {
            let _ = tx.send(job(&worker_cancel));
            ctx.request_repaint();
        });
        self.task = Some(Task {
            rx: Some(rx),
            cancel,
            join: Some(join),
            cancellable: false,
        });
    }
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<String> {
        let roots = [self.command.poll(), self.changes.poll()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if !roots.is_empty() {
            self.files.clear();
        }
        let ready = self
            .task
            .as_ref()
            .and_then(|task| task.rx.as_ref())
            .and_then(|rx| match rx.try_recv() {
                Ok(event) => Some(event),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("拡張の処理が中断しました".into()))
                }
                _ => None,
            });
        if let Some(ready) = ready {
            let task = self.task.take().unwrap();
            let cancelled = task.cancel.load(Ordering::Relaxed);
            drop(task);
            match ready {
                _ if cancelled => {
                    self.message =
                        Some("拡張の処理を中止しました。操作提案は実行していません。".into())
                }
                Ok(Event::List(entries, message)) => {
                    self.entries = entries;
                    if !self
                        .entries
                        .iter()
                        .any(|e| Some(&e.id) == self.selected.as_ref())
                    {
                        self.selected = None;
                    }
                    self.message = Some(message);
                }
                Ok(Event::Install(plan)) => {
                    self.install = Some(plan);
                    self.confirmed = false;
                }
                Ok(Event::Remove(plan)) => {
                    self.remove = Some(plan);
                    self.confirmed = false;
                }
                Ok(Event::Run(output)) => {
                    if let Some(call) = &output.proposal {
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
                                        self.changes.preview(ctx);
                                    }
                                    Err(error) => self.message = Some(error),
                                }
                            }
                            Err(error) => self.message = Some(error),
                        }
                    }
                    self.output = Some(output);
                }
                Err(error) => self.message = Some(error),
            }
        }
        if self.output.as_ref().is_some_and(|o| o.proposal.is_some()) && self.result.is_none() {
            self.result = if self.kind == 1 {
                self.command.tool_result()
            } else {
                self.changes.tool_result()
            };
        }
        roots
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, can_mutate: bool) {
        ui.heading("拡張機能");
        ui.add(egui::Label::new("入力はこのPC内で処理します。ファイル変更やコマンドは、提案ごとに内容を確認してから実行します。").wrap());
        let Some(store) = codebot_core::default_plugins_dir() else {
            ui.label("拡張の保存先を取得できません");
            return;
        };
        let busy = self.is_busy();
        ui.collapsing("拡張を導入", |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                ui.label("導入する拡張のmanifest.json");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.manifest_path)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.install = None;
                    self.remove = None;
                    self.confirmed = false;
                }
                if ui.button("manifestを選択").clicked() {
                    self.picker = Some(crate::path_picker::PathPicker::new(
                        crate::path_picker::Target::PluginManifest,
                        &self.manifest_path,
                    ));
                }
                if ui.button("要求する権限を確認").clicked() {
                    let path = PathBuf::from(&self.manifest_path);
                    self.start(ui.ctx(), move |_| {
                        codebot_core::prepare_plugin_install(&path).map(Event::Install)
                    });
                }
            });
        });
        if let Some(plan) = &self.install {
            ui.separator();
            let manifest = plan.manifest();
            ui.label(format!(
                "{} / {} / {}",
                manifest.name, manifest.id, manifest.version
            ));
            ui.add(egui::Label::new(&manifest.description).wrap());
            ui.label("要求する権限");
            if manifest.permissions.is_empty() {
                ui.label("渡した入力テキストの処理のみ");
            }
            for permission in &manifest.permissions {
                ui.add(egui::Label::new(format!("・{}", permission.label())).wrap());
            }
            ui.collapsing("モジュールと保存先", |ui| {
                ui.add(
                    egui::Label::new(format!(
                        "SHA-256：{}\n{}バイト\n保存先：{} / {}",
                        manifest.sha256,
                        plan.module_bytes(),
                        store.display(),
                        manifest.id
                    ))
                    .wrap(),
                );
                ui.label("SHA-256は整合性の確認で、作者の認証ではありません。");
            });
            ui.label("導入後は無効の状態です。選択して有効にしてから入力を渡します。");
            ui.add_enabled(
                !busy && manifest.supported(),
                egui::Checkbox::new(
                    &mut self.confirmed,
                    "要求する権限とモジュールを確認しました",
                ),
            );
            if ui
                .add_enabled(
                    !busy && manifest.supported() && self.confirmed,
                    egui::Button::new("確認した拡張を導入"),
                )
                .clicked()
            {
                let plan = self.install.take().unwrap();
                self.selected = Some(plan.manifest().id.clone());
                let store = store.clone();
                self.start(ui.ctx(), move |_| {
                    codebot_core::install_plugin(&store, &plan)?;
                    codebot_core::list_plugins(&store).map(|entries| {
                        Event::List(
                            entries,
                            "拡張を導入しました。有効にすると入力を渡せます。".into(),
                        )
                    })
                });
            }
            if !manifest_supported(&self.install) {
                ui.label("直接の通信権限は未対応です。この拡張は導入できません。");
            }
        }
        ui.separator();
        if ui
            .add_enabled(!busy, egui::Button::new("拡張一覧を更新"))
            .clicked()
        {
            let store = store.clone();
            self.start(ui.ctx(), move |_| {
                codebot_core::list_plugins(&store)
                    .map(|entries| Event::List(entries, String::new()))
            });
        }
        let mut choose = None;
        egui::ScrollArea::vertical()
            .id_salt("plugins-list")
            .min_scrolled_height(70.)
            .max_height(180.)
            .show(ui, |ui| {
                for entry in &self.entries {
                    if ui
                        .selectable_label(
                            self.selected.as_ref() == Some(&entry.id),
                            format!(
                                "{} / {} / {}",
                                entry.name,
                                entry.version,
                                if entry.problem.is_some() {
                                    "実行不可"
                                } else if entry.enabled {
                                    "有効"
                                } else {
                                    "無効"
                                }
                            ),
                        )
                        .clicked()
                    {
                        choose = Some(entry.id.clone());
                    }
                }
            });
        if let Some(id) = choose {
            if !busy {
                self.selected = Some(id);
                self.install = None;
                self.remove = None;
                self.confirmed = false;
                self.clear_run();
            }
        }
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| Some(&e.id) == self.selected.as_ref())
            .cloned()
        {
            ui.label(format!("選択：{} / {}", entry.name, entry.id));
            for permission in &entry.permissions {
                ui.add(egui::Label::new(format!("・{}", permission.label())).wrap());
            }
            if let Some(problem) = &entry.problem {
                ui.add(egui::Label::new(problem).wrap());
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy && entry.problem.is_none(),
                        egui::Button::new(if entry.enabled {
                            "無効にする"
                        } else {
                            "有効にする"
                        }),
                    )
                    .clicked()
                {
                    let store = store.clone();
                    let id = entry.id.clone();
                    self.start(ui.ctx(), move |_| {
                        codebot_core::set_plugin_enabled(&store, &id, !entry.enabled)?;
                        codebot_core::list_plugins(&store).map(|entries| {
                            Event::List(entries, "拡張の有効状態を変更しました。".into())
                        })
                    });
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("削除する拡張を確認"))
                    .clicked()
                {
                    let store = store.clone();
                    let id = entry.id.clone();
                    self.start(ui.ctx(), move |_| {
                        codebot_core::prepare_plugin_remove(&store, &id).map(Event::Remove)
                    });
                }
            });
            ui.add_enabled_ui(!busy, |ui| {
                let mut edited = false;
                ui.label("作業フォルダ");
                if ui
                    .add(egui::TextEdit::singleline(&mut self.root).desired_width(f32::INFINITY))
                    .changed()
                {
                    self.files.clear();
                    edited = true;
                }
                ui.label("拡張に渡す入力");
                edited |= ui
                    .add(
                        egui::TextEdit::multiline(&mut self.prompt)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                if !self.files.is_empty() {
                    ui.label(format!("選択した参照ファイル：{}件", self.files.len()));
                    for file in &self.files {
                        ui.label(format!(
                            "{} / {}–{}行",
                            file.path, file.first_line, file.last_line
                        ));
                    }
                    if ui.button("添付を外す").clicked() {
                        self.files.clear();
                        edited = true;
                    }
                }
                if edited {
                    self.clear_run();
                }
                if ui
                    .add_enabled(
                        entry.enabled && entry.problem.is_none() && can_mutate,
                        egui::Button::new("選択した拡張に入力を渡す"),
                    )
                    .clicked()
                {
                    let result = codebot_core::Workspace::open(Path::new(&self.root)).and_then(
                        |workspace| {
                            codebot_core::compose_input(&self.prompt, &self.files)
                                .map(|input| (workspace.root().to_path_buf(), input))
                        },
                    );
                    match result {
                        Ok((root, input)) => {
                            self.root = root.to_string_lossy().into_owned();
                            let has_files = !self.files.is_empty();
                            let id = entry.id.clone();
                            let store = store.clone();
                            self.start(ui.ctx(), move |cancel| {
                                codebot_core::run_plugin(
                                    &store, &id, &input, has_files, &root, cancel,
                                )
                                .map(Event::Run)
                            });
                            if let Some(task) = &mut self.task {
                                task.cancellable = true;
                            }
                        }
                        Err(error) => self.message = Some(error),
                    }
                }
            });
        }
        if let Some(plan) = &self.remove {
            ui.separator();
            ui.add(egui::Label::new(format!("削除対象：{} / {}\nmanifest.json・plugin.wasm・state.jsonの登録データを削除します。",plan.root().display(),plan.id())).wrap());
            ui.add_enabled(
                !busy,
                egui::Checkbox::new(&mut self.confirmed, "削除する拡張を確認しました"),
            );
            if ui
                .add_enabled(
                    !busy && self.confirmed,
                    egui::Button::new("確認した拡張を削除"),
                )
                .clicked()
            {
                let plan = self.remove.take().unwrap();
                let store = store.clone();
                self.start(ui.ctx(), move |_| {
                    codebot_core::remove_plugin(&plan)?;
                    codebot_core::list_plugins(&store).map(|entries| {
                        Event::List(
                            entries,
                            "拡張を削除しました。元の配布ファイルは残ります。".into(),
                        )
                    })
                });
            }
        }
        if let Some(output) = &self.output {
            ui.separator();
            ui.label("拡張の結果");
            ui.add(egui::Label::new(&output.text).wrap());
            if ui.button("結果をコピー").clicked() {
                ui.ctx().copy_text(output.text.clone());
            }
            if output.proposal.is_some() && self.result.is_none() {
                if self.kind == 1 {
                    let _ = self.command.ui(ui, can_mutate);
                } else {
                    self.changes.ui(ui, can_mutate);
                }
                if ui
                    .add_enabled(!self.is_busy(), egui::Button::new("この操作提案を見送る"))
                    .clicked()
                {
                    self.result = Some(ToolResult {
                        status: ToolStatus::Rejected,
                        summary: "利用者が操作提案を見送りました。実行していません。".into(),
                    });
                }
            }
        }
        if let Some(result) = &self.result {
            ui.add(egui::Label::new(&result.summary).wrap());
        }
        if let Some(task) = &self.task {
            ui.label("拡張を処理しています…");
            if task.cancellable && ui.button("中止を要求").clicked() {
                task.cancel.store(true, Ordering::Relaxed);
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(message) = &self.message {
            ui.add(egui::Label::new(message).wrap());
        }
        if !busy {
            if let Some(picker) = &mut self.picker {
                if let Some(path) = picker.ui(ui.ctx()) {
                    self.manifest_path = path;
                    self.install = None;
                    self.remove = None;
                    self.confirmed = false;
                }
                if !picker.is_open() {
                    self.picker = None;
                }
            }
        }
    }
}
fn manifest_supported(plan: &Option<PreparedPluginInstall>) -> bool {
    plan.as_ref().is_none_or(|p| p.manifest().supported())
}
