use codebot_core::{ModelCacheInventory, PreparedCacheDelete};
use eframe::egui;
use std::{sync::mpsc, thread::JoinHandle};
enum Event {
    Inventory(ModelCacheInventory),
    Prepared(PreparedCacheDelete),
    Deleted(ModelCacheInventory),
}
struct Task {
    rx: mpsc::Receiver<Result<Event, String>>,
    join: Option<JoinHandle<()>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
#[derive(Default)]
pub struct CachePanel {
    inventory: Option<ModelCacheInventory>,
    selected: Option<String>,
    plan: Option<PreparedCacheDelete>,
    confirmed: bool,
    message: Option<String>,
    task: Option<Task>,
}
fn size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        return format!("{:.1} GiB", bytes as f64 / 1073741824.);
    }
    format!("{:.1} MiB", bytes as f64 / 1048576.)
}
fn short(id: &str) -> String {
    if id.len() > 52 && id.is_ascii() {
        format!("{}…{}", &id[..32], &id[id.len() - 16..])
    } else {
        id.into()
    }
}
impl CachePanel {
    fn start(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce() -> Result<Event, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let join = std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.task = Some(Task {
            rx,
            join: Some(join),
        });
        self.message = None;
        self.plan = None;
        self.confirmed = false;
    }
    pub fn poll(&mut self) {
        let Some(result) = self
            .task
            .as_ref()
            .and_then(|task| match task.rx.try_recv() {
                Ok(event) => Some(event),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "キャッシュ処理が中断しました。一覧を更新してください".into(),
                )),
                _ => None,
            })
        else {
            return;
        };
        self.task = None;
        match result {
            Ok(Event::Inventory(inventory)) => self.inventory = Some(inventory),
            Ok(Event::Prepared(plan)) => {
                self.plan = Some(plan);
                self.confirmed = false;
            }
            Ok(Event::Deleted(inventory)) => {
                self.inventory = Some(inventory);
                self.selected = None;
                self.message = Some("選択したキャッシュを削除しました。".into());
            }
            Err(error) => self.message = Some(error),
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, can_delete: bool) {
        let Some(root) = codebot_core::LocalConfig::default_cache() else {
            ui.label("キャッシュの保存先を取得できません");
            return;
        };
        let busy = self.task.is_some();
        ui.add(egui::Label::new(root.display().to_string()).wrap());
        if ui
            .add_enabled(!busy, egui::Button::new("キャッシュ一覧を更新"))
            .clicked()
        {
            let root = root.clone();
            self.start(ui.ctx(), move || {
                codebot_core::model_cache_inventory(&root).map(Event::Inventory)
            });
        }
        let mut choose = None;
        if let Some(inventory) = &self.inventory {
            ui.label(format!(
                "モデル・部分ファイル {} / 空き {}",
                size(inventory.total_bytes),
                size(inventory.free_bytes)
            ));
            if inventory.in_use {
                ui.add(egui::Label::new("キャッシュは読み込み・推論・取得・削除で使用中です。停止後に一覧を更新してください。").wrap());
            }
            if inventory.entries.is_empty() {
                ui.label("保存したモデルはありません。");
            }
            egui::ScrollArea::vertical()
                .id_salt("model-cache-list")
                .min_scrolled_height(140.)
                .max_height(180.)
                .show(ui, |ui| {
                    for entry in &inventory.entries {
                        let label = format!(
                            "{} / {} / {} / {}",
                            short(&entry.id),
                            entry.format,
                            size(entry.bytes),
                            if entry.format == "Transformers" && entry.ready {
                                "snapshotあり"
                            } else if entry.ready {
                                "保存済み"
                            } else {
                                "取得途中"
                            }
                        );
                        if ui
                            .add(
                                egui::Button::new(label)
                                    .selected(self.selected.as_ref() == Some(&entry.id))
                                    .frame(false)
                                    .wrap(),
                            )
                            .on_hover_text(&entry.id)
                            .clicked()
                        {
                            choose = Some(entry.id.clone());
                        }
                    }
                });
        }
        if let Some(id) = choose {
            self.selected = Some(id);
            self.plan = None;
            self.confirmed = false;
        }
        let available = can_delete && !busy && self.inventory.as_ref().is_some_and(|i| !i.in_use);
        if ui
            .add_enabled(
                available && self.selected.is_some(),
                egui::Button::new("削除する対象を確認"),
            )
            .clicked()
        {
            let id = self.selected.clone().unwrap();
            self.start(ui.ctx(), move || {
                codebot_core::prepare_cache_delete(&root, &id).map(Event::Prepared)
            });
        }
        if let Some(plan) = &self.plan {
            ui.separator();
            ui.add(
                egui::Label::new(format!(
                    "削除対象：{}\n{}\n{} / {}ファイル",
                    plan.root().display(),
                    plan.entry().id,
                    size(plan.entry().bytes),
                    plan.entry().files
                ))
                .wrap(),
            );
            ui.add(egui::Label::new("この対象のモデル・部分ファイルを削除します。Transformersは同じリポジトリの全revisionが対象です。アプリからは元に戻せません。").wrap());
            egui::ScrollArea::vertical()
                .id_salt("cache-delete-files")
                .max_height(120.)
                .show(ui, |ui| {
                    for name in plan.entry().file_names.iter().take(200) {
                        ui.add(egui::Label::new(name).wrap());
                    }
                    if plan.entry().files > 200 {
                        ui.label(format!(
                            "ほか {}ファイル。全一覧はCLIで確認できます。",
                            plan.entry().files - 200
                        ));
                    }
                });
            ui.add_enabled(
                available,
                crate::checkbox::Checkbox::new(&mut self.confirmed, "削除対象を確認しました"),
            );
            if ui
                .add_enabled(
                    available && self.confirmed,
                    egui::Button::new("確認したキャッシュを削除"),
                )
                .clicked()
            {
                let plan = self.plan.take().unwrap();
                self.start(ui.ctx(), move || {
                    codebot_core::delete_model_cache(&plan)?;
                    codebot_core::model_cache_inventory(plan.root()).map(Event::Deleted)
                });
            }
        }
        if busy {
            ui.label("キャッシュを確認しています…");
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(message) = &self.message {
            ui.add(egui::Label::new(message).wrap());
        }
    }
}
