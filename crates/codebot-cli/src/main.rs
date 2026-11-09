use clap::Parser;
use codebot_core::{execute_backend, execute_messages, Backend, ChatMessage, Config};
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[derive(Parser)]
#[command(version, about = "OSS CodeBot: local diagnostics and AI APIs")]
struct Args {
    /// Input text; reads stdin when omitted
    input: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    /// Interactive conversation; /clear clears history, /trim removes its oldest turn.
    #[arg(long, conflicts_with = "input")]
    interactive: bool,
    /// Show text as it is generated. Ctrl+C cancels the current request.
    #[arg(long, conflicts_with_all = ["proposal", "undo", "preview"])]
    stream: bool,
    /// Diagnose the selected local runtimes and model without inference/install.
    #[arg(long, conflicts_with_all = ["input", "interactive", "stream", "workspace", "proposal", "undo", "yes", "preview", "action"])]
    diagnose_local: bool,
    /// Working directory for explicitly selected reference files.
    #[arg(long)]
    workspace: Option<PathBuf>,
    /// Relative file path to attach; can be repeated. Shown on stderr before send.
    #[arg(long, requires = "workspace", conflicts_with = "interactive")]
    attach: Vec<PathBuf>,
    #[arg(long, default_value_t = 1, requires = "workspace")]
    line_start: usize,
    /// Last line (inclusive); default is end of file. Applies to each attachment.
    #[arg(long, requires = "workspace")]
    line_end: Option<usize>,
    /// Print the selected input and files without calling an AI backend.
    #[arg(long, conflicts_with = "interactive")]
    preview: bool,
    /// Dedicated analysis action: explain, diagnose, or review.
    #[arg(long, value_parser = ["explain", "diagnose", "review", "fix"], conflicts_with = "interactive")]
    action: Option<String>,
    /// Preview a structured change proposal. No files change without --apply.
    #[arg(long, requires = "workspace", conflicts_with_all = ["input","interactive","attach","action","preview","undo"])]
    proposal: Option<PathBuf>,
    #[arg(long, requires = "proposal")]
    apply: bool,
    /// Recovery journal, outside the working directory; required for --apply.
    #[arg(long, requires = "proposal")]
    journal: Option<PathBuf>,
    /// Preview and confirm undo of a saved change journal.
    #[arg(long, requires = "workspace", conflicts_with_all = ["input","interactive","attach","action","preview","proposal"])]
    undo: Option<PathBuf>,
    /// Explicitly approve apply/undo without an interactive prompt.
    #[arg(long)]
    yes: bool,
    /// Select file:edit indexes (1-based); repeat for multiple edits. Default all.
    #[arg(long, requires = "proposal")]
    select: Vec<String>,
}
fn run() -> Result<(), String> {
    let args = Args::parse();
    if args.proposal.is_some() || args.undo.is_some() {
        return change_command(&args);
    }
    if args.yes {
        return Err("--yesは変更の適用・取り消し時だけ指定できます".into());
    }
    let c = Config::load(args.config.as_deref())?;
    if args.diagnose_local {
        let report = codebot_core::diagnose_local(&c.local, &AtomicBool::new(false));
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|_| "診断結果を出力できません")?
        );
        return Ok(());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    if args.stream {
        let signal_cancel = cancel.clone();
        ctrlc::set_handler(move || signal_cancel.store(true, Ordering::Relaxed))
            .map_err(|_| "Ctrl+Cの処理を準備できません")?;
    }
    if args.interactive {
        return interactive(&c, args.stream, &cancel);
    }
    let input = match args.input {
        Some(s) => s,
        None if args.preview || args.action.is_some() => String::new(),
        None => {
            let mut s = String::new();
            io::stdin()
                .take((c.max_input_chars as u64 * 4) + 1)
                .read_to_string(&mut s)
                .map_err(|_| "標準入力を読み込めません")?;
            s
        }
    };
    let backend = Backend::from_id(&c.provider).ok_or("対応していないプロバイダーです")?;
    let input = if let Some(action) = args.action {
        let action = match action.as_str() {
            "diagnose" => codebot_core::AnalysisAction::Diagnose,
            "review" => codebot_core::AnalysisAction::Review,
            "fix" => codebot_core::AnalysisAction::Fix,
            _ => codebot_core::AnalysisAction::Explain,
        };
        format!("{}\n\n{}", action.prompt(), input)
    } else {
        input
    };
    if args.attach.len() > 8 {
        return Err("添付は8ファイルまでです".into());
    }
    let attachments = if let Some(root) = &args.workspace {
        let workspace = codebot_core::Workspace::open(root)?;
        args.attach
            .iter()
            .map(|path| workspace.read(path, args.line_start, args.line_end))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![]
    };
    for attachment in &attachments {
        eprintln!(
            "参照: {} / {}（{}–{}行）",
            attachment.root, attachment.path, attachment.first_line, attachment.last_line
        );
    }
    let input = codebot_core::compose_input(&input, &attachments)?;
    if args.preview {
        println!("{input}");
        return Ok(());
    }
    if args.stream {
        if stream_answer(&c, backend, &[ChatMessage::user(input)], &cancel)?.is_none() {
            return Ok(());
        }
    } else {
        let output = execute_backend(&c, backend, &c.api, None, &input)?;
        println!("{output}");
    }
    eprintln!("INFO request completed");
    Ok(())
}

fn approve(yes: bool) -> Result<bool, String> {
    if yes {
        return Ok(true);
    }
    eprintln!(
        "表示した差分をこの作業フォルダへ反映します。実行する場合は apply と入力してください:"
    );
    let mut line = String::new();
    Read::take(io::stdin().lock(), 32)
        .read_line(&mut line)
        .map_err(|_| "確認入力を読み込めません")?;
    Ok(line.trim() == "apply")
}
fn change_command(args: &Args) -> Result<(), String> {
    let root = args
        .workspace
        .as_deref()
        .ok_or("--workspaceを指定してください")?;
    if let Some(journal) = &args.undo {
        for preview in codebot_core::preview_undo(root, journal)? {
            println!("取り消し: {}\n{}", preview.path, preview.diff);
        }
        if !approve(args.yes)? {
            eprintln!("取り消しを中止しました");
            return Ok(());
        }
        let count = codebot_core::undo_changes(root, journal)?;
        eprintln!("{count}ファイルの変更を取り消しました");
        return Ok(());
    }
    let path = args
        .proposal
        .as_ref()
        .ok_or("変更案のパスを指定してください")?;
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|_| "変更案を開けません")?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|_| "変更案を読み込めません")?;
    let proposal = codebot_core::ChangeProposal::parse(&text)?;
    let mut selected: Vec<Vec<bool>> = proposal
        .changes
        .iter()
        .map(|change| vec![args.select.is_empty(); change.unit_labels().len()])
        .collect();
    for selection in &args.select {
        let (file, edit) = selection
            .split_once(':')
            .ok_or("--selectはファイル番号:変更箇所番号で指定してください")?;
        let file = file
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or("選択番号は1から指定してください")?;
        let edit = edit
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or("選択番号は1から指定してください")?;
        *selected
            .get_mut(file)
            .and_then(|edits| edits.get_mut(edit))
            .ok_or("選択番号が変更案の範囲外です")? = true;
    }
    for (file, change) in proposal.changes.iter().enumerate() {
        for (edit, label) in change.unit_labels().iter().enumerate() {
            eprintln!(
                "選択 {}:{} {} {} {label}",
                file + 1,
                edit + 1,
                change.label(),
                change.path()
            );
        }
    }
    let prepared = codebot_core::prepare_changes(root, &proposal.select(&selected)?)?;
    eprintln!("作業フォルダ: {}", prepared.root().display());
    for preview in prepared.previews() {
        println!("{}\n{}", preview.path, preview.diff);
    }
    if !args.apply {
        return Ok(());
    }
    let journal = args
        .journal
        .as_ref()
        .ok_or("--applyには作業フォルダ外の--journal保存先が必要です")?;
    eprintln!("取り消し用の原本を保存: {}", journal.display());
    if !approve(args.yes)? {
        eprintln!("適用を中止しました");
        return Ok(());
    }
    let count = codebot_core::apply_changes(&prepared, journal)?;
    eprintln!(
        "{count}ファイルに適用しました。取り消し記録: {}",
        journal.display()
    );
    Ok(())
}

fn stream_answer(
    config: &Config,
    backend: Backend,
    messages: &[ChatMessage],
    cancel: &AtomicBool,
) -> Result<Option<String>, String> {
    let mut answer = String::new();
    let mut end = codebot_core::StreamEnd::Failed("処理が中断されました".into());
    let mut stdout = io::stdout().lock();
    let mut output_error = false;
    codebot_core::stream_messages(
        config,
        backend,
        &config.api,
        None,
        messages,
        cancel,
        |event| match event {
            codebot_core::StreamEvent::Delta(text) => {
                if stdout
                    .write_all(text.as_bytes())
                    .and_then(|_| stdout.flush())
                    .is_err()
                {
                    output_error = true;
                    cancel.store(true, Ordering::Relaxed);
                }
                answer.push_str(&text);
            }
            codebot_core::StreamEvent::Finished(status) => end = status,
        },
    );
    let _ = writeln!(stdout);
    if output_error {
        return Err("標準出力へ書き込めません".into());
    }
    match end {
        codebot_core::StreamEnd::Complete => Ok(Some(answer)),
        codebot_core::StreamEnd::Cancelled => {
            eprintln!("生成を停止しました。途中の回答は会話履歴へ送信しません。");
            Ok(None)
        }
        codebot_core::StreamEnd::Failed(error) => Err(error),
    }
}
fn interactive(config: &Config, streaming: bool, cancel: &AtomicBool) -> Result<(), String> {
    let backend = Backend::from_id(&config.provider).ok_or("対応していないプロバイダーです")?;
    eprintln!("対話モード: 各入力と会話履歴を選択した処理先へ送ります。/clear: 履歴消去、/trim: 最古の1往復を削除、/quit: 終了。文字数予算: {}（モデルのトークン上限とは別）", config.max_context_chars);
    let mut history = Vec::new();
    // Keep stdin off the main thread so Ctrl+C also exits an idle streaming CLI.
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let limit = config.max_input_chars as u64 * 4 + 2;
    std::thread::spawn(move || {
        let mut reader = io::stdin().lock();
        loop {
            let mut line = String::new();
            let result = Read::take(&mut reader, limit).read_line(&mut line);
            let done = matches!(result, Ok(0) | Err(_));
            let line = match result {
                Ok(0) => Ok(None),
                Ok(count) if count as u64 == limit && !line.ends_with('\n') => {
                    let _ = reader.skip_until(b'\n');
                    Err("入力が文字数の上限を超えています".to_owned())
                }
                Ok(_) => Ok(Some(line)),
                Err(_) => Err("対話入力を読み込めません".to_owned()),
            };
            if tx.send(line).is_err() || done {
                break;
            }
        }
    });
    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(Ok(Some(line))) => line,
            Ok(Err(error)) => {
                eprintln!("ERROR {error}");
                continue;
            }
            Ok(Ok(None)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
        };
        let input = line.trim();
        match input {
            "/quit" | "/exit" => break,
            "/clear" => {
                history.clear();
                eprintln!("履歴を消去しました");
                continue;
            }
            "/trim" => {
                if history.len() >= 2 {
                    history.drain(..2);
                }
                eprintln!("最古の1往復を削除しました");
                continue;
            }
            "" => continue,
            _ => {}
        }
        let mut messages = history.clone();
        messages.push(ChatMessage::user(input));
        let answer = if streaming {
            stream_answer(config, backend, &messages, cancel)
        } else {
            execute_messages(config, backend, &config.api, None, &messages).map(Some)
        };
        match answer {
            Ok(Some(answer)) => {
                if !streaming {
                    println!("{answer}");
                }
                messages.push(ChatMessage::assistant(answer));
                if backend != Backend::Rules {
                    history = messages;
                }
                eprintln!("INFO request completed");
            }
            Err(error) => eprintln!("ERROR {error}"),
            Ok(None) => {}
        }
        cancel.store(false, Ordering::Relaxed);
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ERROR {e}");
            ExitCode::FAILURE
        }
    }
}
