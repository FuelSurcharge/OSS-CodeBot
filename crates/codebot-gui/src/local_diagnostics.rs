use codebot_core::{LocalConfig, LocalDiagnostics};
use eframe::egui;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

type Source = (String, String, String, codebot_core::LocalEngine);
fn source(config: &LocalConfig) -> Source {
    (
        config.python_path.clone(),
        config.server_path.clone(),
        config.model_path.clone(),
        config.engine,
    )
}
#[derive(Default)]
pub struct Diagnostics {
    task: Option<(mpsc::Receiver<LocalDiagnostics>, Arc<AtomicBool>)>,
    report: Option<LocalDiagnostics>,
    source: Option<Source>,
}
impl Drop for Diagnostics {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.task {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Diagnostics {
    pub fn report(&self, config: &LocalConfig) -> Option<&LocalDiagnostics> {
        (self.source.as_ref() == Some(&source(config)) && self.task.is_none())
            .then_some(self.report.as_ref())
            .flatten()
    }
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, config: &LocalConfig, enabled: bool) {
        if let Some((rx, _)) = &self.task {
            match rx.try_recv() {
                Ok(report) => {
                    self.report = Some(report);
                    self.task = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.report = None;
                    self.source = None;
                    self.task = None;
                }
                _ => {}
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    enabled && self.task.is_none(),
                    egui::Button::new("実行環境を診断"),
                )
                .clicked()
            {
                let (tx, rx) = mpsc::channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = cancel.clone();
                self.task = Some((rx, cancel));
                self.source = Some(source(config));
                self.report = None;
                let config = config.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let report = codebot_core::diagnose_local(&config, &worker_cancel);
                    let _ = tx.send(report);
                    ctx.request_repaint();
                });
            }
            if let Some((_, cancel)) = &self.task {
                ui.label("診断中…");
                if ui.button("診断を中止").clicked() {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        ui.label(
            "選択した実行ファイルを起動して確認します。モデル取得・パッケージ導入は行いません。",
        );
        if let Some(report) = &self.report {
            if self.source.as_ref() != Some(&source(config)) {
                ui.label("パスや形式が変更されています。再診断してください。");
            }
            egui::CollapsingHeader::new("診断結果").default_open(true).show(ui,|ui| {
                for item in &report.items {ui.label(format!("{} {}：{}",if item.ok {"OK"} else {"確認が必要"},item.name,item.detail));}
                let gib=|bytes:u64|format!("{:.1} GiB",bytes as f64/(1024.0*1024.0*1024.0));
                for (label,bytes) in [("システムメモリ合計",report.system_total_bytes),("システムメモリ利用可能",report.system_available_bytes),("CUDAメモリ合計",report.cuda_total_bytes),("CUDAメモリ空き",report.cuda_free_bytes),("MPS推奨メモリ",report.mps_recommended_bytes)] {
                    if let Some(bytes)=bytes {ui.label(format!("{label}：{}",gib(bytes)));}
                }
                ui.label("未取得のメモリ値は表示しません。モデルの読み込み成功や空き容量の確保を保証するものではありません。");
            });
        }
        if self.task.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
}
