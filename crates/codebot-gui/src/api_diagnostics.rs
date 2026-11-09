use codebot_core::{ApiConfig, Backend, ModelCatalog};
use eframe::egui;
use std::{
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};
enum Event {
    Catalog(ModelCatalog),
    Test,
}
struct Task {
    rx: mpsc::Receiver<Result<Event, String>>,
    cancel: Arc<AtomicBool>,
    identity: u64,
    test: bool,
}
#[derive(Default)]
pub struct ApiDiagnostics {
    task: Option<Task>,
    catalog: Option<ModelCatalog>,
    catalog_identity: Option<u64>,
    result: Option<String>,
    result_identity: Option<u64>,
    confirm: bool,
    consent_identity: Option<u64>,
}
fn identity(backend: Backend, api: &ApiConfig, key: &str, model: bool) -> u64 {
    let mut api = api.resolved(backend);
    if !model {
        api.model.clear();
        api.max_output_tokens = 1;
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    backend.id().hash(&mut hash);
    serde_json::to_string(&api)
        .unwrap_or_default()
        .hash(&mut hash);
    key.hash(&mut hash);
    hash.finish()
}
impl Drop for ApiDiagnostics {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl ApiDiagnostics {
    pub fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.cancel.store(true, Ordering::Relaxed);
        }
        self.confirm = false;
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        backend: Backend,
        api: &mut ApiConfig,
        key: &str,
        enabled: bool,
    ) {
        let endpoint = identity(backend, api, key, false);
        let full = identity(backend, api, key, true);
        if self.consent_identity != Some(full) {
            self.confirm = false;
            self.consent_identity = Some(full);
        }
        if self.catalog_identity != Some(endpoint) {
            self.catalog = None;
        }
        if self.result_identity != Some(full) {
            self.result = None;
        }
        if self
            .task
            .as_ref()
            .is_some_and(|t| t.identity != if t.test { full } else { endpoint })
        {
            self.task
                .take()
                .unwrap()
                .cancel
                .store(true, Ordering::Relaxed);
        }
        if let Some(task) = &self.task {
            let done = match task.rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("接続確認が中断しました".into())),
                _ => None,
            };
            if let Some(done) = done {
                self.task = None;
                self.result_identity = Some(full);
                match done {
                    Ok(Event::Catalog(catalog))=>{self.catalog_identity=Some(endpoint);self.catalog=Some(catalog);}
                    Ok(Event::Test)=>self.result=Some("指定モデルへの生成要求が受理されました。回答の品質を検証したものではありません。".into()),
                    Err(error)=>self.result=Some(error),
                }
            }
        }
        if ui
            .add_enabled(
                enabled && self.task.is_none(),
                egui::Button::new("モデル一覧を取得"),
            )
            .clicked()
        {
            self.start(ui.ctx(), backend, api, key, false, endpoint);
        }
        if let Some(catalog) = &self.catalog {
            ui.add(egui::Label::new(&catalog.note).wrap());
            if !catalog.models.is_empty() {
                egui::ComboBox::from_id_salt("available-api-models")
                    .selected_text(if api.model.is_empty() {
                        "取得したモデルから選択"
                    } else {
                        &api.model
                    })
                    .width(300.0)
                    .show_ui(ui, |ui| {
                        for model in &catalog.models {
                            ui.selectable_value(&mut api.model, model.clone(), model);
                        }
                    });
                if !api.model.is_empty() && !catalog.models.contains(&api.model) {
                    ui.label("手入力のモデルIDは表示された一覧にありません。接続確認で可否を確認できます。");
                }
            }
        }
        let full = identity(backend, api, key, true);
        if self.consent_identity != Some(full) {
            self.confirm = false;
            self.consent_identity = Some(full);
        }
        ui.add_space(8.0);
        let destination = api
            .resolved(backend)
            .validate_url()
            .map(|url| url.to_string())
            .unwrap_or_else(|_| "URLが不正です".into());
        ui.add(egui::Label::new(format!("{} / {}\n{}\n接続確認は固定の診断入力を送信します。出力上限は{}トークンで、API料金が発生する場合があります。",backend.label(),api.model,destination,api.max_output_tokens)).wrap());
        ui.add_enabled(
            enabled && self.task.is_none(),
            egui::Checkbox::new(&mut self.confirm, "生成要求による接続確認を実行する"),
        );
        if ui
            .add_enabled(
                enabled && self.task.is_none() && self.confirm && !api.model.trim().is_empty(),
                egui::Button::new("指定モデルへ送信して接続を確認"),
            )
            .clicked()
        {
            self.start(ui.ctx(), backend, api, key, true, full);
            self.confirm = false;
        }
        if let Some(task) = &self.task {
            ui.horizontal(|ui| {
                ui.label("接続を確認しています…");
                if ui.button("確認を中止").clicked() {
                    task.cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(result) = &self.result {
            ui.add(egui::Label::new(result).wrap());
        }
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        backend: Backend,
        api: &ApiConfig,
        key: &str,
        test: bool,
        identity: u64,
    ) {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        self.task = Some(Task {
            rx,
            cancel,
            identity,
            test,
        });
        self.result = None;
        let api = api.clone();
        let key = key.to_owned();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = if test {
                codebot_core::test_api_model(backend, &api, Some(&key), &worker_cancel)
                    .map(|_| Event::Test)
            } else {
                codebot_core::list_api_models(backend, &api, Some(&key), &worker_cancel)
                    .map(Event::Catalog)
            };
            let _ = tx.send(result.map_err(|error| error.to_string()));
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_destinations_or_models_discard_late_results_and_reset_cost_consent() {
        for endpoint_change in [false, true] {
            let mut state = ApiDiagnostics::default();
            let mut api = Backend::Openai.defaults();
            api.model = "first".into();
            let full = identity(Backend::Openai, &api, "key-a", true);
            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, rx) = mpsc::channel();
            state.task = Some(Task {
                rx,
                cancel: cancel.clone(),
                identity: full,
                test: true,
            });
            state.confirm = true;
            state.consent_identity = Some(full);
            state.result_identity = Some(full);
            state.result = Some("old result".into());
            tx.send(Ok(Event::Test)).unwrap();
            if endpoint_change {
                api.base_url = "https://other.example/v1".into();
            } else {
                api.model = "second".into();
            }
            let ctx = egui::Context::default();
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    state.ui(ui, Backend::Openai, &mut api, "key-a", true);
                });
            });
            assert!(state.task.is_none());
            assert!(cancel.load(Ordering::Relaxed));
            assert!(state.result.is_none());
            assert!(!state.confirm);
        }
    }
}
