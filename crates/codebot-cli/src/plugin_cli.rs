use crate::Args;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[derive(clap::Args)]
#[group(skip)]
pub struct PluginArgs {
    #[arg(long, group = "plugin_action")]
    pub plugin_list: bool,
    #[arg(long, group = "plugin_action")]
    pub plugin_install: Option<PathBuf>,
    #[arg(long, group = "plugin_action")]
    pub plugin_enable: Option<String>,
    #[arg(long, group = "plugin_action")]
    pub plugin_disable: Option<String>,
    #[arg(long, group = "plugin_action")]
    pub plugin_remove: Option<String>,
    #[arg(long,group="plugin_action",requires_all=["input","workspace"])]
    pub plugin_run: Option<String>,
    #[arg(long, requires = "plugin_action")]
    plugin_dir: Option<PathBuf>,
    #[arg(long, requires = "plugin_action")]
    plugin_preview: bool,
    /// Review a returned proposal through the normal apply/command confirmations.
    #[arg(long, requires = "plugin_run", conflicts_with = "yes")]
    plugin_review: bool,
    #[arg(long, requires = "plugin_review")]
    plugin_journal_dir: Option<PathBuf>,
}
impl PluginArgs {
    pub fn active(&self) -> bool {
        self.plugin_list
            || self.plugin_install.is_some()
            || self.plugin_enable.is_some()
            || self.plugin_disable.is_some()
            || self.plugin_remove.is_some()
            || self.plugin_run.is_some()
    }
}
pub(super) fn run(args: &Args) -> Result<(), String> {
    let flags = &args.plugin;
    if flags.plugin_run.is_none()
        && (args.input.is_some() || args.workspace.is_some() || !args.attach.is_empty())
    {
        return Err("入力・作業フォルダ・添付は--plugin-runで指定してください".into());
    }
    if flags.plugin_preview && flags.plugin_install.is_none() && flags.plugin_remove.is_none() {
        return Err("--plugin-previewは導入か削除に指定してください".into());
    }
    if args.yes && flags.plugin_install.is_none() && flags.plugin_remove.is_none() {
        return Err(
            "拡張の--yesは導入か削除の確認だけを省略します。操作提案は個別に確認してください"
                .into(),
        );
    }
    let root = flags
        .plugin_dir
        .clone()
        .or_else(codebot_core::default_plugins_dir)
        .ok_or("拡張の保存先を取得できません")?;
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))
        .map_err(|_| "Ctrl+Cを準備できません")?;
    if let Some(path) = &flags.plugin_install {
        let plan = codebot_core::prepare_plugin_install(path)?;
        println!(
            "導入先：{}\n{}",
            root.display(),
            serde_json::to_string_pretty(plan.manifest()).unwrap()
        );
        println!("要求権限：");
        for permission in &plan.manifest().permissions {
            println!("- {}", permission.label());
        }
        println!("モジュール：{}バイト。SHA-256は整合性の確認で、作者の認証ではありません。導入後は無効の状態です。",plan.module_bytes());
        if !plan.manifest().supported() {
            return Err("直接通信の権限は未対応です。この拡張は導入できません".into());
        }
        if flags.plugin_preview {
            return Ok(());
        }
        if !args.yes && !super::tools_cli::confirm("install", &cancel)? {
            eprintln!("拡張の導入を中止しました");
            return Ok(());
        }
        codebot_core::install_plugin(&root, &plan)?;
        eprintln!("拡張を導入しました。実行前に有効にしてください。");
    } else if let Some(id) = flags
        .plugin_enable
        .as_ref()
        .or(flags.plugin_disable.as_ref())
    {
        codebot_core::set_plugin_enabled(&root, id, flags.plugin_enable.is_some())?;
        eprintln!("拡張の有効状態を変更しました。");
    } else if let Some(id) = &flags.plugin_remove {
        let plan = codebot_core::prepare_plugin_remove(&root, id)?;
        println!(
            "削除対象：{} / {}\nmanifest.json・plugin.wasm・state.jsonの登録データを削除します。",
            plan.root().display(),
            plan.id()
        );
        if flags.plugin_preview {
            return Ok(());
        }
        if !args.yes && !super::tools_cli::confirm("remove", &cancel)? {
            eprintln!("拡張の削除を中止しました");
            return Ok(());
        }
        codebot_core::remove_plugin(&plan)?;
        eprintln!("拡張を削除しました。");
    } else if let Some(id) = &flags.plugin_run {
        let workspace = codebot_core::Workspace::open(args.workspace.as_deref().unwrap())?;
        let mut attachments = Vec::new();
        for path in &args.attach {
            attachments.push(workspace.read(path, args.line_start, args.line_end)?);
        }
        let input = codebot_core::compose_input(args.input.as_deref().unwrap(), &attachments)?;
        let result = codebot_core::run_plugin(
            &root,
            id,
            &input,
            !attachments.is_empty(),
            workspace.root(),
            &cancel,
        )?;
        println!("{}", result.text);
        if let Some(call) = result.proposal {
            println!(
                "拡張からの操作提案：\n{}",
                serde_json::to_string_pretty(&call).unwrap()
            );
            if flags.plugin_review {
                let outcome = super::tools_cli::execute_proposal(
                    &call,
                    workspace.root(),
                    flags.plugin_journal_dir.as_deref(),
                    &cancel,
                )?;
                println!("{}", serde_json::to_string_pretty(&outcome).unwrap());
                if matches!(
                    outcome.status,
                    codebot_core::ToolStatus::Failed | codebot_core::ToolStatus::Cancelled
                ) {
                    return Err(
                        "拡張の操作提案を完了できませんでした。表示した結果を確認してください"
                            .into(),
                    );
                }
            } else {
                eprintln!("提案は実行していません。確認フローで扱う場合は--plugin-reviewを指定してください。");
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&codebot_core::list_plugins(&root)?).unwrap()
        );
    }
    Ok(())
}
