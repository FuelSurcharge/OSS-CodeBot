use codebot_core::{
    download_hf_model, DownloadProgress, LocalConfig, LocalConnection, LocalEngine, LocalPreset,
    LocalServer, ModelDevice,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

enum TransferEvent {
    Progress(DownloadProgress),
    Done(Result<PathBuf, String>),
}
struct Transfer {
    rx: mpsc::Receiver<TransferEvent>,
    cancel: Arc<AtomicBool>,
}
struct Startup {
    rx: mpsc::Receiver<Result<LocalServer, String>>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct LocalModels {
    pub config: LocalConfig,
    pub message: Option<String>,
    hf_token: String,
    transfer: Option<Transfer>,
    startup: Option<Startup>,
    server: Option<LocalServer>,
    progress: Option<DownloadProgress>,
    diagnostics: crate::local_diagnostics::Diagnostics,
    picker: Option<crate::path_picker::PathPicker>,
    cache: crate::model_cache::CachePanel,
}
impl LocalModels {
    pub fn stop(&mut self) {
        if let Some(task) = self.transfer.take() {
            task.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(task) = self.startup.take() {
            task.cancel.store(true, Ordering::Relaxed);
        }
        self.server = None;
        self.progress = None;
    }
    pub fn connection(&mut self) -> Result<LocalConnection, String> {
        if let Some(server) = &mut self.server {
            if server.is_running() {
                return Ok(server.connection());
            }
        }
        Err("ローカルモデルを先に読み込んでください".into())
    }
    pub fn poll(&mut self) -> bool {
        self.cache.poll();
        let mut changed = false;
        let mut done = None;
        if let Some(task) = &self.transfer {
            loop {
                match task.rx.try_recv() {
                    Ok(TransferEvent::Progress(p)) => self.progress = Some(p),
                    Ok(TransferEvent::Done(result)) => {
                        done = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        done = Some(Err("ダウンロード処理が終了しました".into()));
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        if let Some(result) = done {
            match result {
                Ok(path) => {
                    self.config.model_path = path.to_string_lossy().into_owned();
                    self.message = Some("配布元のハッシュを検証したモデルを保存しました。「読み込む」で起動できます。".into());
                    changed = true;
                }
                Err(e) => self.message = Some(e),
            }
            self.transfer = None;
            self.progress = None;
        }
        let done = self.startup.as_ref().and_then(|t| match t.rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("読み込み処理が終了しました".into())),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = done {
            match result {
                Ok(s) => {
                    self.server = Some(s);
                    self.message = Some("ローカルモデルを読み込みました".into());
                }
                Err(e) => self.message = Some(e),
            }
            self.startup = None;
        }
        if self.server.as_mut().is_some_and(|s| !s.is_running()) {
            self.server = None;
            self.message = Some(
                "推論エンジンが終了しました。形式・メモリ容量を確認して再読み込みしてください"
                    .into(),
            );
        }
        changed
    }
    fn download(&mut self, ctx: &egui::Context) {
        let Some(cache) = LocalConfig::default_cache() else {
            self.message = Some("モデル保存先を取得できません".into());
            return;
        };
        let config = self.config.clone();
        let token = if self.hf_token.is_empty() {
            std::env::var("HF_TOKEN").ok()
        } else {
            Some(self.hf_token.clone())
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, rx) = mpsc::sync_channel(8);
        let ctx = ctx.clone();
        self.message = None;
        self.progress = None;
        self.transfer = Some(Transfer { rx, cancel });
        std::thread::spawn(move || {
            let mut last = Instant::now() - Duration::from_secs(1);
            let result =
                download_hf_model(&config, &cache, token.as_deref(), &worker_cancel, |p| {
                    if last.elapsed() >= Duration::from_millis(150) {
                        let _ = tx.try_send(TransferEvent::Progress(p));
                        ctx.request_repaint();
                        last = Instant::now();
                    }
                });
            let _ = tx.send(TransferEvent::Done(result));
            ctx.request_repaint();
        });
    }
    fn start(&mut self, ctx: &egui::Context) {
        if self.config.engine == LocalEngine::Transformers
            && self
                .diagnostics
                .report(&self.config)
                .is_some_and(|report| !report.device_available(self.config.device))
        {
            self.message = Some(
                "選択したデバイスを利用できません。診断結果とPython環境を確認してください。".into(),
            );
            return;
        }
        self.server = None;
        self.message = None;
        let config = self.config.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.startup = Some(Startup { rx, cancel });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = LocalServer::start(&config, &worker_cancel);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    /// Returns true when non-secret settings should be persisted.
    pub fn ui(&mut self, ui: &mut egui::Ui, request_busy: bool) -> bool {
        let mut save = false;
        let busy = request_busy || self.transfer.is_some() || self.startup.is_some();
        ui.label("対応する形式を選び、このPC上でモデルを読み込みます。");
        self.diagnostics.ui(ui, &self.config, !busy);
        let busy = busy || self.diagnostics.busy();
        let available_devices = self
            .diagnostics
            .report(&self.config)
            .map(|r| r.devices.clone());
        ui.add_enabled_ui(!busy, |ui| {
            let old_engine = self.config.engine;
            let engine_label = ui.label("実行方式");
            egui::ComboBox::from_id_salt("local-engine").selected_text(self.config.engine.label()).width(300.0)
                .show_ui(ui, |ui| {
                    for engine in [LocalEngine::LlamaCpp, LocalEngine::Transformers] {
                        ui.selectable_value(&mut self.config.engine, engine, engine.label());
                    }
                }).response.labelled_by(engine_label.id);
            if self.config.engine != old_engine {
                self.stop(); self.config.model_path.clear(); self.config.hf_url.clear(); self.config.sha256.clear(); self.message=None; save=true;
            }
            let native = self.config.engine == LocalEngine::Transformers;
            if native {
                let python = ui.label("Python実行ファイル（Transformers環境）");
                if ui.add(egui::TextEdit::singleline(&mut self.config.python_path).hint_text("仮想環境のpython / python.exe").desired_width(f32::INFINITY)).labelled_by(python.id).changed() {self.server=None;}
                if ui.button("Pythonを選択").clicked() { self.picker=Some(crate::path_picker::PathPicker::new(crate::path_picker::Target::Python,&self.config.python_path)); }
                ui.label("torch・transformers・accelerate・safetensors・huggingface_hubが必要です。");
            }
            ui.add_space(8.0);
            ui.label("モデル名（表示用）");
            let previous=self.config.preset;
            egui::ComboBox::from_id_salt("local-preset").selected_text(self.config.preset.label()).width(300.0).show_ui(ui,|ui| {
                for preset in LocalPreset::ALL {ui.selectable_value(&mut self.config.preset,preset,preset.label());}
            });
            if self.config.preset!=previous {self.server=None;self.config.model_path.clear();self.config.hf_url.clear();self.config.sha256.clear();self.message=None;save=true;}
            if self.config.preset!=LocalPreset::Custom {ui.label("このモデル名の配布先・実行方式への対応は未確認です。対応形式のモデルを指定してください。");}
            let path=ui.label(if native {"公式形式モデルのフォルダ（safetensors・config・tokenizer）"} else {"ローカルモデルのパス（GGUF）"});
            if ui.add(egui::TextEdit::singleline(&mut self.config.model_path).hint_text(if native {"/path/to/model-folder"} else {"/path/to/model.gguf"}).desired_width(f32::INFINITY)).labelled_by(path.id).changed() {self.server=None;}
            if ui.button(if native {"モデルフォルダを選択"} else {"GGUFファイルを選択"}).clicked() { self.picker=Some(crate::path_picker::PathPicker::new(if native {crate::path_picker::Target::ModelFolder} else {crate::path_picker::Target::ModelFile},&self.config.model_path)); }
            ui.add_space(8.0);
            let source=ui.label(if native {"またはHugging FaceのリポジトリURL / ID"} else {"またはHugging FaceのファイルURL"});
            ui.add(egui::TextEdit::singleline(&mut self.config.hf_url).hint_text(if native {"https://huggingface.co/owner/model または owner/model"} else {"https://huggingface.co/owner/repo/resolve/main/model.gguf"}).desired_width(f32::INFINITY)).labelled_by(source.id);
            ui.collapsing("ダウンロード設定",|ui| {
                if native {
                    let revision=ui.label("リビジョン（取得時に正確なコミットへ固定）");
                    ui.add(egui::TextEdit::singleline(&mut self.config.hf_revision).desired_width(f32::INFINITY)).labelled_by(revision.id);
                } else {
                    let sha=ui.label("SHA-256（空欄は配布元から取得）");
                    ui.add(egui::TextEdit::singleline(&mut self.config.sha256).desired_width(f32::INFINITY)).labelled_by(sha.id);
                }
                let token=ui.label("HFトークン（保存せず、未入力ならHF_TOKENを参照）");
                ui.add(egui::TextEdit::singleline(&mut self.hf_token).password(true).desired_width(f32::INFINITY)).labelled_by(token.id);
                ui.horizontal(|ui| {ui.label("取得サイズ上限（GiB）");ui.add(egui::DragValue::new(&mut self.config.max_download_gib).range(1..=4096));});
                ui.label(if native {"必要な設定・tokenizer・safetensors分割重みを取得・検証します。配布元の独自コードは実行しません。"} else {"分割GGUFは00001のURLを指定すると、全パートを同じコミットから取得・検証します。中断した取得は再実行で再開します。"});
                ui.label("公開条件・ライセンスへの同意はHugging Face側で行ってください。");
            });
            if ui.add_enabled(!self.config.hf_url.trim().is_empty(),egui::Button::new("Hugging Faceから取得")).clicked() {self.download(ui.ctx());}
            ui.add_space(12.0);
            if !native {
                let runtime=ui.label("llama-server 実行ファイル");
                ui.add(egui::TextEdit::singleline(&mut self.config.server_path).hint_text("llama-server のインストール先").desired_width(f32::INFINITY)).labelled_by(runtime.id);
                if ui.button("llama-serverを選択").clicked() { self.picker=Some(crate::path_picker::PathPicker::new(crate::path_picker::Target::Server,&self.config.server_path)); }
            }
            ui.collapsing("推論設定",|ui| {
                if native {
                    egui::ComboBox::from_id_salt("model-device").selected_text(self.config.device.as_str()).show_ui(ui,|ui| {
                        for device in [ModelDevice::Auto,ModelDevice::Cpu,ModelDevice::Cuda,ModelDevice::Mps] {
                            let available=available_devices.as_ref().map(|list| if device==ModelDevice::Auto {!list.is_empty()} else {list.contains(&device)}).unwrap_or(matches!(device,ModelDevice::Auto|ModelDevice::Cpu));
                            ui.add_enabled_ui(available,|ui| {ui.selectable_value(&mut self.config.device,device,device.as_str());});
                        }
                    });
                    ui.label("autoはCUDA → MPS → CPUの順で選択します。実際の利用可否はPyTorch環境に依存します。");
                    ui.label("GPUは実行環境の診断後、利用を確認したものを選択できます。");
                }
                ui.horizontal(|ui| {ui.label("コンテキスト長");ui.add(egui::DragValue::new(&mut self.config.context_size).range(512..=131072));});
                ui.horizontal(|ui| {ui.label("CPUスレッド数");ui.add(egui::DragValue::new(&mut self.config.threads).range(1..=256));});
                if !native {ui.horizontal(|ui| {ui.label("GPUレイヤー数（0はCPU）");ui.add(egui::DragValue::new(&mut self.config.gpu_layers).range(0..=1000));});}
                ui.horizontal(|ui| {ui.label("出力上限");ui.add(egui::DragValue::new(&mut self.config.max_output_tokens).range(1..=128000));});
                ui.horizontal(|ui| {ui.label("読み込み待ち（秒）");ui.add(egui::DragValue::new(&mut self.config.startup_timeout_seconds).range(1..=1800));});
                ui.horizontal(|ui| {ui.label("応答待ち（秒）");ui.add(egui::DragValue::new(&mut self.config.request_timeout_seconds).range(1..=300));});
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.config.model_path.trim().is_empty(),egui::Button::new("読み込む")).clicked() {self.start(ui.ctx());}
                if ui.add_enabled(self.server.is_some(),egui::Button::new("停止")).clicked() {self.server=None;self.message=Some("モデルを停止しました".into());}
                if ui.button("ローカル設定を保存").clicked() {save=true;}
            });
        });
        ui.collapsing("初回セットアップ・復旧手順",|ui| {
            ui.label("llama.cpp：公式リリースからOS・CPU/GPUに合うパッケージを展開し、llama-serverを選択してください。");
            ui.hyperlink_to("llama.cpp公式リリース","https://github.com/ggml-org/llama.cpp/releases");
            ui.label("Transformers：専用の仮想環境を作り、そのPythonへリポジトリのrequirements.txtを導入してください。既存環境の一括更新は行いません。");
            ui.label(if cfg!(windows) {"作成：python -m venv .venv-transformers\n導入：.venv-transformers\\Scripts\\python.exe -m pip install -r scripts/transformers/requirements.txt"} else {"作成：python3 -m venv .venv-transformers\n導入：.venv-transformers/bin/python -m pip install -r scripts/transformers/requirements.txt"});
            ui.label("CUDA用PyTorchは公式のOS・CUDA対応表で選択してください。GPUが使えない場合はCPU、依存不整合の場合は新しい仮想環境で再作成してPythonパスを切り替えます。");
            ui.hyperlink_to("PyTorch公式の導入手順","https://pytorch.org/get-started/locally/");
            ui.label("自動インストールは行いません。ダウンロード容量は選ぶPyTorch・モデルにより異なります。");
        });
        if !busy {
            if let Some(picker) = &mut self.picker {
                if let Some(path) = picker.ui(ui.ctx()) {
                    match picker.target {
                        crate::path_picker::Target::ModelFile
                        | crate::path_picker::Target::ModelFolder => self.config.model_path = path,
                        crate::path_picker::Target::Python => self.config.python_path = path,
                        crate::path_picker::Target::Server => self.config.server_path = path,
                        crate::path_picker::Target::PluginManifest => {}
                    }
                    self.server = None;
                }
                if !picker.is_open() {
                    self.picker = None;
                }
            }
        }
        if let Some(task) = &self.transfer {
            if let Some(p) = self.progress {
                let mib = p.received as f64 / 1048576.0;
                if let Some(total) = p.total {
                    ui.add(
                        egui::ProgressBar::new(p.received as f32 / total.max(1) as f32)
                            .show_percentage(),
                    );
                    ui.label(format!("{mib:.1} / {:.1} MiB", total as f64 / 1048576.0));
                } else {
                    ui.label(format!("{mib:.1} MiB取得済み"));
                }
            } else {
                ui.label("配布元と検証情報を確認しています…");
            }
            if ui.button("取得をキャンセル").clicked() {
                task.cancel.store(true, Ordering::Relaxed);
                self.message =
                    Some("キャンセルを要求しました（通信応答待ちの場合は時間がかかります）".into());
            }
        }
        if let Some(task) = &self.startup {
            ui.label("モデルを読み込んでいます…");
            if ui.button("読み込みを中止").clicked() {
                task.cancel.store(true, Ordering::Relaxed);
            }
        }
        if self.server.is_some() {
            ui.label("読み込み済み。このPC上で推論します。");
        }
        ui.collapsing("モデルキャッシュ", |ui| {
            self.cache.ui(ui, !busy && self.server.is_none())
        });
        if let Some(message) = &self.message {
            ui.label(message);
        }
        if self.transfer.is_some() || self.startup.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        save
    }
}
impl Drop for LocalModels {
    fn drop(&mut self) {
        if let Some(t) = &self.transfer {
            t.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(t) = &self.startup {
            t.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_progress_followed_by_completion_is_not_lost() {
        let mut state = LocalModels::default();
        let (tx, rx) = mpsc::sync_channel(8);
        state.transfer = Some(Transfer {
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        tx.send(TransferEvent::Progress(DownloadProgress {
            received: 64,
            total: Some(64),
        }))
        .unwrap();
        tx.send(TransferEvent::Done(Ok(PathBuf::from("downloaded.gguf"))))
            .unwrap();
        assert!(state.poll());
        assert_eq!(state.config.model_path, "downloaded.gguf");
        assert!(state.transfer.is_none());
        assert!(!state.poll());
    }
    #[test]
    fn stopping_requests_cancellation_and_drops_the_receiver() {
        let mut state = LocalModels::default();
        let (tx, rx) = mpsc::sync_channel(8);
        let cancel = Arc::new(AtomicBool::new(false));
        state.transfer = Some(Transfer {
            rx,
            cancel: cancel.clone(),
        });
        state.stop();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(tx
            .send(TransferEvent::Done(Err("cancelled".into())))
            .is_err());
    }
}
