use serde::Deserialize;
mod api;
mod changes;
mod conversation;
mod streaming;
pub use changes::{
    apply_changes, new_change_journal_path, prepare_changes, preview_undo, undo_changes,
    ChangePreview, ChangeProposal, FileChange, LineEdit, PreparedChanges,
};
pub use streaming::{stream_messages, StreamEnd, StreamEvent};
mod local;
mod local_diagnostics;
pub use local_diagnostics::{diagnose_local, DiagnosticItem, LocalDiagnostics};
mod transformers;
mod workspace;
pub use api::{execute_backend, execute_messages, ApiConfig, ApiProtocol, ApiProvider, Backend};
pub use conversation::{validate_messages, ChatMessage, ChatRole};
pub use local::{
    download_hf_model, validate_gguf, DownloadProgress, LocalConfig, LocalConnection, LocalEngine,
    LocalPreset, LocalServer, ModelDevice,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub use workspace::{citation_location, compose_input, AnalysisAction, Attachment, Workspace};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: String,
    pub max_input_chars: usize,
    pub max_context_chars: usize,
    pub api: ApiConfig,
    pub local: LocalConfig,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            provider: "rules".into(),
            max_input_chars: 16000,
            max_context_chars: 64000,
            api: ApiConfig::default(),
            local: LocalConfig::default(),
        }
    }
}
pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "OSS-CodeBot", "OSS-CodeBot")
        .map(|p| p.config_dir().join("config.toml"))
}
impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        // Do not include parser snippets: configuration can contain secrets.
        let c: Self =
            toml::from_str(text).map_err(|_| "設定のTOML形式または項目が不正です".to_string())?;
        if Backend::from_id(&c.provider).is_none() {
            return Err("対応していないプロバイダーです".into());
        }
        if !(1..=1_000_000).contains(&c.max_input_chars) {
            return Err("max_input_chars は1〜1000000です".into());
        }
        if !(1..=4_000_000).contains(&c.max_context_chars) {
            return Err("max_context_chars は1〜4000000です".into());
        }
        Ok(c)
    }
    pub fn load(path: Option<&Path>) -> Result<Self, String> {
        let default = default_config_path();
        let Some(p) = path.or(default.as_deref()) else {
            return Ok(Self::default());
        };
        match fs::read_to_string(p) {
            Ok(s) => Self::parse(&s),
            Err(e) if path.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::default())
            }
            Err(_) => Err("設定ファイルを読み込めません".into()),
        }
    }
}
pub trait Provider: Send + Sync {
    fn respond(&self, input: &str) -> Result<String, String>;
}
pub struct Rules;
impl Provider for Rules {
    fn respond(&self, input: &str) -> Result<String, String> {
        let lower = input.to_lowercase();
        let advice = if lower.contains("permission denied") {
            "対象ファイルの所有者と読み書き権限を確認してください。必要な権限だけを付与してください。"
        } else if lower.contains("not found") || lower.contains("no such file") {
            "ファイルのパス、作業ディレクトリ、コマンドのインストール状況とPATHを確認してください。"
        } else if lower.contains("error") || input.contains("エラー") {
            "最初のエラー、再現コマンド、実行環境を確認し、最小の入力で再現させてください。"
        } else {
            "ログのエラー箇所を入力してください。permission denied / not found などの基本的な原因確認を支援します。"
        };
        Ok(format!("ルールベース診断（AI未使用）\n{advice}"))
    }
}
pub fn execute(config: &Config, provider: &dyn Provider, input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("入力が空です".into());
    }
    if input.chars().count() > config.max_input_chars {
        return Err("入力が文字数の上限を超えています".into());
    }
    provider.respond(input)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation() {
        assert!(execute(&Config::default(), &Rules, "  ").is_err());
        let c = Config {
            max_input_chars: 2,
            ..Config::default()
        };
        assert!(execute(&c, &Rules, "日本").is_ok());
        assert!(execute(&c, &Rules, "日本語").is_err());
    }
    #[test]
    fn settings() {
        assert_eq!(
            Config::parse("max_input_chars = 5")
                .unwrap()
                .max_input_chars,
            5
        );
        for s in [
            "provider = 'unknown'",
            "max_input_chars = 0",
            "secret = 'sensitive'",
            "invalid = [",
        ] {
            let e = Config::parse(s).unwrap_err();
            assert!(!e.contains("sensitive"));
        }
        assert!(Config::load(Some(Path::new("/nonexistent-codebot-config"))).is_err());
    }
    #[test]
    fn diagnostics() {
        assert!(execute(&Config::default(), &Rules, "Permission denied")
            .unwrap()
            .contains("所有者"));
        assert!(execute(&Config::default(), &Rules, "file not found")
            .unwrap()
            .contains("PATH"));
    }
    #[test]
    fn provider_error() {
        struct Failed;
        impl Provider for Failed {
            fn respond(&self, _: &str) -> Result<String, String> {
                Err("接続失敗".into())
            }
        }
        assert_eq!(
            execute(&Config::default(), &Failed, "test").unwrap_err(),
            "接続失敗"
        );
    }
}
