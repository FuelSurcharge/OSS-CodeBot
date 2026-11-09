use std::{
    io::{self, BufRead, Read},
    path::PathBuf,
};

#[derive(clap::Args)]
#[group(skip)]
pub struct Args {
    /// List managed models, partial downloads, used bytes and available space.
    #[arg(long,group="cache_action",conflicts_with_all=["input","config","interactive","stream","diagnose_local","list_models","test_api","run_command","git_status","git_operation","github_repository","proposal","undo","attach","action","preview","workspace","yes"])]
    pub cache_list: bool,
    /// Preview and confirm removal of one ID from --cache-list.
    #[arg(long,group="cache_action",conflicts_with_all=["input","config","interactive","stream","diagnose_local","list_models","test_api","run_command","git_status","git_operation","github_repository","proposal","undo","attach","action","preview","workspace"])]
    pub cache_delete: Option<String>,
    #[arg(long, requires = "cache_action")]
    cache_dir: Option<PathBuf>,
    #[arg(long, requires = "cache_delete")]
    cache_preview: bool,
}
pub fn run(args: &Args, yes: bool) -> Result<(), String> {
    let root = args
        .cache_dir
        .clone()
        .or_else(codebot_core::LocalConfig::default_cache)
        .ok_or("キャッシュの保存先を取得できません")?;
    if let Some(id) = &args.cache_delete {
        let plan = codebot_core::prepare_cache_delete(&root, id)?;
        println!(
            "削除対象：{}\n{}",
            plan.root().display(),
            serde_json::to_string_pretty(plan.entry()).unwrap()
        );
        println!("選択したモデル・部分ファイルを削除します。アプリからは元に戻せません。");
        if args.cache_preview {
            return Ok(());
        }
        if !yes {
            eprintln!("表示した対象を削除する場合は delete と入力してください:");
            let mut text = String::new();
            Read::take(io::stdin().lock(), 32)
                .read_line(&mut text)
                .map_err(|_| "確認入力を読み込めません")?;
            if text.trim() != "delete" {
                eprintln!("削除を中止しました");
                return Ok(());
            }
        }
        codebot_core::delete_model_cache(&plan)?;
        eprintln!("選択したキャッシュを削除しました。");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&codebot_core::model_cache_inventory(&root)?).unwrap()
        );
    }
    Ok(())
}
