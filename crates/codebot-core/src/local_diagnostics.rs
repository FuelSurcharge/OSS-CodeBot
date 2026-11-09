use crate::{LocalConfig, LocalEngine, ModelDevice};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct LocalDiagnostics {
    pub items: Vec<DiagnosticItem>,
    pub devices: Vec<ModelDevice>,
    pub cuda_free_bytes: Option<u64>,
    pub cuda_total_bytes: Option<u64>,
    pub mps_recommended_bytes: Option<u64>,
    pub system_total_bytes: Option<u64>,
    pub system_available_bytes: Option<u64>,
}
#[derive(Deserialize)]
struct Probe {
    python: String,
    packages: std::collections::BTreeMap<String, Option<String>>,
    devices: std::collections::BTreeMap<String, bool>,
    imports: std::collections::BTreeMap<String, bool>,
    cuda_free: Option<u64>,
    cuda_total: Option<u64>,
    mps_recommended: Option<u64>,
    system_total: Option<u64>,
    system_available: Option<u64>,
}
const SCRIPT: &str = include_str!("../../../scripts/transformers/diagnose.py");
fn safe_version(value: &str) -> Option<String> {
    (value.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-_".contains(&b)))
    .then(|| value.to_owned())
}
fn compatible(name: &str, version: &str) -> bool {
    let mut parts = version.split('.');
    let Some(major) = parts.next().and_then(|v| v.parse::<u32>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|v| v.parse::<u32>().ok()) else {
        return false;
    };
    match name {
        "Python" => major == 3 && minor >= 10,
        "torch" => major == 2 && minor >= 6,
        "transformers" => major == 4 && minor >= 57 || major == 5,
        "accelerate" => major == 1 && minor >= 10,
        "safetensors" => major == 0 && minor >= 6,
        "huggingface_hub" => major == 0 && minor >= 34 || major == 1,
        _ => false,
    }
}
fn run_probe(
    command: &mut Command,
    cancel: &AtomicBool,
    capture_stderr: bool,
) -> Result<Vec<u8>, &'static str> {
    if cancel.load(Ordering::Relaxed) {
        return Err("診断をキャンセルしました");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if capture_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "実行ファイルが見つからないか、起動できません")?;
    let mut pipes: Vec<Box<dyn Read + Send>> = vec![Box::new(child.stdout.take().unwrap())];
    if let Some(stderr) = child.stderr.take() {
        pipes.push(Box::new(stderr));
    }
    let expected = pipes.len();
    let (tx, rx) = mpsc::channel();
    for pipe in pipes {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut data = Vec::new();
            let result = pipe.take(32769).read_to_end(&mut data).map(|_| data);
            let _ = tx.send(result);
        });
    }
    drop(tx);
    let start = Instant::now();
    let mut output = Vec::new();
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err("診断をキャンセルしました");
        }
        if start.elapsed() > Duration::from_secs(15) {
            break Err("診断がタイムアウトしました。実行ファイルを確認してください");
        }
        if let Ok(data) = rx.try_recv() {
            match data {
                Ok(data) if data.len() <= 32768 => output.push(data),
                _ => break Err("診断出力が上限を超えたか、読み込めません"),
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    break Err("実行に失敗しました。依存ライブラリと実行権限を確認してください");
                }
                while output.len() < expected {
                    match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(Ok(data)) if data.len() <= 32768 => output.push(data),
                        _ => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err("診断出力を読み込めません");
                        }
                    }
                }
                break Ok(output.into_iter().flatten().collect());
            }
            Err(_) => break Err("診断プロセスの状態を確認できません"),
            _ => thread::sleep(Duration::from_millis(25)),
        }
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}
impl LocalDiagnostics {
    fn item(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        self.items.push(DiagnosticItem {
            name: name.into(),
            ok,
            detail: detail.into(),
        });
    }
    pub fn device_available(&self, device: ModelDevice) -> bool {
        device == ModelDevice::Auto && !self.devices.is_empty() || self.devices.contains(&device)
    }
}

/// Runs only the user-selected interpreter (-I) and llama-server (--version).
/// Never downloads, installs packages, opens a model, or executes repository code.
pub fn diagnose_local(config: &LocalConfig, cancel: &AtomicBool) -> LocalDiagnostics {
    let mut report = LocalDiagnostics {
        items: vec![],
        devices: vec![],
        cuda_free_bytes: None,
        cuda_total_bytes: None,
        mps_recommended_bytes: None,
        system_total_bytes: None,
        system_available_bytes: None,
    };
    // All OSes use the same small Python protocol. Lack of Python does not block
    // probing a GGUF header or the native llama.cpp executable.
    let mut python = Command::new(config.python_path.trim());
    python
        .args(["-I", "-c", SCRIPT])
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1");
    match run_probe(&mut python, cancel, false).and_then(|data| {
        serde_json::from_slice::<Probe>(&data).map_err(|_| "Pythonの診断形式が不正です")
    }) {
        Ok(probe) => {
            let version = safe_version(&probe.python);
            report.item(
                "Python",
                version.as_ref().is_some_and(|v| compatible("Python", v)),
                version
                    .map(|v| {
                        if compatible("Python", &v) {
                            v
                        } else {
                            format!("{v}：Python 3.10以降の3.x環境を選択してください")
                        }
                    })
                    .unwrap_or_else(|| "不明なバージョン".into()),
            );
            for name in [
                "torch",
                "transformers",
                "accelerate",
                "safetensors",
                "huggingface_hub",
            ] {
                let version = probe
                    .packages
                    .get(name)
                    .and_then(|v| v.as_deref())
                    .and_then(safe_version);
                let works = if name == "torch" {
                    probe.devices.get("cpu") == Some(&true)
                } else {
                    probe.imports.get(name) == Some(&true)
                };
                report.item(
                    name,
                    version.as_ref().is_some_and(|v|compatible(name,v)) && works,
                    match version {
                        Some(v) if !compatible(name,&v) => format!("{v}：同梱requirements.txtの対応範囲外です。専用環境で再導入してください"),
                        Some(v) if works => v,
                        Some(v) => {
                            format!("{v}：読み込みに失敗。専用仮想環境の依存を確認してください")
                        }
                        None => "未導入。専用仮想環境に導入してください".into(),
                    },
                );
            }
            for device in [ModelDevice::Cpu, ModelDevice::Cuda, ModelDevice::Mps] {
                let available = probe.devices.get(device.as_str()) == Some(&true);
                report.item(
                    device.as_str(),
                    available,
                    if available {
                        "PyTorchでテンソル割り当てを確認"
                    } else {
                        "このPython環境では利用不可"
                    },
                );
                if available {
                    report.devices.push(device);
                }
            }
            report.cuda_free_bytes = probe.cuda_free;
            report.cuda_total_bytes = probe.cuda_total;
            report.mps_recommended_bytes = probe.mps_recommended;
            report.system_total_bytes = probe.system_total;
            report.system_available_bytes = probe.system_available;
        }
        Err(error) => report.item(
            "Python",
            false,
            format!("{error}。Pythonを導入し、専用仮想環境の実行ファイルを選んでください"),
        ),
    }
    if !cancel.load(Ordering::Relaxed) {
        let mut command = Command::new(config.server_path.trim());
        command.arg("--version");
        match run_probe(&mut command, cancel, true) {
            Ok(data) => {
                // Expose only a bounded version token, never raw engine output.
                let version = String::from_utf8_lossy(&data)
                    .split_whitespace()
                    .find(|s| s.chars().any(|c| c.is_ascii_digit()) && safe_version(s).is_some())
                    .map(str::to_owned);
                report.item(
                    "llama-server",
                    true,
                    version
                        .map(|v| format!("起動可能（{v}）"))
                        .unwrap_or_else(|| {
                            "--versionの終了コード0を確認。バージョン文字列は未取得".into()
                        }),
                );
            }
            Err(error) => report.item(
                "llama-server",
                false,
                format!("{error}。llama.cpp公式リリースを展開して実行ファイルを選んでください"),
            ),
        }
    }
    let model = std::path::Path::new(config.model_path.trim());
    let valid = if config.model_path.trim().is_empty() {
        Err("モデルを選んでください".into())
    } else {
        match config.engine {
            LocalEngine::LlamaCpp => crate::validate_gguf(model),
            LocalEngine::Transformers => crate::transformers::validate_folder(model),
        }
    };
    report.item(
        "モデル形式",
        valid.is_ok(),
        valid.err().unwrap_or_else(|| match config.engine {
            LocalEngine::LlamaCpp => {
                "GGUF v2/v3ヘッダーを確認。エンジンのモデル対応は読み込み時に確認".into()
            }
            LocalEngine::Transformers => {
                "config.jsonとsafetensorsを確認。tokenizerとモデル互換性は読み込み時に確認".into()
            }
        }),
    );
    system_memory(&mut report);
    report
}

#[cfg(target_os = "linux")]
fn system_memory(report: &mut LocalDiagnostics) {
    if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
        let value = |label: &str| {
            text.lines().find_map(|line| {
                line.strip_prefix(label)?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(1024)
            })
        };
        report.system_total_bytes = value("MemTotal:");
        report.system_available_bytes = value("MemAvailable:");
    }
}
#[cfg(not(target_os = "linux"))]
fn system_memory(_report: &mut LocalDiagnostics) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_tools_and_invalid_models_return_safe_recovery_steps() {
        let config = LocalConfig {
            python_path: "/codebot-missing-python".into(),
            server_path: "/codebot-missing-server".into(),
            model_path: "/private-test-model".into(),
            ..Default::default()
        };
        let report = diagnose_local(&config, &AtomicBool::new(false));
        assert!(report.items.iter().all(|i| !i.ok));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("/private-test-model"));
        assert!(json.contains("仮想環境"));
        assert!(!report.device_available(ModelDevice::Cuda));
    }
    #[test]
    fn versions_cannot_contain_paths_control_sequences_or_secret_prose() {
        assert!(safe_version("2.6.0+cpu").is_some());
        for raw in ["\u{1b}[0m", "/private/token", "some secret", ""] {
            assert!(safe_version(raw).is_none());
        }
        assert!(!compatible("transformers", "4.56.0"));
        assert!(!compatible("torch", "3.0.0"));
        assert!(compatible("transformers", "5.22.1"));
    }
    #[test]
    fn probes_bound_output_cancel_owned_processes_and_never_echo_errors() {
        let python = if cfg!(windows) { "python" } else { "python3" };
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let setter = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            worker_cancel.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        let error = run_probe(
            Command::new(python).args(["-I", "-c", "import time; time.sleep(20)"]),
            &cancel,
            false,
        )
        .unwrap_err();
        setter.join().unwrap();
        assert!(error.contains("キャンセル"));
        assert!(start.elapsed() < Duration::from_secs(5));
        let error = run_probe(
            Command::new(python).args(["-I", "-c", "print('private-diagnostic-test' * 3000)"]),
            &AtomicBool::new(false),
            false,
        )
        .unwrap_err();
        assert!(!error.contains("private-diagnostic-test"));
        let output = run_probe(
            Command::new(python).args([
                "-I",
                "-c",
                "import sys; print('version: 1234',file=sys.stderr)",
            ]),
            &AtomicBool::new(false),
            true,
        )
        .unwrap();
        assert!(String::from_utf8(output).unwrap().contains("1234"));
    }
}
