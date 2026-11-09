use crate::Args;
use codebot_core::{GithubClient, GithubConfig, GithubKind, GithubWrite};
use std::{
    io::{self, BufRead, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
fn ledger(args: &Args) -> Result<PathBuf, String> {
    args.github_ledger_dir
        .clone()
        .or_else(codebot_core::github_ledger_dir)
        .ok_or_else(|| "送信記録の保存先を取得できません".into())
}
fn print(value: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}
fn approve(yes: bool, word: &str) -> Result<bool, String> {
    if yes {
        return Ok(true);
    }
    eprintln!("表示した対象・内容で実行する場合は {word} と入力してください:");
    let mut text = String::new();
    Read::take(io::stdin().lock(), 32)
        .read_line(&mut text)
        .map_err(|_| "確認入力を読み込めません")?;
    Ok(text.trim() == word)
}
pub(super) fn run(args: &Args) -> Result<(), String> {
    if args.github_preview && args.github_write.is_none() && args.github_branch.is_none() {
        return Err("--github-previewは投稿・ブランチの確認に指定してください".into());
    }
    let config = GithubConfig {
        api_url: args.github_api_url.clone(),
        repository: args.github_repository.clone().unwrap(),
        token_env: args.github_token_env.clone(),
        ..Default::default()
    };
    if args.github_history {
        let records =
            codebot_core::github_post_records(&ledger(args)?).map_err(|e| e.to_string())?;
        let records = records
            .into_iter()
            .filter(|record| {
                record
                    .repository
                    .eq_ignore_ascii_case(config.repository.trim())
            })
            .collect::<Vec<_>>();
        print(serde_json::to_value(records).unwrap());
        return Ok(());
    }
    let client = GithubClient::new(&config, None).map_err(|e| e.to_string())?;
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))
        .map_err(|_| "Ctrl+Cの処理を準備できません")?;
    let kind = if args.github_kind == "pull" {
        GithubKind::Pull
    } else {
        GithubKind::Issue
    };
    if args.github_check {
        print(
            serde_json::to_value(client.repository(&cancel).map_err(|e| e.to_string())?).unwrap(),
        );
    } else if args.github_list {
        print(
            serde_json::to_value(
                client
                    .list(kind, args.github_page, &cancel)
                    .map_err(|e| e.to_string())?,
            )
            .unwrap(),
        );
    } else if let Some(number) = args.github_read {
        print(
            serde_json::to_value(
                client
                    .document(kind, number, &cancel)
                    .map_err(|e| e.to_string())?,
            )
            .unwrap(),
        );
    } else if let Some(number) = args.github_branch {
        let document = client
            .document(GithubKind::Issue, number, &cancel)
            .map_err(|e| e.to_string())?;
        let root = args
            .workspace
            .as_deref()
            .ok_or("--workspaceを指定してください")?;
        let operation = codebot_core::GitOperation::CreateBranch {
            name: document.branch_name(),
        };
        let prepared = codebot_core::prepare_git_operation(root, operation, &cancel)?;
        println!("Issue：{}\n{}", document.url, prepared.summary());
        print(serde_json::to_value(prepared.snapshot()).unwrap());
        if args.github_preview {
            return Ok(());
        }
        if !approve(args.yes, "git")? {
            eprintln!("ブランチ作成を中止しました");
            return Ok(());
        }
        codebot_core::execute_git_operation(&prepared, &cancel)?;
        eprintln!("Issueからの作業ブランチを作成しました。");
    } else if let Some(path) = &args.github_write {
        let mut raw = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "投稿内容のJSONを読めません")?
            .take(512 * 1024 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| "投稿内容のJSONを読めません")?;
        if raw.len() > 512 * 1024 {
            return Err("投稿内容が512 KiBを超えています".into());
        }
        let action: GithubWrite =
            serde_json::from_slice(&raw).map_err(|_| "投稿内容のJSON形式が不正です")?;
        let prepared = client.prepare_write(action).map_err(|e| e.to_string())?;
        println!("投稿先：{} / {}", prepared.api_url(), prepared.repository());
        print(serde_json::to_value(prepared.action()).unwrap());
        println!("送信記録：{}", ledger(args)?.display());
        if args.github_preview {
            return Ok(());
        }
        if !approve(args.yes, "post")? {
            eprintln!("投稿を中止しました");
            return Ok(());
        }
        let receipt = codebot_core::post_github_write(&prepared, &ledger(args)?, &cancel)
            .map_err(|e| e.to_string())?;
        print(serde_json::to_value(receipt).unwrap());
    }
    Ok(())
}
